use serde::{Deserialize, Serialize};

/// One OHLCV bar. `t` is the bar open time in unix seconds (UTC).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

impl Candle {
    pub fn body(&self) -> f64 {
        (self.c - self.o).abs()
    }
    pub fn range(&self) -> f64 {
        self.h - self.l
    }
    pub fn bullish(&self) -> bool {
        self.c >= self.o
    }
    pub fn upper_wick(&self) -> f64 {
        self.h - self.o.max(self.c)
    }
    pub fn lower_wick(&self) -> f64 {
        self.o.min(self.c) - self.l
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Interval {
    M5,
    M15,
    H1,
    #[default]
    D1,
    W1,
}

impl Interval {
    pub const ALL: [Interval; 5] = [Interval::M5, Interval::M15, Interval::H1, Interval::D1, Interval::W1];

    pub fn label(self) -> &'static str {
        match self {
            Interval::M5 => "5m",
            Interval::M15 => "15m",
            Interval::H1 => "1h",
            Interval::D1 => "1D",
            Interval::W1 => "1W",
        }
    }
    /// Yahoo `interval` parameter.
    pub fn yahoo(self) -> &'static str {
        match self {
            Interval::M5 => "5m",
            Interval::M15 => "15m",
            Interval::H1 => "60m",
            Interval::D1 => "1d",
            Interval::W1 => "1wk",
        }
    }
    /// Longest Yahoo `range` allowed for this interval.
    pub fn yahoo_range(self) -> &'static str {
        match self {
            Interval::M5 | Interval::M15 => "60d",
            Interval::H1 => "730d",
            Interval::D1 => "20y",
            Interval::W1 => "max",
        }
    }
    pub fn secs(self) -> i64 {
        match self {
            Interval::M5 => 300,
            Interval::M15 => 900,
            Interval::H1 => 3600,
            Interval::D1 => 86_400,
            Interval::W1 => 604_800,
        }
    }
    pub fn intraday(self) -> bool {
        self.secs() < 86_400
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Series {
    pub symbol: String,
    pub interval: Interval,
    pub candles: Vec<Candle>,
    /// Unix seconds when this was downloaded or imported.
    pub fetched_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Buy => 1.0,
            Side::Sell => -1.0,
        }
    }
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn fmt_date(t: i64, intraday: bool) -> String {
    let Some(dt) = chrono::DateTime::from_timestamp(t, 0) else { return "?".into() };
    let dt = dt.with_timezone(&chrono::Local);
    if intraday { dt.format("%b %d %H:%M").to_string() } else { dt.format("%Y-%m-%d").to_string() }
}

/// Small deterministic RNG (xorshift64*), enough for picking drills.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos | 1)
    }
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `lo..hi`. Returns `lo` when the range is empty.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        if hi <= lo { lo } else { lo + (self.next_u64() % (hi - lo) as u64) as usize }
    }
}
