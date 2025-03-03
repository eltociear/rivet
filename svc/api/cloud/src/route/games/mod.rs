use std::str::FromStr;

use api_helper::{
	anchor::{WatchIndexQuery, WatchResponse},
	ctx::Ctx,
};
use proto::backend::{self, pkg::*};
use rivet_claims::ClaimsDecode;
use rivet_cloud_server::models;
use rivet_convert::{ApiInto, ApiTryFrom, ApiTryInto};
use rivet_operation::prelude::*;

use crate::{auth::Auth, convert, fetch};

pub mod avatars;
pub mod builds;
pub mod cdn;
pub mod matchmaker;
pub mod namespaces;
pub mod tokens;
pub mod versions;

const MAX_LOGO_UPLOAD_SIZE: i64 = util::file_size::megabytes(5) as i64;
const MAX_BANNER_UPLOAD_SIZE: i64 = util::file_size::megabytes(10) as i64;

// MARK: GET /games
pub async fn list(
	ctx: Ctx<Auth>,
	watch_index: WatchIndexQuery,
) -> GlobalResult<models::GetGamesResponse> {
	let accessible_games = ctx.auth().accessible_games(ctx.op_ctx()).await?;

	// Wait for an update if needed
	let update_ts = if let Some(anchor) = watch_index.to_consumer()? {
		// Error if a cloud token tries to watch this endpoint, game update
		// messages for teams aren't implemented
		if let Some(user_id) = accessible_games.user_id {
			let game_update_sub = tail_anchor!([ctx, anchor] user_dev::msg::game_update(user_id));

			util::macros::select_with_timeout!({
				event = game_update_sub => {
					let event = event?;

					event.msg_ts()
				}
			})
		} else {
			panic_with!(
				API_UNAUTHORIZED,
				reason = "Cloud token cannot watch `/games`"
			);
		}
	} else {
		Default::default()
	};
	let update_ts = update_ts.unwrap_or_else(util::timestamp::now);

	let games = fetch::game_summary_fetch(ctx.op_ctx(), accessible_games.game_ids).await?;
	let groups = fetch::group::summaries(
		ctx.op_ctx(),
		accessible_games.user_id,
		accessible_games.team_ids,
	)
	.await?;

	Ok(models::GetGamesResponse {
		games,
		groups,
		watch: convert::watch_response(WatchResponse::new(update_ts + 1)),
	})
}

// MARK: POST /games
pub async fn create(
	ctx: Ctx<Auth>,
	body: models::CreateGameRequest,
) -> GlobalResult<models::CreateGameResponse> {
	let developer_group_id = Uuid::from_str(body.developer_group_id.as_str())?;

	let user_id = ctx.auth().claims()?.as_user().ok();

	ctx.auth()
		.check_team_write(ctx.op_ctx(), developer_group_id)
		.await?;

	// Create game
	let game_id = {
		let create_game_res = op!([ctx] game_create {
			name_id: body.name_id.clone(),
			display_name: body.display_name.clone(),
			url: format!("https://{}.{}", body.name_id, util::env::domain_cdn()),
			tags: Vec::new(),
			description: String::new(),
			developer_team_id: Some(developer_group_id.into()),
			creator_user_id: user_id.as_ref().map(|x| x.user_id.into()),
		})
		.await?;

		op!([ctx] cloud_game_config_create {
			game_id: create_game_res.game_id,
		})
		.await?;

		internal_unwrap!(create_game_res.game_id).as_uuid()
	};

	// Publish default version
	let default_version_id = {
		let default_version_config =
			gen_default_version_config(&ctx, game_id, &body.display_name).await?;
		let publish_res = op!([ctx] cloud_version_publish {
			game_id: Some(game_id.into()),
			display_name: "0.0.1".into(),
			config: Some(default_version_config),
			creator_user_id: user_id.as_ref().map(|x| x.user_id.into()),
		})
		.await?;

		internal_unwrap!(publish_res.version_id).as_uuid()
	};

	// Create default namespaces
	for (ns_name, ns_name_id) in &[("Production", "prod"), ("Staging", "staging")] {
		let create_res = op!([ctx] game_namespace_create {
			game_id: Some(game_id.into()),
			display_name: ns_name.to_string(),
			version_id: Some(default_version_id.into()),
			name_id: ns_name_id.to_string(),
		})
		.await?;

		op!([ctx] cloud_namespace_create {
			namespace_id: create_res.namespace_id,
			creator_user_id: user_id.as_ref().map(|x| x.user_id.into()),
		})
		.await?;
	}

	Ok(models::CreateGameResponse {
		game_id: game_id.to_string(),
	})
}

async fn gen_default_version_config(
	ctx: &Ctx<Auth>,
	game_id: Uuid,
	display_name: &str,
) -> GlobalResult<backend::cloud::VersionConfig> {
	let list_regions_res = op!([ctx] region_list {
		..Default::default()
	})
	.await?;

	let (site_id, build_id) = tokio::try_join!(
		gen_default_site(ctx, game_id, display_name),
		gen_default_build(ctx, game_id, display_name),
	)?;

	Ok(backend::cloud::VersionConfig {
		cdn: Some(backend::cdn::VersionConfig {
			site_id: Some(site_id.into()),
			routes: Vec::new(),
		}),
		matchmaker: Some(backend::matchmaker::VersionConfig {
			lobby_groups: vec![backend::matchmaker::LobbyGroup {
				name_id: "default".into(),

				regions: list_regions_res
					.region_ids
					.iter()
					.map(|region_id| backend::matchmaker::lobby_group::Region {
						region_id: Some(*region_id),
						tier_name_id: util_mm::defaults::TIER_NAME_ID.to_owned(),
						idle_lobbies: None,
					})
					.collect(),
				max_players_normal: 32,
				max_players_direct: 32,
				max_players_party: 32,

				runtime: Some(
					backend::matchmaker::lobby_runtime::Docker {
						build_id: Some(build_id.into()),
						args: Vec::new(),
						env_vars: vec![backend::matchmaker::lobby_runtime::EnvVar {
							key: "PORT".into(),
							value: "80".into(),
						}],
						network_mode: backend::matchmaker::lobby_runtime::NetworkMode::Bridge
							as i32,
						ports: vec![backend::matchmaker::lobby_runtime::Port {
							label: "default".into(),
							target_port: Some(80),
							port_range: None,
							proxy_protocol: backend::matchmaker::lobby_runtime::ProxyProtocol::Https
								as i32,
							proxy_kind: backend::matchmaker::lobby_runtime::ProxyKind::GameGuard
								as i32,
						}],
					}
					.into(),
				),
			}],
			captcha: None,
		}),
		kv: Some(backend::kv::VersionConfig {}),
		identity: Some(backend::identity::VersionConfig {
			custom_display_names: Vec::new(),
			custom_avatars: Vec::new(),
		}),
		module: Some(backend::module::GameVersionConfig {
			dependencies: Vec::new(),
		}),
	})
}

async fn gen_default_site(
	ctx: &Ctx<Auth>,
	game_id: Uuid,
	display_name: &str,
) -> GlobalResult<Uuid> {
	struct GameFile {
		path: String,
		content_type: String,
		contents: String,
	}

	impl GameFile {
		pub fn new(path: impl ToString, content_type: impl ToString, contents: String) -> GameFile {
			GameFile {
				path: path.to_string(),
				content_type: content_type.to_string(),
				contents,
			}
		}
	}

	let files = vec![
		GameFile::new(
			"index.html",
			"text/html",
			include_str!("../../../default-site/index.html")
				.replace("__DISPLAY_NAME__", display_name),
		),
		GameFile::new(
			"styles.css",
			"text/css",
			include_str!("../../../default-site/styles.css").to_string(),
		),
		GameFile::new(
			"img/frog.svg",
			"image/svg+xml",
			include_str!("../../../default-site/img/frog.svg").to_string(),
		),
		GameFile::new(
			"img/logo.svg",
			"image/svg+xml",
			include_str!("../../../default-site/img/logo.svg").to_string(),
		),
	];

	// Prepare the files
	let prepare_files = files
		.iter()
		.map(|f| backend::upload::PrepareFile {
			path: f.path.clone(),
			mime: Some(f.content_type.clone()),
			content_length: f.contents.len() as u64,
			..Default::default()
		})
		.collect::<Vec<_>>();
	let create_res = op!([ctx] cdn_site_create {
		game_id: Some(game_id.into()),
		display_name: display_name.to_owned(),
		files: prepare_files,
	})
	.await?;
	let site_id = internal_unwrap!(create_res.site_id).as_uuid();
	let upload_id = internal_unwrap!(create_res.upload_id).as_uuid();

	// TODO: Parallelize (RIV-1113)
	// Publish the files
	for req in &create_res.presigned_requests {
		if let Some(file) = files.iter().find(|f| f.path == req.path) {
			let url = &req.url;
			tracing::info!(%url, "uploading file");
			let res = reqwest::Client::new()
				.put(url)
				.header(reqwest::header::CONTENT_TYPE, &file.content_type)
				.header(reqwest::header::CONTENT_LENGTH, file.contents.len())
				.body(file.contents.clone())
				.send()
				.await?;
			if res.status().is_success() {
				tracing::info!("successfully uploaded");
			} else {
				tracing::warn!(status = ?res.status(), "failure uploading");
			}
		} else {
			tracing::warn!(
				?req,
				"failed to find default game file to upload for prepared request"
			);
			continue;
		}
	}

	// Complete the upload
	op!([ctx] upload_complete {
		upload_id: Some(upload_id.into()),
		bucket: Some("bucket-cdn".into()),
	})
	.await?;

	Ok(site_id)
}

async fn gen_default_build(
	ctx: &Ctx<Auth>,
	game_id: Uuid,
	display_name: &str,
) -> GlobalResult<Uuid> {
	let create_res = op!([ctx] build_create {
		game_id: Some(game_id.into()),
		display_name: display_name.to_owned(),
		default_build_kind: Some("game-multiplayer".into()),
		..Default::default()
	})
	.await?;
	let build_id = internal_unwrap!(create_res.build_id).as_uuid();

	Ok(build_id)
}

// MARK: GET /games/{}
pub async fn get(
	ctx: Ctx<Auth>,
	game_id: Uuid,
	watch_index: WatchIndexQuery,
) -> GlobalResult<models::GetGameByIdResponse> {
	ctx.auth().check_game_read(ctx.op_ctx(), game_id).await?;

	// Wait for an update if needed
	let update_ts = if let Some(anchor) = watch_index.to_consumer()? {
		let game_update_sub = tail_anchor!([ctx, anchor] game::msg::update(game_id));

		util::macros::select_with_timeout!({
			event = game_update_sub => {
				let event = event?;

				event.msg_ts()
			}
		})
	} else {
		Default::default()
	};
	let update_ts = update_ts.unwrap_or_else(util::timestamp::now);

	let summary =
		internal_unwrap_owned!(fetch::game_summary_fetch_one(ctx.op_ctx(), game_id).await?);

	let (ns_list_res, version_list_res) = tokio::try_join!(
		op!([ctx] game_namespace_list {
			game_ids: vec![game_id.into()],
		}),
		op!([ctx] game_version_list {
			game_ids: vec![game_id.into()],
		}),
	)?;
	let ns_list = internal_unwrap_owned!(ns_list_res.games.first());
	let version_list = internal_unwrap_owned!(version_list_res.games.first());

	// Fetch cloud and game data for the associated namespaces
	let (ns_get_res, cloud_ns_get_res, version_get_res, cloud_version_get_res) = tokio::try_join!(
		op!([ctx] game_namespace_get {
			namespace_ids: ns_list.namespace_ids.clone(),
		}),
		op!([ctx] cloud_namespace_get {
			namespace_ids: ns_list.namespace_ids.clone(),
		}),
		op!([ctx] game_version_get {
			version_ids: version_list.version_ids.clone(),
		}),
		op!([ctx] cloud_version_get {
			version_ids: version_list.version_ids.clone(),
		}),
	)?;

	let mut namespaces = Vec::new();
	for cloud_ns in &cloud_ns_get_res.namespaces {
		// Find associated namespace data, if exists
		let ns = if let Some(ns) = ns_get_res
			.namespaces
			.iter()
			.find(|x| x.namespace_id == cloud_ns.namespace_id)
		{
			ns
		} else {
			tracing::warn!(namespace_id = ?cloud_ns.namespace_id, "missing game namespace");
			continue;
		};

		namespaces.push(models::NamespaceSummary::try_from(ns.clone())?);
	}
	namespaces.sort_by(|a, b| a.display_name.cmp(&b.display_name));

	let mut versions = Vec::new();
	for cloud_version in &cloud_version_get_res.versions {
		// Find associated version data, if exists
		let version = if let Some(version) = version_get_res
			.versions
			.iter()
			.find(|x| x.version_id == cloud_version.version_id)
		{
			version
		} else {
			tracing::warn!(version_id = ?cloud_version.version_id, "missing game version");
			continue;
		};

		versions.push(models::VersionSummary::try_from(version.clone())?);
	}
	versions.sort_by_key(|v| v.create_ts);

	let regions = fetch::region_summary_fetch_all(ctx.op_ctx()).await?;

	Ok(models::GetGameByIdResponse {
		game: models::GameFull {
			game_id: summary.game_id,
			create_ts: summary.create_ts,
			name_id: summary.name_id,
			display_name: summary.display_name,
			developer_group_id: summary.developer_group_id,
			total_player_count: summary.total_player_count,
			logo_url: summary.logo_url,
			banner_url: summary.banner_url,

			namespaces,
			versions,
			available_regions: regions,
		},
		watch: convert::watch_response(WatchResponse::new(update_ts + 1)),
	})
}

// MARK: POST /games/validate
pub async fn validate(
	ctx: Ctx<Auth>,
	body: models::ValidateGameRequest,
) -> GlobalResult<models::ValidateGameResponse> {
	let res = op!([ctx] game_validate {
		name_id: body.name_id,
		display_name: body.display_name
	})
	.await?;

	Ok(models::ValidateGameResponse {
		errors: res
			.errors
			.into_iter()
			.map(ApiInto::api_into)
			.collect::<Vec<_>>(),
	})
}

// MARK: POST /games/logo-upload/prepare
pub async fn prepare_logo_upload(
	ctx: Ctx<Auth>,
	game_id: Uuid,
	body: models::GameLogoUploadPrepareRequest,
) -> GlobalResult<models::GameLogoUploadPrepareResponse> {
	ctx.auth().check_game_write(ctx.op_ctx(), game_id).await?;

	let user_id = ctx.auth().claims()?.as_user().ok().map(|x| x.user_id);

	assert_with!(
		body.content_length >= 0,
		CLOUD_INVALID_CONFIG,
		error = "`content_length` out of bounds"
	);
	assert_with!(body.content_length < MAX_LOGO_UPLOAD_SIZE, UPLOAD_TOO_LARGE);

	let ext = if body.path.ends_with(".png") {
		"png"
	} else if body.path.ends_with(".jpg") || body.path.ends_with(".jpeg") {
		"jpeg"
	} else {
		internal_panic!("invalid file type (allowed: .png, .jpg)");
	};

	// Create the upload
	let upload_prepare_res = op!([ctx] upload_prepare {
		bucket: "bucket-game-logo".to_owned(),
		files: vec![
			backend::upload::PrepareFile {
				path: format!("logo.{ext}"),
				mime: Some(format!("image/{ext}")),
				content_length: body.content_length.try_into()?,
				nsfw_score_threshold: Some(util_nsfw::score_thresholds::GAME_LOGO),
				..Default::default()
			},
		],
		user_id: user_id.map(Into::into),
	})
	.await?;

	let upload_id = internal_unwrap!(upload_prepare_res.upload_id).as_uuid();
	let presigned_request = internal_unwrap_owned!(upload_prepare_res.presigned_requests.first());

	Ok(models::GameLogoUploadPrepareResponse {
		upload_id: upload_id.to_string(),
		presigned_request: presigned_request.clone().try_into()?,
	})
}

// MARK: POST /games/{}/logo-upload/{}/prepare
pub async fn complete_logo_upload(
	ctx: Ctx<Auth>,
	game_id: Uuid,
	upload_id: Uuid,
	_body: models::GameLogoUploadCompleteRequest,
) -> GlobalResult<models::GameLogoUploadCompleteResponse> {
	ctx.auth().check_game_write(ctx.op_ctx(), game_id).await?;

	op!([ctx] game_logo_upload_complete {
		game_id: Some(game_id.into()),
		upload_id: Some(upload_id.into()),
	})
	.await?;

	Ok(models::GameLogoUploadCompleteResponse {})
}

// MARK: POST /games/banner-upload/prepare
pub async fn prepare_banner_upload(
	ctx: Ctx<Auth>,
	game_id: Uuid,
	body: models::GameBannerUploadPrepareRequest,
) -> GlobalResult<models::GameBannerUploadPrepareResponse> {
	ctx.auth().check_game_write(ctx.op_ctx(), game_id).await?;

	let user_id = ctx.auth().claims()?.as_user().ok().map(|x| x.user_id);

	assert_with!(
		body.content_length >= 0,
		CLOUD_INVALID_CONFIG,
		error = "`content_length` out of bounds"
	);
	assert_with!(
		body.content_length < MAX_BANNER_UPLOAD_SIZE,
		UPLOAD_TOO_LARGE
	);

	let ext = if body.path.ends_with(".png") {
		"png"
	} else if body.path.ends_with(".jpg") || body.path.ends_with(".jpeg") {
		"jpeg"
	} else {
		internal_panic!("invalid file type (allowed: .png, .jpg)");
	};

	// Create the upload
	let upload_prepare_res = op!([ctx] upload_prepare {
		bucket: "bucket-game-banner".to_owned(),
		files: vec![
			backend::upload::PrepareFile {
				path: format!("banner.{ext}"),
				mime: Some(format!("image/{ext}")),
				content_length: body.content_length.try_into()?,
				nsfw_score_threshold: Some(util_nsfw::score_thresholds::GAME_BANNER),
				..Default::default()
			},
		],
		user_id: user_id.map(Into::into),
	})
	.await?;

	let upload_id = internal_unwrap!(upload_prepare_res.upload_id).as_uuid();
	let presigned_request = internal_unwrap_owned!(upload_prepare_res.presigned_requests.first());

	Ok(models::GameBannerUploadPrepareResponse {
		upload_id: upload_id.to_string(),
		presigned_request: presigned_request.clone().try_into()?,
	})
}

// MARK: POST /games/{}/banner-upload/{}/complete
pub async fn complete_banner_upload(
	ctx: Ctx<Auth>,
	game_id: Uuid,
	upload_id: Uuid,
	_body: models::GameBannerUploadCompleteRequest,
) -> GlobalResult<models::GameBannerUploadCompleteResponse> {
	ctx.auth().check_game_write(ctx.op_ctx(), game_id).await?;

	op!([ctx] game_banner_upload_complete {
		game_id: Some(game_id.into()),
		upload_id: Some(upload_id.into()),
	})
	.await?;

	Ok(models::GameBannerUploadCompleteResponse {})
}
