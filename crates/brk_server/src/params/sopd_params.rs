use schemars::JsonSchema;
use serde::Deserialize;

use brk_types::{Cohort, Date, Height, UrpdAggregation};

/// Path parameters for `/api/sopd/{cohort}/{date}`.
#[derive(Deserialize, JsonSchema)]
pub struct SopdParams {
    pub cohort: Cohort,
    #[schemars(with = "String", example = &"2024-01-01")]
    pub date: Date,
}

/// Path parameters for per-cohort SOPD endpoints.
#[derive(Deserialize, JsonSchema)]
pub struct SopdCohortParam {
    pub cohort: Cohort,
}

/// Path parameters for `/api/sopd/{cohort}/block/{height}`.
#[derive(Deserialize, JsonSchema)]
pub struct SopdBlockParam {
    pub cohort: Cohort,
    pub height: Height,
}

/// Query parameters for SOPD endpoints. Reuses `UrpdAggregation` since SOPD
/// shares URPD's bucketing strategies.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SopdQuery {
    /// Aggregation strategy. Default: raw (no aggregation). Accepts `bucket` as alias.
    #[serde(default, rename = "agg", alias = "bucket")]
    pub aggregation: UrpdAggregation,
}
