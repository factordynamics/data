#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Unified data provider interface for quantitative finance.
//!
//! This crate provides a unified interface for fetching financial data from
//! multiple providers. It re-exports core types and provider implementations,
//! and provides a [`DataProviderRegistry`] for managing multiple providers
//! with automatic fallback behavior.
//!
//! # Features
//!
//! - `yahoo` - Yahoo Finance provider for price and reference data
//! - `edgar` - SEC EDGAR provider for fundamental data
//! - `fmp` - Financial Modeling Prep provider
//! - `nasdaq` - NASDAQ tick data provider
//! - `ibkr` - Interactive Brokers provider
//! - `cache-sqlite` - SQLite-based caching
//!
//! # Example
//!
//! ```rust,ignore
//! use data::{DataProviderRegistry, Symbol, DataFrequency};
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> data::Result<()> {
//!     let registry = DataProviderRegistry::new()
//!         .with_yahoo();
//!
//!     let symbol = Symbol::new("AAPL");
//!     let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
//!     let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
//!
//!     let ohlcv = registry.fetch_ohlcv(&symbol, start, end, DataFrequency::Daily).await?;
//!     println!("{:?}", ohlcv);
//!
//!     Ok(())
//! }
//! ```

// Core types and traits
pub use data_core::*;

// Cache implementations
#[cfg(feature = "cache-sqlite")]
pub use data_cache::SqliteCache;
pub use data_cache::{InMemoryCache, NoopCache};

// Providers
#[cfg(feature = "edgar")]
pub use data_edgar::EdgarProvider;
#[cfg(feature = "fmp")]
pub use data_fmp::FmpProvider;
#[cfg(feature = "ibkr")]
pub use data_ibkr::IbkrProvider;
#[cfg(feature = "nasdaq")]
pub use data_nasdaq::NasdaqProvider;
#[cfg(feature = "yahoo")]
pub use data_yahoo::YahooProvider;

mod registry;
pub use registry::DataProviderRegistry;
