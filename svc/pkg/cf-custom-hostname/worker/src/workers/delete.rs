use chirp_worker::prelude::*;
use proto::backend::pkg::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CloudflareError {
	errors: Vec<CloudflareErrorEntry>,
}

#[derive(Debug, Deserialize)]
struct CloudflareErrorEntry {
	code: usize,
	// message: String,
}

#[worker(name = "cf-custom-hostname-delete")]
async fn worker(
	ctx: OperationContext<cf_custom_hostname::msg::delete::Message>,
) -> GlobalResult<()> {
	let namespace_id = internal_unwrap!(ctx.namespace_id).as_uuid();
	let crdb = ctx.crdb("db-cf-custom-hostname").await?;

	let custom_hostnames_res = op!([ctx] cf_custom_hostname_resolve_hostname {
		hostnames: vec![ctx.hostname.clone()],
	})
	.await?;
	let custom_hostname =
		if let Some(custom_hostname) = custom_hostnames_res.custom_hostnames.first() {
			custom_hostname
		} else {
			tracing::info!(%namespace_id, hostname=%ctx.hostname, "custom hostname does not exist");

			msg!([ctx] cf_custom_hostname::msg::delete_complete(namespace_id, &ctx.hostname) {
				namespace_id: ctx.namespace_id,
				hostname: ctx.hostname.clone(),
			})
			.await?;

			return Ok(());
		};
	let identifier = internal_unwrap_owned!(custom_hostname.identifier).as_uuid();

	let res = reqwest::Client::new()
		.delete(format!(
			"https://api.cloudflare.com/client/v4/zones/{zone_id}/custom_hostnames/{identifier}",
			zone_id = util::env::cloudflare::zone::game::id(),
			identifier = identifier,
		))
		.header(
			reqwest::header::AUTHORIZATION,
			format!("Bearer {}", util::env::cloudflare::auth_token()),
		)
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let text = res.text().await;

		// Gracefully handle error if possible
		if let Ok(text) = &text {
			match serde_json::from_str::<CloudflareError>(text) {
				Ok(err_body) => {
					if err_body.errors.iter().any(|x| x.code == 1436) {
						tracing::warn!(hostname=?ctx.hostname, "hostname does not exist");
					}
				}
				Err(err) => {
					tracing::warn!(?err, %text, "failed to decode error");
					internal_panic!("failed to delete custom hostname");
				}
			}
		} else {
			tracing::error!(hostname=?ctx.hostname, ?status, "failed to delete custom hostname");
			internal_panic!("failed to delete custom hostname");
		}
	}

	let (subscription_id,) = sqlx::query_as::<_, (Uuid,)>(indoc!(
		"
		SELECT subscription_id
		FROM custom_hostnames
		WHERE identifier = $1
		"
	))
	.bind(identifier)
	.fetch_one(&crdb)
	.await?;

	// TODO: Delete stripe subscription for hostname

	sqlx::query(indoc!(
		"
		DELETE FROM custom_hostnames
		WHERE identifier = $1
		"
	))
	.bind(identifier)
	.execute(&crdb)
	.await?;

	msg!([ctx] cf_custom_hostname::msg::delete_complete(namespace_id, &ctx.hostname) {
		namespace_id: ctx.namespace_id,
		hostname: ctx.hostname.clone(),
	})
	.await?;

	Ok(())
}
