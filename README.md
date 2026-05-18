# brk-sopd-copd

**SOPD** (Spent Output Price Distribution) and **COPD** (Created Output Price
Distribution) — flow primitives extending
[Bitcoin Research Kit (BRK)](https://github.com/bitcoinresearchkit/brk)'s URPD
into a closed three-primitive system for UTXO-only on-chain data.

The endpoints power [bitcointerminal.net](https://bitcointerminal.net) — a
Bitcoin analytics terminal positioned around the **stock/flow analytical lens**
for UTXO-only on-chain data.

## Table of contents

- [Why SOPD + COPD](#why-sopd--copd)
- [API contract](#api-contract)
- [Worked example](#worked-example)
- [Source files](#source-files)
- [Implementation guide](#implementation-guide)
  - [1. Data structures](#1-data-structures)
  - [2. Compute hooks](#2-compute-hooks)
  - [3. Day-boundary flush](#3-day-boundary-flush)
  - [4. Per-block retention](#4-per-block-retention)
  - [5. On-disk wire format](#5-on-disk-wire-format-sopdraw)
  - [6. Query and API layer](#6-query-and-api-layer)
- [Convention 1: coinbase and OP_RETURN](#convention-1-coinbase-and-op_return)
- [Verification recipe](#verification-recipe)
- [Aggregation strategies](#aggregation-strategies)
- [License](#license)
- [Credits and prior art](#credits-and-prior-art)

## Why SOPD + COPD

[BRK](https://github.com/bitcoinresearchkit/brk) ships URPD (UTXO Realized
Price Distribution) — the stock primitive of all UTXOs binned by their
realized (creation) price. SOPD and COPD complete the trio:

| Primitive | Side | Question answered |
|---|---|---|
| **URPD** | stock | Where does held supply sit on the cost curve? |
| **SOPD** | flow (destocking) | At what cost basis is supply being spent in window *t*? |
| **COPD** | flow (production) | At what price is new supply being created in window *t*? |

The conservation identity `ΔURPD ≡ COPD − SOPD` (modulo cohort-transition
edge cases and OP_RETURN burns) makes URPD + SOPD + COPD a closed
three-primitive system: every per-bin change in the stock is fully
accounted for by the flow primitives.

For the `all` cohort, cohort-transition terms cancel out and the identity
holds *exactly* modulo OP_RETURN — see [Convention 1](#convention-1-coinbase-and-op_return).

## API contract

Both primitives expose six routes each — four daily plus two per-block —
mirroring BRK's existing URPD route family. Replace `sopd` with `copd`
for the receive-side mirror.

### Routes

| Method | Path | Returns | Cache |
|---|---|---|---|
| `GET` | `/api/sopd` | `Vec<Cohort>` — cohorts with SOPD data | Deploy |
| `GET` | `/api/sopd/{cohort}/dates` | `Vec<Date>` — UTC days with SOPD data, ascending | Tip |
| `GET` | `/api/sopd/{cohort}` | `Sopd` — latest available date for the cohort | Tip |
| `GET` | `/api/sopd/{cohort}/{date}` | `Sopd` — snapshot for `(cohort, date)` | Date-strategy |
| `GET` | `/api/sopd/{cohort}/blocks` | `Vec<Height>` — block heights with per-block SOPD (rolling window) | Tip |
| `GET` | `/api/sopd/{cohort}/block/{height}` | `Sopd` — per-block DELTA at `(cohort, height)` | Height-strategy |

Query parameters on `/api/sopd/{cohort}/{date}`, `/api/sopd/{cohort}`, and
`/api/sopd/{cohort}/block/{height}`:

- `agg` (optional): aggregation strategy. See [Aggregation strategies](#aggregation-strategies).
  Defaults to `Raw` (no re-binning).

### Response shape

The response type is the Rust struct `Sopd` (see
[`crates/brk_types/src/sopd.rs`](crates/brk_types/src/sopd.rs)):

```rust
pub struct Sopd {
    pub cohort: Cohort,
    pub date: Date,
    pub aggregation: UrpdAggregation,
    /// Close price on `date`, in USD. Anchor for `realized_pnl`.
    pub close: Dollars,
    /// Sum of `spent` across all buckets, in BTC.
    pub total_spent: Bitcoin,
    pub buckets: Vec<SopdBucket>,
}

pub struct SopdBucket {
    /// Lower bound of the cost-basis bucket, in USD. For the default
    /// `Raw` aggregation this is the dollar-rounded cost-basis price as
    /// persisted on disk (see Day-boundary flush §3). Other `agg`
    /// strategies re-bin further at read time.
    pub price_floor: Dollars,
    /// Spent supply that had this cost basis, in BTC.
    pub spent: Bitcoin,
    /// Cost-basis value contribution in USD: Σ (cost_basis_price × spent)
    /// over the coins in this bucket. How much realized cap was removed.
    pub cost_basis_value: Dollars,
    /// Realized P&L in USD against the close on the snapshot date:
    /// (close − price_floor) × spent. Positive = profit-side spending,
    /// negative = loss-side spending.
    pub realized_pnl: Dollars,
}
```

JSON example (`GET /api/sopd/all/2024-06-01`):

```json
{
  "cohort": "all",
  "date": "2024-06-01",
  "aggregation": "raw",
  "close": 67500.42,
  "total_spent": 42.21331842,
  "buckets": [
    {
      "price_floor": 500.00,
      "spent": 0.41200000,
      "cost_basis_value": 206.00,
      "realized_pnl": 27796.97
    }
  ]
}
```

COPD has the same shape; `spent` and `cost_basis_value` semantically read as
*created* and *creation-price value* respectively, and `realized_pnl` for
COPD is the unrealized P&L of the newly-created cohort against the day's
close (almost zero for non-coinbase outputs because the creation price *is*
the spot price on the same block — see [Convention 1](#convention-1-coinbase-and-op_return)).

For the per-block routes (`/api/sopd/{cohort}/block/{height}` and the COPD
mirror), the response shape is identical; `date` echoes the UTC date of the
block. The data represents the **delta within that block only**, not a
cumulative snapshot.

## Worked example

A minimal synthetic UTXO scenario showing how the identity reads off the
two flow primitives. UTC day `D`, single cohort `all`, prices in USD.

**State at end of day `D−1`:**

| Price bucket | URPD supply (BTC) |
|---|---|
| $30,000 | 100 |
| $40,000 | 50 |

**Events on day `D`:**

- Spend 10 BTC last moved at $30,000. (Cost basis = $30k.)
- Spend 5 BTC last moved at $40,000. (Cost basis = $40k.)
- Receive 20 BTC of newly-mined supply at spot $60,000. (Coinbase.)
- Receive 8 BTC at spot $60,200. (UTXO churn at end-of-day spot.)

**SOPD for day `D`:**

| `price_floor` | `spent` | `cost_basis_value` | `realized_pnl` (vs close $60,500) |
|---|---|---|---|
| $30,000 | 10 BTC | $300,000 | $305,000 |
| $40,000 | 5 BTC | $200,000 | $102,500 |

`total_spent` = 15 BTC.

**COPD for day `D`:**

| `price_floor` | `spent` (= created) | `cost_basis_value` (= creation-price value) | `realized_pnl` (vs close $60,500) |
|---|---|---|---|
| $60,000 | 20 BTC | $1,200,000 | $10,000 |
| $60,200 | 8 BTC | $481,600 | $2,400 |

`total_spent` = 28 BTC of created supply.

**URPD diff (end-of-day `D` minus end-of-day `D−1`):**

| Price bucket | ΔURPD (BTC) |
|---|---|
| $30,000 | −10 |
| $40,000 | −5 |
| $60,000 | +20 |
| $60,200 | +8 |

**Identity check** (per bin): `ΔURPD(P) = COPD(P) − SOPD(P)`

| Price | ΔURPD | COPD | SOPD | COPD − SOPD |
|---|---|---|---|---|
| $30,000 | −10 | 0 | 10 | −10 ✓ |
| $40,000 | −5 | 0 | 5 | −5 ✓ |
| $60,000 | +20 | 20 | 0 | +20 ✓ |
| $60,200 | +8 | 8 | 0 | +8 ✓ |

This is the per-bin form of the identity. For the `all` cohort it holds
exactly modulo OP_RETURN ([Convention 1](#convention-1-coinbase-and-op_return)).

## Source files

The repository contains the SOPD/COPD source modules as deployed, organized
under the same crate tree as upstream BRK:

```
crates/
├── brk_types/src/
│   ├── sopd.rs            # Sopd response struct + build()
│   ├── sopd_bucket.rs     # SopdBucket struct
│   └── sopd_raw.rs        # SopdRaw on-disk codec (pco-compressed)
├── brk_server/src/
│   ├── api/
│   │   ├── sopd.rs        # 6 axum routes for SOPD
│   │   └── copd.rs        # 6 axum routes for COPD
│   └── params/
│       └── sopd_params.rs # path + query parameter types
└── brk_query/src/impl/
    ├── sopd.rs            # query layer for SOPD
    └── copd.rs            # query layer for COPD
```

These files reference BRK upstream types (`Cohort`, `Date`, `Cents`,
`CentsCompact`, `Sats`, etc.) and helper modules (`extended`, `params`)
that live in the rest of the BRK codebase. To integrate, drop the files
into the same paths in a BRK checkout and wire up `add_sopd_routes()` /
`add_copd_routes()` in `crates/brk_server/src/api/mod.rs`.

## Implementation guide

### 1. Data structures

SOPD and COPD live inside the cohort state alongside URPD. Each primitive
is a **two-tier accumulator**: a UTC-day map flushed at day boundary, plus
a per-block map flushed every block.

```rust
// crates/brk_computer/src/distribution/state/cohort/base.rs

pub struct CohortState<R, C> {
    // ... existing URPD / supply / cost-basis fields ...

    /// SOPD daily accumulator — spent sats keyed by cost-basis price.
    /// Reset at day boundary.
    sopd_today: BTreeMap<CentsCompact, Sats>,
    /// Per-block SOPD delta — spends within the current block only.
    /// Written then cleared every block.
    sopd_block: BTreeMap<CentsCompact, Sats>,
    /// Opt-in flag — cohorts that don't persist SOPD leave this false.
    tracks_sopd: bool,

    /// COPD daily accumulator — created sats keyed by creation price
    /// (= spot at the block where the UTXO came into existence). Reset
    /// at day boundary. Mirror of `sopd_today` on the receive side.
    copd_today: BTreeMap<CentsCompact, Sats>,
    /// Per-block COPD delta — UTXOs created in the current block only.
    copd_block: BTreeMap<CentsCompact, Sats>,
    /// Opt-in flag — tracked independently of `tracks_sopd` so cohorts
    /// can opt into one without the other, though in practice paired.
    tracks_copd: bool,
}
```

Where:

- `CentsCompact` is BRK's `u32` newtype for USD-cent price keys (already
  used by URPD).
- `Sats` is BRK's `u64` newtype for sats (already used everywhere).
- `BTreeMap` keeps keys sorted, so the on-disk serializer iterates in
  ascending price order without a separate sort step.

Builder methods to opt in:

```rust
impl<R, C> CohortState<R, C> {
    pub(crate) fn with_sopd_tracking(mut self) -> Self {
        self.tracks_sopd = true;
        self
    }

    pub(crate) fn with_copd_tracking(mut self) -> Self {
        self.tracks_copd = true;
        self
    }
}
```

### 2. Compute hooks

Both hooks live inside existing `CohortState` methods. They are gated on
the opt-in flags so cohorts that don't persist these maps skip the work
entirely.

**COPD hook — inside `receive_utxo_snapshot`:**

```rust
if self.tracks_copd {
    let key = CentsCompact::from(snapshot.realized_price);
    *self.copd_today.entry(key).or_insert(Sats::ZERO) += supply.value;
    *self.copd_block.entry(key).or_insert(Sats::ZERO) += supply.value;
}
```

- `snapshot.realized_price` is the spot price at the block where the UTXO
  came into existence. For a freshly-received UTXO the cost basis *is* the
  current price, so `realized_price` is exactly the creation price.
- `supply.value` is the sats contributed by this snapshot.

**SOPD hook — inside `send_utxo_precomputed`:**

```rust
if self.tracks_sopd {
    let key = CentsCompact::from(pre.prev_price);
    *self.sopd_today.entry(key).or_insert(Sats::ZERO) += pre.sats;
    *self.sopd_block.entry(key).or_insert(Sats::ZERO) += pre.sats;
}
```

- `pre.prev_price` is the cost basis of the spent UTXO — the spot price
  at the block where this UTXO was last moved (its creation). BRK
  populates this from `chain_state[receive_height].price` at the spend
  site (`crates/brk_computer/src/distribution/cohorts/utxo/send.rs`).
- `pre.sats` is the sats in the spent UTXO.

Four lines per hook, two hooks total per cohort.

### 3. Day-boundary flush

At UTC midnight rollover, for each cohort that has `tracks_sopd` (or
`tracks_copd`) set:

1. Iterate the `_today` BTreeMap and round each price key to the nearest
   dollar via `Cents::round_to_dollar(COST_BASIS_PRICE_DIGITS)`, merging
   consecutive equal rounded keys. `COST_BASIS_PRICE_DIGITS = 5` means
   dollar precision below $100,000, expanding to nearest $10 / $100 / etc.
   at higher orders of magnitude (5 significant digits applied to the
   dollar value).
2. Serialize the rounded `(price, sats)` pairs to a `SopdRaw` byte blob
   ([§5](#5-on-disk-wire-format-sopdraw)).
3. Write the blob to `<base>/<cohort>/sopd/<date>` (or `<cohort>/copd/<date>`).
4. Call `self.sopd_today.clear()` (or `self.copd_today.clear()`).

The day boundary uses the same UTC day-rollover BRK already uses for URPD
day-snapshots. The on-disk file naming follows the same convention as
URPD's existing day-snapshots so the query layer uses a single `read_dir`
+ `read` pattern.

The write-time dollar-rounding is the same precision used by URPD, so all
three primitives bin at consistent granularity. When the `agg` query
parameter is `raw` (the default), the API returns these dollar-precision
buckets directly. Other `agg` strategies re-bin further at read time.

### 4. Per-block retention

`sopd_block` and `copd_block` are written every block, then cleared.
The implementation retains a **rolling 25,920-block window** on disk
for both primitives (`BLOCK_WINDOW_LEN: u32 = 25_920` — roughly six
months of blocks), with stride-gated eviction: older heights stay on
disk only if they match a coarser stride (1 / 3 / 6 / 12 blocks at
nested depth thresholds), so the per-cohort footprint stays bounded
while the lookback remains useful for multi-month per-block analytics.

### 5. On-disk wire format (`SopdRaw`)

Both SOPD and COPD reuse a single on-disk codec, `SopdRaw` (see
[`crates/brk_types/src/sopd_raw.rs`](crates/brk_types/src/sopd_raw.rs)).
It is a direct binary mirror of BRK's existing `UrpdRaw` codec — same
pco-compressed format, same `BTreeMap<CentsCompact, Sats>` underlying
type.

```
+--------+--------+--------+--------+--------+--------+
| HEADER (24 bytes)                                    |
+--------+--------+--------+--------+--------+--------+
| entry_count: u64 (little-endian)                     |
| keys_blob_len: u64 (little-endian)                   |
| values_blob_len: u64 (little-endian)                 |
+--------+--------+--------+--------+--------+--------+
| KEYS BLOB (keys_blob_len bytes)                      |
| pco-compressed Vec<u32> — price-cent keys, sorted    |
+--------+--------+--------+--------+--------+--------+
| VALUES BLOB (values_blob_len bytes)                  |
| pco-compressed Vec<u64> — sats values                |
+--------+--------+--------+--------+--------+--------+
```

- Compression: `pco::standalone::simple_compress` with default
  `ChunkConfig`. Decompression: `pco::standalone::simple_decompress`.
- Keys are written as `Vec<u32>` (each entry is one `CentsCompact::inner()`).
- Values are written as `Vec<u64>` (each entry is one `u64::from(Sats)`).
- Keys and values are written in **BTreeMap iteration order**, i.e. sorted
  ascending by price. This lets the decoder skip a re-sort step.

### 6. Query and API layer

The query layer (see [`crates/brk_query/src/impl/sopd.rs`](crates/brk_query/src/impl/sopd.rs)
and [`copd.rs`](crates/brk_query/src/impl/copd.rs)) exposes the following
methods (COPD has the symmetric set):

```rust
impl Query {
    pub fn sopd_cohorts(&self) -> Result<Vec<Cohort>>;
    pub fn sopd_dates(&self, cohort: &Cohort) -> Result<Vec<Date>>;
    pub fn sopd_block_heights(&self, cohort: &Cohort) -> Result<Vec<Height>>;
    pub fn sopd_at(&self, cohort: &Cohort, date: Date, agg: UrpdAggregation) -> Result<Sopd>;
    pub fn sopd_at_block(&self, cohort: &Cohort, height: Height, agg: UrpdAggregation) -> Result<Sopd>;
    pub fn sopd_latest(&self, cohort: &Cohort, agg: UrpdAggregation) -> Result<Sopd>;
}
```

`*_at`, `*_at_block`, and `*_latest` all reconstruct a `Sopd` via
`Sopd::build(cohort, date, close_cents, &raw, aggregation)` (see
[`crates/brk_types/src/sopd.rs`](crates/brk_types/src/sopd.rs)).
`Sopd::build` does the per-bucket aggregation, computes
`cost_basis_value` and `realized_pnl`, and converts cents → `Dollars`
and sats → `Bitcoin` for the wire format.

The API layer (see [`crates/brk_server/src/api/sopd.rs`](crates/brk_server/src/api/sopd.rs)
and [`copd.rs`](crates/brk_server/src/api/copd.rs)) registers the six
routes for each primitive via the `aide::axum::ApiRouter` traits
`ApiSopdRoutes` and `ApiCopdRoutes`.

## Convention 1: coinbase and OP_RETURN

**Convention 1**: coinbase outputs ARE included in COPD (per BRK's existing
UTXO ingest semantics — coinbase outputs enter the UTXO set and behave like
any other received UTXO at the receive hook). OP_RETURN outputs are NOT
included in COPD or SOPD because they never enter the UTXO set.

This convention has two consequences worth flagging:

1. The COPD daily total includes the block subsidy (and fees credited to
   the miner). For the `all` cohort on day `D`, `total_spent` (COPD)
   exceeds the net non-coinbase creation by ~`subsidy × blocks_per_day`.

2. The conservation identity `ΔURPD = COPD − SOPD` holds exactly for the
   `all` cohort modulo subsidy issuance. Subsidy enters URPD via COPD on
   day `D` and only ever exits via SOPD on a later day, so the identity
   is balanced over the lifetime of any sat but accumulates the subsidy
   contribution on the day it's mined.

OP_RETURN exclusion is the simpler half: BRK's UTXO ingest already filters
OP_RETURN outputs, so the hooks above never see them.

## Verification recipe

Four checks to confirm an implementation produces correct outputs. Run them
on the `all` cohort against historical data.

### Check 1: SOPD totals tie to existing realized cap on spend

For any date `D`:

```
Σ_{bucket b in SOPD(D)} b.cost_basis_value
  ≡ realized_cap_destroyed_on(D)   (within rounding)
```

The right-hand side is BRK's existing realized-cap-on-spend total for the
day. If your SOPD totals don't match the existing scalar within ε, the
hook is keying off the wrong field (likely `pre.current_ps` instead of
`pre.prev_price`).

### Check 2: COPD totals tie to existing realized cap on receive

For any date `D`:

```
Σ_{bucket b in COPD(D)} b.cost_basis_value
  ≡ realized_cap_created_on(D)   (within rounding)
```

Same logic, receive side. Mismatch usually means the hook is reading
`snapshot.price_sats` (a different price representation) instead of
`snapshot.realized_price`.

### Check 3: Per-bin conservation identity

For any date `D` and any price bin `P` on the `all` cohort:

```
URPD(D, P) − URPD(D−1, P)  ≡  COPD(D, P) − SOPD(D, P)
```

Subsidy issuance adds a small positive residual to the right-hand side
each day (Convention 1). Outside subsidy, the equality should hold within
single-sat noise.

The [worked example](#worked-example) above is the smallest scenario that
exercises this check across all four sign combinations.

### Check 4: Empty cohorts are empty

For any cohort with `tracks_sopd = false` (or `tracks_copd = false`):

```
GET /api/sopd/<cohort>/dates  →  404
```

The opt-in flag is the only thing that determines whether the cohort
appears in the cohorts list or the dates list. If your implementation
serves empty arrays for non-tracking cohorts, the gating is in the wrong
place — it should be at the persistence layer, not the query layer.

## Aggregation strategies

The `agg` query parameter on the per-snapshot endpoints controls how
the on-disk buckets get re-binned into the response. The strategies are
reused from URPD — same `UrpdAggregation` enum — so any strategy that
works for URPD works for SOPD and COPD.

The default is `Raw`, which returns the buckets at their on-disk
precision (dollar-rounded with 5-significant-digit scaling — see
[§3 Day-boundary flush](#3-day-boundary-flush)) without further re-binning.
Other strategies (log-bin, fixed-step, percentile-based) re-aggregate the
on-disk buckets at read time; they're documented in BRK's URPD spec.

## License

MIT — same as upstream BRK. See [LICENSE](LICENSE) in this repo and
[BRK's LICENSE](https://github.com/bitcoinresearchkit/brk/blob/main/docs/LICENSE.md).

## Credits and prior art

Built on top of [Bitcoin Research Kit](https://github.com/bitcoinresearchkit/brk).
URPD is
the direct technical prior art these primitives extend — without URPD's
existing bucketing, codec, and route family, this work would be
substantially larger.

**SOPD is not novel work.** The metric was introduced by **Renato
Shirakashi** and is published by [Glassnode](https://glassnode.com)
alongside the related SOPR family (Spent Output Profit Ratio, aSOPR,
STH-SOPR, LTH-SOPR). The contribution here is implementation-side
only: a BRK-native daily + per-block endpoint serving SOPD as a
standard JSON response on top of BRK's URPD infrastructure, with no
dependency on external data providers.

COPD as named here is the symmetric receive-side mirror — newly-created
UTXOs binned by their creation price. To the author's knowledge no
widely-published metric mirrors SOPD on the receive side under this
exact name, but conceptually-similar receive-side views may exist in
other on-chain analytics catalogs and are likely derivable from
established primitives. The role of COPD in this work is operational:
to close the URPD stock identity `ΔURPD = COPD − SOPD` for the `all`
cohort, so that flow accounting on the UTXO set is a closed system.

## Contact

- Twitter/X: [@RenshuBTC](https://x.com/RenshuBTC)
- Terminal: [bitcointerminal.net](https://bitcointerminal.net)
- Issues / questions: [open an issue](https://github.com/renshuBTC/brk-sopd-copd/issues)
  on this repo, or reach out via Twitter.
