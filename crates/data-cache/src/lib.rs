#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/factordynamics/data/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Caching implementations for quant data providers.
//!
//! This crate provides implementations of the [`DataCache`] trait from `data-core`:
//!
//! - [`SqliteCache`] - Persistent SQLite-based cache (default, requires `sqlite` feature)
//! - [`InMemoryCache`] - Simple in-memory cache for testing
//! - [`NoopCache`] - No-op cache that doesn't store anything

/// In-memory cache implementation.
pub mod memory;
/// No-op cache implementation.
pub mod noop;

/// SQLite-based cache implementation.
#[cfg(feature = "sqlite")]
pub mod sqlite;

// Re-export the trait for convenience
pub use data_core::DataCache;

// Re-export implementations
pub use memory::InMemoryCache;
pub use noop::NoopCache;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteCache;
