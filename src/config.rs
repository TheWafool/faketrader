//! Tunable numbers live here so behaviour can change without touching logic.
//! User-editable values (cash, fees) are copied into `Settings` on first run.

use serde::{Deserialize, Serialize};

/// Broker and account defaults. Copied into saved settings, then editable in the app.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrokerConfig {
    pub starting_cash: f64,
    /// Flat fee per order fill, in dollars.
    pub commission_per_order: f64,
    /// Fee per share, in dollars.
    pub commission_per_share: f64,
    /// Slippage applied to market and stop fills, in basis points of price.
    pub slippage_bps: f64,
    /// Max gross exposure as a multiple of equity (1.0 = cash account, 2.0 = Reg-T margin).
    pub leverage: f64,
    pub allow_short: bool,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            starting_cash: 25_000.0,
            commission_per_order: 0.0,
            commission_per_share: 0.005,
            slippage_bps: 2.0,
            leverage: 2.0,
            allow_short: true,
        }
    }
}

/// Replay drill defaults.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReplayConfig {
    /// Bars visible before the first playable bar.
    pub lookback_bars: usize,
    /// Bars you trade through in one session.
    pub session_bars: usize,
    /// Hide ticker and dates until the session ends.
    pub blind: bool,
    /// Autoplay speed in bars per second.
    pub autoplay_bps: f32,
    /// Risk per trade used by the "size by risk" button, as a fraction of equity.
    pub risk_per_trade: f64,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            lookback_bars: 150,
            session_bars: 120,
            blind: true,
            autoplay_bps: 4.0,
            risk_per_trade: 0.01,
        }
    }
}

/// Pattern quiz defaults.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct QuizConfig {
    /// Bars after the signal used to judge whether the call was right.
    pub horizon_bars: usize,
    /// Bars shown before the signal bar.
    pub context_bars: usize,
    /// Move needed (in ATRs) for "up" or "down" to count; smaller moves count as flat.
    pub min_move_atr: f64,
}

impl Default for QuizConfig {
    fn default() -> Self {
        Self { horizon_bars: 10, context_bars: 90, min_move_atr: 0.5 }
    }
}

/// Pattern detector thresholds.
pub mod patterns {
    /// Body/range ratio under which a candle counts as a doji.
    pub const DOJI_BODY_RATIO: f64 = 0.1;
    /// Wick must be at least this many bodies long for hammer / shooting star.
    pub const HAMMER_WICK_BODIES: f64 = 2.0;
    /// Bars on each side for a swing high/low pivot.
    pub const PIVOT_SPAN: usize = 5;
    /// Two pivots count as "equal" when within this fraction of price.
    pub const DOUBLE_TOLERANCE: f64 = 0.02;
    /// Minimum bars between the two peaks of a double top/bottom.
    pub const DOUBLE_MIN_GAP: usize = 8;
    pub const DOUBLE_MAX_GAP: usize = 60;
    /// Breakout lookback window.
    pub const BREAKOUT_BARS: usize = 55;
    /// Bull/bear flag: pole length and minimum pole move in ATRs.
    pub const FLAG_POLE_BARS: usize = 8;
    pub const FLAG_POLE_ATR: f64 = 4.0;
    pub const FLAG_BARS: usize = 6;
    /// Flag consolidation must retrace less than this share of the pole.
    pub const FLAG_MAX_RETRACE: f64 = 0.5;
    pub const FAST_MA: usize = 50;
    pub const SLOW_MA: usize = 200;
}

/// Tickers offered on first run. Users can edit the list.
pub const DEFAULT_WATCHLIST: &[&str] = &[
    "SPY", "QQQ", "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AMD", "JPM", "XOM",
    "KO", "DIS", "NFLX", "BA", "COST", "IWM",
];

/// Indicator periods.
pub mod indicators {
    pub const SMA_FAST: usize = 20;
    pub const SMA_SLOW: usize = 50;
    pub const EMA: usize = 9;
    pub const BB_PERIOD: usize = 20;
    pub const BB_STDDEV: f64 = 2.0;
    pub const RSI: usize = 14;
    pub const MACD_FAST: usize = 12;
    pub const MACD_SLOW: usize = 26;
    pub const MACD_SIGNAL: usize = 9;
    pub const ATR: usize = 14;
}

/// How long cached price data stays fresh, in seconds, by bar interval.
pub fn cache_ttl_secs(interval_secs: i64) -> i64 {
    if interval_secs >= 86_400 { 6 * 3600 } else { 120 }
}

/// Paper desk polling interval in seconds.
pub const DESK_POLL_SECS: u64 = 20;
