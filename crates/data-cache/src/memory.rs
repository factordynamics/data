//! In-memory cache implementation.

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use data_core::{DataCache, FinancialStatement, KeyMetrics, PeriodType, Result, Symbol};
use polars::prelude::{ChunkAgg, DataFrame};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

/// Cache entry with timestamp for TTL-based invalidation.
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    data: T,
    cached_at: chrono::DateTime<Utc>,
}

impl<T> CacheEntry<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            cached_at: Utc::now(),
        }
    }

    fn is_stale(&self, ttl: Duration) -> bool {
        let age = Utc::now().signed_duration_since(self.cached_at);
        age > chrono::TimeDelta::from_std(ttl).unwrap_or(chrono::TimeDelta::MAX)
    }
}

/// Key for OHLCV cache entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OhlcvKey {
    provider: String,
    symbol: String,
    start: NaiveDate,
    end: NaiveDate,
}

/// Key for financials cache entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FinancialsKey {
    provider: String,
    symbol: String,
    period_type: PeriodType,
}

/// Key for metrics cache entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MetricsKey {
    provider: String,
    symbol: String,
    date: NaiveDate,
}

/// Simple in-memory cache for testing and development.
///
/// Data is stored in `RwLock`-protected `HashMap`s and is lost when the cache
/// is dropped. DataFrames and other types are cloned on get/put operations.
#[derive(Debug, Default)]
pub struct InMemoryCache {
    ohlcv: RwLock<HashMap<OhlcvKey, CacheEntry<DataFrame>>>,
    financials: RwLock<HashMap<FinancialsKey, CacheEntry<Vec<FinancialStatement>>>>,
    metrics: RwLock<HashMap<MetricsKey, CacheEntry<KeyMetrics>>>,
}

impl InMemoryCache {
    /// Create a new empty in-memory cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DataCache for InMemoryCache {
    #[instrument(skip(self), fields(provider = %provider, symbol = %symbol))]
    async fn get_ohlcv(
        &self,
        provider: &str,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Option<DataFrame>> {
        let key = OhlcvKey {
            provider: provider.to_string(),
            symbol: symbol.to_string(),
            start,
            end,
        };

        let cache = self.ohlcv.read().await;
        match cache.get(&key) {
            Some(entry) => {
                debug!("Cache hit for OHLCV data");
                Ok(Some(entry.data.clone()))
            }
            None => {
                debug!("Cache miss for OHLCV data");
                Ok(None)
            }
        }
    }

    #[instrument(skip(self, data), fields(provider = %provider, symbol = %symbol))]
    async fn put_ohlcv(&self, provider: &str, symbol: &Symbol, data: &DataFrame) -> Result<()> {
        let key = OhlcvKey {
            provider: provider.to_string(),
            symbol: symbol.to_string(),
            // Extract date range from DataFrame
            start: extract_min_date(data).unwrap_or(NaiveDate::MIN),
            end: extract_max_date(data).unwrap_or(NaiveDate::MAX),
        };

        let mut cache = self.ohlcv.write().await;
        cache.insert(key, CacheEntry::new(data.clone()));
        debug!("Cached {} OHLCV rows", data.height());
        Ok(())
    }

    #[instrument(skip(self), fields(provider = %provider, symbol = %symbol))]
    async fn get_financials(
        &self,
        provider: &str,
        symbol: &Symbol,
        period_type: PeriodType,
    ) -> Result<Option<Vec<FinancialStatement>>> {
        let key = FinancialsKey {
            provider: provider.to_string(),
            symbol: symbol.to_string(),
            period_type,
        };

        let cache = self.financials.read().await;
        match cache.get(&key) {
            Some(entry) => {
                debug!("Cache hit for financials");
                Ok(Some(entry.data.clone()))
            }
            None => {
                debug!("Cache miss for financials");
                Ok(None)
            }
        }
    }

    #[instrument(skip(self, statements), fields(provider = %provider, symbol = %symbol, count = statements.len()))]
    async fn put_financials(
        &self,
        provider: &str,
        symbol: &Symbol,
        statements: &[FinancialStatement],
    ) -> Result<()> {
        // Group statements by period type
        let mut quarterly: Vec<FinancialStatement> = Vec::new();
        let mut annual: Vec<FinancialStatement> = Vec::new();

        for stmt in statements {
            match stmt.period_type {
                PeriodType::Quarterly => quarterly.push(stmt.clone()),
                PeriodType::Annual => annual.push(stmt.clone()),
            }
        }

        let mut cache = self.financials.write().await;

        if !quarterly.is_empty() {
            let key = FinancialsKey {
                provider: provider.to_string(),
                symbol: symbol.to_string(),
                period_type: PeriodType::Quarterly,
            };
            cache.insert(key, CacheEntry::new(quarterly));
        }

        if !annual.is_empty() {
            let key = FinancialsKey {
                provider: provider.to_string(),
                symbol: symbol.to_string(),
                period_type: PeriodType::Annual,
            };
            cache.insert(key, CacheEntry::new(annual));
        }

        debug!("Cached {} financial statements", statements.len());
        Ok(())
    }

    #[instrument(skip(self), fields(provider = %provider, symbol = %symbol))]
    async fn get_metrics(
        &self,
        provider: &str,
        symbol: &Symbol,
        date: NaiveDate,
    ) -> Result<Option<KeyMetrics>> {
        let key = MetricsKey {
            provider: provider.to_string(),
            symbol: symbol.to_string(),
            date,
        };

        let cache = self.metrics.read().await;
        match cache.get(&key) {
            Some(entry) => {
                debug!("Cache hit for metrics");
                Ok(Some(entry.data.clone()))
            }
            None => {
                debug!("Cache miss for metrics");
                Ok(None)
            }
        }
    }

    #[instrument(skip(self, metrics), fields(provider = %provider, symbol = %symbol))]
    async fn put_metrics(
        &self,
        provider: &str,
        symbol: &Symbol,
        metrics: &KeyMetrics,
    ) -> Result<()> {
        let key = MetricsKey {
            provider: provider.to_string(),
            symbol: symbol.to_string(),
            date: metrics.date,
        };

        let mut cache = self.metrics.write().await;
        cache.insert(key, CacheEntry::new(metrics.clone()));
        debug!("Cached metrics");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn invalidate_stale(&self, ttl: Duration) -> Result<usize> {
        let mut total_removed = 0usize;

        // Invalidate stale OHLCV entries
        {
            let mut cache = self.ohlcv.write().await;
            let before = cache.len();
            cache.retain(|_, entry| !entry.is_stale(ttl));
            total_removed += before - cache.len();
        }

        // Invalidate stale financials entries
        {
            let mut cache = self.financials.write().await;
            let before = cache.len();
            cache.retain(|_, entry| !entry.is_stale(ttl));
            total_removed += before - cache.len();
        }

        // Invalidate stale metrics entries
        {
            let mut cache = self.metrics.write().await;
            let before = cache.len();
            cache.retain(|_, entry| !entry.is_stale(ttl));
            total_removed += before - cache.len();
        }

        if total_removed > 0 {
            debug!("Invalidated {} stale cache entries", total_removed);
        }

        Ok(total_removed)
    }

    #[instrument(skip(self))]
    async fn clear(&self) -> Result<()> {
        self.ohlcv.write().await.clear();
        self.financials.write().await.clear();
        self.metrics.write().await.clear();
        debug!("Cleared all cache entries");
        Ok(())
    }
}

/// Extract minimum date from a DataFrame's "date" column.
#[allow(dead_code)]
fn extract_min_date(df: &DataFrame) -> Option<NaiveDate> {
    let dates = df.column("date").ok()?;
    let dates = dates.date().ok()?;
    // Get the physical i32 representation and find min
    let physical = dates.0.clone();
    let min_days: i32 = ChunkAgg::min(&physical)?;
    // Polars dates are days since Unix epoch (1970-01-01)
    chrono::NaiveDate::from_num_days_from_ce_opt(min_days + 719_163)
}

/// Extract maximum date from a DataFrame's "date" column.
#[allow(dead_code)]
fn extract_max_date(df: &DataFrame) -> Option<NaiveDate> {
    let dates = df.column("date").ok()?;
    let dates = dates.date().ok()?;
    // Get the physical i32 representation and find max
    let physical = dates.0.clone();
    let max_days: i32 = ChunkAgg::max(&physical)?;
    // Polars dates are days since Unix epoch (1970-01-01)
    chrono::NaiveDate::from_num_days_from_ce_opt(max_days + 719_163)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use polars::prelude::*;

    #[tokio::test]
    async fn test_memory_cache_ohlcv() {
        let cache = InMemoryCache::new();
        let symbol = Symbol::new("AAPL");
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();

        // Initially no data
        let result = cache.get_ohlcv("test", &symbol, start, end).await.unwrap();
        assert!(result.is_none());

        // Create test DataFrame
        let df = DataFrame::new(vec![
            Column::new("symbol".into(), vec!["AAPL", "AAPL"]),
            Column::new("date".into(), vec!["2024-01-02", "2024-01-03"]),
            Column::new("open".into(), vec![150.0, 151.0]),
            Column::new("high".into(), vec![152.0, 153.0]),
            Column::new("low".into(), vec![149.0, 150.0]),
            Column::new("close".into(), vec![151.0, 152.0]),
            Column::new("volume".into(), vec![1000000.0, 1100000.0]),
        ])
        .unwrap();

        // Store data
        cache.put_ohlcv("test", &symbol, &df).await.unwrap();

        // Retrieve data - note we need the exact same key
        let result = cache
            .get_ohlcv("test", &symbol, NaiveDate::MIN, NaiveDate::MAX)
            .await
            .unwrap();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_memory_cache_metrics() {
        let cache = InMemoryCache::new();
        let symbol = Symbol::new("AAPL");
        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

        // Initially no data
        let result = cache.get_metrics("test", &symbol, date).await.unwrap();
        assert!(result.is_none());

        // Create test metrics
        let metrics = KeyMetrics {
            symbol: symbol.clone(),
            date,
            market_cap: Some(3_000_000_000_000.0),
            pe_ratio: Some(28.5),
            ..Default::default()
        };

        // Store data
        cache.put_metrics("test", &symbol, &metrics).await.unwrap();

        // Retrieve data
        let result = cache.get_metrics("test", &symbol, date).await.unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.market_cap, Some(3_000_000_000_000.0));
    }

    #[tokio::test]
    async fn test_memory_cache_clear() {
        let cache = InMemoryCache::new();
        let symbol = Symbol::new("AAPL");
        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

        // Store some data
        let metrics = KeyMetrics::new(symbol.clone(), date);
        cache.put_metrics("test", &symbol, &metrics).await.unwrap();

        // Clear cache
        cache.clear().await.unwrap();

        // Verify data is gone
        let result = cache.get_metrics("test", &symbol, date).await.unwrap();
        assert!(result.is_none());
    }
}
