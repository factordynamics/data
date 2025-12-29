//! Cache trait for storing fetched financial data.
//!
//! This module defines the [`DataCache`] trait that provides a unified interface
//! for caching OHLCV data, financial statements, and key metrics.

use async_trait::async_trait;
use chrono::NaiveDate;
use polars::prelude::DataFrame;
use std::time::Duration;

use crate::{
    error::Result,
    frequency::PeriodType,
    types::{FinancialStatement, KeyMetrics, Symbol},
};

/// Trait for caching fetched financial data.
///
/// Implementations can store data in various backends (SQLite, in-memory, etc.)
/// to avoid repeated API calls and improve performance.
#[async_trait]
pub trait DataCache: Send + Sync {
    /// Retrieves cached OHLCV data for a symbol within a date range.
    ///
    /// Returns `Ok(Some(df))` if cached data exists, `Ok(None)` if not cached.
    async fn get_ohlcv(
        &self,
        provider: &str,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Option<DataFrame>>;

    /// Stores OHLCV data in the cache.
    async fn put_ohlcv(&self, provider: &str, symbol: &Symbol, data: &DataFrame) -> Result<()>;

    /// Retrieves cached financial statements for a symbol.
    ///
    /// Returns `Ok(Some(statements))` if cached, `Ok(None)` if not cached.
    async fn get_financials(
        &self,
        provider: &str,
        symbol: &Symbol,
        period_type: PeriodType,
    ) -> Result<Option<Vec<FinancialStatement>>>;

    /// Stores financial statements in the cache.
    async fn put_financials(
        &self,
        provider: &str,
        symbol: &Symbol,
        statements: &[FinancialStatement],
    ) -> Result<()>;

    /// Retrieves cached key metrics for a symbol on a specific date.
    ///
    /// Returns `Ok(Some(metrics))` if cached, `Ok(None)` if not cached.
    async fn get_metrics(
        &self,
        provider: &str,
        symbol: &Symbol,
        date: NaiveDate,
    ) -> Result<Option<KeyMetrics>>;

    /// Stores key metrics in the cache.
    async fn put_metrics(
        &self,
        provider: &str,
        symbol: &Symbol,
        metrics: &KeyMetrics,
    ) -> Result<()>;

    /// Removes cache entries older than the specified TTL.
    ///
    /// Returns the number of entries invalidated.
    async fn invalidate_stale(&self, ttl: Duration) -> Result<usize>;

    /// Clears all cached data.
    async fn clear(&self) -> Result<()>;
}
