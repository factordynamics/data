//! Data frequency and period type definitions.
//!
//! This module defines [`DataFrequency`] for specifying the granularity of time series data
//! and [`PeriodType`] for fundamental data periods.

use serde::{Deserialize, Serialize};

/// Frequency/granularity of time series data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataFrequency {
    /// Individual trades/quotes.
    Tick,
    /// One-second bars.
    Second,
    /// One-minute bars.
    Minute,
    /// Five-minute bars.
    FiveMinute,
    /// Fifteen-minute bars.
    FifteenMinute,
    /// Thirty-minute bars.
    ThirtyMinute,
    /// Hourly bars.
    Hourly,
    /// Daily bars.
    Daily,
    /// Weekly bars.
    Weekly,
    /// Monthly bars.
    Monthly,
    /// Quarterly data (for fundamentals).
    Quarterly,
    /// Annual data (for fundamentals).
    Annual,
}

impl DataFrequency {
    /// Returns true if this is an intraday frequency (tick through hourly).
    #[must_use]
    pub const fn is_intraday(&self) -> bool {
        matches!(
            self,
            Self::Tick
                | Self::Second
                | Self::Minute
                | Self::FiveMinute
                | Self::FifteenMinute
                | Self::ThirtyMinute
                | Self::Hourly
        )
    }

    /// Returns true if this is a fundamental data frequency (quarterly or annual).
    #[must_use]
    pub const fn is_fundamental(&self) -> bool {
        matches!(self, Self::Quarterly | Self::Annual)
    }
}

/// Period type for fundamental financial data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeriodType {
    /// Annual reporting period.
    #[default]
    Annual,
    /// Quarterly reporting period.
    Quarterly,
}
