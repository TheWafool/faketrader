//! Replay drills and the pattern quiz.

use crate::broker::{Account, Trade};
use crate::config::{BrokerConfig, QuizConfig, ReplayConfig};
use crate::model::{Candle, Interval, Rng, Series};
use crate::patterns::{self, Pattern, Signal};
use crate::stats::{self, Stats};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct Replay {
    pub series: Series,
    /// First bar you trade on.
    pub start: usize,
    /// Last bar of the session (inclusive).
    pub end: usize,
    /// Last revealed bar.
    pub cursor: usize,
    pub account: Account,
    pub blind: bool,
    pub alias: String,
    pub signals: Vec<Signal>,
    pub plan: String,
    pub report: Option<SessionReport>,
}

/// Saved summary of a finished replay session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionReport {
    pub symbol: String,
    pub interval: Interval,
    pub start_t: i64,
    pub end_t: i64,
    pub bars: usize,
    pub return_pct: f64,
    pub hold_pct: f64,
    pub max_dd: f64,
    pub trades: Vec<Trade>,
    pub plan: String,
    pub finished_at: i64,
}

impl SessionReport {
    pub fn stats(&self) -> Stats {
        stats::compute(&self.trades)
    }
    /// Letter grade from excess return, risk discipline and drawdown.
    pub fn grade(&self) -> &'static str {
        let s = self.stats();
        let mut pts = 0.0;
        pts += ((self.return_pct - self.hold_pct) / 2.0).clamp(-3.0, 3.0);
        pts += if s.trades == 0 { -1.0 } else { 2.0 * s.with_stop as f64 / s.trades as f64 };
        pts -= (self.max_dd * 100.0 / 5.0).min(3.0);
        if s.profit_factor > 1.5 {
            pts += 1.0;
        }
        match pts {
            p if p >= 3.0 => "A",
            p if p >= 1.5 => "B",
            p if p >= 0.0 => "C",
            p if p >= -1.5 => "D",
            _ => "F",
        }
    }
}

const ALIASES: &[&str] = &["Alder", "Birch", "Cedar", "Dogwood", "Elm", "Fir", "Ginkgo", "Hazel", "Ironwood", "Juniper", "Kapok", "Larch", "Maple", "Nutmeg", "Oak", "Pine", "Quince", "Rowan", "Spruce", "Teak", "Upas", "Willow", "Yew", "Zelkova"];

impl Replay {
    /// A session on a random window of `series`. Returns `None` when the series is too short.
    pub fn random(series: Series, cfg: &ReplayConfig, broker: &BrokerConfig, rng: &mut Rng) -> Option<Self> {
        let n = series.candles.len();
        let need = cfg.lookback_bars + cfg.session_bars + 1;
        let (lookback, session) = if n >= need {
            (cfg.lookback_bars, cfg.session_bars)
        } else if n >= 80 {
            (n / 2, n / 2 - 1)
        } else {
            return None;
        };
        let start = rng.range(lookback, n - session);
        Some(Self::at(series, start, session, cfg.blind, broker, rng))
    }

    pub fn at(series: Series, start: usize, session: usize, blind: bool, broker: &BrokerConfig, rng: &mut Rng) -> Self {
        let end = (start + session - 1).min(series.candles.len() - 1);
        let mut account = Account::new(broker.clone(), "Replay");
        let sym = series.symbol.clone();
        // Prime the account with the last visible close so the ticket can price orders.
        let cursor = start - 1;
        account.on_bar(&sym, &series.candles[cursor]);
        account.record_equity(series.candles[cursor].t);
        let signals = patterns::detect(&series.candles[..=end]);
        let alias = ALIASES[rng.range(0, ALIASES.len())].to_string();
        Self { series, start, end, cursor, account, blind, alias, signals, plan: String::new(), report: None }
    }

    pub fn visible(&self) -> &[Candle] {
        &self.series.candles[..=self.cursor]
    }
    pub fn symbol(&self) -> &str {
        &self.series.symbol
    }
    pub fn display_name(&self) -> &str {
        if self.blind && self.report.is_none() { &self.alias } else { &self.series.symbol }
    }
    pub fn bars_done(&self) -> usize {
        self.cursor + 1 - self.start
    }
    pub fn bars_total(&self) -> usize {
        self.end + 1 - self.start
    }
    pub fn finished(&self) -> bool {
        self.report.is_some()
    }

    /// Reveal the next bar. Returns false once the session is over.
    pub fn step(&mut self) -> bool {
        if self.finished() {
            return false;
        }
        if self.cursor >= self.end {
            self.finish();
            return false;
        }
        self.cursor += 1;
        let bar = self.series.candles[self.cursor];
        let sym = self.series.symbol.clone();
        self.account.on_bar(&sym, &bar);
        self.account.record_equity(bar.t);
        true
    }

    pub fn last_close(&self) -> f64 {
        self.series.candles[self.cursor].c
    }

    /// Close out at the current close and build the report.
    pub fn finish(&mut self) {
        if self.finished() {
            return;
        }
        let bar = self.series.candles[self.cursor];
        let sym = self.series.symbol.clone();
        self.account.flatten(&sym, bar.c, bar.t);
        self.account.orders.clear();
        self.account.record_equity(bar.t);
        let start_px = self.series.candles[self.start - 1].c;
        let curve: Vec<f64> = self.account.equity_curve.iter().map(|e| e.1).collect();
        let start_eq = self.account.cfg.starting_cash;
        self.report = Some(SessionReport {
            symbol: sym,
            interval: self.series.interval,
            start_t: self.series.candles[self.start].t,
            end_t: bar.t,
            bars: self.bars_done(),
            return_pct: (self.account.equity() / start_eq - 1.0) * 100.0,
            hold_pct: (bar.c / start_px - 1.0) * 100.0,
            max_dd: stats::max_drawdown(&curve),
            trades: self.account.trades.clone(),
            plan: self.plan.clone(),
            finished_at: crate::model::now_unix(),
        });
    }

    /// Signals whose completing bar is already visible.
    pub fn visible_signals(&self) -> impl Iterator<Item = &Signal> {
        self.signals.iter().filter(|s| s.index <= self.cursor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Call {
    Higher,
    Lower,
    Sideways,
}

impl Call {
    pub fn label(self) -> &'static str {
        match self {
            Call::Higher => "Higher",
            Call::Lower => "Lower",
            Call::Sideways => "Sideways",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    /// The pattern is shown; call the next move.
    CallMove,
    /// The pattern is hidden; pick its name.
    NamePattern { choices: Vec<Pattern> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Call(Call),
    Name(Pattern),
}

pub struct QuizItem {
    pub series: Series,
    pub signal: Signal,
    pub question: Question,
    /// Move over the horizon, in ATRs.
    pub move_atr: f64,
    pub actual: Call,
    pub answer: Option<Answer>,
    pub horizon: usize,
}

impl QuizItem {
    pub fn correct(&self) -> Option<bool> {
        Some(match self.answer.as_ref()? {
            Answer::Call(c) => *c == self.actual,
            Answer::Name(p) => *p == self.signal.pattern,
        })
    }
    /// Bars to show: context only, or context plus the outcome once answered.
    pub fn view_end(&self) -> usize {
        if self.answer.is_some() { self.signal.index + self.horizon } else { self.signal.index }
    }
}

pub fn classify(move_atr: f64, cfg: &QuizConfig) -> Call {
    if move_atr > cfg.min_move_atr {
        Call::Higher
    } else if move_atr < -cfg.min_move_atr {
        Call::Lower
    } else {
        Call::Sideways
    }
}

/// Pick a quiz question from `series`. `only` limits which patterns can appear.
pub fn make_question(series: &Series, cfg: &QuizConfig, only: &[Pattern], rng: &mut Rng) -> Option<QuizItem> {
    let c = &series.candles;
    let signals: Vec<Signal> = patterns::detect(c)
        .into_iter()
        .filter(|s| s.index >= cfg.context_bars.min(60) && s.index + cfg.horizon_bars < c.len())
        .filter(|s| only.is_empty() || only.contains(&s.pattern))
        .collect();
    if signals.is_empty() {
        return None;
    }
    // Spread picks across pattern types so frequent candles don't crowd out rare chart patterns.
    let mut kinds: Vec<Pattern> = signals.iter().map(|s| s.pattern).collect();
    kinds.sort();
    kinds.dedup();
    let kind = kinds[rng.range(0, kinds.len())];
    let of_kind: Vec<&Signal> = signals.iter().filter(|s| s.pattern == kind).collect();
    let signal = *of_kind[rng.range(0, of_kind.len())];
    let move_atr = patterns::outcome(c, signal.index, cfg.horizon_bars)?;
    let question = if rng.range(0, 3) == 0 {
        let mut choices = vec![signal.pattern];
        let pool: Vec<Pattern> = Pattern::ALL.iter().copied().filter(|p| *p != signal.pattern && p.is_candle() == signal.pattern.is_candle()).collect();
        while choices.len() < 4 && choices.len() <= pool.len() {
            let p = pool[rng.range(0, pool.len())];
            if !choices.contains(&p) {
                choices.push(p);
            }
        }
        // Shuffle.
        for i in (1..choices.len()).rev() {
            choices.swap(i, rng.range(0, i + 1));
        }
        Question::NamePattern { choices }
    } else {
        Question::CallMove
    };
    Some(QuizItem {
        series: series.clone(),
        signal,
        question,
        move_atr,
        actual: classify(move_atr, cfg),
        answer: None,
        horizon: cfg.horizon_bars,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tally {
    pub attempts: u32,
    pub correct: u32,
}

impl Tally {
    pub fn rate(&self) -> f64 {
        if self.attempts == 0 { 0.0 } else { self.correct as f64 / self.attempts as f64 }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QuizStats {
    pub calls: BTreeMap<Pattern, Tally>,
    pub names: BTreeMap<Pattern, Tally>,
    pub streak: u32,
    pub best_streak: u32,
}

impl QuizStats {
    pub fn record(&mut self, item: &QuizItem) {
        let Some(ok) = item.correct() else { return };
        let map = match item.question {
            Question::CallMove => &mut self.calls,
            Question::NamePattern { .. } => &mut self.names,
        };
        let t = map.entry(item.signal.pattern).or_default();
        t.attempts += 1;
        if ok {
            t.correct += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
        } else {
            self.streak = 0;
        }
    }
    pub fn total(&self) -> Tally {
        let mut t = Tally::default();
        for v in self.calls.values().chain(self.names.values()) {
            t.attempts += v.attempts;
            t.correct += v.correct;
        }
        t
    }
}

/// How each pattern actually played out across a set of series.
#[derive(Clone, Debug, Default)]
pub struct BaseRate {
    pub count: u32,
    pub higher: u32,
    pub lower: u32,
    pub avg_move_atr: f64,
}

pub fn base_rates<'a>(all: impl IntoIterator<Item = &'a Series>, cfg: &QuizConfig) -> BTreeMap<Pattern, BaseRate> {
    let mut out: BTreeMap<Pattern, BaseRate> = BTreeMap::new();
    for s in all {
        let c = &s.candles;
        let atr = crate::indicators::atr(c, crate::config::indicators::ATR);
        for sig in patterns::detect(c) {
            let Some(end) = c.get(sig.index + cfg.horizon_bars) else { continue };
            let a = atr[sig.index];
            if !a.is_finite() || a <= 0.0 {
                continue;
            }
            let m = (end.c - c[sig.index].c) / a;
            let r = out.entry(sig.pattern).or_default();
            r.count += 1;
            r.avg_move_atr += m;
            match classify(m, cfg) {
                Call::Higher => r.higher += 1,
                Call::Lower => r.lower += 1,
                Call::Sideways => {}
            }
        }
    }
    for r in out.values_mut() {
        if r.count > 0 {
            r.avg_move_atr /= r.count as f64;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(n: usize) -> Series {
        let candles = (0..n)
            .map(|i| {
                let x = i as f64;
                let p = 100.0 + 8.0 * (x / 11.0).sin() + 3.0 * (x / 3.7).cos();
                Candle { t: i as i64 * 86_400, o: p - 0.5, h: p + 1.5, l: p - 1.5, c: p + 0.4 * (x / 2.0).sin(), v: 1e6 }
            })
            .collect();
        Series { symbol: "TEST".into(), interval: Interval::D1, candles, fetched_at: 0 }
    }

    #[test]
    fn replay_runs_to_report() {
        let mut rng = Rng::new(7);
        let mut r = Replay::random(series(400), &ReplayConfig::default(), &BrokerConfig::default(), &mut rng).unwrap();
        assert!(r.visible().len() >= 150);
        while r.step() {}
        let rep = r.report.as_ref().unwrap();
        assert_eq!(rep.bars, 120);
        assert_eq!(rep.return_pct, 0.0);
        assert!(r.visible_signals().all(|s| s.index <= r.cursor));
    }

    #[test]
    fn quiz_question_has_answer_in_choices() {
        let mut rng = Rng::new(3);
        let s = series(600);
        for _ in 0..20 {
            if let Some(q) = make_question(&s, &QuizConfig::default(), &[], &mut rng) {
                if let Question::NamePattern { choices } = &q.question {
                    assert!(choices.contains(&q.signal.pattern));
                }
                assert!(q.signal.index + q.horizon < s.candles.len());
            }
        }
    }
}
