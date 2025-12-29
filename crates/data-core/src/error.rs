//! Error types for data operations.
//!
//! This module defines [`DataError`] which covers all error cases that can occur
//! when fetching, parsing, or caching financial data.

use thiserror::Error;

/// Errors that can occur during data operations.
#[derive(Error, Debug)]
pub enum DataError {
    /// Network-related errors (connection failures, timeouts, etc.).
    #[error("Network error: {0}")]
    Network(String),

    /// Rate limit exceeded by a provider.
    #[error("Rate limited by {provider}: retry after {retry_after:?}")]
    RateLimited {
        /// The provider that rate limited the request.
        provider: String,
        /// Suggested time to wait before retrying.
        retry_after: Option<std::time::Duration>,
    },

    /// The requested symbol was not found.
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    /// Data is not available for the requested symbol and date range.
    #[error("Data not available for {symbol} in range {start} to {end}")]
    DataNotAvailable {
        /// The symbol that was requested.
        symbol: String,
        /// Start of the requested date range.
        start: String,
        /// End of the requested date range.
        end: String,
    },

    /// Error parsing data from a provider.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Error interacting with the cache.
    #[error("Cache error: {0}")]
    Cache(String),

    /// The requested provider is not configured.
    #[error("Provider not configured: {0}")]
    ProviderNotConfigured(String),

    /// An invalid parameter was provided.
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Authentication failed for a provider.
    #[error("Authentication failed for provider {0}")]
    AuthenticationFailed(String),

    /// The requested feature is not supported.
    #[error("Feature not supported: {0}")]
    NotSupported(String),

    /// Any other error.
    #[error("{0}")]
    Other(String),
}

/// Result type alias using [`DataError`].
pub type Result<T> = std::result::Result<T, DataError>;
