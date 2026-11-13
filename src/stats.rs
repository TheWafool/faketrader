//! Performance numbers for the journal and session reports.

use crate::broker::Trade;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub net: f64,
    pub gross_win: f64,
    pub gross_loss: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    /// Average net P&L per trade.
    pub expectancy: f64,
    /// Average R over trades that had a stop at entry.
    pub avg_r: Option<f64>,
    pub with_stop: usize,
    /// Share of the best price reached that winners kept (exit efficiency).
    pub capture: Option<f64>,
    pub max_win_streak: usize,
    pub max_loss_streak: usize,
    pub fees: f64,
}

pub fn compute<'a>(trades: impl IntoIterator<Item = &'a Trade>) -> Stats {
    let mut s = Stats::default();
    let (mut rsum, mut cap_sum, mut cap_n) = (0.0, 0.0, 0usize);
    let (mut ws, mut ls) = (0usize, 0usize);
    for t in trades {
        s.trades += 1;
        s.net += t.pnl;
        s.fees += t.fees;
        if t.pnl > 0.0 {
            s.wins += 1;
            s.gross_win += t.pnl;
            s.largest_win = s.largest_win.max(t.pnl);
            ws += 1;
            ls = 0;
            if t.mfe > 0.0 {
                cap_sum += ((t.exit_px - t.entry_px) * t.side.sign() / t.mfe).clamp(0.0, 1.0);
                cap_n += 1;
            }
        } else {
            s.losses += 1;
            s.gross_loss += -t.pnl;
            s.largest_loss = s.largest_loss.min(t.pnl);
            ls += 1;
            ws = 0;
        }
        s.max_win_streak = s.max_win_streak.max(ws);
        s.max_loss_streak = s.max_loss_streak.max(ls);
        if let Some(r) = t.r_multiple {
            rsum += r;
            s.with_stop += 1;
        }
    }
    if s.trades > 0 {
        s.win_rate = s.wins as f64 / s.trades as f64;
        s.expectancy = s.net / s.trades as f64;
    }
    s.avg_win = if s.wins > 0 { s.gross_win / s.wins as f64 } else { 0.0 };
    s.avg_loss = if s.losses > 0 { -s.gross_loss / s.losses as f64 } else { 0.0 };
    s.profit_factor = if s.gross_loss > 0.0 { s.gross_win / s.gross_loss } else if s.gross_win > 0.0 { f64::INFINITY } else { 0.0 };
    s.avg_r = (s.with_stop > 0).then(|| rsum / s.with_stop as f64);
    s.capture = (cap_n > 0).then(|| cap_sum / cap_n as f64);
    s
}

/// Largest peak-to-trough fall, as a fraction of the peak.
pub fn max_drawdown(curve: &[f64]) -> f64 {
    let mut peak = f64::MIN;
    let mut dd: f64 = 0.0;
    for &v in curve {
        peak = peak.max(v);
        if peak > 0.0 {
            dd = dd.max((peak - v) / peak);
        }
    }
    dd
}

/// Cumulative P&L after each trade, starting at 0.
pub fn pnl_curve(trades: &[&Trade]) -> Vec<f64> {
    let mut acc = 0.0;
    std::iter::once(0.0).chain(trades.iter().map(|t| { acc += t.pnl; acc })).collect()
}

pub fn by_tag<'a>(trades: impl IntoIterator<Item = &'a Trade>) -> BTreeMap<String, Stats> {
    let mut groups: BTreeMap<String, Vec<&Trade>> = BTreeMap::new();
    for t in trades {
        let k = if t.tag.is_empty() { "untagged".to_string() } else { t.tag.clone() };
        groups.entry(k).or_default().push(t);
    }
    groups.into_iter().map(|(k, v)| (k, compute(v))).collect()
}

/// Plain-language feedback for a set of trades. Each line is one observation.
pub fn coaching(s: &Stats, return_pct: f64, hold_pct: f64) -> Vec<String> {
    let mut out = Vec::new();
    if s.trades == 0 {
        out.push("No trades this session. Sitting out is a position, but practice needs reps: take at least one planned trade next time.".into());
        return out;
    }
    let diff = return_pct - hold_pct;
    if diff >= 0.0 {
        out.push(format!("You beat buy-and-hold by {diff:.1} points."));
    } else {
        out.push(format!("Buy-and-hold beat you by {:.1} points. Ask whether each trade had an edge over just holding.", -diff));
    }
    if s.with_stop < s.trades {
        out.push(format!("{} of {} trades had no stop at entry, so their risk was undefined. Set the stop on the ticket before you click.", s.trades - s.with_stop, s.trades));
    }
    if s.losses > 0 && s.wins > 0 && s.avg_loss.abs() > s.avg_win * 1.5 {
        out.push(format!("Average loss (${:.0}) is much bigger than average win (${:.0}). Cut losers at the stop instead of hoping.", s.avg_loss.abs(), s.avg_win));
    }
    if let Some(c) = s.capture {
        if c < 0.4 && s.wins >= 2 {
            out.push(format!("Winners kept {:.0}% of their best move. Consider a target or a trailing stop.", c * 100.0));
        }
    }
    if s.max_loss_streak >= 4 {
        out.push(format!("{} losses in a row at one point. After 3, pause and re-read the chart before the next entry.", s.max_loss_streak));
    }
    if let Some(r) = s.avg_r {
        out.push(format!("Average result {r:+.2}R per trade with a stop."));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Side;

    fn t(pnl: f64) -> Trade {
        Trade {
            id: 0,
            symbol: "X".into(),
            side: Side::Buy,
            qty: 1.0,
            entry_t: 0,
            entry_px: 100.0,
            exit_t: 1,
            exit_px: 100.0 + pnl,
            pnl,
            fees: 0.0,
            r_multiple: None,
            mfe: pnl.max(0.0) * 2.0,
            mae: 0.0,
            tag: String::new(),
            notes: String::new(),
            mode: String::new(),
            grade: None,
            equity_at_entry: 0.0,
        }
    }

    #[test]
    fn basic_stats() {
        let v = [t(10.0), t(-5.0), t(20.0), t(-5.0)];
        let s = compute(&v);
        assert_eq!(s.trades, 4);
        assert_eq!(s.win_rate, 0.5);
        assert_eq!(s.net, 20.0);
        assert_eq!(s.profit_factor, 3.0);
        assert_eq!(s.capture, Some(0.5));
        assert!((max_drawdown(&[100.0, 120.0, 90.0, 130.0]) - 0.25).abs() < 1e-9);
    }
}
