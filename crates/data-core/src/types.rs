//! Core data types for financial market data.
//!
//! This module defines the fundamental data structures:
//!
//! - [`Symbol`] - Trading symbol/ticker
//! - [`OhlcvBar`] - OHLCV price bar
//! - [`Tick`] - Individual trade or quote
//! - [`TickData`] - Collection of ticks with helper methods
//! - [`FinancialStatement`] - Financial statement data
//! - [`KeyMetrics`] - Key financial metrics and ratios
//! - [`CompanyInfo`] - Company reference information

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::frequency::PeriodType;

/// A trading symbol/ticker.
///
/// Symbols are automatically uppercased on creation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(String);

impl Symbol {
    /// Creates a new symbol from a string, converting to uppercase.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into().to_uppercase())
    }

    /// Returns the symbol as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Symbol {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// OHLCV (Open, High, Low, Close, Volume) bar data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OhlcvBar {
    /// Timestamp of the bar.
    pub timestamp: DateTime<Utc>,
    /// Opening price.
    pub open: f64,
    /// Highest price during the period.
    pub high: f64,
    /// Lowest price during the period.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Trading volume.
    pub volume: f64,
    /// Split/dividend adjusted closing price.
    pub adjusted_close: Option<f64>,
}

impl OhlcvBar {
    /// Creates a new OHLCV bar.
    #[must_use]
    pub const fn new(
        timestamp: DateTime<Utc>,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) -> Self {
        Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
            adjusted_close: None,
        }
    }

    /// Sets the adjusted close price.
    #[must_use]
    pub const fn with_adjusted_close(mut self, adjusted_close: f64) -> Self {
        self.adjusted_close = Some(adjusted_close);
        self
    }
}

/// A single tick (trade or quote).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    /// Symbol for this tick.
    pub symbol: Symbol,
    /// Timestamp of the tick.
    pub timestamp: DateTime<Utc>,
    /// Trade/quote price.
    pub price: f64,
    /// Trade/quote size.
    pub size: f64,
    /// Exchange where the tick occurred.
    pub exchange: Option<String>,
    /// Trade conditions (e.g., "regular", "odd lot").
    pub conditions: Vec<String>,
}

impl Tick {
    /// Creates a new tick with required fields.
    #[must_use]
    pub const fn new(symbol: Symbol, timestamp: DateTime<Utc>, price: f64, size: f64) -> Self {
        Self {
            symbol,
            timestamp,
            price,
            size,
            exchange: None,
            conditions: Vec::new(),
        }
    }

    /// Sets the exchange for this tick.
    #[must_use]
    pub fn with_exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = Some(exchange.into());
        self
    }

    /// Sets the trade conditions for this tick.
    #[must_use]
    pub fn with_conditions(mut self, conditions: Vec<String>) -> Self {
        self.conditions = conditions;
        self
    }
}

/// Collection of tick data with helper methods.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TickData {
    ticks: Vec<Tick>,
}

impl TickData {
    /// Creates an empty tick collection.
    #[must_use]
    pub const fn new() -> Self {
        Self { ticks: Vec::new() }
    }

    /// Creates a tick collection from a vector of ticks.
    #[must_use]
    pub const fn from_ticks(ticks: Vec<Tick>) -> Self {
        Self { ticks }
    }

    /// Adds a tick to the collection.
    pub fn push(&mut self, tick: Tick) {
        self.ticks.push(tick);
    }

    /// Returns the number of ticks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    /// Returns true if there are no ticks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    /// Returns an iterator over the ticks.
    pub fn iter(&self) -> impl Iterator<Item = &Tick> {
        self.ticks.iter()
    }

    /// Consumes the collection and returns the underlying vector.
    #[must_use]
    pub fn into_inner(self) -> Vec<Tick> {
        self.ticks
    }

    /// Filters ticks to only those for a specific symbol.
    #[must_use]
    pub fn filter_by_symbol(&self, symbol: &Symbol) -> Self {
        Self {
            ticks: self
                .ticks
                .iter()
                .filter(|t| &t.symbol == symbol)
                .cloned()
                .collect(),
        }
    }

    /// Returns the time range covered by these ticks.
    #[must_use]
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        if self.ticks.is_empty() {
            return None;
        }
        let min = self.ticks.iter().map(|t| t.timestamp).min()?;
        let max = self.ticks.iter().map(|t| t.timestamp).max()?;
        Some((min, max))
    }

    /// Calculates the volume-weighted average price (VWAP).
    #[must_use]
    pub fn vwap(&self) -> Option<f64> {
        if self.ticks.is_empty() {
            return None;
        }
        let total_value: f64 = self.ticks.iter().map(|t| t.price * t.size).sum();
        let total_volume: f64 = self.ticks.iter().map(|t| t.size).sum();
        if total_volume == 0.0 {
            return None;
        }
        Some(total_value / total_volume)
    }
}

impl IntoIterator for TickData {
    type Item = Tick;
    type IntoIter = std::vec::IntoIter<Tick>;

    fn into_iter(self) -> Self::IntoIter {
        self.ticks.into_iter()
    }
}

impl FromIterator<Tick> for TickData {
    fn from_iter<I: IntoIterator<Item = Tick>>(iter: I) -> Self {
        Self {
            ticks: iter.into_iter().collect(),
        }
    }
}

/// Comprehensive financial statement data.
///
/// Contains balance sheet, income statement, and cash flow items.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FinancialStatement {
    /// Stock symbol.
    pub symbol: Symbol,
    /// End date of the reporting period.
    pub period_end: NaiveDate,
    /// Type of period (annual or quarterly).
    pub period_type: PeriodType,
    /// Fiscal year.
    pub fiscal_year: Option<i32>,
    /// Fiscal quarter (1-4).
    pub fiscal_quarter: Option<i32>,

    // Balance Sheet - Assets
    /// Total assets.
    pub total_assets: Option<f64>,
    /// Current assets.
    pub current_assets: Option<f64>,
    /// Cash and cash equivalents.
    pub cash_and_equivalents: Option<f64>,
    /// Inventory.
    pub inventory: Option<f64>,
    /// Accounts receivable.
    pub accounts_receivable: Option<f64>,

    // Balance Sheet - Liabilities
    /// Total liabilities.
    pub total_liabilities: Option<f64>,
    /// Current liabilities.
    pub current_liabilities: Option<f64>,
    /// Long-term debt.
    pub long_term_debt: Option<f64>,
    /// Short-term debt.
    pub short_term_debt: Option<f64>,
    /// Total debt.
    pub total_debt: Option<f64>,
    /// Accounts payable.
    pub accounts_payable: Option<f64>,

    // Balance Sheet - Equity
    /// Stockholders' equity.
    pub stockholders_equity: Option<f64>,

    // Income Statement
    /// Total revenue.
    pub revenue: Option<f64>,
    /// Cost of revenue (COGS).
    pub cost_of_revenue: Option<f64>,
    /// Gross profit.
    pub gross_profit: Option<f64>,
    /// Operating expenses.
    pub operating_expenses: Option<f64>,
    /// Operating income.
    pub operating_income: Option<f64>,
    /// Net income.
    pub net_income: Option<f64>,
    /// EBITDA.
    pub ebitda: Option<f64>,
    /// Basic earnings per share.
    pub eps_basic: Option<f64>,
    /// Diluted earnings per share.
    pub eps_diluted: Option<f64>,
    /// Interest expense.
    pub interest_expense: Option<f64>,

    // Cash Flow Statement
    /// Operating cash flow.
    pub operating_cash_flow: Option<f64>,
    /// Investing cash flow.
    pub investing_cash_flow: Option<f64>,
    /// Financing cash flow.
    pub financing_cash_flow: Option<f64>,
    /// Capital expenditures.
    pub capital_expenditures: Option<f64>,
    /// Free cash flow.
    pub free_cash_flow: Option<f64>,
    /// Dividends paid.
    pub dividends_paid: Option<f64>,

    // Shares
    /// Basic shares outstanding.
    pub shares_outstanding: Option<f64>,
    /// Diluted shares outstanding.
    pub shares_outstanding_diluted: Option<f64>,
}

impl FinancialStatement {
    /// Creates a new financial statement with required fields.
    #[must_use]
    pub fn new(symbol: Symbol, period_end: NaiveDate, period_type: PeriodType) -> Self {
        Self {
            symbol,
            period_end,
            period_type,
            ..Default::default()
        }
    }
}

/// Key financial metrics and ratios.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyMetrics {
    /// Stock symbol.
    pub symbol: Symbol,
    /// Date of the metrics.
    pub date: NaiveDate,

    // Valuation
    /// Market capitalization.
    pub market_cap: Option<f64>,
    /// Enterprise value.
    pub enterprise_value: Option<f64>,
    /// Price-to-earnings ratio.
    pub pe_ratio: Option<f64>,
    /// Forward price-to-earnings ratio.
    pub forward_pe: Option<f64>,
    /// Price-to-book ratio.
    pub pb_ratio: Option<f64>,
    /// Price-to-sales ratio.
    pub ps_ratio: Option<f64>,
    /// Price/earnings-to-growth ratio.
    pub peg_ratio: Option<f64>,
    /// EV/EBITDA ratio.
    pub ev_to_ebitda: Option<f64>,

    // Profitability
    /// Return on equity.
    pub roe: Option<f64>,
    /// Return on assets.
    pub roa: Option<f64>,
    /// Return on invested capital.
    pub roic: Option<f64>,
    /// Gross profit margin.
    pub gross_margin: Option<f64>,
    /// Operating profit margin.
    pub operating_margin: Option<f64>,
    /// Net profit margin.
    pub net_margin: Option<f64>,

    // Liquidity & Solvency
    /// Debt-to-equity ratio.
    pub debt_to_equity: Option<f64>,
    /// Current ratio.
    pub current_ratio: Option<f64>,
    /// Quick ratio.
    pub quick_ratio: Option<f64>,

    // Dividends
    /// Dividend yield.
    pub dividend_yield: Option<f64>,
    /// Dividend payout ratio.
    pub payout_ratio: Option<f64>,

    // Risk & Price
    /// Beta coefficient.
    pub beta: Option<f64>,
    /// 52-week high price.
    pub week_52_high: Option<f64>,
    /// 52-week low price.
    pub week_52_low: Option<f64>,
}

impl KeyMetrics {
    /// Creates new key metrics with required fields.
    #[must_use]
    pub fn new(symbol: Symbol, date: NaiveDate) -> Self {
        Self {
            symbol,
            date,
            ..Default::default()
        }
    }
}

/// Company reference information.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompanyInfo {
    /// Stock symbol.
    pub symbol: Symbol,
    /// Company name.
    pub name: String,
    /// Primary exchange.
    pub exchange: String,
    /// Business sector.
    pub sector: String,
    /// Industry within the sector.
    pub industry: String,
    /// Country of incorporation.
    pub country: String,
    /// Trading currency.
    pub currency: String,
    /// SEC CIK number.
    pub cik: Option<String>,
    /// Business description.
    pub description: Option<String>,
}

impl CompanyInfo {
    /// Creates new company info with required fields.
    #[must_use]
    pub fn new(
        symbol: Symbol,
        name: impl Into<String>,
        exchange: impl Into<String>,
        sector: impl Into<String>,
        industry: impl Into<String>,
        country: impl Into<String>,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            symbol,
            name: name.into(),
            exchange: exchange.into(),
            sector: sector.into(),
            industry: industry.into(),
            country: country.into(),
            currency: currency.into(),
            cik: None,
            description: None,
        }
    }

    /// Sets the SEC CIK number.
    #[must_use]
    pub fn with_cik(mut self, cik: impl Into<String>) -> Self {
        self.cik = Some(cik.into());
        self
    }

    /// Sets the business description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}
