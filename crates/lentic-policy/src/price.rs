//! ZEC/USD for the policy signer.
//!
//! Until an authenticated oracle is wired in (Pyth's public Hermes endpoints now require an
//! API key), the signer takes the median of five public exchange tickers, which is the interim
//! source the design allows ("TEE-attested exchange median"). A reading needs at least three
//! answering exchanges. USDT pairs are treated as USD. In development a test price can
//! override the feed so the flood stage can be shown on demand.

use std::time::Duration;

use anyhow::{Result, bail};
use futures_util::future::join_all;
use lentic_core::Price;
use serde::Serialize;
use serde_json::Value;

const MIN_SOURCES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceKind {
    /// Median of public exchange tickers.
    ExchangeMedian,
    /// Fixed by configuration.
    Fixed,
    /// Set through the dev route.
    TestOverride,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePrice {
    pub source: &'static str,
    pub usd: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceReading {
    pub price: Price,
    pub usd: f64,
    pub kind: PriceKind,
    pub sources: Vec<SourcePrice>,
    /// Unix seconds when the reading was taken.
    pub at: u64,
}

pub enum Feed {
    Exchanges(reqwest::Client),
    Fixed(f64),
}

impl Feed {
    pub fn exchanges() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("lentic-policy")
            .build()
            .expect("static client config");
        Feed::Exchanges(client)
    }

    pub async fn read(&self, now: u64) -> Result<PriceReading> {
        match self {
            Feed::Fixed(usd) => Ok(reading(*usd, PriceKind::Fixed, vec![], now)),
            Feed::Exchanges(client) => {
                let fetches = [
                    ("coinbase", "https://api.coinbase.com/v2/prices/ZEC-USD/spot", "/data/amount"),
                    ("kraken", "https://api.kraken.com/0/public/Ticker?pair=ZECUSD", "/result/XZECZUSD/c/0"),
                    ("gemini", "https://api.gemini.com/v1/pubticker/zecusd", "/last"),
                    ("binance", "https://api.binance.com/api/v3/ticker/price?symbol=ZECUSDT", "/price"),
                    ("okx", "https://www.okx.com/api/v5/market/ticker?instId=ZEC-USDT", "/data/0/last"),
                ];
                let results = join_all(fetches.iter().map(|(name, url, ptr)| async move {
                    let v: Value = client.get(*url).send().await.ok()?.json().await.ok()?;
                    let usd = match v.pointer(ptr)? {
                        Value::String(s) => s.parse::<f64>().ok()?,
                        Value::Number(n) => n.as_f64()?,
                        _ => return None,
                    };
                    (usd.is_finite() && usd > 0.0).then_some(SourcePrice { source: name, usd })
                }))
                .await;
                let sources: Vec<SourcePrice> = results.into_iter().flatten().collect();
                if sources.len() < MIN_SOURCES {
                    bail!("only {} of 5 exchanges answered; need {MIN_SOURCES}", sources.len());
                }
                let usd = median(sources.iter().map(|s| s.usd).collect());
                Ok(reading(usd, PriceKind::ExchangeMedian, sources, now))
            }
        }
    }
}

pub fn reading(usd: f64, kind: PriceKind, sources: Vec<SourcePrice>, at: u64) -> PriceReading {
    PriceReading { price: Price::from_usd(usd), usd, kind, sources, at }
}

pub fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_and_even() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(vec![4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn median_ignores_one_outlier() {
        assert_eq!(median(vec![1369.4, 1369.8, 1369.0, 1369.9, 9_999.0]), 1369.8);
    }

    #[tokio::test]
    async fn fixed_feed() {
        let r = Feed::Fixed(800.0).read(7).await.unwrap();
        assert_eq!(r.price.usd_e8, 80_000_000_000);
        assert_eq!(r.kind, PriceKind::Fixed);
        assert_eq!(r.at, 7);
    }
}
