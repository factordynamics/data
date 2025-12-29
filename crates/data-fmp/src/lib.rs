#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Financial Modeling Prep (FMP) data provider.
//!
//! This crate implements the data-core traits for the
//! [Financial Modeling Prep](https://financialmodelingprep.com/) API.
//!
//! # Usage
//!
//! ```rust,ignore
//! use data_fmp::FmpProvider;
//! use data_core::{PriceDataProvider, FundamentalDataProvider, Symbol, DataFrequency};
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let provider = FmpProvider::new("your_api_key");
//!
//!     // Fetch historical prices
//!     let symbol = Symbol::new("AAPL");
//!     let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
//!     let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
//!
//!     let prices = provider.fetch_ohlcv(&symbol, start, end, DataFrequency::Daily).await?;
//!
//!     // Fetch financials
//!     let financials = provider.fetch_financials(&symbol, data_core::PeriodType::Annual, Some(5)).await?;
//!
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use data_core::{
    CompanyInfo, DataError, DataFrequency, DataProvider, FinancialStatement,
    FundamentalDataProvider, KeyMetrics, PeriodType, PriceDataProvider, ReferenceDataProvider,
    Result, Symbol,
};
use polars::prelude::*;
use reqwest::Client;
use serde::Deserialize;
use std::fmt;

/// Base URL for the FMP stable API.
const FMP_BASE_URL: &str = "https://financialmodelingprep.com/stable";

/// Supported data frequencies for FMP.
const SUPPORTED_FREQUENCIES: &[DataFrequency] = &[DataFrequency::Daily];

/// Financial Modeling Prep data provider.
///
/// Provides access to:
/// - Historical daily prices
/// - Income statements, balance sheets, cash flow statements
/// - Key metrics and financial ratios
/// - Company profiles and reference data
#[derive(Clone)]
pub struct FmpProvider {
    client: Client,
    api_key: String,
}

impl fmt::Debug for FmpProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FmpProvider")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl FmpProvider {
    /// Create a new FMP provider with the given API key.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
        }
    }

    /// Create a new FMP provider with a custom HTTP client.
    #[must_use]
    pub fn with_client(client: Client, api_key: impl Into<String>) -> Self {
        Self {
            client,
            api_key: api_key.into(),
        }
    }

    /// Build a URL with the API key appended.
    fn url(&self, endpoint: &str) -> String {
        if endpoint.contains('?') {
            format!("{FMP_BASE_URL}/{endpoint}&apikey={}", self.api_key)
        } else {
            format!("{FMP_BASE_URL}/{endpoint}?apikey={}", self.api_key)
        }
    }

    /// Make a GET request and parse the JSON response.
    async fn get<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let url = self.url(endpoint);
        tracing::debug!("FMP request: {}", endpoint);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DataError::RateLimited {
                provider: "FMP".to_string(),
                retry_after: None,
            });
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(DataError::Network(format!("HTTP {status}: {text}")));
        }

        let text = response
            .text()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        // Check for FMP error responses
        if text.contains("\"Error Message\"") || text.contains("\"error\"") {
            return Err(DataError::Network(text));
        }

        serde_json::from_str(&text).map_err(|e| DataError::Parse(format!("{e}: {text}")))
    }

    /// Fetch income statements from FMP API.
    async fn fetch_income_statements(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FmpIncomeStatement>> {
        let period = match period_type {
            PeriodType::Annual => "annual",
            PeriodType::Quarterly => "quarter",
        };
        let limit_param = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
        let endpoint = format!(
            "income-statement?symbol={}&period={period}{limit_param}",
            symbol.as_str()
        );
        self.get(&endpoint).await
    }

    /// Fetch balance sheets from FMP API.
    async fn fetch_balance_sheets(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FmpBalanceSheet>> {
        let period = match period_type {
            PeriodType::Annual => "annual",
            PeriodType::Quarterly => "quarter",
        };
        let limit_param = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
        let endpoint = format!(
            "balance-sheet-statement?symbol={}&period={period}{limit_param}",
            symbol.as_str()
        );
        self.get(&endpoint).await
    }

    /// Fetch cash flow statements from FMP API.
    async fn fetch_cash_flows(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FmpCashFlow>> {
        let period = match period_type {
            PeriodType::Annual => "annual",
            PeriodType::Quarterly => "quarter",
        };
        let limit_param = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
        let endpoint = format!(
            "cash-flow-statement?symbol={}&period={period}{limit_param}",
            symbol.as_str()
        );
        self.get(&endpoint).await
    }

    /// Fetch key metrics from FMP API.
    async fn fetch_key_metrics_raw(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FmpKeyMetrics>> {
        let period = match period_type {
            PeriodType::Annual => "annual",
            PeriodType::Quarterly => "quarter",
        };
        let limit_param = limit.map(|l| format!("&limit={l}")).unwrap_or_default();
        let endpoint = format!(
            "key-metrics?symbol={}&period={period}{limit_param}",
            symbol.as_str()
        );
        self.get(&endpoint).await
    }

    /// Fetch company profile from FMP API.
    async fn fetch_profile(&self, symbol: &Symbol) -> Result<Vec<FmpProfile>> {
        let endpoint = format!("profile?symbol={}", symbol.as_str());
        self.get(&endpoint).await
    }

    /// Fetch historical daily prices from FMP API.
    async fn fetch_historical_prices(
        &self,
        symbol: &Symbol,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> Result<Vec<FmpHistoricalPrice>> {
        let mut params = String::new();
        if let Some(f) = from {
            params.push_str(&format!("&from={f}"));
        }
        if let Some(t) = to {
            params.push_str(&format!("&to={t}"));
        }

        let endpoint = format!(
            "historical-price-eod/full?symbol={}{}",
            symbol.as_str(),
            params
        );
        self.get(&endpoint).await
    }
}

impl DataProvider for FmpProvider {
    fn name(&self) -> &str {
        "FMP"
    }

    fn description(&self) -> &str {
        "Financial Modeling Prep - Financial data and stock market API"
    }

    fn supported_frequencies(&self) -> &[DataFrequency] {
        SUPPORTED_FREQUENCIES
    }
}

#[async_trait]
impl PriceDataProvider for FmpProvider {
    async fn fetch_ohlcv(
        &self,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame> {
        // FMP only supports daily data
        if frequency != DataFrequency::Daily {
            return Err(DataError::NotSupported(format!(
                "FMP only supports Daily frequency, got {:?}",
                frequency
            )));
        }

        let prices = self
            .fetch_historical_prices(symbol, Some(start), Some(end))
            .await?;

        if prices.is_empty() {
            return Err(DataError::DataNotAvailable {
                symbol: symbol.to_string(),
                start: start.to_string(),
                end: end.to_string(),
            });
        }

        // Convert to DataFrame
        let dates: Vec<String> = prices.iter().map(|p| p.date.clone()).collect();
        let opens: Vec<f64> = prices.iter().map(|p| p.open).collect();
        let highs: Vec<f64> = prices.iter().map(|p| p.high).collect();
        let lows: Vec<f64> = prices.iter().map(|p| p.low).collect();
        let closes: Vec<f64> = prices.iter().map(|p| p.close).collect();
        let adj_closes: Vec<f64> = prices.iter().map(|p| p.adj_close).collect();
        let volumes: Vec<f64> = prices.iter().map(|p| p.volume).collect();

        // Parse dates to NaiveDate
        let parsed_dates: Vec<i32> = dates
            .iter()
            .filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|d| d.num_days_from_ce())
            .collect();

        let df = DataFrame::new(vec![
            Column::new("date".into(), parsed_dates),
            Column::new("open".into(), opens),
            Column::new("high".into(), highs),
            Column::new("low".into(), lows),
            Column::new("close".into(), closes),
            Column::new("adj_close".into(), adj_closes),
            Column::new("volume".into(), volumes),
        ])
        .map_err(|e| DataError::Parse(e.to_string()))?;

        // Cast date column to proper date type and sort
        let df = df
            .lazy()
            .with_column(col("date").cast(DataType::Date))
            .sort(["date"], Default::default())
            .collect()
            .map_err(|e| DataError::Parse(e.to_string()))?;

        Ok(df)
    }
}

#[async_trait]
impl FundamentalDataProvider for FmpProvider {
    async fn fetch_financials(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FinancialStatement>> {
        // Fetch all three statement types in parallel
        let (income_result, balance_result, cash_result) = tokio::join!(
            self.fetch_income_statements(symbol, period_type, limit),
            self.fetch_balance_sheets(symbol, period_type, limit),
            self.fetch_cash_flows(symbol, period_type, limit),
        );

        let income_statements = income_result.unwrap_or_default();
        let balance_sheets = balance_result.unwrap_or_default();
        let cash_flows = cash_result.unwrap_or_default();

        // Merge data by date
        let mut statements: Vec<FinancialStatement> = Vec::new();

        for income in &income_statements {
            let date = match NaiveDate::parse_from_str(&income.date, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };

            let mut stmt = FinancialStatement::new(symbol.clone(), date, period_type);

            // Income statement fields
            stmt.revenue = Some(income.revenue);
            stmt.cost_of_revenue = Some(income.cost_of_revenue);
            stmt.gross_profit = Some(income.gross_profit);
            stmt.operating_expenses = Some(income.operating_expenses);
            stmt.operating_income = Some(income.operating_income);
            stmt.net_income = Some(income.net_income);
            stmt.ebitda = Some(income.ebitda);
            stmt.eps_basic = Some(income.eps);
            stmt.eps_diluted = Some(income.eps_diluted);
            stmt.shares_outstanding = Some(income.weighted_average_shs_out);

            // Find matching balance sheet
            if let Some(balance) = balance_sheets.iter().find(|b| b.date == income.date) {
                stmt.total_assets = Some(balance.total_assets);
                stmt.current_assets = Some(balance.total_current_assets);
                stmt.cash_and_equivalents = Some(balance.cash_and_cash_equivalents);
                stmt.total_liabilities = Some(balance.total_liabilities);
                stmt.current_liabilities = Some(balance.total_current_liabilities);
                stmt.total_debt = Some(balance.total_debt);
                stmt.stockholders_equity = Some(balance.total_stockholders_equity);
            }

            // Find matching cash flow statement
            if let Some(cash) = cash_flows.iter().find(|c| c.date == income.date) {
                stmt.operating_cash_flow = Some(cash.operating_cash_flow);
                stmt.capital_expenditures = Some(cash.capital_expenditure);
                stmt.free_cash_flow = Some(cash.free_cash_flow);
                stmt.dividends_paid = Some(cash.dividends_paid);
            }

            statements.push(stmt);
        }

        if statements.is_empty() {
            return Err(DataError::SymbolNotFound(symbol.to_string()));
        }

        Ok(statements)
    }

    async fn fetch_metrics(&self, symbol: &Symbol, date: NaiveDate) -> Result<KeyMetrics> {
        // Fetch metrics for the closest period
        let fmp_metrics = self
            .fetch_key_metrics_raw(symbol, PeriodType::Annual, Some(1))
            .await?;

        let metrics = fmp_metrics
            .first()
            .ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))?;

        let metrics_date = NaiveDate::parse_from_str(&metrics.date, "%Y-%m-%d").unwrap_or(date);

        let mut result = KeyMetrics::new(symbol.clone(), metrics_date);

        result.market_cap = Some(metrics.market_cap);
        result.enterprise_value = Some(metrics.enterprise_value);
        result.pe_ratio = Some(metrics.pe_ratio);
        result.pb_ratio = Some(metrics.pb_ratio);
        result.ps_ratio = Some(metrics.price_to_sales_ratio);
        result.roe = Some(metrics.roe);
        result.current_ratio = Some(metrics.current_ratio);
        result.debt_to_equity = Some(metrics.debt_to_equity);
        result.dividend_yield = Some(metrics.dividend_yield);

        Ok(result)
    }
}

#[async_trait]
impl ReferenceDataProvider for FmpProvider {
    async fn company_info(&self, symbol: &Symbol) -> Result<CompanyInfo> {
        let profiles = self.fetch_profile(symbol).await?;

        let profile = profiles
            .first()
            .ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))?;

        let mut info = CompanyInfo::new(
            symbol.clone(),
            &profile.company_name,
            &profile.exchange_short_name,
            &profile.sector,
            &profile.industry,
            &profile.country,
            &profile.currency,
        );

        if let Some(ref cik) = profile.cik {
            info = info.with_cik(cik);
        }

        if let Some(ref desc) = profile.description {
            info = info.with_description(desc);
        }

        Ok(info)
    }

    async fn universe(&self, universe_id: &str) -> Result<Vec<Symbol>> {
        // FMP supports various stock lists
        let endpoint = match universe_id {
            "sp500" => "sp500-constituent".to_string(),
            "nasdaq100" => "nasdaq-constituent".to_string(),
            "dowjones" => "dowjones-constituent".to_string(),
            _ => {
                return Err(DataError::InvalidParameter(format!(
                    "Unknown universe: {universe_id}. Supported: sp500, nasdaq100, dowjones"
                )));
            }
        };

        let constituents: Vec<FmpConstituent> = self.get(&endpoint).await?;

        Ok(constituents
            .into_iter()
            .map(|c| Symbol::new(c.symbol))
            .collect())
    }

    async fn supports_symbol(&self, symbol: &Symbol) -> Result<bool> {
        match self.fetch_profile(symbol).await {
            Ok(profiles) => Ok(!profiles.is_empty()),
            Err(DataError::SymbolNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

// ============================================================================
// FMP API Response Types
// ============================================================================

/// FMP Income Statement response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FmpIncomeStatement {
    date: String,
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    revenue: f64,
    #[serde(default)]
    cost_of_revenue: f64,
    #[serde(default)]
    gross_profit: f64,
    #[serde(default)]
    operating_expenses: f64,
    #[serde(default)]
    operating_income: f64,
    #[serde(default)]
    net_income: f64,
    #[serde(default)]
    eps: f64,
    #[serde(default)]
    eps_diluted: f64,
    #[serde(default)]
    weighted_average_shs_out: f64,
    #[serde(default)]
    ebitda: f64,
}

/// FMP Balance Sheet response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FmpBalanceSheet {
    date: String,
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    total_assets: f64,
    #[serde(default)]
    total_current_assets: f64,
    #[serde(default)]
    cash_and_cash_equivalents: f64,
    #[serde(default)]
    total_liabilities: f64,
    #[serde(default)]
    total_current_liabilities: f64,
    #[serde(default)]
    total_debt: f64,
    #[serde(default)]
    total_stockholders_equity: f64,
}

/// FMP Cash Flow Statement response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FmpCashFlow {
    date: String,
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    operating_cash_flow: f64,
    #[serde(default)]
    capital_expenditure: f64,
    #[serde(default)]
    free_cash_flow: f64,
    #[serde(default)]
    dividends_paid: f64,
}

/// FMP Key Metrics response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FmpKeyMetrics {
    date: String,
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    market_cap: f64,
    #[serde(default)]
    enterprise_value: f64,
    #[serde(default)]
    pe_ratio: f64,
    #[serde(default)]
    pb_ratio: f64,
    #[serde(default)]
    price_to_sales_ratio: f64,
    #[serde(default)]
    roe: f64,
    #[serde(default)]
    current_ratio: f64,
    #[serde(default)]
    debt_to_equity: f64,
    #[serde(default)]
    dividend_yield: f64,
}

/// FMP Company Profile response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FmpProfile {
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    company_name: String,
    #[serde(default)]
    exchange_short_name: String,
    #[serde(default)]
    sector: String,
    #[serde(default)]
    industry: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    currency: String,
    cik: Option<String>,
    description: Option<String>,
}

/// FMP Historical Price response.
#[derive(Debug, Clone, Deserialize)]
struct FmpHistoricalPrice {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[serde(rename = "adjClose", default)]
    adj_close: f64,
    #[serde(default)]
    volume: f64,
}

/// FMP Index Constituent response.
#[derive(Debug, Clone, Deserialize)]
struct FmpConstituent {
    symbol: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_building() {
        let provider = FmpProvider::new("test_key");
        assert_eq!(
            provider.url("quote?symbol=AAPL"),
            "https://financialmodelingprep.com/stable/quote?symbol=AAPL&apikey=test_key"
        );
        assert_eq!(
            provider.url("profile"),
            "https://financialmodelingprep.com/stable/profile?apikey=test_key"
        );
    }

    #[test]
    fn test_provider_metadata() {
        let provider = FmpProvider::new("test_key");
        assert_eq!(provider.name(), "FMP");
        assert!(!provider.description().is_empty());
        assert_eq!(provider.supported_frequencies(), &[DataFrequency::Daily]);
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = FmpProvider::new("secret_key_12345");
        let debug_str = format!("{:?}", provider);
        assert!(!debug_str.contains("secret_key_12345"));
        assert!(debug_str.contains("[REDACTED]"));
    }
}
