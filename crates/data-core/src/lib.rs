#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Core traits and types for quant data providers.
//!
//! This crate provides the foundational abstractions for working with financial data:
//!
//! - [`DataProvider`](provider::DataProvider) - Base trait for all providers
//! - [`PriceDataProvider`](provider::PriceDataProvider) - OHLCV price data
//! - [`FundamentalDataProvider`](provider::FundamentalDataProvider) - Financial statements and metrics
//! - [`TickDataProvider`](provider::TickDataProvider) - Tick-level market data
//! - [`ReferenceDataProvider`](provider::ReferenceDataProvider) - Company metadata
//! - [`DataCache`](cache::DataCache) - Caching abstraction

/// Cache trait and types for storing fetched data.
pub mod cache;
/// Error types for data operations.
pub mod error;
/// Data frequency and period type definitions.
pub mod frequency;
/// Provider traits for fetching market data.
pub mod provider;
/// Core data types (Symbol, OHLCV, Tick, etc.).
pub mod types;

// Re-export commonly used items at crate root
pub use cache::DataCache;
pub use error::{DataError, Result};
pub use frequency::{DataFrequency, PeriodType};
pub use provider::{
    DataProvider, FundamentalDataProvider, PriceDataProvider, ReferenceDataProvider,
    TickDataProvider,
};
pub use types::{CompanyInfo, FinancialStatement, KeyMetrics, OhlcvBar, Symbol, Tick, TickData};
