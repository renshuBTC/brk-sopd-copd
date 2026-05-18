use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Bitcoin, Cents, CentsSats, CentsSigned, Cohort, Date, Dollars, Sats, SopdBucket, SopdRaw,
    UrpdAggregation,
};

/// Spent Output Profit Distribution for a cohort on a specific date.
///
/// Spent supply is grouped by the close price at which each spent UTXO was
/// last moved (its cost basis). Each bucket exposes three values: spent supply
/// in BTC, cost-basis value contribution in USD (sum of `cost_basis * spent`
/// over the coins in the bucket), and realized P&L in USD
/// (`(close - price_floor) * spent`, sign indicates profit/loss).
///
/// Reuses `UrpdAggregation` since SOPD shares URPD's bucketing strategies.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Sopd {
    pub cohort: Cohort,
    pub date: Date,
    /// Aggregation strategy applied to the buckets.
    pub aggregation: UrpdAggregation,
    /// Close price on `date`, in USD. Anchor for `realized_pnl`.
    pub close: Dollars,
    /// Sum of `spent` across all buckets, in BTC.
    pub total_spent: Bitcoin,
    pub buckets: Vec<SopdBucket>,
}

#[derive(Default, Clone, Copy)]
struct BucketAccum {
    spent: Sats,
    cost_basis_value: CentsSats,
}

impl Sopd {
    /// Build from the raw on-disk distribution plus context.
    pub fn build(
        cohort: Cohort,
        date: Date,
        close_cents: Cents,
        raw: &SopdRaw,
        aggregation: UrpdAggregation,
    ) -> Self {
        let mut agg: FxHashMap<Cents, BucketAccum> =
            FxHashMap::with_capacity_and_hasher(raw.map.len(), Default::default());
        for (&price_cents, &sats) in &raw.map {
            let price = Cents::from(price_cents);
            let key = aggregation.bucket_floor(price);
            let slot = agg.entry(key).or_default();
            slot.spent += sats;
            slot.cost_basis_value += CentsSats::from_price_sats(price, sats);
        }

        let mut sorted: Vec<_> = agg.into_iter().collect();
        sorted.sort_unstable_by_key(|&(price, _)| price);

        let close = Dollars::from(close_cents);
        let total_spent: Sats = raw.map.values().copied().sum();

        let buckets = sorted
            .into_iter()
            .map(|(price_floor_cents, slot)| {
                let cost_basis_cents = slot.cost_basis_value.to_cents();
                let close_value_cents =
                    CentsSats::from_price_sats(close_cents, slot.spent).to_cents();
                let pnl = CentsSigned::from(close_value_cents.inner())
                    - CentsSigned::from(cost_basis_cents.inner());
                SopdBucket {
                    price_floor: Dollars::from(price_floor_cents),
                    spent: Bitcoin::from(slot.spent),
                    cost_basis_value: Dollars::from(cost_basis_cents),
                    realized_pnl: Dollars::from(pnl),
                }
            })
            .collect();

        Self {
            cohort,
            date,
            aggregation,
            close,
            total_spent: Bitcoin::from(total_spent),
            buckets,
        }
    }
}
