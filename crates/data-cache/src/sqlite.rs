//! SQLite-based cache implementation.

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use data_core::{DataCache, DataError, FinancialStatement, KeyMetrics, PeriodType, Result, Symbol};
use polars::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, instrument};

/// SQLite-based cache for market data.
///
/// This cache stores data in a SQLite database file, providing persistence across
/// application restarts. It uses `tokio::task::spawn_blocking` for async compatibility.
#[derive(Debug)]
pub struct SqliteCache {
    conn: Mutex<Connection>,
}

impl SqliteCache {
    /// Create a new SQLite cache at the given path.
    ///
    /// # Arguments
    /// * `path` - Path to the SQLite database file
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened or schema creation fails.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| DataError::Cache(e.to_string()))?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.initialize_schema()?;
        Ok(cache)
    }

    /// Create an in-memory SQLite cache.
    ///
    /// Useful for testing; data is lost when the cache is dropped.
    ///
    /// # Errors
    /// Returns an error if schema creation fails.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| DataError::Cache(e.to_string()))?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.initialize_schema()?;
        Ok(cache)
    }

    /// Initialize the database schema.
    fn initialize_schema(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        // OHLCV cache table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ohlcv_cache (
                provider TEXT NOT NULL,
                symbol TEXT NOT NULL,
                date TEXT NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume REAL NOT NULL,
                adjusted_close REAL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (provider, symbol, date)
            )",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ohlcv_provider_symbol_date
             ON ohlcv_cache(provider, symbol, date)",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        // Financials cache table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS financials_cache (
                provider TEXT NOT NULL,
                symbol TEXT NOT NULL,
                period_end TEXT NOT NULL,
                period_type TEXT NOT NULL,
                fiscal_year INTEGER,
                fiscal_quarter INTEGER,
                data_json TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (provider, symbol, period_end, period_type)
            )",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_financials_provider_symbol
             ON financials_cache(provider, symbol)",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        // Metrics cache table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS metrics_cache (
                provider TEXT NOT NULL,
                symbol TEXT NOT NULL,
                date TEXT NOT NULL,
                data_json TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (provider, symbol, date)
            )",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_metrics_provider_symbol
             ON metrics_cache(provider, symbol)",
            [],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        debug!("SQLite cache schema initialized");
        Ok(())
    }

    /// Convert period type to database string.
    fn period_type_to_str(pt: PeriodType) -> &'static str {
        match pt {
            PeriodType::Annual => "A",
            PeriodType::Quarterly => "Q",
        }
    }

    /// Convert database string to period type.
    #[allow(dead_code)]
    fn str_to_period_type(s: &str) -> Result<PeriodType> {
        match s {
            "A" => Ok(PeriodType::Annual),
            "Q" => Ok(PeriodType::Quarterly),
            _ => Err(DataError::Parse(format!("Invalid period type: {}", s))),
        }
    }
}

#[async_trait]
impl DataCache for SqliteCache {
    #[instrument(skip(self), fields(provider = %provider, symbol = %symbol))]
    async fn get_ohlcv(
        &self,
        provider: &str,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Option<DataFrame>> {
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();
        let start_str = start.to_string();
        let end_str = end.to_string();

        // Clone the connection for spawn_blocking
        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT symbol, date, open, high, low, close, volume, adjusted_close
                 FROM ohlcv_cache
                 WHERE provider = ?1 AND symbol = ?2 AND date >= ?3 AND date <= ?4
                 ORDER BY date ASC",
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let mut symbols = Vec::new();
        let mut dates = Vec::new();
        let mut opens = Vec::new();
        let mut highs = Vec::new();
        let mut lows = Vec::new();
        let mut closes = Vec::new();
        let mut volumes = Vec::new();
        let mut adj_closes: Vec<Option<f64>> = Vec::new();

        let rows = stmt
            .query_map(params![provider, symbol_str, start_str, end_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                ))
            })
            .map_err(|e| DataError::Cache(e.to_string()))?;

        for row in rows {
            let (sym, date, open, high, low, close, volume, adj_close) =
                row.map_err(|e| DataError::Cache(e.to_string()))?;
            symbols.push(sym);
            dates.push(date);
            opens.push(open);
            highs.push(high);
            lows.push(low);
            closes.push(close);
            volumes.push(volume);
            adj_closes.push(adj_close);
        }

        if dates.is_empty() {
            debug!("No cached OHLCV data found");
            return Ok(None);
        }

        debug!("Found {} cached OHLCV rows", dates.len());

        let df = DataFrame::new(vec![
            Column::new("symbol".into(), symbols),
            Column::new("date".into(), dates),
            Column::new("open".into(), opens),
            Column::new("high".into(), highs),
            Column::new("low".into(), lows),
            Column::new("close".into(), closes),
            Column::new("volume".into(), volumes),
            Column::new("adjusted_close".into(), adj_closes),
        ])
        .map_err(|e| DataError::Cache(e.to_string()))?;

        // Convert date strings to Date type
        let df = df
            .lazy()
            .with_column(col("date").cast(DataType::Date))
            .collect()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        Ok(Some(df))
    }

    #[instrument(skip(self, data), fields(provider = %provider, symbol = %symbol))]
    async fn put_ohlcv(&self, provider: &str, symbol: &Symbol, data: &DataFrame) -> Result<()> {
        let cached_at = Utc::now().to_rfc3339();
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();

        // Extract columns
        let symbols = data
            .column("symbol")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .str()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let dates = data
            .column("date")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .cast(&DataType::String)
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let dates = dates.str().map_err(|e| DataError::Cache(e.to_string()))?;
        let opens = data
            .column("open")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .f64()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let highs = data
            .column("high")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .f64()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let lows = data
            .column("low")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .f64()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let closes = data
            .column("close")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .f64()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let volumes = data
            .column("volume")
            .map_err(|e| DataError::Cache(e.to_string()))?
            .f64()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        // adjusted_close may be optional
        let adj_closes = data
            .column("adjusted_close")
            .ok()
            .and_then(|c| c.f64().ok());

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        for i in 0..data.height() {
            let sym = symbols.get(i).unwrap_or(&symbol_str);
            let date = dates
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing date".to_string()))?;
            let open = opens
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing open".to_string()))?;
            let high = highs
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing high".to_string()))?;
            let low = lows
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing low".to_string()))?;
            let close = closes
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing close".to_string()))?;
            let volume = volumes
                .get(i)
                .ok_or_else(|| DataError::Cache("Missing volume".to_string()))?;
            let adj_close = adj_closes.as_ref().and_then(|c| c.get(i));

            tx.execute(
                "INSERT OR REPLACE INTO ohlcv_cache
                 (provider, symbol, date, open, high, low, close, volume, adjusted_close, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    provider, sym, date, open, high, low, close, volume, adj_close, cached_at
                ],
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;
        }

        tx.commit().map_err(|e| DataError::Cache(e.to_string()))?;
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
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();
        let period_type_str = Self::period_type_to_str(period_type);

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT data_json FROM financials_cache
                 WHERE provider = ?1 AND symbol = ?2 AND period_type = ?3
                 ORDER BY period_end DESC",
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let rows = stmt
            .query_map(params![provider, symbol_str, period_type_str], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let mut statements = Vec::new();
        for row in rows {
            let json = row.map_err(|e| DataError::Cache(e.to_string()))?;
            let stmt: FinancialStatement =
                serde_json::from_str(&json).map_err(|e| DataError::Parse(e.to_string()))?;
            statements.push(stmt);
        }

        if statements.is_empty() {
            debug!("No cached financials found");
            return Ok(None);
        }

        debug!("Found {} cached financial statements", statements.len());
        Ok(Some(statements))
    }

    #[instrument(skip(self, statements), fields(provider = %provider, symbol = %symbol, count = statements.len()))]
    async fn put_financials(
        &self,
        provider: &str,
        symbol: &Symbol,
        statements: &[FinancialStatement],
    ) -> Result<()> {
        let cached_at = Utc::now().to_rfc3339();
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        for stmt in statements {
            let period_type_str = Self::period_type_to_str(stmt.period_type);
            let data_json =
                serde_json::to_string(stmt).map_err(|e| DataError::Parse(e.to_string()))?;

            tx.execute(
                "INSERT OR REPLACE INTO financials_cache
                 (provider, symbol, period_end, period_type, fiscal_year, fiscal_quarter, data_json, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    provider,
                    symbol_str,
                    stmt.period_end.to_string(),
                    period_type_str,
                    stmt.fiscal_year,
                    stmt.fiscal_quarter,
                    data_json,
                    cached_at
                ],
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;
        }

        tx.commit().map_err(|e| DataError::Cache(e.to_string()))?;
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
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();
        let date_str = date.to_string();

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let result = conn
            .query_row(
                "SELECT data_json FROM metrics_cache
                 WHERE provider = ?1 AND symbol = ?2 AND date = ?3",
                params![provider, symbol_str, date_str],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        match result {
            Some(json) => {
                let metrics: KeyMetrics =
                    serde_json::from_str(&json).map_err(|e| DataError::Parse(e.to_string()))?;
                debug!("Found cached metrics");
                Ok(Some(metrics))
            }
            None => {
                debug!("No cached metrics found");
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
        let cached_at = Utc::now().to_rfc3339();
        let provider = provider.to_string();
        let symbol_str = symbol.to_string();
        let date_str = metrics.date.to_string();
        let data_json =
            serde_json::to_string(metrics).map_err(|e| DataError::Parse(e.to_string()))?;

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        conn.execute(
            "INSERT OR REPLACE INTO metrics_cache
             (provider, symbol, date, data_json, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![provider, symbol_str, date_str, data_json, cached_at],
        )
        .map_err(|e| DataError::Cache(e.to_string()))?;

        debug!("Cached metrics");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn invalidate_stale(&self, ttl: Duration) -> Result<usize> {
        let cutoff = Utc::now()
            - chrono::Duration::from_std(ttl)
                .map_err(|e| DataError::Cache(format!("Invalid TTL duration: {}", e)))?;
        let cutoff_str = cutoff.to_rfc3339();

        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        let mut total_deleted = 0usize;

        // Delete stale OHLCV data
        let deleted = conn
            .execute(
                "DELETE FROM ohlcv_cache WHERE cached_at < ?1",
                params![cutoff_str],
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;
        total_deleted += deleted;

        // Delete stale financials
        let deleted = conn
            .execute(
                "DELETE FROM financials_cache WHERE cached_at < ?1",
                params![cutoff_str],
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;
        total_deleted += deleted;

        // Delete stale metrics
        let deleted = conn
            .execute(
                "DELETE FROM metrics_cache WHERE cached_at < ?1",
                params![cutoff_str],
            )
            .map_err(|e| DataError::Cache(e.to_string()))?;
        total_deleted += deleted;

        if total_deleted > 0 {
            debug!("Invalidated {} stale cache entries", total_deleted);
        }

        Ok(total_deleted)
    }

    #[instrument(skip(self))]
    async fn clear(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DataError::Cache(e.to_string()))?;

        conn.execute("DELETE FROM ohlcv_cache", [])
            .map_err(|e| DataError::Cache(e.to_string()))?;
        conn.execute("DELETE FROM financials_cache", [])
            .map_err(|e| DataError::Cache(e.to_string()))?;
        conn.execute("DELETE FROM metrics_cache", [])
            .map_err(|e| DataError::Cache(e.to_string()))?;

        debug!("Cleared all cache entries");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[tokio::test]
    async fn test_sqlite_cache_initialization() {
        let cache = SqliteCache::in_memory();
        assert!(cache.is_ok());
    }

    #[tokio::test]
    async fn test_ohlcv_cache() {
        let cache = SqliteCache::in_memory().unwrap();
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
            Column::new(
                "adjusted_close".into(),
                vec![Some(151.0), Some(152.0)] as Vec<Option<f64>>,
            ),
        ])
        .unwrap();

        // Store data
        cache.put_ohlcv("test", &symbol, &df).await.unwrap();

        // Retrieve data
        let result = cache.get_ohlcv("test", &symbol, start, end).await.unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.height(), 2);
    }

    #[tokio::test]
    async fn test_financials_cache() {
        let cache = SqliteCache::in_memory().unwrap();
        let symbol = Symbol::new("AAPL");

        // Initially no data
        let result = cache
            .get_financials("test", &symbol, PeriodType::Quarterly)
            .await
            .unwrap();
        assert!(result.is_none());

        // Create test financial statement
        let stmt = FinancialStatement {
            symbol: symbol.clone(),
            period_end: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            period_type: PeriodType::Quarterly,
            fiscal_year: Some(2024),
            fiscal_quarter: Some(1),
            revenue: Some(94_930_000_000.0),
            net_income: Some(14_736_000_000.0),
            ..Default::default()
        };

        // Store data
        cache
            .put_financials("test", &symbol, &[stmt])
            .await
            .unwrap();

        // Retrieve data
        let result = cache
            .get_financials("test", &symbol, PeriodType::Quarterly)
            .await
            .unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].fiscal_year, Some(2024));
    }

    #[tokio::test]
    async fn test_metrics_cache() {
        let cache = SqliteCache::in_memory().unwrap();
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
    async fn test_clear_cache() {
        let cache = SqliteCache::in_memory().unwrap();
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
