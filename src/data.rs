//! Price data: Yahoo Finance download, on-disk cache, CSV import.

use crate::config;
use crate::model::{now_unix, Candle, Interval, Series};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) FakeTrader/0.1";

pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "FakeTrader", "FakeTrader")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".faketrader"))
}

fn cache_path(symbol: &str, interval: Interval) -> PathBuf {
    let safe: String = symbol.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    data_dir().join("cache").join(format!("{}_{}.json", safe, interval.yahoo()))
}

pub fn load_cached(symbol: &str, interval: Interval) -> Option<Series> {
    let text = std::fs::read_to_string(cache_path(symbol, interval)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cache(series: &Series) -> Result<()> {
    let path = cache_path(&series.symbol, series.interval);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_string(series)?)?;
    Ok(())
}

/// Cached series if fresh, otherwise download. Falls back to stale cache when offline.
pub fn get_series(symbol: &str, interval: Interval, force: bool) -> Result<Series> {
    let symbol = symbol.trim().to_uppercase();
    let cached = load_cached(&symbol, interval);
    if let Some(c) = &cached {
        let fresh = now_unix() - c.fetched_at < config::cache_ttl_secs(interval.secs());
        if fresh && !force {
            return Ok(c.clone());
        }
    }
    match fetch_yahoo(&symbol, interval, interval.yahoo_range()) {
        Ok(s) => {
            let _ = save_cache(&s);
            Ok(s)
        }
        Err(e) => cached.ok_or(e),
    }
}

#[derive(Deserialize)]
struct YChart {
    chart: YChartInner,
}
#[derive(Deserialize)]
struct YChartInner {
    result: Option<Vec<YResult>>,
    error: Option<YError>,
}
#[derive(Deserialize)]
struct YError {
    description: Option<String>,
}
#[derive(Deserialize)]
struct YResult {
    timestamp: Option<Vec<i64>>,
    indicators: YIndicators,
}
#[derive(Deserialize)]
struct YIndicators {
    quote: Vec<YQuote>,
}
#[derive(Deserialize)]
struct YQuote {
    open: Vec<Option<f64>>,
    high: Vec<Option<f64>>,
    low: Vec<Option<f64>>,
    close: Vec<Option<f64>>,
    volume: Vec<Option<f64>>,
}

fn http_get(url: &str) -> Result<String> {
    let mut resp = ureq::get(url).header("User-Agent", UA).call().map_err(|e| anyhow!("{e}"))?;
    resp.body_mut().with_config().limit(50 * 1024 * 1024).read_to_string().map_err(|e| anyhow!("{e}"))
}

pub fn fetch_yahoo(symbol: &str, interval: Interval, range: &str) -> Result<Series> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}&includePrePost=false",
        symbol,
        interval.yahoo(),
        range
    );
    let text = http_get(&url).with_context(|| format!("Couldn't download {symbol}. Check the ticker and your connection."))?;
    parse_yahoo(symbol, interval, &text)
}

pub fn parse_yahoo(symbol: &str, interval: Interval, text: &str) -> Result<Series> {
    let y: YChart = serde_json::from_str(text).context("Yahoo sent a response FakeTrader can't read")?;
    if let Some(err) = y.chart.error {
        bail!("{}: {}", symbol, err.description.unwrap_or_else(|| "unknown ticker".into()));
    }
    let r = y.chart.result.and_then(|mut v| v.pop()).ok_or_else(|| anyhow!("No data for {symbol}"))?;
    let ts = r.timestamp.unwrap_or_default();
    let q = r.indicators.quote.into_iter().next().ok_or_else(|| anyhow!("No prices for {symbol}"))?;
    let mut candles = Vec::with_capacity(ts.len());
    for (i, &t) in ts.iter().enumerate() {
        let get = |v: &Vec<Option<f64>>| v.get(i).copied().flatten();
        if let (Some(o), Some(h), Some(l), Some(c)) = (get(&q.open), get(&q.high), get(&q.low), get(&q.close)) {
            if o > 0.0 && h >= l {
                candles.push(Candle { t, o, h, l, c, v: get(&q.volume).unwrap_or(0.0) });
            }
        }
    }
    if candles.is_empty() {
        bail!("{symbol} has no bars at {} resolution", interval.label());
    }
    Ok(Series { symbol: symbol.to_string(), interval, candles, fetched_at: now_unix() })
}

/// Latest price (and its bar) for the paper desk.
pub fn fetch_last(symbol: &str) -> Result<Candle> {
    let s = fetch_yahoo(symbol, Interval::M5, "1d")?;
    s.candles.last().copied().ok_or_else(|| anyhow!("No quote for {symbol}"))
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub symbol: String,
    pub name: String,
    pub kind: String,
    pub exchange: String,
}

pub fn search(query: &str) -> Result<Vec<SearchHit>> {
    #[derive(Deserialize)]
    struct R {
        quotes: Vec<Q>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Q {
        symbol: String,
        shortname: Option<String>,
        longname: Option<String>,
        quote_type: Option<String>,
        exch_disp: Option<String>,
    }
    let q: String = query.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '.' || *c == '-').collect();
    let url = format!("https://query1.finance.yahoo.com/v1/finance/search?q={}&quotesCount=10&newsCount=0", q.replace(' ', "+"));
    let r: R = serde_json::from_str(&http_get(&url)?)?;
    Ok(r.quotes
        .into_iter()
        .map(|q| SearchHit {
            name: q.longname.or(q.shortname).unwrap_or_default(),
            kind: q.quote_type.unwrap_or_default(),
            exchange: q.exch_disp.unwrap_or_default(),
            symbol: q.symbol,
        })
        .collect())
}

/// Import a CSV with Date/Open/High/Low/Close/Volume columns (Yahoo, Stooq, most brokers).
/// Header names are matched case-insensitively; dates may be ISO dates, ISO datetimes or unix seconds.
pub fn import_csv(path: &Path, symbol: &str, interval: Interval) -> Result<Series> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).from_path(path)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_lowercase()).collect();
    let col = |names: &[&str]| headers.iter().position(|h| names.iter().any(|n| h == n || h.starts_with(n)));
    let (ti, oi, hi, li, ci) = (
        col(&["date", "time", "timestamp", "datetime"]).context("CSV needs a Date column")?,
        col(&["open", "o"]).context("CSV needs an Open column")?,
        col(&["high", "h"]).context("CSV needs a High column")?,
        col(&["low", "l"]).context("CSV needs a Low column")?,
        col(&["close", "c", "adj close"]).context("CSV needs a Close column")?,
    );
    let vi = col(&["volume", "vol", "v"]);
    let mut candles = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let f = |i: usize| rec.get(i).and_then(|s| s.replace(',', "").parse::<f64>().ok());
        let Some(t) = rec.get(ti).and_then(parse_time) else { continue };
        if let (Some(o), Some(h), Some(l), Some(c)) = (f(oi), f(hi), f(li), f(ci)) {
            candles.push(Candle { t, o, h, l, c, v: vi.and_then(f).unwrap_or(0.0) });
        }
    }
    candles.sort_by_key(|c| c.t);
    candles.dedup_by_key(|c| c.t);
    if candles.len() < 30 {
        bail!("Found {} usable rows. FakeTrader needs at least 30 bars.", candles.len());
    }
    let s = Series { symbol: symbol.to_uppercase(), interval, candles, fetched_at: now_unix() + 10 * 365 * 86_400 };
    save_cache(&s)?;
    Ok(s)
}

fn parse_time(s: &str) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime};
    if let Ok(n) = s.parse::<i64>() {
        return Some(if n > 10_000_000_000 { n / 1000 } else { n });
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%m/%d/%Y %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp());
        }
    }
    for fmt in ["%Y-%m-%d", "%m/%d/%Y", "%Y%m%d", "%d.%m.%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.and_hms_opt(14, 30, 0)?.and_utc().timestamp());
        }
    }
    if s.len() >= 19 {
        return parse_time(&s[..19]);
    }
    None
}

/// Write candles to CSV (for backing up imported data or sharing a drill).
pub fn export_csv(path: &Path, s: &Series) -> Result<()> {
    let mut w = csv::Writer::from_path(path)?;
    w.write_record(["Date", "Open", "High", "Low", "Close", "Volume"])?;
    for c in &s.candles {
        let d = chrono::DateTime::from_timestamp(c.t, 0).map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        w.write_record([d, c.o.to_string(), c.h.to_string(), c.l.to_string(), c.c.to_string(), c.v.to_string()])?;
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yahoo_with_nulls() {
        let j = r#"{"chart":{"result":[{"timestamp":[1,2,3],"indicators":{"quote":[{
            "open":[1.0,null,3.0],"high":[2.0,2.0,4.0],"low":[0.5,1.0,2.5],"close":[1.5,1.5,3.5],"volume":[10,20,null]}]}}],"error":null}}"#;
        let s = parse_yahoo("X", Interval::D1, j).unwrap();
        assert_eq!(s.candles.len(), 2);
        assert_eq!(s.candles[1].v, 0.0);
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_time("1700000000"), Some(1_700_000_000));
        assert!(parse_time("2024-01-05").is_some());
        assert!(parse_time("2024-01-05 09:30:00").is_some());
        assert!(parse_time("2024-01-05T09:30:00-05:00").is_some());
        assert_eq!(parse_time("garbage"), None);
    }
}
