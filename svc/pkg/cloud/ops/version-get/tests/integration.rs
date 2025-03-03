use chirp_worker::prelude::*;
use proto::backend::{self, pkg::*};

#[worker_test]
async fn empty(ctx: TestCtx) {
	let region_res = op!([ctx] faker_region {}).await.unwrap();

	let game_res = op!([ctx] faker_game {
		..Default::default()
	})
	.await
	.unwrap();

	let build_res = op!([ctx] faker_build {
		game_id: game_res.game_id,
		image: faker::build::Image::MmLobbyAutoReady as i32,
	})
	.await
	.unwrap();

	let version_publish_res = op!([ctx] cloud_version_publish {
			game_id: game_res.game_id,
			display_name: "0.0.1".into(),
			config: Some(backend::cloud::VersionConfig {
				cdn: None,
				matchmaker: Some(backend::matchmaker::VersionConfig {
					lobby_groups: vec![
						backend::matchmaker::LobbyGroup {
							name_id: "test".into(),

							regions: vec![
								backend::matchmaker::lobby_group::Region {
									region_id: region_res.region_id,
									tier_name_id: util_mm::test::TIER_NAME_ID.to_owned(),
									idle_lobbies: None,
								},
							],
							max_players_normal: 8,
							max_players_direct: 10,
							max_players_party: 12,

							runtime: Some(backend::matchmaker::lobby_runtime::Docker {
								build_id: build_res.build_id,
								args: Vec::new(),
								env_vars: vec![
									backend::matchmaker::lobby_runtime::EnvVar {
										key: "HELLO".into(),
										value: "world".into(),
									},
								],
								network_mode: backend::matchmaker::lobby_runtime::NetworkMode::Bridge as i32,
								ports: vec![
									backend::matchmaker::lobby_runtime::Port {
										label: "1234".into(),
										target_port: Some(1234),
										port_range: None,
										proxy_protocol: backend::matchmaker::lobby_runtime::ProxyProtocol::Https as i32,
										proxy_kind: backend::matchmaker::lobby_runtime::ProxyKind::GameGuard as i32,
									},
								],
							}.into()),
						},
					],
					captcha: None,
				}),
				kv: None,
				identity: Some(backend::identity::VersionConfig {
					custom_display_names: vec![backend::identity::CustomDisplayName {
						display_name: "Guest".to_string(),
					}],
					custom_avatars: Vec::new(),
				}),
				module: Some(backend::module::GameVersionConfig {
					dependencies: Vec::new(),
				}),
			})
	})
	.await
	.unwrap();
	let version_id = version_publish_res.version_id.as_ref().unwrap().as_uuid();

	let res = op!([ctx] cloud_version_get {
		version_ids: vec![version_id.into()],
	})
	.await
	.unwrap();

	let version_res = res.versions.first().expect("version not returned");
	let version_res_config = version_res.config.as_ref().unwrap();
	version_res_config
		.matchmaker
		.as_ref()
		.expect("mm config not set");
}
