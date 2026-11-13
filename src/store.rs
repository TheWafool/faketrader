//! Saved app state: settings, paper desk account, journal, quiz progress.

use crate::broker::{Account, Trade};
use crate::config::{self, BrokerConfig, QuizConfig, ReplayConfig};
use crate::model::Interval;
use crate::trainer::{QuizStats, SessionReport};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Overlays {
    pub sma_fast: bool,
    pub sma_slow: bool,
    pub ema: bool,
    pub bollinger: bool,
    pub vwap: bool,
    pub volume: bool,
    pub lower: LowerPane,
    /// Mark detected patterns on replay and desk charts.
    pub patterns: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self { sma_fast: true, sma_slow: true, ema: false, bollinger: false, vwap: false, volume: true, lower: LowerPane::Rsi, patterns: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LowerPane {
    None,
    #[default]
    Rsi,
    Macd,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub broker: BrokerConfig,
    pub replay: ReplayConfig,
    pub quiz: QuizConfig,
    pub theme: ThemeChoice,
    pub watchlist: Vec<String>,
    pub replay_interval: Interval,
    pub overlays: Overlays,
    pub ui_scale: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            broker: BrokerConfig::default(),
            replay: ReplayConfig::default(),
            quiz: QuizConfig::default(),
            theme: ThemeChoice::Dark,
            watchlist: config::DEFAULT_WATCHLIST.iter().map(|s| s.to_string()).collect(),
            replay_interval: Interval::D1,
            overlays: Overlays::default(),
            ui_scale: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub version: u32,
    pub settings: Settings,
    pub desk: Account,
    pub journal: Vec<Trade>,
    pub sessions: Vec<SessionReport>,
    pub quiz: QuizStats,
    pub desk_symbol: String,
}

impl Default for Saved {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            version: 1,
            desk: Account::new(settings.broker.clone(), "Desk"),
            settings,
            journal: Vec::new(),
            sessions: Vec::new(),
            quiz: QuizStats::default(),
            desk_symbol: "SPY".into(),
        }
    }
}

pub fn state_path() -> PathBuf {
    crate::data::data_dir().join("state.json")
}

/// Load saved state. A file that fails to parse is kept as a backup, never overwritten.
pub fn load() -> (Saved, Option<String>) {
    let path = state_path();
    let Ok(text) = std::fs::read_to_string(&path) else { return (Saved::default(), None) };
    match serde_json::from_str(&text) {
        Ok(s) => (s, None),
        Err(e) => {
            let backup = path.with_extension(format!("broken-{}.json", crate::model::now_unix()));
            let _ = std::fs::rename(&path, &backup);
            (Saved::default(), Some(format!("Saved data couldn't be read ({e}). It was moved to {} and a fresh profile started.", backup.display())))
        }
    }
}

/// Write via a temp file and rename, so a crash mid-write can't corrupt the save.
pub fn save(s: &Saved) -> Result<()> {
    let path = state_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
