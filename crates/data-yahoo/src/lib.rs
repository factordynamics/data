#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Yahoo Finance data provider.
//!
//! This crate provides a Yahoo Finance data provider that implements the
//! [`DataProvider`], [`PriceDataProvider`], and [`ReferenceDataProvider`]
//! traits from `data-core`.
//!
//! # Features
//!
//! - Fetch OHLCV data using Yahoo Finance's chart API
//! - Built-in rate limiting (1 request per second by default)
//! - Automatic adjusted close calculation
//! - Company info lookup
//! - Symbol validation
//!
//! # Example
//!
//! ```no_run
//! use data_yahoo::YahooProvider;
//! use data_core::{PriceDataProvider, Symbol, DataFrequency};
//! use chrono::NaiveDate;
//!
//! # async fn example() -> data_core::Result<()> {
//! let provider = YahooProvider::new();
//! let symbol = Symbol::new("AAPL");
//! let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
//! let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
//!
//! let df = provider.fetch_ohlcv(&symbol, start, end, DataFrequency::Daily).await?;
//! println!("Fetched {} rows", df.height());
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{NaiveDate, TimeZone, Utc};
use data_core::{
    CompanyInfo, DataError, DataFrequency, DataProvider, PriceDataProvider, ReferenceDataProvider,
    Result, Symbol,
};
use polars::prelude::*;
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Yahoo Finance chart API base URL.
const CHART_API_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";

/// Yahoo Finance quote summary API base URL.
const QUOTE_SUMMARY_URL: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary";

/// Default rate limit delay in milliseconds.
const DEFAULT_RATE_LIMIT_MS: u64 = 1000;

/// User agent for HTTP requests.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36";

/// Yahoo Finance data provider.
///
/// Implements [`DataProvider`], [`PriceDataProvider`], and [`ReferenceDataProvider`].
#[derive(Debug)]
pub struct YahooProvider {
    client: reqwest::Client,
    rate_limit_ms: u64,
    last_request_time: AtomicU64,
}

impl YahooProvider {
    /// Create a new Yahoo Finance provider with default settings.
    ///
    /// Uses built-in rate limiting of 1 request per second.
    #[must_use]
    pub fn new() -> Self {
        Self::with_rate_limit(Duration::from_millis(DEFAULT_RATE_LIMIT_MS))
    }

    /// Create a new Yahoo Finance provider with a custom HTTP client.
    ///
    /// Uses the provided client for all HTTP requests. Rate limiting
    /// is still applied.
    #[must_use]
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            rate_limit_ms: DEFAULT_RATE_LIMIT_MS,
            last_request_time: AtomicU64::new(0),
        }
    }

    /// Create a new Yahoo Finance provider with custom rate limiting.
    #[must_use]
    pub fn with_rate_limit(rate_limit: Duration) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            rate_limit_ms: rate_limit.as_millis() as u64,
            last_request_time: AtomicU64::new(0),
        }
    }

    /// Apply rate limiting before making a request.
    async fn apply_rate_limit(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let last = self.last_request_time.load(Ordering::Relaxed);
        let elapsed = now.saturating_sub(last);

        if elapsed < self.rate_limit_ms {
            let wait_time = self.rate_limit_ms - elapsed;
            debug!("Rate limiting: waiting {}ms", wait_time);
            sleep(Duration::from_millis(wait_time)).await;
        }

        self.last_request_time.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Ordering::Relaxed,
        );
    }

    /// Build the chart API URL for a symbol and date range.
    fn build_chart_url(
        &self,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> String {
        let start_ts = start
            .and_hms_opt(0, 0, 0)
            .map(|dt| Utc.from_utc_datetime(&dt).timestamp())
            .unwrap_or(0);

        let end_ts = end
            .and_hms_opt(23, 59, 59)
            .map(|dt| Utc.from_utc_datetime(&dt).timestamp())
            .unwrap_or(0);

        let interval = match frequency {
            DataFrequency::Minute => "1m",
            DataFrequency::FiveMinute => "5m",
            DataFrequency::FifteenMinute => "15m",
            DataFrequency::ThirtyMinute => "30m",
            DataFrequency::Hourly => "1h",
            DataFrequency::Daily => "1d",
            DataFrequency::Weekly => "1wk",
            DataFrequency::Monthly => "1mo",
            _ => "1d", // Default to daily for unsupported frequencies
        };

        format!(
            "{}/{}?period1={}&period2={}&interval={}&includeAdjustedClose=true",
            CHART_API_URL,
            symbol.as_str(),
            start_ts,
            end_ts,
            interval
        )
    }

    /// Parse Yahoo Finance chart response into a DataFrame.
    fn parse_chart_response(&self, symbol: &Symbol, response: ChartResponse) -> Result<DataFrame> {
        let result = response
            .chart
            .result
            .into_iter()
            .next()
            .ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))?;

        let timestamps = result.timestamp.unwrap_or_default();

        if timestamps.is_empty() {
            return Err(DataError::DataNotAvailable {
                symbol: symbol.to_string(),
                start: "N/A".to_string(),
                end: "N/A".to_string(),
            });
        }

        let quote = result
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or_else(|| DataError::Parse("Missing quote data".to_string()))?;

        let adj_close = result
            .indicators
            .adjclose
            .and_then(|ac| ac.into_iter().next())
            .map(|ac| ac.adjclose)
            .unwrap_or_default();

        // Convert timestamps to dates
        let dates: Vec<i32> = timestamps
            .iter()
            .map(|&ts| {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .map(|dt| dt.date_naive())
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            })
            .map(|d| (d - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days() as i32)
            .collect();

        let symbols: Vec<&str> = vec![symbol.as_str(); dates.len()];
        let opens: Vec<Option<f64>> = quote.open;
        let highs: Vec<Option<f64>> = quote.high;
        let lows: Vec<Option<f64>> = quote.low;
        let closes: Vec<Option<f64>> = quote.close;
        let volumes: Vec<Option<u64>> = quote.volume;

        // Pad adjusted close if needed
        let adj_closes: Vec<Option<f64>> = if adj_close.len() == dates.len() {
            adj_close
        } else {
            closes.clone()
        };

        let date_col = Column::new("date".into(), dates)
            .cast(&DataType::Date)
            .map_err(|e| DataError::Other(e.to_string()))?;

        let df = DataFrame::new(vec![
            Column::new("symbol".into(), symbols),
            date_col,
            Column::new("open".into(), opens),
            Column::new("high".into(), highs),
            Column::new("low".into(), lows),
            Column::new("close".into(), closes),
            Column::new("volume".into(), volumes),
            Column::new("adjusted_close".into(), adj_closes),
        ])
        .map_err(|e| DataError::Other(e.to_string()))?;

        Ok(df)
    }

    /// Fetch quote summary data for a symbol.
    async fn fetch_quote_summary(&self, symbol: &Symbol) -> Result<QuoteSummaryResponse> {
        self.apply_rate_limit().await;

        let url = format!(
            "{}/{}?modules=assetProfile,summaryDetail,defaultKeyStatistics",
            QUOTE_SUMMARY_URL,
            symbol.as_str()
        );

        debug!("Fetching quote summary: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DataError::RateLimited {
                provider: "Yahoo Finance".to_string(),
                retry_after: Some(Duration::from_secs(60)),
            });
        }

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(DataError::SymbolNotFound(symbol.to_string()));
        }

        if !response.status().is_success() {
            return Err(DataError::Network(format!(
                "HTTP {} for {}",
                response.status(),
                symbol
            )));
        }

        response
            .json::<QuoteSummaryResponse>()
            .await
            .map_err(|e| DataError::Parse(e.to_string()))
    }
}

impl Default for YahooProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DataProvider for YahooProvider {
    fn name(&self) -> &str {
        "Yahoo Finance"
    }

    fn description(&self) -> &str {
        "Yahoo Finance data provider for OHLCV and company information"
    }

    fn supported_frequencies(&self) -> &[DataFrequency] {
        &[
            DataFrequency::Minute,
            DataFrequency::FiveMinute,
            DataFrequency::FifteenMinute,
            DataFrequency::ThirtyMinute,
            DataFrequency::Hourly,
            DataFrequency::Daily,
            DataFrequency::Weekly,
            DataFrequency::Monthly,
        ]
    }
}

#[async_trait]
impl PriceDataProvider for YahooProvider {
    async fn fetch_ohlcv(
        &self,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame> {
        // Validate date range
        if start > end {
            return Err(DataError::InvalidParameter(format!(
                "Start date {} is after end date {}",
                start, end
            )));
        }

        // Apply rate limiting
        self.apply_rate_limit().await;

        let url = self.build_chart_url(symbol, start, end, frequency);
        debug!("Fetching OHLCV: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DataError::RateLimited {
                provider: "Yahoo Finance".to_string(),
                retry_after: Some(Duration::from_secs(60)),
            });
        }

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(DataError::SymbolNotFound(symbol.to_string()));
        }

        if !response.status().is_success() {
            return Err(DataError::Network(format!(
                "HTTP {} for {}",
                response.status(),
                symbol
            )));
        }

        let chart_response: ChartResponse = response
            .json()
            .await
            .map_err(|e| DataError::Parse(e.to_string()))?;

        // Check for API-level errors
        if let Some(error) = chart_response.chart.error {
            if error.code == "Not Found" {
                return Err(DataError::SymbolNotFound(symbol.to_string()));
            }
            return Err(DataError::Other(format!(
                "{}: {}",
                error.code, error.description
            )));
        }

        self.parse_chart_response(symbol, chart_response)
    }
}

#[async_trait]
impl ReferenceDataProvider for YahooProvider {
    async fn company_info(&self, symbol: &Symbol) -> Result<CompanyInfo> {
        let summary = self.fetch_quote_summary(symbol).await?;

        let result = summary
            .quote_summary
            .result
            .into_iter()
            .next()
            .ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))?;

        let profile = result.asset_profile.unwrap_or_default();

        Ok(CompanyInfo::new(
            symbol.clone(),
            profile
                .long_business_summary
                .as_deref()
                .unwrap_or("Unknown"),
            profile.exchange.as_deref().unwrap_or("Unknown"),
            profile.sector.as_deref().unwrap_or("Unknown"),
            profile.industry.as_deref().unwrap_or("Unknown"),
            profile.country.as_deref().unwrap_or("Unknown"),
            "USD", // Yahoo Finance primarily uses USD
        )
        .with_description(
            profile
                .long_business_summary
                .unwrap_or_else(|| "No description available".to_string()),
        ))
    }

    async fn universe(&self, universe_id: &str) -> Result<Vec<Symbol>> {
        // Yahoo Finance doesn't have a direct universe API
        // This could be extended to support index constituents
        warn!(
            "Universe lookup not directly supported by Yahoo Finance: {}",
            universe_id
        );
        Err(DataError::NotSupported(
            "Universe lookup is not supported by Yahoo Finance".to_string(),
        ))
    }

    async fn supports_symbol(&self, symbol: &Symbol) -> Result<bool> {
        // Try to fetch a small amount of data to validate the symbol
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(5);

        match self
            .fetch_ohlcv(symbol, start, end, DataFrequency::Daily)
            .await
        {
            Ok(_) => Ok(true),
            Err(DataError::SymbolNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

// ============================================================================
// Yahoo Finance API Response Types
// ============================================================================

/// Chart API response.
#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: ChartResult,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    result: Vec<ChartData>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ChartData {
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<QuoteData>,
    adjclose: Option<Vec<AdjClose>>,
}

#[derive(Debug, Deserialize)]
struct QuoteData {
    open: Vec<Option<f64>>,
    high: Vec<Option<f64>>,
    low: Vec<Option<f64>>,
    close: Vec<Option<f64>>,
    volume: Vec<Option<u64>>,
}

#[derive(Debug, Deserialize)]
struct AdjClose {
    adjclose: Vec<Option<f64>>,
}

/// Quote Summary API response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteSummaryResponse {
    quote_summary: QuoteSummaryResult,
}

#[derive(Debug, Deserialize)]
struct QuoteSummaryResult {
    result: Vec<QuoteSummaryData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteSummaryData {
    asset_profile: Option<AssetProfile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetProfile {
    sector: Option<String>,
    industry: Option<String>,
    country: Option<String>,
    exchange: Option<String>,
    long_business_summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_chart_url() {
        let provider = YahooProvider::new();
        let symbol = Symbol::new("AAPL");
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();

        let url = provider.build_chart_url(&symbol, start, end, DataFrequency::Daily);

        assert!(url.contains("AAPL"));
        assert!(url.contains("interval=1d"));
        assert!(url.contains("includeAdjustedClose=true"));
    }

    #[test]
    fn test_provider_info() {
        let provider = YahooProvider::new();

        assert_eq!(provider.name(), "Yahoo Finance");
        assert!(!provider.supported_frequencies().is_empty());
        assert!(
            provider
                .supported_frequencies()
                .contains(&DataFrequency::Daily)
        );
    }

    #[test]
    fn test_default() {
        let provider = YahooProvider::default();
        assert_eq!(provider.name(), "Yahoo Finance");
    }
}
