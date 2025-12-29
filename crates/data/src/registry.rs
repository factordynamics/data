//! Data provider registry for managing multiple providers with fallback behavior.

use std::sync::Arc;

use chrono::NaiveDate;
use polars::prelude::DataFrame;
use tracing::{debug, warn};

use data_core::{
    DataCache, DataError, DataFrequency, FinancialStatement, FundamentalDataProvider, KeyMetrics,
    PeriodType, PriceDataProvider, ReferenceDataProvider, Result, Symbol, TickDataProvider,
};

/// Registry for managing multiple data providers with automatic fallback.
///
/// The `DataProviderRegistry` allows you to register multiple providers for each
/// data type (price, fundamental, tick, reference) and will automatically try
/// them in order until one succeeds.
///
/// # Example
///
/// ```rust,ignore
/// use data::{DataProviderRegistry, Symbol, DataFrequency};
/// use chrono::NaiveDate;
///
/// let registry = DataProviderRegistry::new()
///     .with_yahoo()
///     .with_edgar("MyApp/1.0 (contact@example.com)");
///
/// let symbol = Symbol::new("AAPL");
/// let data = registry.fetch_ohlcv(
///     &symbol,
///     NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
///     NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
///     DataFrequency::Daily,
/// ).await?;
/// ```
#[derive(Default)]
pub struct DataProviderRegistry {
    price_providers: Vec<Arc<dyn PriceDataProvider>>,
    fundamental_providers: Vec<Arc<dyn FundamentalDataProvider>>,
    tick_providers: Vec<Arc<dyn TickDataProvider>>,
    reference_providers: Vec<Arc<dyn ReferenceDataProvider>>,
    cache: Option<Arc<dyn DataCache>>,
}

impl std::fmt::Debug for DataProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataProviderRegistry")
            .field(
                "price_providers",
                &self
                    .price_providers
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>(),
            )
            .field(
                "fundamental_providers",
                &self
                    .fundamental_providers
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>(),
            )
            .field(
                "tick_providers",
                &self
                    .tick_providers
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>(),
            )
            .field(
                "reference_providers",
                &self
                    .reference_providers
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>(),
            )
            .field("cache", &self.cache.as_ref().map(|_| "configured"))
            .finish()
    }
}

impl DataProviderRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new registry with a cache.
    #[must_use]
    pub fn with_cache(cache: Arc<dyn DataCache>) -> Self {
        Self {
            cache: Some(cache),
            ..Default::default()
        }
    }

    /// Set the cache for this registry.
    #[must_use]
    pub fn set_cache(mut self, cache: Arc<dyn DataCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Register a price data provider.
    pub fn register_price(&mut self, provider: Arc<dyn PriceDataProvider>) {
        debug!(provider = provider.name(), "Registering price provider");
        self.price_providers.push(provider);
    }

    /// Register a fundamental data provider.
    pub fn register_fundamental(&mut self, provider: Arc<dyn FundamentalDataProvider>) {
        debug!(
            provider = provider.name(),
            "Registering fundamental provider"
        );
        self.fundamental_providers.push(provider);
    }

    /// Register a tick data provider.
    pub fn register_tick(&mut self, provider: Arc<dyn TickDataProvider>) {
        debug!(provider = provider.name(), "Registering tick provider");
        self.tick_providers.push(provider);
    }

    /// Register a reference data provider.
    pub fn register_reference(&mut self, provider: Arc<dyn ReferenceDataProvider>) {
        debug!(provider = provider.name(), "Registering reference provider");
        self.reference_providers.push(provider);
    }

    /// Fetch OHLCV data, trying providers in order until one succeeds.
    ///
    /// If a cache is configured, it will be checked first and results will
    /// be cached on success.
    pub async fn fetch_ohlcv(
        &self,
        symbol: &Symbol,
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame> {
        if self.price_providers.is_empty() {
            return Err(DataError::ProviderNotConfigured(
                "No price providers registered".to_string(),
            ));
        }

        // Check cache first
        if let Some(cache) = &self.cache {
            // Try each provider's cache key
            for provider in &self.price_providers {
                if let Ok(Some(cached)) = cache.get_ohlcv(provider.name(), symbol, start, end).await
                {
                    debug!(
                        provider = provider.name(),
                        symbol = %symbol,
                        "Cache hit for OHLCV data"
                    );
                    return Ok(cached);
                }
            }
        }

        // Try each provider in order
        let mut last_error = None;
        for provider in &self.price_providers {
            debug!(
                provider = provider.name(),
                symbol = %symbol,
                "Fetching OHLCV data"
            );

            match provider.fetch_ohlcv(symbol, start, end, frequency).await {
                Ok(data) => {
                    // Cache the result
                    if let Some(cache) = &self.cache {
                        if let Err(e) = cache.put_ohlcv(provider.name(), symbol, &data).await {
                            warn!(
                                provider = provider.name(),
                                error = %e,
                                "Failed to cache OHLCV data"
                            );
                        }
                    }
                    return Ok(data);
                }
                Err(e) => {
                    warn!(
                        provider = provider.name(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| DataError::Other("All providers failed with no error".to_string())))
    }

    /// Fetch OHLCV data for multiple symbols.
    pub async fn fetch_ohlcv_batch(
        &self,
        symbols: &[Symbol],
        start: NaiveDate,
        end: NaiveDate,
        frequency: DataFrequency,
    ) -> Result<DataFrame> {
        if self.price_providers.is_empty() {
            return Err(DataError::ProviderNotConfigured(
                "No price providers registered".to_string(),
            ));
        }

        // Try each provider in order
        let mut last_error = None;
        for provider in &self.price_providers {
            debug!(
                provider = provider.name(),
                symbol_count = symbols.len(),
                "Fetching batch OHLCV data"
            );

            match provider
                .fetch_ohlcv_batch(symbols, start, end, frequency)
                .await
            {
                Ok(data) => return Ok(data),
                Err(e) => {
                    warn!(
                        provider = provider.name(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| DataError::Other("All providers failed with no error".to_string())))
    }

    /// Fetch financial statements, trying providers in order until one succeeds.
    pub async fn fetch_financials(
        &self,
        symbol: &Symbol,
        period_type: PeriodType,
        limit: Option<usize>,
    ) -> Result<Vec<FinancialStatement>> {
        if self.fundamental_providers.is_empty() {
            return Err(DataError::ProviderNotConfigured(
                "No fundamental providers registered".to_string(),
            ));
        }

        // Check cache first
        if let Some(cache) = &self.cache {
            for provider in &self.fundamental_providers {
                if let Ok(Some(cached)) = cache
                    .get_financials(provider.name(), symbol, period_type)
                    .await
                {
                    debug!(
                        provider = provider.name(),
                        symbol = %symbol,
                        "Cache hit for financials"
                    );
                    // Apply limit if specified
                    let result = match limit {
                        Some(n) => cached.into_iter().take(n).collect(),
                        None => cached,
                    };
                    return Ok(result);
                }
            }
        }

        // Try each provider in order
        let mut last_error = None;
        for provider in &self.fundamental_providers {
            debug!(
                provider = provider.name(),
                symbol = %symbol,
                "Fetching financials"
            );

            match provider.fetch_financials(symbol, period_type, limit).await {
                Ok(data) => {
                    // Cache the result
                    if let Some(cache) = &self.cache {
                        if let Err(e) = cache.put_financials(provider.name(), symbol, &data).await {
                            warn!(
                                provider = provider.name(),
                                error = %e,
                                "Failed to cache financials"
                            );
                        }
                    }
                    return Ok(data);
                }
                Err(e) => {
                    warn!(
                        provider = provider.name(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| DataError::Other("All providers failed with no error".to_string())))
    }

    /// Fetch key metrics for a symbol on a specific date.
    pub async fn fetch_metrics(&self, symbol: &Symbol, date: NaiveDate) -> Result<KeyMetrics> {
        if self.fundamental_providers.is_empty() {
            return Err(DataError::ProviderNotConfigured(
                "No fundamental providers registered".to_string(),
            ));
        }

        // Check cache first
        if let Some(cache) = &self.cache {
            for provider in &self.fundamental_providers {
                if let Ok(Some(cached)) = cache.get_metrics(provider.name(), symbol, date).await {
                    debug!(
                        provider = provider.name(),
                        symbol = %symbol,
                        "Cache hit for metrics"
                    );
                    return Ok(cached);
                }
            }
        }

        // Try each provider in order
        let mut last_error = None;
        for provider in &self.fundamental_providers {
            debug!(
                provider = provider.name(),
                symbol = %symbol,
                "Fetching metrics"
            );

            match provider.fetch_metrics(symbol, date).await {
                Ok(data) => {
                    // Cache the result
                    if let Some(cache) = &self.cache {
                        if let Err(e) = cache.put_metrics(provider.name(), symbol, &data).await {
                            warn!(
                                provider = provider.name(),
                                error = %e,
                                "Failed to cache metrics"
                            );
                        }
                    }
                    return Ok(data);
                }
                Err(e) => {
                    warn!(
                        provider = provider.name(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| DataError::Other("All providers failed with no error".to_string())))
    }

    // Builder methods for easy setup with specific providers

    /// Add the Yahoo Finance provider.
    #[cfg(feature = "yahoo")]
    #[must_use]
    pub fn with_yahoo(mut self) -> Self {
        let provider = Arc::new(data_yahoo::YahooProvider::new());
        self.register_price(provider.clone());
        self.register_reference(provider);
        self
    }

    /// Add the SEC EDGAR provider.
    #[cfg(feature = "edgar")]
    #[must_use]
    pub fn with_edgar(mut self, user_agent: &str) -> Self {
        let provider = Arc::new(data_edgar::EdgarProvider::new(user_agent));
        self.register_fundamental(provider.clone());
        self.register_reference(provider);
        self
    }

    /// Add the Financial Modeling Prep provider.
    #[cfg(feature = "fmp")]
    #[must_use]
    pub fn with_fmp(mut self, api_key: &str) -> Self {
        let provider = Arc::new(data_fmp::FmpProvider::new(api_key));
        self.register_price(provider.clone());
        self.register_fundamental(provider.clone());
        self.register_reference(provider);
        self
    }

    /// Add the NASDAQ tick data provider.
    #[cfg(feature = "nasdaq")]
    #[must_use]
    pub fn with_nasdaq(mut self, api_key: &str) -> Self {
        let provider = Arc::new(data_nasdaq::NasdaqProvider::new(api_key));
        self.register_tick(provider);
        self
    }

    /// Add the Interactive Brokers provider.
    ///
    /// # Arguments
    ///
    /// * `host` - Host address for TWS or IB Gateway (usually "127.0.0.1")
    /// * `port` - Port number (7496 for TWS paper, 7497 for TWS live, 4001/4002 for Gateway)
    #[cfg(feature = "ibkr")]
    #[must_use]
    pub fn with_ibkr(mut self, host: &str, port: u16) -> Self {
        let provider = Arc::new(data_ibkr::IbkrProvider::new(host, port));
        self.register_price(provider.clone());
        self.register_tick(provider.clone());
        self.register_fundamental(provider);
        self
    }
}
