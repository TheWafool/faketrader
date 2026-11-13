//! Pattern detection. Every detector only looks at bars up to and including the
//! signal bar, so signals are what a trader could have seen at that close.

use crate::config::patterns as p;
use crate::indicators;
use crate::model::Candle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Pattern {
    Doji,
    Hammer,
    ShootingStar,
    BullishEngulfing,
    BearishEngulfing,
    MorningStar,
    EveningStar,
    ThreeWhiteSoldiers,
    ThreeBlackCrows,
    DoubleTop,
    DoubleBottom,
    HeadAndShoulders,
    InverseHeadAndShoulders,
    BullFlag,
    BearFlag,
    RangeBreakout,
    RangeBreakdown,
    GoldenCross,
    DeathCross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bias {
    Bullish,
    Bearish,
    Neutral,
}

impl Pattern {
    pub const ALL: [Pattern; 19] = [
        Pattern::Doji,
        Pattern::Hammer,
        Pattern::ShootingStar,
        Pattern::BullishEngulfing,
        Pattern::BearishEngulfing,
        Pattern::MorningStar,
        Pattern::EveningStar,
        Pattern::ThreeWhiteSoldiers,
        Pattern::ThreeBlackCrows,
        Pattern::DoubleTop,
        Pattern::DoubleBottom,
        Pattern::HeadAndShoulders,
        Pattern::InverseHeadAndShoulders,
        Pattern::BullFlag,
        Pattern::BearFlag,
        Pattern::RangeBreakout,
        Pattern::RangeBreakdown,
        Pattern::GoldenCross,
        Pattern::DeathCross,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Pattern::Doji => "Doji",
            Pattern::Hammer => "Hammer",
            Pattern::ShootingStar => "Shooting star",
            Pattern::BullishEngulfing => "Bullish engulfing",
            Pattern::BearishEngulfing => "Bearish engulfing",
            Pattern::MorningStar => "Morning star",
            Pattern::EveningStar => "Evening star",
            Pattern::ThreeWhiteSoldiers => "Three white soldiers",
            Pattern::ThreeBlackCrows => "Three black crows",
            Pattern::DoubleTop => "Double top",
            Pattern::DoubleBottom => "Double bottom",
            Pattern::HeadAndShoulders => "Head and shoulders",
            Pattern::InverseHeadAndShoulders => "Inverse head and shoulders",
            Pattern::BullFlag => "Bull flag",
            Pattern::BearFlag => "Bear flag",
            Pattern::RangeBreakout => "Range breakout",
            Pattern::RangeBreakdown => "Range breakdown",
            Pattern::GoldenCross => "Golden cross",
            Pattern::DeathCross => "Death cross",
        }
    }

    pub fn bias(self) -> Bias {
        use Pattern::*;
        match self {
            Doji => Bias::Neutral,
            Hammer | BullishEngulfing | MorningStar | ThreeWhiteSoldiers | DoubleBottom | InverseHeadAndShoulders | BullFlag
            | RangeBreakout | GoldenCross => Bias::Bullish,
            _ => Bias::Bearish,
        }
    }

    /// Short lesson shown in the quiz and the pattern library.
    pub fn lesson(self) -> &'static str {
        use Pattern::*;
        match self {
            Doji => "Open and close almost equal. Buyers and sellers fought to a draw. On its own it says little; after a long run it can mark exhaustion.",
            Hammer => "Long lower wick after a decline. Sellers pushed price down, buyers took it back by the close. Wants confirmation from the next bar.",
            ShootingStar => "Long upper wick after a rally. Buyers pushed up, sellers rejected the high. The mirror of a hammer.",
            BullishEngulfing => "A green body that fully covers the prior red body. Control flipped to buyers within one bar.",
            BearishEngulfing => "A red body that fully covers the prior green body. Control flipped to sellers within one bar.",
            MorningStar => "Big red bar, small indecisive bar, then a green bar closing past the middle of the first. A three-bar bottom.",
            EveningStar => "Big green bar, small indecisive bar, then a red bar closing past the middle of the first. A three-bar top.",
            ThreeWhiteSoldiers => "Three green bars in a row, each closing higher near its high. Steady buying, though often stretched short term.",
            ThreeBlackCrows => "Three red bars in a row, each closing lower near its low. Steady selling.",
            DoubleTop => "Two highs at about the same price with a dip between. Signals when price closes under that dip (the neckline).",
            DoubleBottom => "Two lows at about the same price with a bounce between. Signals when price closes above the bounce high.",
            HeadAndShoulders => "Three highs, the middle one highest, shoulders roughly level. Signals on a close below the neckline.",
            InverseHeadAndShoulders => "Three lows, the middle one lowest. Signals on a close above the neckline.",
            BullFlag => "A sharp move up (the pole), then a shallow pullback. Signals when price breaks above the pullback.",
            BearFlag => "A sharp drop, then a weak bounce. Signals when price breaks below the bounce.",
            RangeBreakout => "First close above the highest high of the last 55 bars. The classic turtle entry.",
            RangeBreakdown => "First close below the lowest low of the last 55 bars.",
            GoldenCross => "50-bar average crosses above the 200-bar average. A slow trend signal, late by design.",
            DeathCross => "50-bar average crosses below the 200-bar average.",
        }
    }

    pub fn is_candle(self) -> bool {
        (self as u8) <= (Pattern::ThreeBlackCrows as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub pattern: Pattern,
    /// First bar that belongs to the pattern (for highlighting).
    pub start: usize,
    /// Bar whose close completes the pattern.
    pub index: usize,
}

/// Prior trend over `n` bars: positive when price rose into bar `i`.
fn trend(c: &[Candle], i: usize, n: usize) -> f64 {
    if i < n { 0.0 } else { c[i].c - c[i - n].c }
}

fn candle_patterns(c: &[Candle], i: usize, atr: f64, out: &mut Vec<Signal>) {
    let b = &c[i];
    let r = b.range();
    if r <= 0.0 || !atr.is_finite() {
        return;
    }
    let mut push = |pattern, start| out.push(Signal { pattern, start, index: i });
    let down = trend(c, i.saturating_sub(1), 5) < 0.0;
    let up = trend(c, i.saturating_sub(1), 5) > 0.0;

    if b.body() / r < p::DOJI_BODY_RATIO && r > 0.6 * atr {
        push(Pattern::Doji, i);
    }
    let body = b.body().max(r * 0.02);
    if down && b.lower_wick() >= p::HAMMER_WICK_BODIES * body && b.upper_wick() <= body && r > 0.8 * atr {
        push(Pattern::Hammer, i);
    }
    if up && b.upper_wick() >= p::HAMMER_WICK_BODIES * body && b.lower_wick() <= body && r > 0.8 * atr {
        push(Pattern::ShootingStar, i);
    }
    if i >= 1 {
        let a = &c[i - 1];
        if down && !a.bullish() && b.bullish() && b.o <= a.c && b.c >= a.o && b.body() > a.body() && b.body() > 0.5 * atr {
            push(Pattern::BullishEngulfing, i - 1);
        }
        if up && a.bullish() && !b.bullish() && b.o >= a.c && b.c <= a.o && b.body() > a.body() && b.body() > 0.5 * atr {
            push(Pattern::BearishEngulfing, i - 1);
        }
    }
    if i >= 2 {
        let (a, m) = (&c[i - 2], &c[i - 1]);
        let mid_a = (a.o + a.c) / 2.0;
        if !a.bullish() && a.body() > atr * 0.8 && m.body() < a.body() * 0.4 && b.bullish() && b.c > mid_a && trend(c, i - 2, 5) < 0.0 {
            push(Pattern::MorningStar, i - 2);
        }
        if a.bullish() && a.body() > atr * 0.8 && m.body() < a.body() * 0.4 && !b.bullish() && b.c < mid_a && trend(c, i - 2, 5) > 0.0 {
            push(Pattern::EveningStar, i - 2);
        }
        let three = [&c[i - 2], &c[i - 1], b];
        let strong = |x: &Candle| x.body() > 0.5 * atr && x.body() / x.range().max(1e-9) > 0.6;
        if three.iter().all(|x| x.bullish() && strong(x)) && c[i - 1].c > c[i - 2].c && b.c > c[i - 1].c {
            push(Pattern::ThreeWhiteSoldiers, i - 2);
        }
        if three.iter().all(|x| !x.bullish() && strong(x)) && c[i - 1].c < c[i - 2].c && b.c < c[i - 1].c {
            push(Pattern::ThreeBlackCrows, i - 2);
        }
    }
}

/// Swing pivots confirmed by bar `i` (pivot at `i - PIVOT_SPAN`).
fn pivot_at(c: &[Candle], j: usize, high: bool) -> bool {
    let s = p::PIVOT_SPAN;
    if j < s || j + s >= c.len() {
        return false;
    }
    let v = |k: usize| if high { c[k].h } else { -c[k].l };
    (j - s..=j + s).all(|k| k == j || v(k) < v(j))
}

pub fn detect(c: &[Candle]) -> Vec<Signal> {
    let mut out = Vec::new();
    if c.len() < 30 {
        return out;
    }
    let atr = indicators::atr(c, crate::config::indicators::ATR);
    let close = indicators::closes(c);
    let fast = indicators::sma(&close, p::FAST_MA);
    let slow = indicators::sma(&close, p::SLOW_MA);

    // Confirmed pivots so far: (bar index, price).
    let mut highs: Vec<(usize, f64)> = Vec::new();
    let mut lows: Vec<(usize, f64)> = Vec::new();
    // Each structural pattern fires once per setup.
    let mut used_top: Option<usize> = None;
    let mut used_bot: Option<usize> = None;
    let mut used_hs: Option<usize> = None;
    let mut used_ihs: Option<usize> = None;
    let mut in_breakout = false;
    let mut in_breakdown = false;

    for i in 1..c.len() {
        let a = atr[i];
        candle_patterns(c, i, a, &mut out);

        if i >= p::PIVOT_SPAN {
            let j = i - p::PIVOT_SPAN;
            if pivot_at(&c[..=i], j, true) {
                highs.push((j, c[j].h));
            }
            if pivot_at(&c[..=i], j, false) {
                lows.push((j, c[j].l));
            }
        }

        // Double top: last two pivot highs level, close breaks the low between them.
        if let [.., (j1, h1), (j2, h2)] = highs[..] {
            let gap = j2 - j1;
            if used_top != Some(j2) && (p::DOUBLE_MIN_GAP..=p::DOUBLE_MAX_GAP).contains(&gap) && (h1 - h2).abs() / h1 < p::DOUBLE_TOLERANCE {
                let neck = c[j1..=j2].iter().map(|x| x.l).fold(f64::MAX, f64::min);
                if c[i].c < neck && c[i - 1].c >= neck && i - j2 < 30 {
                    out.push(Signal { pattern: Pattern::DoubleTop, start: j1, index: i });
                    used_top = Some(j2);
                }
            }
        }
        if let [.., (j1, l1), (j2, l2)] = lows[..] {
            let gap = j2 - j1;
            if used_bot != Some(j2) && (p::DOUBLE_MIN_GAP..=p::DOUBLE_MAX_GAP).contains(&gap) && (l1 - l2).abs() / l1 < p::DOUBLE_TOLERANCE {
                let neck = c[j1..=j2].iter().map(|x| x.h).fold(f64::MIN, f64::max);
                if c[i].c > neck && c[i - 1].c <= neck && i - j2 < 30 {
                    out.push(Signal { pattern: Pattern::DoubleBottom, start: j1, index: i });
                    used_bot = Some(j2);
                }
            }
        }
        // Head and shoulders: three pivot highs, middle highest, shoulders level.
        if let [.., (s1, a1), (hd, hh), (s2, a2)] = highs[..] {
            if used_hs != Some(s2) && hh > a1.max(a2) * 1.01 && (a1 - a2).abs() / a1 < p::DOUBLE_TOLERANCE * 1.5 && s2 - s1 <= 2 * p::DOUBLE_MAX_GAP {
                let neck = c[s1..=hd].iter().map(|x| x.l).fold(f64::MAX, f64::min).max(c[hd..=s2].iter().map(|x| x.l).fold(f64::MAX, f64::min));
                if c[i].c < neck && c[i - 1].c >= neck && i - s2 < 30 {
                    out.push(Signal { pattern: Pattern::HeadAndShoulders, start: s1, index: i });
                    used_hs = Some(s2);
                }
            }
        }
        if let [.., (s1, a1), (hd, hl), (s2, a2)] = lows[..] {
            if used_ihs != Some(s2) && hl < a1.min(a2) * 0.99 && (a1 - a2).abs() / a1 < p::DOUBLE_TOLERANCE * 1.5 && s2 - s1 <= 2 * p::DOUBLE_MAX_GAP {
                let neck = c[s1..=hd].iter().map(|x| x.h).fold(f64::MIN, f64::max).min(c[hd..=s2].iter().map(|x| x.h).fold(f64::MIN, f64::max));
                if c[i].c > neck && c[i - 1].c <= neck && i - s2 < 30 {
                    out.push(Signal { pattern: Pattern::InverseHeadAndShoulders, start: s1, index: i });
                    used_ihs = Some(s2);
                }
            }
        }

        // Range breakout / breakdown, first close only.
        if i > p::BREAKOUT_BARS {
            let w = &c[i - p::BREAKOUT_BARS..i];
            let hi = w.iter().map(|x| x.h).fold(f64::MIN, f64::max);
            let lo = w.iter().map(|x| x.l).fold(f64::MAX, f64::min);
            let above = c[i].c > hi;
            let below = c[i].c < lo;
            if above && !in_breakout {
                out.push(Signal { pattern: Pattern::RangeBreakout, start: i - p::BREAKOUT_BARS, index: i });
            }
            if below && !in_breakdown {
                out.push(Signal { pattern: Pattern::RangeBreakdown, start: i - p::BREAKOUT_BARS, index: i });
            }
            // Re-arm once price is back inside the middle of the range.
            let mid = (hi + lo) / 2.0;
            in_breakout = if above { true } else { in_breakout && c[i].c > mid };
            in_breakdown = if below { true } else { in_breakdown && c[i].c < mid };
        }

        // Flags: pole, then a tight counter-move, then a break of the flag.
        let (pb, fb) = (p::FLAG_POLE_BARS, p::FLAG_BARS);
        if i > pb + fb + 1 && a.is_finite() {
            let pole_start = i - fb - pb;
            let pole_end = i - fb;
            let pole = c[pole_end].c - c[pole_start].c;
            let flag = &c[pole_end + 1..i];
            let fhi = flag.iter().map(|x| x.h).fold(f64::MIN, f64::max);
            let flo = flag.iter().map(|x| x.l).fold(f64::MAX, f64::min);
            if pole > p::FLAG_POLE_ATR * a && c[pole_end].c - flo < pole * p::FLAG_MAX_RETRACE && c[i].c > fhi && c[i - 1].c <= fhi {
                out.push(Signal { pattern: Pattern::BullFlag, start: pole_start, index: i });
            }
            if -pole > p::FLAG_POLE_ATR * a && fhi - c[pole_end].c < -pole * p::FLAG_MAX_RETRACE && c[i].c < flo && c[i - 1].c >= flo {
                out.push(Signal { pattern: Pattern::BearFlag, start: pole_start, index: i });
            }
        }

        if fast[i].is_finite() && slow[i].is_finite() && fast[i - 1].is_finite() && slow[i - 1].is_finite() {
            if fast[i - 1] <= slow[i - 1] && fast[i] > slow[i] {
                out.push(Signal { pattern: Pattern::GoldenCross, start: i.saturating_sub(20), index: i });
            }
            if fast[i - 1] >= slow[i - 1] && fast[i] < slow[i] {
                out.push(Signal { pattern: Pattern::DeathCross, start: i.saturating_sub(20), index: i });
            }
        }
    }
    out
}

/// What price did over `horizon` bars after a signal, measured in ATRs from the signal close.
pub fn outcome(c: &[Candle], index: usize, horizon: usize) -> Option<f64> {
    let atr = indicators::atr(&c[..=index], crate::config::indicators::ATR);
    let a = *atr.last()?;
    let end = c.get(index + horizon)?;
    if !a.is_finite() || a <= 0.0 {
        return None;
    }
    Some((end.c - c[index].c) / a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64) -> Candle {
        Candle { t, o, h, l, c, v: 1000.0 }
    }

    #[test]
    fn finds_bullish_engulfing_after_decline() {
        let mut c: Vec<Candle> = (0..30).map(|i| { let p = 100.0 - i as f64 * 0.5; bar(i, p + 0.2, p + 0.6, p - 0.6, p - 0.2) }).collect();
        let last = c.last().unwrap().c;
        c.push(bar(30, last - 0.3, last + 2.5, last - 0.5, last + 2.2));
        let s = detect(&c);
        assert!(s.iter().any(|s| s.pattern == Pattern::BullishEngulfing && s.index == 30), "{s:?}");
    }

    #[test]
    fn breakout_fires_once() {
        let mut c: Vec<Candle> = (0..80).map(|i| { let p = 100.0 + ((i % 6) as f64 - 3.0); bar(i, p, p + 1.0, p - 1.0, p) }).collect();
        for k in 0..5 {
            let p = 106.0 + k as f64;
            c.push(bar(80 + k, p, p + 1.0, p - 0.5, p + 0.8));
        }
        let n = detect(&c).iter().filter(|s| s.pattern == Pattern::RangeBreakout).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn no_lookahead() {
        // Signals on a prefix must match signals of the full series up to that point.
        let c: Vec<Candle> = (0..400).map(|i| { let x = i as f64; let p = 100.0 + 10.0 * (x / 17.0).sin() + x * 0.05; bar(i, p - 0.3, p + 1.2, p - 1.1, p + 0.3 * (x / 3.0).cos()) }).collect();
        let full = detect(&c);
        let cut = 300;
        let prefix = detect(&c[..=cut]);
        let a: Vec<_> = full.iter().filter(|s| s.index <= cut).collect();
        let b: Vec<_> = prefix.iter().collect();
        assert_eq!(a, b);
    }
}
