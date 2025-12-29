//! No-op cache implementation.

use async_trait::async_trait;
use chrono::NaiveDate;
use data_core::{DataCache, FinancialStatement, KeyMetrics, PeriodType, Result, Symbol};
use polars::prelude::DataFrame;
use std::time::Duration;
use tracing::trace;

/// A no-op cache that doesn't store anything.
///
/// All `get_*` methods return `Ok(None)` and all `put_*` methods return `Ok(())`.
/// Useful for disabling caching or testing code paths without cache hits.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopCache;

impl NoopCache {
    /// Create a new no-op cache.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DataCache for NoopCache {
    async fn get_ohlcv(
        &self,
        _provider: &str,
        _symbol: &Symbol,
        _start: NaiveDate,
        _end: NaiveDate,
    ) -> Result<Option<DataFrame>> {
        trace!("NoopCache: get_ohlcv called, returning None");
        Ok(None)
    }

    async fn put_ohlcv(&self, _provider: &str, _symbol: &Symbol, _data: &DataFrame) -> Result<()> {
        trace!("NoopCache: put_ohlcv called, doing nothing");
        Ok(())
    }

    async fn get_financials(
        &self,
        _provider: &str,
        _symbol: &Symbol,
        _period_type: PeriodType,
    ) -> Result<Option<Vec<FinancialStatement>>> {
        trace!("NoopCache: get_financials called, returning None");
        Ok(None)
    }

    async fn put_financials(
        &self,
        _provider: &str,
        _symbol: &Symbol,
        _statements: &[FinancialStatement],
    ) -> Result<()> {
        trace!("NoopCache: put_financials called, doing nothing");
        Ok(())
    }

    async fn get_metrics(
        &self,
        _provider: &str,
        _symbol: &Symbol,
        _date: NaiveDate,
    ) -> Result<Option<KeyMetrics>> {
        trace!("NoopCache: get_metrics called, returning None");
        Ok(None)
    }

    async fn put_metrics(
        &self,
        _provider: &str,
        _symbol: &Symbol,
        _metrics: &KeyMetrics,
    ) -> Result<()> {
        trace!("NoopCache: put_metrics called, doing nothing");
        Ok(())
    }

    async fn invalidate_stale(&self, _ttl: Duration) -> Result<usize> {
        trace!("NoopCache: invalidate_stale called, returning 0");
        Ok(0)
    }

    async fn clear(&self) -> Result<()> {
        trace!("NoopCache: clear called, doing nothing");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use polars::prelude::*;

    #[tokio::test]
    async fn test_noop_cache_get_returns_none() {
        let cache = NoopCache::new();
        let symbol = Symbol::new("AAPL");
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();

        // All get operations should return None
        assert!(
            cache
                .get_ohlcv("test", &symbol, start, end)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .get_financials("test", &symbol, PeriodType::Quarterly)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .get_metrics("test", &symbol, start)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_noop_cache_put_succeeds() {
        let cache = NoopCache::new();
        let symbol = Symbol::new("AAPL");

        // Create test DataFrame
        let df = DataFrame::new(vec![
            Column::new("symbol".into(), vec!["AAPL"]),
            Column::new("date".into(), vec!["2024-01-02"]),
            Column::new("open".into(), vec![150.0]),
            Column::new("high".into(), vec![152.0]),
            Column::new("low".into(), vec![149.0]),
            Column::new("close".into(), vec![151.0]),
            Column::new("volume".into(), vec![1000000.0]),
        ])
        .unwrap();

        // All put operations should succeed
        assert!(cache.put_ohlcv("test", &symbol, &df).await.is_ok());

        let stmt = FinancialStatement::new(
            symbol.clone(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            PeriodType::Quarterly,
        );
        assert!(cache.put_financials("test", &symbol, &[stmt]).await.is_ok());

        let metrics = KeyMetrics::new(
            symbol.clone(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        );
        assert!(cache.put_metrics("test", &symbol, &metrics).await.is_ok());
    }

    #[tokio::test]
    async fn test_noop_cache_management() {
        let cache = NoopCache::new();

        // invalidate_stale should return 0 (nothing to invalidate)
        let removed = cache
            .invalidate_stale(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(removed, 0);

        // clear should succeed
        assert!(cache.clear().await.is_ok());
    }

    #[test]
    fn test_noop_cache_is_copy() {
        let cache1 = NoopCache::new();
        let cache2 = cache1; // Copy
        let _cache3 = cache2; // Still works because Copy
    }
}
