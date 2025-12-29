//! Provider traits for fetching market data.
//!
//! This module defines the core provider traits:
//!
//! - [`DataProvider`] - Base trait for all data providers
//! - [`PriceDataProvider`] - OHLCV price data
//! - [`FundamentalDataProvider`] - Financial statements and metrics
//! - [`TickDataProvider`] - Tick-level market data
//! - [`ReferenceDataProvider`] - Company metadata and universe definitions

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use futures::Stream;
use polars::prelude::DataFrame;
use std::fmt::Debug;
use std::pin::Pin;

use crate::{
    error::Result,
    frequency::{DataFrequency, PeriodType},
    types::{CompanyInfo, FinancialStatement, KeyMetrics, Symbol, Tick},
};

/// Base trait for all data providers.
///
/// All data providers must implement this trait to provide basic metadata
/// about the provider and its capabilities.
pub trait DataProvider: Send + Sync + Debug {
    /// Returns the name of this provider (e.g., "Yahoo Finance").
    fn name(&self) -> &str;

    /// Returns a description of this provider.
    fn description(&self) -> &str;

    /// Returns the data frequencies supported by this provider.
    fn supported_frequencies(&self) -> &[DataFrequency];
}

/// Provider for OHLCV price data.
///
/// Implement this trait to provide historical price data.
#[async_trait]
pub trait PriceDataProvider: DataProvider {
    /// Fetches OHLCV data for a single symbol.
    ///
    /// Returns a DataFrame with columns: date, open, high, low, close, volume, adjusted_close.
    async fn fetch_ohlcv(
        &self,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame>;

    /// Fetches OHLCV data for multiple symbols.
    ///
    /// Default implementation calls `fetch_ohlcv` sequentially for each symbol.
    /// Providers can override with an optimized batch implementation.
    async fn fetch_ohlcv_batch(
        &self,
        symbols: &[Symbol],
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame> {
        use polars::prelude::*;

        let mut frames = Vec::with_capacity(symbols.len());

        for symbol in symbols {
            match self.fetch_ohlcv(symbol, start, end, frequency).await {
                Ok(mut df) => {
                    let symbol_name = PlSmallStr::from("symbol");
                    let df = if df.get_column_names().contains(&&symbol_name) {
                        df
                    } else {
                        let symbol_col = Column::new(
                            PlSmallStr::from("symbol"),
                            vec![symbol.as_str(); df.height()],
                        );
                        df.with_column(symbol_col)
                            .map_err(|e| crate::error::DataError::Other(e.to_string()))?
                            .clone()
                    };
                    frames.push(df);
                }
                Err(crate::error::DataError::SymbolNotFound(_)) => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        if frames.is_empty() {
            return Ok(DataFrame::empty());
        }

        let combined = concat(
            frames
                .iter()
                .map(|df| df.clone().lazy())
                .collect::<Vec<_>>(),
            UnionArgs::default(),
        )
        .map_err(|e| crate::error::DataError::Other(e.to_string()))?
        .collect()
        .map_err(|e| crate::error::DataError::Other(e.to_string()))?;

        Ok(combined)
    }
}

/// Provider for fundamental financial data.
///
/// Implement this trait to provide financial statements and key metrics.
#[async_trait]
pub trait FundamentalDataProvider: DataProvider {
    /// Fetches financial statements for a symbol.
    ///
    /// # Arguments
    ///
    /// * `symbol` - The stock symbol
    /// * `period_type` - Annual or Quarterly
    /// * `limit` - Maximum number of periods to return (most recent first)
    async fn fetch_financials(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FinancialStatement>>;

    /// Fetches key financial metrics for a symbol on a specific date.
    async fn fetch_metrics(&self, symbol: &Symbol, date: NaiveDate) -> Result<KeyMetrics>;
}

/// Provider for tick-level market data.
///
/// Implement this trait to provide real-time and historical tick data.
#[async_trait]
pub trait TickDataProvider: DataProvider {
    /// Fetches historical tick data for a symbol.
    async fn fetch_ticks(
        &self,
        symbol: &Symbol,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Tick>>;

    /// Subscribes to real-time tick data for multiple symbols.
    ///
    /// Returns a stream of ticks as they arrive.
    async fn subscribe(
        &self,
        symbols: &[Symbol],
    ) -> Result<Pin<Box<dyn Stream<Item = Tick> + Send>>>;
}

/// Provider for reference/metadata.
///
/// Implement this trait to provide company information and universe definitions.
#[async_trait]
pub trait ReferenceDataProvider: DataProvider {
    /// Fetches company information for a symbol.
    async fn company_info(&self, symbol: &Symbol) -> Result<CompanyInfo>;

    /// Fetches symbols in a named universe (e.g., "sp500", "nasdaq100").
    async fn universe(&self, universe_id: &str) -> Result<Vec<Symbol>>;

    /// Checks if a symbol is supported by this provider.
    async fn supports_symbol(&self, symbol: &Symbol) -> Result<bool>;
}
