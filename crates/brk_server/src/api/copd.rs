use aide::axum::{ApiRouter, routing::get_with};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Uri},
};
use brk_types::{Cohort, Date, Sopd, Version};

use crate::{
    CacheStrategy,
    extended::TransformResponseExtended,
    params::{Empty, SopdBlockParam, SopdCohortParam, SopdParams, SopdQuery},
};

use super::AppState;

/// COPD (Created Output Price Distribution) — receive-side mirror of
/// SOPD. Same request and response shapes (cohort + date / height path,
/// optional `agg` query, sorted bin/sats response), so we reuse the
/// SOPD param + Sopd response types verbatim. The semantic difference —
/// SOPD counts SPENDS, COPD counts CREATIONS — is documented in each
/// endpoint description; the wire format is shared so client code can
/// use the same parsers.
pub trait ApiCopdRoutes {
    fn add_copd_routes(self) -> Self;
}

impl ApiCopdRoutes for ApiRouter<AppState> {
    fn add_copd_routes(self) -> Self {
        self.api_route(
            "/api/copd",
            get_with(
                async |uri: Uri, headers: HeaderMap, _: Empty, State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Deploy, &uri, |q| q.copd_cohorts())
                        .await
                },
                |op| {
                    op.id("list_copd_cohorts")
                        .urpd_tag()
                        .summary("Available COPD cohorts")
                        .description(
                            "Cohorts for which COPD data is available. Returns names like \
                            `all`, `sth`, `lth`, `utxos_under_1h_old`.",
                        )
                        .json_response::<Vec<Cohort>>()
                        .not_modified()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/copd/{cohort}/dates",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       _: Empty,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.copd_dates(&params.cohort)
                        })
                        .await
                },
                |op| {
                    op.id("list_copd_dates")
                        .urpd_tag()
                        .summary("Available COPD dates")
                        .description(
                            "Dates for which a daily COPD snapshot is available for the cohort. \
                            One entry per UTC day, sorted ascending.",
                        )
                        .json_response::<Vec<Date>>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/copd/{cohort}/blocks",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       _: Empty,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.copd_block_heights(&params.cohort)
                        })
                        .await
                },
                |op| {
                    op.id("list_copd_block_heights")
                        .urpd_tag()
                        .summary("Available per-block COPD heights")
                        .description(
                            "Block heights for which a per-block COPD snapshot is available \
                            for the cohort. Rolling window of the most recent 25,920 blocks, \
                            stride-gated by depth, sorted ascending.",
                        )
                        .json_response::<Vec<brk_types::Height>>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/copd/{cohort}/block/{height}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdBlockParam>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    let strategy = state.height_strategy(Version::ONE, params.height);
                    state
                        .respond_json(&headers, strategy, &uri, move |q| {
                            q.copd_at_block(&params.cohort, params.height, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_copd_at_block")
                        .urpd_tag()
                        .summary("COPD at block height")
                        .description(
                            "COPD (Created Output Price Distribution) for a (cohort, height) \
                            pair. Per-block DELTA — UTXOs CREATED in that block only, binned by \
                            their creation price (= block's spot price at creation). Mirror of \
                            `/api/sopd/{cohort}/block/{height}` on the receive side. Window: \
                            most recent 25,920 blocks, stride-gated by depth.",
                        )
                        .json_response::<Sopd>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/copd/{cohort}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.copd_latest(&params.cohort, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_copd")
                        .urpd_tag()
                        .summary("Latest COPD")
                        .description(
                            "Daily COPD for the most recent available date in the cohort. \
                            The response's `date` field echoes which date was served.\n\n\
                            See the COPD tag description for response shape and `agg` options.",
                        )
                        .json_response::<Sopd>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/copd/{cohort}/{date}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdParams>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    let strategy = state.date_strategy(Version::ONE, params.date);
                    state
                        .respond_json(&headers, strategy, &uri, move |q| {
                            q.copd_at(&params.cohort, params.date, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_copd_at")
                        .urpd_tag()
                        .summary("COPD at date")
                        .description(
                            "Daily COPD for a (cohort, date) pair. Returns the SOPD wire \
                            format with `total_spent` semantically reading as `total_created` \
                            and bin `spent` reading as `created`. Sum of all blocks in that \
                            UTC day, binned by creation price.\n\n\
                            See the COPD tag description for unit conventions and `agg` options.",
                        )
                        .json_response::<Sopd>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
    }
}
