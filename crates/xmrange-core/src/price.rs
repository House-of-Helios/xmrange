//! The XMR price feed, opt-in and confirmed by the user (doctrine §1b): fetched only through Tor,
//! never direct, so the price service sees a Tor exit and nothing about the Mac or its wallet. Hostnames
//! go to the SOCKS proxy unresolved (ureq's SOCKS5 sends the domain), so no DNS lookup leaves the
//! Mac either. Kraken's public ticker first (an official, keyless API), Yahoo Finance's chart
//! endpoint as the fallback (unofficial: it can change or rate-limit).
use anyhow::{anyhow, bail, Result};
use std::time::Duration;

pub const KRAKEN: &str = "https://api.kraken.com/0/public/Ticker?pair=XMRUSD";
pub const YAHOO: &str = "https://query1.finance.yahoo.com/v8/finance/chart/XMR-USD?range=1d&interval=1d";
/// A price outside this range is a broken reply, not a market.
const SANE_USD: std::ops::RangeInclusive<f64> = 1.0..=1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quote {
    pub usd: f64,
    pub source: &'static str,
}

/// Kraken: `{"error":[],"result":{"XXMRZUSD":{"c":["123.45","0.5"],…}}}`, the last trade price.
pub fn parse_kraken(v: &serde_json::Value) -> Option<f64> {
    let pair = v.get("result")?.as_object()?.values().next()?;
    pair.get("c")?.get(0)?.as_str()?.parse().ok()
}

/// Yahoo: `{"chart":{"result":[{"meta":{"regularMarketPrice":123.4,…}}]}}`.
pub fn parse_yahoo(v: &serde_json::Value) -> Option<f64> {
    v.get("chart")?.get("result")?.get(0)?.get("meta")?.get("regularMarketPrice")?.as_f64()
}

fn sane(usd: Option<f64>) -> Option<f64> {
    usd.filter(|p| p.is_finite() && SANE_USD.contains(p))
}

/// One quote through the SOCKS proxy at `socks`: Kraken, else Yahoo.
pub fn fetch(socks: &str) -> Result<Quote> {
    let agent = ureq::AgentBuilder::new()
        .proxy(ureq::Proxy::new(format!("socks5://{socks}"))?)
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0")
        .build();
    let get = |url: &str| -> Result<serde_json::Value> { Ok(agent.get(url).call()?.into_json()?) };
    let kraken = get(KRAKEN).map(|v| sane(parse_kraken(&v)));
    if let Ok(Some(usd)) = kraken {
        return Ok(Quote { usd, source: "kraken" });
    }
    match get(YAHOO).map(|v| sane(parse_yahoo(&v))) {
        Ok(Some(usd)) => Ok(Quote { usd, source: "yahoo" }),
        Ok(None) => bail!("no price in either reply"),
        Err(e) => Err(anyhow!("kraken: {}; yahoo: {e}", kraken.err().map_or("no price".to_string(), |k| k.to_string()))),
    }
}

/// One quote through Tor, on the app's shared Arti (`tor::acquire`): the one mining or the node
/// already runs, else one started for this fetch and stopped after it, unless mining took a lease
/// meanwhile. Never a second Arti on the same port.
pub fn fetch_through_tor(cancel: &dyn Fn() -> bool) -> Result<Quote> {
    let _lease = crate::tor::acquire(cancel)?;
    fetch(&crate::tor::socks_addr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kraken_and_yahoo_replies_parse_and_nonsense_is_refused() {
        let k: serde_json::Value = serde_json::from_str(r#"{"error":[],"result":{"XXMRZUSD":{"a":["1"],"c":["318.42000000","0.25"]}}}"#).unwrap();
        assert_eq!(sane(parse_kraken(&k)), Some(318.42));
        let y: serde_json::Value = serde_json::from_str(r#"{"chart":{"result":[{"meta":{"currency":"USD","regularMarketPrice":317.9}}],"error":null}}"#).unwrap();
        assert_eq!(sane(parse_yahoo(&y)), Some(317.9));
        let bad: serde_json::Value = serde_json::from_str(r#"{"error":["EQuery:Unknown asset pair"]}"#).unwrap();
        assert_eq!(parse_kraken(&bad), None);
        assert_eq!(sane(Some(0.0)), None);
        assert_eq!(sane(Some(f64::NAN)), None);
    }

    /// Live, through the bundled Arti: `cargo devtest -p xmrange-core --lib price -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_quote_through_tor() {
        let tor = crate::tor::Tor::start(&|| false).unwrap();
        let socks = crate::tor::socks_addr();
        let agent = || ureq::AgentBuilder::new().proxy(ureq::Proxy::new(format!("socks5://{socks}")).unwrap()).timeout(Duration::from_secs(30)).user_agent("Mozilla/5.0").build();
        let k = agent().get(KRAKEN).call().map_err(|e| e.to_string()).and_then(|r| r.into_json::<serde_json::Value>().map_err(|e| e.to_string())).map(|v| parse_kraken(&v));
        let y = agent().get(YAHOO).call().map_err(|e| e.to_string()).and_then(|r| r.into_json::<serde_json::Value>().map_err(|e| e.to_string())).map(|v| parse_yahoo(&v));
        println!("kraken {k:?}\nyahoo {y:?}");
        tor.stop();
        assert!(matches!(k, Ok(Some(_))) || matches!(y, Ok(Some(_))), "neither source answered through tor");
    }
}
