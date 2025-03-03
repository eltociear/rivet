use proto::backend::pkg::*;
use rivet_operation::prelude::*;

#[operation(name = "cdn-site-list-for-game")]
async fn handle(
	ctx: OperationContext<cdn::site_list_for_game::Request>,
) -> GlobalResult<cdn::site_list_for_game::Response> {
	let game_id = internal_unwrap!(ctx.game_id).as_uuid();

	let site_ids = sqlx::query_as::<_, (Uuid,)>(indoc!(
		"
		SELECT site_id
		FROM sites
		WHERE game_id = $1
		"
	))
	.bind(game_id)
	.fetch_all(&ctx.crdb("db-cdn").await?)
	.await?
	.into_iter()
	.map(|(id,)| common::Uuid::from(id))
	.collect::<Vec<_>>();

	Ok(cdn::site_list_for_game::Response { site_ids })
}
