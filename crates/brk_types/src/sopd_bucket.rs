use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Bitcoin, Dollars};

/// A single bucket in a SOPD snapshot — spent outputs grouped by cost basis.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SopdBucket {
    /// Lower bound of the cost-basis bucket, in USD. Equals the exact cost-basis
    /// price for `Raw`.
    pub price_floor: Dollars,
    /// Spent supply that had this cost basis, in BTC. Coins spent on the snapshot
    /// date that were last moved at a price within this bucket.
    pub spent: Bitcoin,
    /// Cost-basis value contribution in USD: sum of `cost_basis_price * spent`
    /// over the coins in this bucket. (How much realized cap was removed.)
    pub cost_basis_value: Dollars,
    /// Realized P&L in USD against the close on the snapshot date:
    /// `(close - price_floor) * spent`. Positive = profit-side spending,
    /// negative = loss-side spending. Can be negative.
    pub realized_pnl: Dollars,
}
