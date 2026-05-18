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

pub trait ApiSopdRoutes {
    fn add_sopd_routes(self) -> Self;
}

impl ApiSopdRoutes for ApiRouter<AppState> {
    fn add_sopd_routes(self) -> Self {
        self.api_route(
            "/api/sopd",
            get_with(
                async |uri: Uri, headers: HeaderMap, _: Empty, State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Deploy, &uri, |q| q.sopd_cohorts())
                        .await
                },
                |op| {
                    op.id("list_sopd_cohorts")
                        .urpd_tag()
                        .summary("Available SOPD cohorts")
                        .description(
                            "Cohorts for which SOPD data is available. Returns names like \
                            `all`, `sth`, `lth`, `utxos_under_1h_old`.",
                        )
                        .json_response::<Vec<Cohort>>()
                        .not_modified()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/sopd/{cohort}/dates",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       _: Empty,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.sopd_dates(&params.cohort)
                        })
                        .await
                },
                |op| {
                    op.id("list_sopd_dates")
                        .urpd_tag()
                        .summary("Available SOPD dates")
                        .description(
                            "Dates for which a SOPD snapshot is available for the cohort. \
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
            "/api/sopd/{cohort}/blocks",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       _: Empty,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.sopd_block_heights(&params.cohort)
                        })
                        .await
                },
                |op| {
                    op.id("list_sopd_block_heights")
                        .urpd_tag()
                        .summary("Available per-block SOPD heights")
                        .description(
                            "Block heights for which a per-block SOPD snapshot is available \
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
            "/api/sopd/{cohort}/block/{height}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdBlockParam>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    let strategy = state.height_strategy(Version::ONE, params.height);
                    state
                        .respond_json(&headers, strategy, &uri, move |q| {
                            q.sopd_at_block(&params.cohort, params.height, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_sopd_at_block")
                        .urpd_tag()
                        .summary("SOPD at block height")
                        .description(
                            "SOPD for a (cohort, height) pair. This is a per-block DELTA — \
                            spends within that block only, not a cumulative snapshot. Same \
                            response shape as `/api/sopd/{cohort}/{date}`; the `date` field \
                            echoes the UTC date of the block. Window: most recent 25,920 \
                            blocks, stride-gated by depth.",
                        )
                        .json_response::<Sopd>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/sopd/{cohort}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdCohortParam>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    state
                        .respond_json(&headers, CacheStrategy::Tip, &uri, move |q| {
                            q.sopd_latest(&params.cohort, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_sopd")
                        .urpd_tag()
                        .summary("Latest SOPD")
                        .description(
                            "SOPD for the most recent available date in the cohort. \
                            The response's `date` field echoes which date was served.\n\n\
                            See the SOPD tag description for the response shape and `agg` options.",
                        )
                        .json_response::<Sopd>()
                        .not_modified()
                        .not_found()
                        .server_error()
                },
            ),
        )
        .api_route(
            "/api/sopd/{cohort}/{date}",
            get_with(
                async |uri: Uri,
                       headers: HeaderMap,
                       Path(params): Path<SopdParams>,
                       Query(query): Query<SopdQuery>,
                       State(state): State<AppState>| {
                    let strategy = state.date_strategy(Version::ONE, params.date);
                    state
                        .respond_json(&headers, strategy, &uri, move |q| {
                            q.sopd_at(&params.cohort, params.date, query.aggregation)
                        })
                        .await
                },
                |op| {
                    op.id("get_sopd_at")
                        .urpd_tag()
                        .summary("SOPD at date")
                        .description(
                            "SOPD for a (cohort, date) pair. Returns \
                            `{ cohort, date, aggregation, close, total_spent, buckets }` where \
                            each bucket is `{ price_floor, spent, cost_basis_value, realized_pnl }`.\n\n\
                            See the SOPD tag description for unit conventions and `agg` options.",
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
