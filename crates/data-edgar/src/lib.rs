#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! SEC EDGAR data provider for financial statements.
//!
//! This crate provides access to SEC EDGAR filings including:
//!
//! - CIK (Central Index Key) lookup from ticker symbols
//! - Company facts from the EDGAR API
//! - XBRL data parsing for financial metrics
//! - Financial statement extraction
//!
//! # Example
//!
//! ```no_run
//! use data_edgar::EdgarProvider;
//! use data_core::{FundamentalDataProvider, ReferenceDataProvider, Symbol, PeriodType};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let provider = EdgarProvider::new("MyApp/1.0 (contact@example.com)");
//!
//!     // Look up company info
//!     let symbol = Symbol::new("AAPL");
//!     let info = provider.company_info(&symbol).await?;
//!     println!("Company: {} (CIK: {:?})", info.name, info.cik);
//!
//!     // Fetch financial statements
//!     let statements = provider.fetch_financials(&symbol, PeriodType::Annual, Some(5)).await?;
//!     for stmt in statements {
//!         println!("Period: {} - Revenue: {:?}", stmt.period_end, stmt.revenue);
//!     }
//!
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use chrono::NaiveDate;
use data_core::{
    CompanyInfo, DataError, DataFrequency, DataProvider, FinancialStatement,
    FundamentalDataProvider, KeyMetrics, PeriodType, ReferenceDataProvider, Result, Symbol,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};
use tracing::{debug, warn};

/// SEC EDGAR API base URL
const EDGAR_BASE_URL: &str = "https://data.sec.gov";

/// SEC company tickers URL
const COMPANY_TICKERS_URL: &str = "https://www.sec.gov/files/company_tickers.json";

/// Default rate limit: 10 requests per second (SEC requirement)
const DEFAULT_RATE_LIMIT: Duration = Duration::from_millis(100);

/// Rate limiter to ensure we don't exceed SEC's rate limits
#[derive(Debug)]
struct RateLimiter {
    last_request: Instant,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            last_request: Instant::now() - min_interval,
            min_interval,
        }
    }

    async fn wait(&mut self) {
        let elapsed = self.last_request.elapsed();
        if elapsed < self.min_interval {
            sleep(self.min_interval - elapsed).await;
        }
        self.last_request = Instant::now();
    }
}

/// SEC EDGAR data provider.
///
/// Provides access to SEC EDGAR filings for fundamental data and company information.
/// Implements rate limiting per SEC requirements (max 10 requests/second).
#[derive(Debug)]
pub struct EdgarProvider {
    client: reqwest::Client,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    #[allow(dead_code)]
    user_agent: String,
}

impl EdgarProvider {
    /// Create a new EDGAR provider with the specified user agent.
    ///
    /// The SEC requires identifying user agent headers. Format should be:
    /// "AppName/Version (contact@email.com)"
    ///
    /// # Arguments
    /// * `user_agent` - User agent string identifying your application
    ///
    /// # Example
    /// ```
    /// use data_edgar::EdgarProvider;
    ///
    /// let provider = EdgarProvider::new("MyApp/1.0 (contact@example.com)");
    /// ```
    pub fn new(user_agent: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(DEFAULT_RATE_LIMIT))),
            user_agent: user_agent.to_string(),
        }
    }

    /// Create a new EDGAR provider with a custom HTTP client.
    ///
    /// # Arguments
    /// * `client` - Pre-configured reqwest client
    /// * `user_agent` - User agent string (for identification purposes)
    ///
    /// # Example
    /// ```
    /// use data_edgar::EdgarProvider;
    /// use std::time::Duration;
    ///
    /// let client = reqwest::Client::builder()
    ///     .timeout(Duration::from_secs(60))
    ///     .user_agent("MyApp/1.0 (contact@example.com)")
    ///     .build()
    ///     .unwrap();
    ///
    /// let provider = EdgarProvider::with_client(client, "MyApp/1.0 (contact@example.com)");
    /// ```
    pub fn with_client(client: reqwest::Client, user_agent: &str) -> Self {
        Self {
            client,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(DEFAULT_RATE_LIMIT))),
            user_agent: user_agent.to_string(),
        }
    }

    /// Look up a company's CIK number from its ticker symbol.
    ///
    /// # Arguments
    /// * `ticker` - Stock ticker symbol (e.g., "AAPL")
    ///
    /// # Returns
    /// The company's CIK number as a zero-padded 10-digit string
    pub async fn get_cik(&self, ticker: &str) -> Result<String> {
        if ticker.is_empty() {
            return Err(DataError::InvalidParameter("Empty ticker".to_string()));
        }

        let ticker_upper = ticker.to_uppercase();

        // Rate limit
        self.rate_limiter.lock().await.wait().await;

        debug!("Fetching company tickers from SEC");
        let response = self
            .client
            .get(COMPANY_TICKERS_URL)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(DataError::Network(format!(
                "Failed to fetch company tickers: HTTP {}",
                response.status()
            )));
        }

        let data: HashMap<String, CompanyTickerInfo> = response
            .json()
            .await
            .map_err(|e| DataError::Parse(format!("Failed to parse company tickers: {}", e)))?;

        // Search for ticker in the response
        for company in data.values() {
            if company.ticker.to_uppercase() == ticker_upper {
                // CIK should be zero-padded to 10 digits
                let cik = format!("{:0>10}", company.cik_str);
                debug!("Found CIK {} for ticker {}", cik, ticker);
                return Ok(cik);
            }
        }

        Err(DataError::SymbolNotFound(ticker.to_string()))
    }

    /// Fetch company facts from SEC EDGAR.
    ///
    /// # Arguments
    /// * `cik` - Company's CIK number (will be zero-padded)
    ///
    /// # Returns
    /// Company facts response containing all XBRL facts
    async fn fetch_company_facts(&self, cik: &str) -> Result<CompanyFactsResponse> {
        let cik_padded = format!("{:0>10}", cik);

        // Rate limit
        self.rate_limiter.lock().await.wait().await;

        let url = format!(
            "{}/api/xbrl/companyfacts/CIK{}.json",
            EDGAR_BASE_URL, cik_padded
        );

        debug!("Fetching company facts from {}", url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(DataError::Network(format!(
                "Failed to fetch company facts for CIK {}: HTTP {}",
                cik_padded,
                response.status()
            )));
        }

        let facts: CompanyFactsResponse = response
            .json()
            .await
            .map_err(|e| DataError::Parse(format!("Failed to parse company facts: {}", e)))?;

        Ok(facts)
    }

    /// Fetch company submissions/filings metadata.
    ///
    /// # Arguments
    /// * `cik` - Company's CIK number (will be zero-padded)
    async fn fetch_company_submissions(&self, cik: &str) -> Result<CompanySubmissions> {
        let cik_padded = format!("{:0>10}", cik);

        // Rate limit
        self.rate_limiter.lock().await.wait().await;

        let url = format!("{}/submissions/CIK{}.json", EDGAR_BASE_URL, cik_padded);

        debug!("Fetching company submissions from {}", url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DataError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(DataError::Network(format!(
                "Failed to fetch company submissions for CIK {}: HTTP {}",
                cik_padded,
                response.status()
            )));
        }

        let submissions: CompanySubmissions = response
            .json()
            .await
            .map_err(|e| DataError::Parse(format!("Failed to parse submissions: {}", e)))?;

        Ok(submissions)
    }

    /// Extract a fact value from company facts response.
    fn extract_fact(
        &self,
        facts: &CompanyFactsResponse,
        concept: &str,
        period_type: Option<PeriodType>,
        fiscal_year: Option<i32>,
        fiscal_period: Option<&str>,
    ) -> Option<f64> {
        let tags = get_xbrl_tags(concept)?;

        // Try US-GAAP taxonomy first, then DEI
        for taxonomy in ["us-gaap", "dei"] {
            if let Some(taxonomy_facts) = facts.facts.get(taxonomy) {
                for tag in &tags {
                    if let Some(tag_facts) = taxonomy_facts.get(*tag)
                        && let Some(units) = &tag_facts.units
                    {
                        // Try USD first for monetary values, then shares, then pure numbers
                        for unit_type in ["USD", "shares", "pure"] {
                            if let Some(values) = units.get(unit_type) {
                                // Filter by period type and fiscal period if specified
                                let filtered: Vec<&FactValue> = values
                                    .iter()
                                    .filter(|v| {
                                        // Filter by form type if period type is specified
                                        if let Some(pt) = period_type
                                            && let Some(form) = &v.form
                                        {
                                            match pt {
                                                PeriodType::Quarterly => {
                                                    if form != "10-Q" {
                                                        return false;
                                                    }
                                                }
                                                PeriodType::Annual => {
                                                    if form != "10-K" {
                                                        return false;
                                                    }
                                                }
                                            }
                                        }

                                        // Filter by fiscal year if specified
                                        if let Some(fy) = fiscal_year
                                            && v.fy != Some(fy)
                                        {
                                            return false;
                                        }

                                        // Filter by fiscal period if specified
                                        if let Some(fp) = fiscal_period
                                            && let Some(v_fp) = &v.fp
                                            && v_fp != fp
                                        {
                                            return false;
                                        }

                                        true
                                    })
                                    .collect();

                                // Return the most recent value
                                if let Some(fact) = filtered.last() {
                                    return Some(fact.val);
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Extract a single financial statement for a specific period.
    #[allow(clippy::too_many_arguments)]
    fn extract_statement(
        &self,
        facts: &CompanyFactsResponse,
        symbol: &Symbol,
        period_end: NaiveDate,
        period_type: PeriodType,
        fiscal_year: i32,
        fiscal_quarter: Option<i32>,
        fiscal_period: Option<&str>,
    ) -> FinancialStatement {
        let mut stmt = FinancialStatement::new(symbol.clone(), period_end, period_type);
        stmt.fiscal_year = Some(fiscal_year);
        stmt.fiscal_quarter = fiscal_quarter;

        // Balance Sheet - Assets
        stmt.total_assets = self.extract_fact(
            facts,
            "Assets",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.current_assets = self.extract_fact(
            facts,
            "AssetsCurrent",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.cash_and_equivalents = self.extract_fact(
            facts,
            "CashAndCashEquivalents",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.inventory = self.extract_fact(
            facts,
            "Inventory",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.accounts_receivable = self.extract_fact(
            facts,
            "AccountsReceivable",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        // Balance Sheet - Liabilities
        stmt.total_liabilities = self.extract_fact(
            facts,
            "Liabilities",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.current_liabilities = self.extract_fact(
            facts,
            "LiabilitiesCurrent",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.long_term_debt = self.extract_fact(
            facts,
            "LongTermDebt",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.short_term_debt = self.extract_fact(
            facts,
            "ShortTermDebt",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.accounts_payable = self.extract_fact(
            facts,
            "AccountsPayable",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        // Calculate total debt if not directly available
        stmt.total_debt = match (stmt.long_term_debt, stmt.short_term_debt) {
            (Some(long), Some(short)) => Some(long + short),
            (Some(long), None) => Some(long),
            (None, Some(short)) => Some(short),
            _ => self.extract_fact(
                facts,
                "TotalDebt",
                Some(period_type),
                Some(fiscal_year),
                fiscal_period,
            ),
        };

        // Balance Sheet - Equity
        stmt.stockholders_equity = self.extract_fact(
            facts,
            "StockholdersEquity",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        // Income Statement
        stmt.revenue = self.extract_fact(
            facts,
            "Revenue",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.cost_of_revenue = self.extract_fact(
            facts,
            "CostOfRevenue",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.gross_profit = self.extract_fact(
            facts,
            "GrossProfit",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.operating_income = self.extract_fact(
            facts,
            "OperatingIncome",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.net_income = self.extract_fact(
            facts,
            "NetIncome",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.ebitda = self.extract_fact(
            facts,
            "EBITDA",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.eps_basic = self.extract_fact(
            facts,
            "EarningsPerShareBasic",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.eps_diluted = self.extract_fact(
            facts,
            "EarningsPerShareDiluted",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.interest_expense = self.extract_fact(
            facts,
            "InterestExpense",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        // Cash Flow Statement
        stmt.operating_cash_flow = self.extract_fact(
            facts,
            "OperatingCashFlow",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.investing_cash_flow = self.extract_fact(
            facts,
            "InvestingCashFlow",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.financing_cash_flow = self.extract_fact(
            facts,
            "FinancingCashFlow",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.capital_expenditures = self.extract_fact(
            facts,
            "CapitalExpenditures",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );
        stmt.dividends_paid = self.extract_fact(
            facts,
            "DividendsPaid",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        // Calculate free cash flow if both components are available
        stmt.free_cash_flow = match (stmt.operating_cash_flow, stmt.capital_expenditures) {
            (Some(ocf), Some(capex)) => Some(ocf - capex.abs()),
            _ => None,
        };

        // Shares
        stmt.shares_outstanding = self
            .extract_fact(
                facts,
                "SharesOutstanding",
                Some(period_type),
                Some(fiscal_year),
                fiscal_period,
            )
            .or_else(|| {
                self.extract_fact(
                    facts,
                    "WeightedAverageSharesOutstandingBasic",
                    Some(period_type),
                    Some(fiscal_year),
                    fiscal_period,
                )
            });
        stmt.shares_outstanding_diluted = self.extract_fact(
            facts,
            "WeightedAverageSharesOutstandingDiluted",
            Some(period_type),
            Some(fiscal_year),
            fiscal_period,
        );

        stmt
    }
}

impl DataProvider for EdgarProvider {
    fn name(&self) -> &str {
        "SEC EDGAR"
    }

    fn description(&self) -> &str {
        "SEC EDGAR data provider for fundamental financial data from 10-K and 10-Q filings"
    }

    fn supported_frequencies(&self) -> &[DataFrequency] {
        &[DataFrequency::Quarterly, DataFrequency::Annual]
    }
}

#[async_trait]
impl FundamentalDataProvider for EdgarProvider {
    async fn fetch_financials(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FinancialStatement>> {
        // First, get the CIK for this symbol
        let cik = self.get_cik(symbol.as_str()).await?;

        // Fetch company facts
        let facts = self.fetch_company_facts(&cik).await?;

        let mut statements = Vec::new();

        // Extract unique periods from the facts
        let mut periods: HashMap<(i32, String, String), (NaiveDate, String)> = HashMap::new();

        // Scan through all facts to find unique periods
        for taxonomy_facts in facts.facts.values() {
            for tag_facts in taxonomy_facts.values() {
                if let Some(units) = &tag_facts.units {
                    for values in units.values() {
                        for value in values {
                            if let (Some(fy), Some(fp), Some(form)) =
                                (&value.fy, &value.fp, &value.form)
                            {
                                // Filter by period type
                                let matches_period = match period_type {
                                    PeriodType::Annual => form == "10-K",
                                    PeriodType::Quarterly => form == "10-Q",
                                };

                                if matches_period {
                                    if let Ok(end_date) =
                                        NaiveDate::parse_from_str(&value.end, "%Y-%m-%d")
                                    {
                                        periods.insert(
                                            (*fy, fp.clone(), form.clone()),
                                            (end_date, form.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Extract financial statement for each period
        for ((fy, fp, form), (end_date, _)) in periods {
            let pt = match form.as_str() {
                "10-K" => PeriodType::Annual,
                "10-Q" => PeriodType::Quarterly,
                _ => continue,
            };

            let fiscal_quarter = if pt == PeriodType::Quarterly {
                fp.chars()
                    .nth(1)
                    .and_then(|c| c.to_digit(10))
                    .map(|d| d as i32)
            } else {
                None
            };

            let stmt =
                self.extract_statement(&facts, symbol, end_date, pt, fy, fiscal_quarter, Some(&fp));

            statements.push(stmt);
        }

        // Sort by period end date (most recent first)
        statements.sort_by(|a, b| b.period_end.cmp(&a.period_end));

        // Apply limit if specified
        if let Some(limit) = limit {
            statements.truncate(limit);
        }

        Ok(statements)
    }

    async fn fetch_metrics(&self, symbol: &Symbol, date: NaiveDate) -> Result<KeyMetrics> {
        // Fetch the most recent financial statement to compute metrics
        let statements = self
            .fetch_financials(symbol, PeriodType::Annual, Some(1))
            .await?;

        let stmt = statements
            .first()
            .ok_or_else(|| DataError::DataNotAvailable {
                symbol: symbol.to_string(),
                start: date.to_string(),
                end: date.to_string(),
            })?;

        let mut metrics = KeyMetrics::new(symbol.clone(), date);

        // Compute metrics from financial statement
        // Note: Market cap and valuation ratios require price data which EDGAR doesn't provide

        // Profitability ratios
        if let (Some(ni), Some(eq)) = (stmt.net_income, stmt.stockholders_equity) {
            if eq > 0.0 {
                metrics.roe = Some(ni / eq);
            }
        }

        if let (Some(ni), Some(assets)) = (stmt.net_income, stmt.total_assets) {
            if assets > 0.0 {
                metrics.roa = Some(ni / assets);
            }
        }

        if let (Some(gross_profit), Some(revenue)) = (stmt.gross_profit, stmt.revenue) {
            if revenue > 0.0 {
                metrics.gross_margin = Some(gross_profit / revenue);
            }
        }

        if let (Some(op_income), Some(revenue)) = (stmt.operating_income, stmt.revenue) {
            if revenue > 0.0 {
                metrics.operating_margin = Some(op_income / revenue);
            }
        }

        if let (Some(ni), Some(revenue)) = (stmt.net_income, stmt.revenue) {
            if revenue > 0.0 {
                metrics.net_margin = Some(ni / revenue);
            }
        }

        // Liquidity ratios
        if let (Some(debt), Some(equity)) = (stmt.long_term_debt, stmt.stockholders_equity) {
            if equity > 0.0 {
                metrics.debt_to_equity = Some(debt / equity);
            }
        }

        if let (Some(ca), Some(cl)) = (stmt.current_assets, stmt.current_liabilities) {
            if cl > 0.0 {
                metrics.current_ratio = Some(ca / cl);
            }
        }

        // Quick ratio: (Current Assets - Inventory) / Current Liabilities
        if let (Some(ca), Some(cl)) = (stmt.current_assets, stmt.current_liabilities) {
            if cl > 0.0 {
                let inventory = stmt.inventory.unwrap_or(0.0);
                metrics.quick_ratio = Some((ca - inventory) / cl);
            }
        }

        Ok(metrics)
    }
}

#[async_trait]
impl ReferenceDataProvider for EdgarProvider {
    async fn company_info(&self, symbol: &Symbol) -> Result<CompanyInfo> {
        // Get CIK first
        let cik = self.get_cik(symbol.as_str()).await?;

        // Fetch company submissions for metadata
        let submissions = self.fetch_company_submissions(&cik).await?;

        // Get SIC description as industry (SEC uses SIC codes)
        let industry = submissions
            .sic_description
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        // Create company info
        let info = CompanyInfo::new(
            symbol.clone(),
            &submissions.name,
            submissions.exchanges.first().cloned().unwrap_or_default(),
            "Unknown", // SEC doesn't provide sector classification
            industry,
            "US", // EDGAR is US-only
            "USD",
        )
        .with_cik(&cik);

        Ok(info)
    }

    async fn universe(&self, _universe_id: &str) -> Result<Vec<Symbol>> {
        // EDGAR doesn't have pre-defined universes
        // Could potentially return all tickers from company_tickers.json
        warn!("EDGAR provider does not support pre-defined universes");
        Err(DataError::NotSupported(
            "EDGAR does not support pre-defined universes".to_string(),
        ))
    }

    async fn supports_symbol(&self, symbol: &Symbol) -> Result<bool> {
        // Check if we can find a CIK for this symbol
        match self.get_cik(symbol.as_str()).await {
            Ok(_) => Ok(true),
            Err(DataError::SymbolNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

// =============================================================================
// XBRL Tag Mappings
// =============================================================================

/// Get possible XBRL tags for a concept.
///
/// Different companies may use different XBRL tags for the same concept.
/// This function returns all possible tags for a given concept.
fn get_xbrl_tags(concept: &str) -> Option<Vec<&'static str>> {
    match concept {
        // Assets
        "Assets" => Some(vec!["Assets"]),
        "AssetsCurrent" => Some(vec!["AssetsCurrent"]),
        "CashAndCashEquivalents" => Some(vec![
            "CashAndCashEquivalentsAtCarryingValue",
            "Cash",
            "CashCashEquivalentsAndShortTermInvestments",
        ]),
        "Inventory" => Some(vec!["InventoryNet", "Inventories"]),
        "AccountsReceivable" => Some(vec![
            "AccountsReceivableNetCurrent",
            "AccountsReceivableNet",
            "ReceivablesNetCurrent",
        ]),

        // Liabilities
        "Liabilities" => Some(vec!["Liabilities", "LiabilitiesAndStockholdersEquity"]),
        "LiabilitiesCurrent" => Some(vec!["LiabilitiesCurrent"]),
        "LongTermDebt" => Some(vec![
            "LongTermDebt",
            "LongTermDebtNoncurrent",
            "LongTermDebtAndCapitalLeaseObligations",
        ]),
        "ShortTermDebt" => Some(vec![
            "ShortTermBorrowings",
            "DebtCurrent",
            "CurrentPortionOfLongTermDebt",
        ]),
        "TotalDebt" => Some(vec!["Debt", "TotalDebt"]),
        "AccountsPayable" => Some(vec![
            "AccountsPayableCurrent",
            "AccountsPayableAndAccruedLiabilitiesCurrent",
        ]),

        // Equity
        "StockholdersEquity" => Some(vec![
            "StockholdersEquity",
            "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest",
        ]),

        // Revenue
        "Revenue" => Some(vec![
            "Revenues",
            "RevenueFromContractWithCustomerExcludingAssessedTax",
            "SalesRevenueNet",
            "RevenueFromContractWithCustomerIncludingAssessedTax",
        ]),
        "CostOfRevenue" => Some(vec![
            "CostOfRevenue",
            "CostOfGoodsAndServicesSold",
            "CostOfGoodsSold",
        ]),
        "GrossProfit" => Some(vec!["GrossProfit"]),

        // Operating Income
        "OperatingIncome" => Some(vec![
            "OperatingIncomeLoss",
            "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest",
        ]),
        "OperatingExpenses" => Some(vec!["OperatingExpenses"]),

        // Net Income
        "NetIncome" => Some(vec![
            "NetIncomeLoss",
            "ProfitLoss",
            "NetIncomeLossAvailableToCommonStockholdersBasic",
        ]),

        // EBITDA
        "EBITDA" => Some(vec![
            "EarningsBeforeInterestTaxesDepreciationAndAmortization",
        ]),

        // EPS
        "EarningsPerShareBasic" => Some(vec!["EarningsPerShareBasic"]),
        "EarningsPerShareDiluted" => Some(vec!["EarningsPerShareDiluted"]),

        // Interest
        "InterestExpense" => Some(vec!["InterestExpense", "InterestPaid"]),

        // Cash Flow
        "OperatingCashFlow" => Some(vec![
            "NetCashProvidedByUsedInOperatingActivities",
            "CashProvidedByUsedInOperatingActivities",
        ]),
        "InvestingCashFlow" => Some(vec!["NetCashProvidedByUsedInInvestingActivities"]),
        "FinancingCashFlow" => Some(vec!["NetCashProvidedByUsedInFinancingActivities"]),
        "CapitalExpenditures" => Some(vec![
            "PaymentsToAcquirePropertyPlantAndEquipment",
            "PaymentsForCapitalImprovements",
            "CapitalExpendituresIncurredButNotYetPaid",
        ]),
        "DividendsPaid" => Some(vec![
            "PaymentsOfDividends",
            "PaymentsOfDividendsCommonStock",
        ]),

        // Shares
        "SharesOutstanding" => Some(vec![
            "CommonStockSharesOutstanding",
            "CommonStockSharesIssued",
        ]),
        "WeightedAverageSharesOutstandingBasic" => {
            Some(vec!["WeightedAverageNumberOfSharesOutstandingBasic"])
        }
        "WeightedAverageSharesOutstandingDiluted" => {
            Some(vec!["WeightedAverageNumberOfDilutedSharesOutstanding"])
        }

        _ => None,
    }
}

// =============================================================================
// SEC API Response Types
// =============================================================================

/// Company ticker information from SEC JSON.
#[derive(Debug, Deserialize)]
struct CompanyTickerInfo {
    /// CIK as a number (SEC returns this as an integer)
    cik_str: u64,
    /// Ticker symbol
    ticker: String,
    /// Company name
    #[allow(dead_code)]
    title: String,
}

/// Response from the SEC EDGAR Company Facts API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompanyFactsResponse {
    /// CIK number
    #[allow(dead_code)]
    cik: u64,
    /// Entity name
    #[allow(dead_code)]
    entity_name: String,
    /// Facts organized by taxonomy and tag
    facts: HashMap<String, HashMap<String, TagFacts>>,
}

/// Facts for a specific XBRL tag.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TagFacts {
    /// Label/description
    label: String,
    /// Description
    description: Option<String>,
    /// Units (USD, shares, etc.) containing the actual fact values
    units: Option<HashMap<String, Vec<FactValue>>>,
}

/// A single fact value with metadata.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct FactValue {
    /// End date of the period
    end: String,
    /// Value
    val: f64,
    /// Accession number
    #[serde(default)]
    accn: Option<String>,
    /// Fiscal year
    #[serde(default)]
    fy: Option<i32>,
    /// Fiscal period
    #[serde(default)]
    fp: Option<String>,
    /// Form type
    #[serde(default)]
    form: Option<String>,
    /// Filed date
    #[serde(default)]
    filed: Option<String>,
    /// Frame (instant or duration)
    #[serde(default)]
    frame: Option<String>,
}

/// Company submissions/filings metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompanySubmissions {
    /// Company name
    name: String,
    /// List of exchanges
    #[serde(default)]
    exchanges: Vec<String>,
    /// SIC code
    #[serde(default)]
    #[allow(dead_code)]
    sic: Option<String>,
    /// SIC description
    #[serde(default)]
    sic_description: Option<String>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_xbrl_tags() {
        assert!(get_xbrl_tags("Assets").is_some());
        assert!(get_xbrl_tags("Revenue").is_some());
        assert!(get_xbrl_tags("NetIncome").is_some());
        assert!(get_xbrl_tags("NonexistentConcept").is_none());
    }

    #[test]
    fn test_provider_traits() {
        let provider = EdgarProvider::new("Test/1.0 (test@example.com)");

        assert_eq!(provider.name(), "SEC EDGAR");
        assert!(!provider.description().is_empty());
        assert!(
            provider
                .supported_frequencies()
                .contains(&DataFrequency::Annual)
        );
        assert!(
            provider
                .supported_frequencies()
                .contains(&DataFrequency::Quarterly)
        );
    }

    #[test]
    fn test_cik_padding() {
        let cik = "320193";
        let padded = format!("{:0>10}", cik);
        assert_eq!(padded, "0000320193");
        assert_eq!(padded.len(), 10);
    }

    #[test]
    fn test_symbol_creation() {
        let symbol = Symbol::new("AAPL");
        assert_eq!(symbol.as_str(), "AAPL");

        let symbol_lower = Symbol::new("aapl");
        assert_eq!(symbol_lower.as_str(), "AAPL");
    }
}
