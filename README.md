# data

[![CI](https://github.com/factordynamics/data/actions/workflows/ci.yml/badge.svg)](https://github.com/factordynamics/data/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/data.svg)](https://crates.io/crates/data)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Unified data provider interface for quantitative finance.

## Overview

Data provides a modular system for fetching financial market data from multiple providers with built-in caching and automatic fallback behavior.

## Architecture

The framework is organized as a Cargo workspace:

- `data-core`: Core traits and types (`DataProvider`, `PriceDataProvider`, `FundamentalDataProvider`)
- `data-cache`: Caching implementations (SQLite, in-memory, no-op)
- `data-yahoo`: Yahoo Finance provider for price and reference data
- `data-edgar`: SEC EDGAR provider for fundamental data
- `data-fmp`: Financial Modeling Prep provider
- `data-nasdaq`: NASDAQ tick data provider (stub)
- `data-ibkr`: Interactive Brokers provider (stub)

## Example

```rust
use data::{DataProviderRegistry, Symbol, DataFrequency};
use chrono::NaiveDate;

#[tokio::main]
async fn main() -> data::Result<()> {
    let registry = DataProviderRegistry::new()
        .with_yahoo();

    let symbol = Symbol::new("AAPL");
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let ohlcv = registry.fetch_ohlcv(&symbol, start, end, DataFrequency::Daily).await?;
    println!("{:?}", ohlcv);

    Ok(())
}
```

## Development

Requires Rust 1.85+ and [just](https://github.com/casey/just).

```bash
just ci    # Run full CI suite (fmt, clippy, test, udeps)
just test  # Run tests
just doc   # Generate documentation
```

## License

MIT OR Apache-2.0 - see [LICENSE](LICENSE).
