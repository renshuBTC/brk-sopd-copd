use std::{fs, path::PathBuf};

use brk_error::{Error, OptionData, Result};
use brk_types::{Cohort, Date, Day1, Height, Sopd, SopdRaw, UrpdAggregation};
use vecdb::{ReadableOptionVec, ReadableVec};

use crate::Query;

impl Query {
    /// Available cohorts for SOPD.
    pub fn sopd_cohorts(&self) -> Result<Vec<Cohort>> {
        let states_path = &self.computer().distribution.states_path;

        let mut cohorts: Vec<Cohort> = fs::read_dir(states_path)?
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                if !states_path.join(&name).join("sopd").exists() {
                    return None;
                }
                Cohort::new(name)
            })
            .collect();

        cohorts.sort_unstable();
        Ok(cohorts)
    }

    pub(crate) fn sopd_dir(&self, cohort: &str) -> Result<PathBuf> {
        let dir = self
            .computer()
            .distribution
            .states_path
            .join(cohort)
            .join("sopd");
        if !dir.exists() {
            let valid = self
                .sopd_cohorts()
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

    /// Available dates for a cohort.
    pub fn sopd_dates(&self, cohort: &Cohort) -> Result<Vec<Date>> {
        let dir = self.sopd_dir(cohort)?;
        let mut dates: Vec<Date> = fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        dates.sort();
        Ok(dates)
    }

    /// Raw SOPD data for a cohort on a specific date.
    pub fn sopd_raw(&self, cohort: &Cohort, date: Date) -> Result<SopdRaw> {
        let path = self.sopd_dir(cohort)?.join(date.to_string());
        if !path.exists() {
            return Err(Error::NotFound(format!(
                "No SOPD for cohort '{cohort}' on {date}"
            )));
        }
        SopdRaw::deserialize(&fs::read(&path)?)
    }

    /// SOPD for a cohort on a specific date.
    pub fn sopd_at(&self, cohort: &Cohort, date: Date, agg: UrpdAggregation) -> Result<Sopd> {
        let raw = self.sopd_raw(cohort, date)?;
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

    /// SOPD for the most recently available date in a cohort.
    pub fn sopd_latest(&self, cohort: &Cohort, agg: UrpdAggregation) -> Result<Sopd> {
        let dates = self.sopd_dates(cohort)?;
        let date = *dates
            .last()
            .ok_or_else(|| Error::NotFound(format!("No SOPD available for cohort '{cohort}'")))?;
        self.sopd_at(cohort, date, agg)
    }

    // --- Phase 3c: per-block SOPD (rolling 144-block window) ---

    pub(crate) fn sopd_block_dir(&self, cohort: &str) -> Result<PathBuf> {
        let dir = self
            .computer()
            .distribution
            .states_path
            .join(cohort)
            .join("sopd_block");
        if !dir.exists() {
            return Err(Error::NotFound(format!(
                "No per-block SOPD for cohort '{cohort}'"
            )));
        }
        Ok(dir)
    }

    /// Available block heights in a cohort's rolling per-block SOPD window.
    pub fn sopd_block_heights(&self, cohort: &Cohort) -> Result<Vec<Height>> {
        let dir = self.sopd_block_dir(cohort)?;
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

    /// SOPD for a cohort at a specific block height. This is a per-block DELTA
    /// (spends within that block only), not a cumulative snapshot.
    pub fn sopd_at_block(
        &self,
        cohort: &Cohort,
        height: Height,
        agg: UrpdAggregation,
    ) -> Result<Sopd> {
        let path = self
            .sopd_block_dir(cohort)?
            .join(u32::from(height).to_string());
        if !path.exists() {
            return Err(Error::NotFound(format!(
                "No per-block SOPD for cohort '{cohort}' at height {}",
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
