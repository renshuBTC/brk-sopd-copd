use std::{fs, path::PathBuf};

use brk_error::{Error, OptionData, Result};
use brk_types::{Cohort, Date, Day1, Height, Sopd, SopdRaw, UrpdAggregation};
use vecdb::{ReadableOptionVec, ReadableVec};

use crate::Query;

/// COPD (Created Output Price Distribution) — the receive-side mirror
/// of SOPD. Same on-disk format (pco-compressed sorted (price, sats)
/// pairs) and same response shape, so we re-use `SopdRaw` for
/// deserialization and `Sopd` as the response type. The semantics
/// differ — COPD bins UTXOs CREATED in the window by their creation
/// price; SOPD bins UTXOs SPENT in the window by their cost basis.
///
/// Wire-format reuse keeps the implementation minimal. A dedicated
/// `Copd` / `CopdRaw` pair can be introduced later if the semantic
/// labels need to differ at the API boundary (e.g. `total_created`
/// instead of `total_spent`).
///
/// Endpoint surface (mirrors SOPD):
///   - copd_cohorts()        → list of cohorts with COPD data
///   - copd_dates(cohort)    → daily snapshot dates available
///   - copd_at(cohort, date) → daily COPD for a (cohort, date) pair
///   - copd_latest(cohort)   → daily COPD for the most recent date
///   - copd_at_block(...)    → per-block delta (rolling 25,920 window)
impl Query {
    // ─── daily COPD ─────────────────────────────────────────────────

    /// Available cohorts for COPD (daily layer). A cohort qualifies if
    /// its `<cohort>/copd` subdir exists on disk.
    pub fn copd_cohorts(&self) -> Result<Vec<Cohort>> {
        let states_path = &self.computer().distribution.states_path;

        let mut cohorts: Vec<Cohort> = fs::read_dir(states_path)?
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                if !states_path.join(&name).join("copd").exists() {
                    return None;
                }
                Cohort::new(name)
            })
            .collect();

        cohorts.sort_unstable();
        Ok(cohorts)
    }

    pub(crate) fn copd_dir(&self, cohort: &str) -> Result<PathBuf> {
        let dir = self
            .computer()
            .distribution
            .states_path
            .join(cohort)
            .join("copd");
        if !dir.exists() {
            let valid = self
                .copd_cohorts()
                .unwrap_or_default()
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::NotFound(format!(
                "Unknown cohort '{cohort}'. Available: {valid}"
            )));
        }
        Ok(dir)
    }

    /// Available dates for a cohort's daily COPD.
    pub fn copd_dates(&self, cohort: &Cohort) -> Result<Vec<Date>> {
        let dir = self.copd_dir(cohort)?;
        let mut dates: Vec<Date> = fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        dates.sort();
        Ok(dates)
    }

    /// Raw COPD data for a cohort on a specific date.
    pub fn copd_raw(&self, cohort: &Cohort, date: Date) -> Result<SopdRaw> {
        let path = self.copd_dir(cohort)?.join(date.to_string());
        if !path.exists() {
            return Err(Error::NotFound(format!(
                "No COPD for cohort '{cohort}' on {date}"
            )));
        }
        SopdRaw::deserialize(&fs::read(&path)?)
    }

    /// COPD for a cohort on a specific date.
    pub fn copd_at(&self, cohort: &Cohort, date: Date, agg: UrpdAggregation) -> Result<Sopd> {
        let raw = self.copd_raw(cohort, date)?;
        let day1 = Day1::try_from(date).map_err(|e| Error::Parse(e.to_string()))?;
        let close = self
            .computer()
            .prices
            .split
            .close
            .cents
            .day1
            .collect_one_flat(day1)
            .ok_or_else(|| Error::NotFound(format!("No price data for {date}")))?;
        Ok(Sopd::build(cohort.clone(), date, close, &raw, agg))
    }

    /// COPD for the most recently available date in a cohort.
    pub fn copd_latest(&self, cohort: &Cohort, agg: UrpdAggregation) -> Result<Sopd> {
        let dates = self.copd_dates(cohort)?;
        let date = *dates
            .last()
            .ok_or_else(|| Error::NotFound(format!("No COPD available for cohort '{cohort}'")))?;
        self.copd_at(cohort, date, agg)
    }

    // ─── per-block COPD (rolling 25,920-block window) ───────────────

    pub(crate) fn copd_block_dir(&self, cohort: &str) -> Result<PathBuf> {
        let dir = self
            .computer()
            .distribution
            .states_path
            .join(cohort)
            .join("copd_block");
        if !dir.exists() {
            return Err(Error::NotFound(format!(
                "No per-block COPD for cohort '{cohort}'"
            )));
        }
        Ok(dir)
    }

    /// Available block heights in a cohort's rolling per-block COPD window.
    pub fn copd_block_heights(&self, cohort: &Cohort) -> Result<Vec<Height>> {
        let dir = self.copd_block_dir(cohort)?;
        let mut heights: Vec<Height> = fs::read_dir(&dir)?
            .filter_map(|entry| {
                entry
                    .ok()?
                    .file_name()
                    .to_str()?
                    .parse::<u32>()
                    .ok()
                    .map(Height::from)
            })
            .collect();
        heights.sort_unstable();
        Ok(heights)
    }

    /// COPD for a cohort at a specific block height. Per-block DELTA —
    /// UTXOs CREATED in that block only, binned by creation price.
    pub fn copd_at_block(
        &self,
        cohort: &Cohort,
        height: Height,
        agg: UrpdAggregation,
    ) -> Result<Sopd> {
        let path = self
            .copd_block_dir(cohort)?
            .join(u32::from(height).to_string());
        if !path.exists() {
            return Err(Error::NotFound(format!(
                "No per-block COPD for cohort '{cohort}' at height {}",
                u32::from(height)
            )));
        }
        let raw = SopdRaw::deserialize(&fs::read(&path)?)?;
        let close = self
            .computer()
            .prices
            .spot
            .cents
            .height
            .collect_one(height)
            .data()?;
        let timestamp = self
            .indexer()
            .vecs
            .blocks
            .timestamp
            .collect_one(height)
            .data()?;
        let date = Date::from(timestamp);
        Ok(Sopd::build(cohort.clone(), date, close, &raw, agg))
    }
}
