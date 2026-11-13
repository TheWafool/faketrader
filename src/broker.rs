//! Simulated broker. Orders fill against bars (replay) or ticks (paper desk).
//!
//! Fill rules, kept conservative so practice results are not flattering:
//! - Market orders fill at the next bar's open, plus slippage.
//! - Stops fill at the stop price, or at the open if price gapped through it, plus slippage.
//! - Limits fill at the limit price, or at the open if it gapped past it. No slippage.
//! - If a bracket's stop and target are both inside one bar, the stop wins.

use crate::config::BrokerConfig;
use crate::model::{Candle, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderKind {
    Market,
    Limit(f64),
    Stop(f64),
}

impl OrderKind {
    pub fn label(&self) -> String {
        match self {
            OrderKind::Market => "Market".into(),
            OrderKind::Limit(p) => format!("Limit {p:.2}"),
            OrderKind::Stop(p) => format!("Stop {p:.2}"),
        }
    }
    pub fn price(&self) -> Option<f64> {
        match *self {
            OrderKind::Market => None,
            OrderKind::Limit(p) | OrderKind::Stop(p) => Some(p),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub id: u64,
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub kind: OrderKind,
    /// Stop-loss and take-profit placed once this order fills.
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    /// Orders sharing a group cancel each other when one fills.
    pub oco: Option<u64>,
    /// Only shrinks a position; never opens or flips one.
    pub reduce_only: bool,
    pub created_t: i64,
    /// Setup tag the trader picked (e.g. "breakout"). Carried into the trade record.
    pub tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    /// Signed shares: positive long, negative short.
    pub qty: f64,
    pub avg: f64,
    pub opened_t: i64,
    pub initial_stop: Option<f64>,
    /// Best and worst prices seen while open.
    pub best: f64,
    pub worst: f64,
    /// Entry fees not yet assigned to a closed trade.
    pub open_fees: f64,
    pub tag: String,
}

impl Position {
    pub fn dir(&self) -> f64 {
        self.qty.signum()
    }
    pub fn unrealized(&self, last: f64) -> f64 {
        (last - self.avg) * self.qty
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub t: i64,
    pub order_id: u64,
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub price: f64,
    pub fee: f64,
}

/// A closed (or partly closed) round trip.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Trade {
    pub id: u64,
    pub symbol: String,
    /// `Buy` for a long trade, `Sell` for a short.
    pub side: Side,
    pub qty: f64,
    pub entry_t: i64,
    pub entry_px: f64,
    pub exit_t: i64,
    pub exit_px: f64,
    /// Net of fees.
    pub pnl: f64,
    pub fees: f64,
    pub r_multiple: Option<f64>,
    /// Max favourable / adverse excursion, per share.
    pub mfe: f64,
    pub mae: f64,
    pub tag: String,
    #[serde(default)]
    pub notes: String,
    /// Where it happened: "Replay", "Desk".
    #[serde(default)]
    pub mode: String,
    /// Self-graded execution, 1 to 5.
    #[serde(default)]
    pub grade: Option<u8>,
    /// Starting equity of the account at entry, for % stats.
    #[serde(default)]
    pub equity_at_entry: f64,
}

impl Trade {
    pub fn return_pct(&self) -> f64 {
        if self.entry_px == 0.0 { 0.0 } else { (self.exit_px - self.entry_px) / self.entry_px * self.side.sign() * 100.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub cfg: BrokerConfig,
    pub cash: f64,
    pub positions: BTreeMap<String, Position>,
    pub orders: Vec<Order>,
    pub fills: Vec<Fill>,
    pub trades: Vec<Trade>,
    pub equity_curve: Vec<(i64, f64)>,
    pub last: BTreeMap<String, f64>,
    pub next_id: u64,
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything needed to place an order, from the ticket.
#[derive(Clone, Debug, Default)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: Option<Side>,
    pub qty: f64,
    pub kind: Option<OrderKind>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub tag: String,
}

impl Account {
    pub fn new(cfg: BrokerConfig, mode: &str) -> Self {
        Self {
            cash: cfg.starting_cash,
            cfg,
            positions: BTreeMap::new(),
            orders: Vec::new(),
            fills: Vec::new(),
            trades: Vec::new(),
            equity_curve: Vec::new(),
            last: BTreeMap::new(),
            next_id: 1,
            mode: mode.to_string(),
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    pub fn last_price(&self, symbol: &str) -> Option<f64> {
        self.last.get(symbol).copied()
    }

    pub fn equity(&self) -> f64 {
        self.cash + self.positions.values().map(|p| p.qty * self.last.get(&p.symbol).copied().unwrap_or(p.avg)).sum::<f64>()
    }

    pub fn gross_exposure(&self) -> f64 {
        self.positions.values().map(|p| (p.qty * self.last.get(&p.symbol).copied().unwrap_or(p.avg)).abs()).sum()
    }

    pub fn buying_power(&self) -> f64 {
        (self.equity() * self.cfg.leverage - self.gross_exposure()).max(0.0)
    }

    pub fn realized(&self) -> f64 {
        self.trades.iter().map(|t| t.pnl).sum()
    }

    pub fn position(&self, symbol: &str) -> Option<&Position> {
        self.positions.get(symbol)
    }

    pub fn place(&mut self, r: OrderRequest, t: i64) -> Result<u64, Rejected> {
        let side = r.side.ok_or_else(|| Rejected("Pick buy or sell.".into()))?;
        let kind = r.kind.unwrap_or(OrderKind::Market);
        if !(r.qty.is_finite() && r.qty >= 1.0) {
            return Err(Rejected("Quantity must be at least 1 share.".into()));
        }
        if let Some(p) = kind.price() {
            if !(p.is_finite() && p > 0.0) {
                return Err(Rejected("Enter a price above zero.".into()));
            }
        }
        let reference = kind.price().or_else(|| self.last_price(&r.symbol)).ok_or_else(|| Rejected("No price yet for this symbol.".into()))?;
        for (name, lvl) in [("Stop-loss", r.stop_loss), ("Take-profit", r.take_profit)] {
            if let Some(l) = lvl {
                if !(l.is_finite() && l > 0.0) {
                    return Err(Rejected(format!("{name} must be a price above zero.")));
                }
            }
        }
        let below = |x: Option<f64>| x.is_none_or(|x| x < reference);
        let above = |x: Option<f64>| x.is_none_or(|x| x > reference);
        match side {
            Side::Buy if !(below(r.stop_loss) && above(r.take_profit)) => {
                return Err(Rejected("For a buy, the stop goes below entry and the target above.".into()));
            }
            Side::Sell if !(above(r.stop_loss) && below(r.take_profit)) => {
                return Err(Rejected("For a short, the stop goes above entry and the target below.".into()));
            }
            _ => {}
        }
        let cur = self.positions.get(&r.symbol).map(|p| p.qty).unwrap_or(0.0);
        let pending: f64 = self.orders.iter().filter(|o| o.symbol == r.symbol && !o.reduce_only).map(|o| o.qty * o.side.sign()).sum();
        let after = cur + pending + r.qty * side.sign();
        if after < 0.0 && !self.cfg.allow_short {
            return Err(Rejected("Shorting is off. Turn it on in Settings.".into()));
        }
        let added_exposure = (after.abs() - (cur + pending).abs()).max(0.0) * reference;
        if added_exposure > self.buying_power() + 1e-6 {
            return Err(Rejected(format!(
                "Not enough buying power: this needs ${:.0}, you have ${:.0}.",
                added_exposure,
                self.buying_power()
            )));
        }
        let id = self.id();
        self.orders.push(Order {
            id,
            symbol: r.symbol,
            side,
            qty: r.qty.floor(),
            kind,
            stop_loss: r.stop_loss,
            take_profit: r.take_profit,
            oco: None,
            reduce_only: false,
            created_t: t,
            tag: r.tag,
        });
        Ok(id)
    }

    /// Place a protective stop or target for an existing position.
    pub fn place_exit(&mut self, symbol: &str, kind: OrderKind, t: i64) -> Result<u64, Rejected> {
        let pos = self.positions.get(symbol).ok_or_else(|| Rejected("No open position.".into()))?;
        let side = if pos.qty > 0.0 { Side::Sell } else { Side::Buy };
        let qty = pos.qty.abs();
        let id = self.id();
        self.orders.push(Order {
            id,
            symbol: symbol.into(),
            side,
            qty,
            kind,
            stop_loss: None,
            take_profit: None,
            oco: None,
            reduce_only: true,
            created_t: t,
            tag: String::new(),
        });
        Ok(id)
    }

    pub fn cancel(&mut self, id: u64) {
        self.orders.retain(|o| o.id != id);
    }

    pub fn cancel_symbol(&mut self, symbol: &str) {
        self.orders.retain(|o| o.symbol != symbol);
    }

    /// Move a resting order's price (dragging a line on the chart).
    pub fn modify_price(&mut self, id: u64, price: f64) {
        if !(price.is_finite() && price > 0.0) {
            return;
        }
        if let Some(o) = self.orders.iter_mut().find(|o| o.id == id) {
            o.kind = match o.kind {
                OrderKind::Limit(_) => OrderKind::Limit(price),
                OrderKind::Stop(_) => OrderKind::Stop(price),
                k => k,
            };
        }
    }

    /// Close a position right now at `price` (plus slippage) and cancel its orders.
    pub fn flatten(&mut self, symbol: &str, price: f64, t: i64) {
        self.cancel_symbol(symbol);
        if let Some(p) = self.positions.get(symbol) {
            let side = if p.qty > 0.0 { Side::Sell } else { Side::Buy };
            let qty = p.qty.abs();
            let px = self.slip(price, side);
            self.apply_fill(0, symbol, side, qty, px, t, None, "");
        }
    }

    fn slip(&self, price: f64, side: Side) -> f64 {
        price * (1.0 + side.sign() * self.cfg.slippage_bps / 10_000.0)
    }

    /// Process one bar for `symbol`. Orders placed before this bar can fill inside it.
    pub fn on_bar(&mut self, symbol: &str, bar: &Candle) {
        // Two passes so bracket legs created by an entry fill can trigger in the same bar.
        for _ in 0..2 {
            let mut ids: Vec<(u8, u64)> = self
                .orders
                .iter()
                .filter(|o| o.symbol == symbol)
                .map(|o| {
                    let rank = match o.kind {
                        OrderKind::Market => 0,
                        OrderKind::Stop(_) => 1,
                        OrderKind::Limit(_) => 2,
                    };
                    (rank, o.id)
                })
                .collect();
            ids.sort();
            let mut filled_any = false;
            for (_, id) in ids {
                let Some(o) = self.orders.iter().find(|o| o.id == id).cloned() else { continue };
                let px = match (o.kind, o.side) {
                    (OrderKind::Market, s) => Some(self.slip(bar.o, s)),
                    (OrderKind::Stop(p), Side::Buy) if bar.h >= p => Some(self.slip(bar.o.max(p), Side::Buy)),
                    (OrderKind::Stop(p), Side::Sell) if bar.l <= p => Some(self.slip(bar.o.min(p), Side::Sell)),
                    (OrderKind::Limit(p), Side::Buy) if bar.l <= p => Some(bar.o.min(p)),
                    (OrderKind::Limit(p), Side::Sell) if bar.h >= p => Some(bar.o.max(p)),
                    _ => None,
                };
                let Some(px) = px else { continue };
                self.orders.retain(|x| x.id != id);
                if let Some(g) = o.oco {
                    self.orders.retain(|x| x.oco != Some(g));
                }
                let mut qty = o.qty;
                if o.reduce_only {
                    let held = self.positions.get(symbol).map(|p| p.qty).unwrap_or(0.0);
                    if held * o.side.sign() >= 0.0 {
                        continue;
                    }
                    qty = qty.min(held.abs());
                }
                self.apply_fill(o.id, symbol, o.side, qty, px, bar.t, o.stop_loss, &o.tag);
                filled_any = true;
                if o.stop_loss.is_some() || o.take_profit.is_some() {
                    let group = self.id();
                    let exit = o.side.opposite();
                    for kind in [o.stop_loss.map(OrderKind::Stop), o.take_profit.map(OrderKind::Limit)].into_iter().flatten() {
                        let id = self.id();
                        self.orders.push(Order {
                            id,
                            symbol: symbol.into(),
                            side: exit,
                            qty,
                            kind,
                            stop_loss: None,
                            take_profit: None,
                            oco: Some(group),
                            reduce_only: true,
                            created_t: bar.t,
                            tag: String::new(),
                        });
                    }
                }
            }
            if !filled_any {
                break;
            }
        }
        if let Some(p) = self.positions.get_mut(symbol) {
            if p.qty > 0.0 {
                p.best = p.best.max(bar.h);
                p.worst = p.worst.min(bar.l);
            } else {
                p.best = p.best.min(bar.l);
                p.worst = p.worst.max(bar.h);
            }
        }
        self.last.insert(symbol.to_string(), bar.c);
    }

    pub fn record_equity(&mut self, t: i64) {
        let e = self.equity();
        match self.equity_curve.last_mut() {
            Some(last) if last.0 == t => last.1 = e,
            _ => self.equity_curve.push((t, e)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_fill(&mut self, order_id: u64, symbol: &str, side: Side, qty: f64, price: f64, t: i64, stop: Option<f64>, tag: &str) {
        if qty <= 0.0 {
            return;
        }
        let fee = self.cfg.commission_per_order + self.cfg.commission_per_share * qty;
        self.cash -= side.sign() * qty * price + fee;
        self.fills.push(Fill { t, order_id, symbol: symbol.into(), side, qty, price, fee });
        let equity_now = self.equity();
        let d = side.sign() * qty;
        let cur = self.positions.get(symbol).map(|p| p.qty).unwrap_or(0.0);

        if cur == 0.0 || cur.signum() == d.signum() {
            let p = self.positions.entry(symbol.to_string()).or_insert_with(|| Position {
                symbol: symbol.into(),
                qty: 0.0,
                avg: price,
                opened_t: t,
                initial_stop: stop,
                best: price,
                worst: price,
                open_fees: 0.0,
                tag: tag.into(),
            });
            p.avg = (p.avg * p.qty.abs() + price * qty) / (p.qty.abs() + qty);
            p.qty += d;
            p.open_fees += fee;
            if p.initial_stop.is_none() {
                p.initial_stop = stop;
            }
            return;
        }

        // Reducing, closing or flipping.
        let id = self.id();
        let p = self.positions.get_mut(symbol).unwrap();
        let close_qty = qty.min(p.qty.abs());
        let share = close_qty / p.qty.abs();
        let entry_fees = p.open_fees * share;
        p.open_fees -= entry_fees;
        let exit_fee = fee * close_qty / qty;
        let dir = p.dir();
        let gross = (price - p.avg) * close_qty * dir;
        let r_multiple = p.initial_stop.and_then(|s| {
            let risk = (p.avg - s).abs();
            (risk > 0.0).then(|| (price - p.avg) * dir / risk)
        });
        let trade = Trade {
            id,
            symbol: symbol.into(),
            side: if dir > 0.0 { Side::Buy } else { Side::Sell },
            qty: close_qty,
            entry_t: p.opened_t,
            entry_px: p.avg,
            exit_t: t,
            exit_px: price,
            pnl: gross - entry_fees - exit_fee,
            fees: entry_fees + exit_fee,
            r_multiple,
            mfe: ((p.best - p.avg) * dir).max(0.0),
            mae: ((p.avg - p.worst) * dir).max(0.0),
            tag: p.tag.clone(),
            notes: String::new(),
            mode: self.mode.clone(),
            grade: None,
            equity_at_entry: equity_now,
        };
        p.qty += d.signum() * close_qty;
        let flat = p.qty.abs() < 1e-9;
        self.trades.push(trade);
        if flat {
            self.positions.remove(symbol);
            // Protective orders have nothing left to protect.
            self.orders.retain(|o| !(o.symbol == symbol && o.reduce_only));
        }
        let rest = qty - close_qty;
        if rest > 1e-9 {
            self.positions.insert(
                symbol.to_string(),
                Position {
                    symbol: symbol.into(),
                    qty: d.signum() * rest,
                    avg: price,
                    opened_t: t,
                    initial_stop: stop,
                    best: price,
                    worst: price,
                    open_fees: fee - exit_fee,
                    tag: tag.into(),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BrokerConfig {
        BrokerConfig { starting_cash: 10_000.0, commission_per_order: 0.0, commission_per_share: 0.0, slippage_bps: 0.0, leverage: 2.0, allow_short: true }
    }
    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64) -> Candle {
        Candle { t, o, h, l, c, v: 0.0 }
    }
    fn req(side: Side, qty: f64, kind: OrderKind) -> OrderRequest {
        OrderRequest { symbol: "X".into(), side: Some(side), qty, kind: Some(kind), ..Default::default() }
    }

    #[test]
    fn long_round_trip_with_bracket_stop_wins_tie() {
        let mut a = Account::new(cfg(), "Test");
        a.on_bar("X", &bar(0, 100.0, 101.0, 99.0, 100.0));
        let mut r = req(Side::Buy, 10.0, OrderKind::Market);
        r.stop_loss = Some(95.0);
        r.take_profit = Some(110.0);
        a.place(r, 0).unwrap();
        a.on_bar("X", &bar(1, 100.0, 102.0, 99.0, 101.0));
        assert_eq!(a.position("X").unwrap().qty, 10.0);
        assert_eq!(a.orders.len(), 2);
        // Bar touches both stop and target: stop wins.
        a.on_bar("X", &bar(2, 101.0, 111.0, 94.0, 105.0));
        assert!(a.position("X").is_none());
        assert!(a.orders.is_empty());
        let t = &a.trades[0];
        assert_eq!(t.exit_px, 95.0);
        assert!((t.pnl + 50.0).abs() < 1e-9);
        assert!((t.r_multiple.unwrap() + 1.0).abs() < 1e-9);
        assert!((a.cash - 9_950.0).abs() < 1e-9);
    }

    #[test]
    fn short_and_flip() {
        let mut a = Account::new(cfg(), "Test");
        a.on_bar("X", &bar(0, 50.0, 50.0, 50.0, 50.0));
        a.place(req(Side::Sell, 10.0, OrderKind::Market), 0).unwrap();
        a.on_bar("X", &bar(1, 50.0, 50.0, 48.0, 48.0));
        assert_eq!(a.position("X").unwrap().qty, -10.0);
        a.place(req(Side::Buy, 30.0, OrderKind::Market), 1).unwrap();
        a.on_bar("X", &bar(2, 45.0, 46.0, 44.0, 45.0));
        assert!((a.trades[0].pnl - 50.0).abs() < 1e-9);
        let p = a.position("X").unwrap();
        assert_eq!(p.qty, 20.0);
        assert_eq!(p.avg, 45.0);
        assert!((a.equity() - 10_050.0).abs() < 1e-9);
    }

    #[test]
    fn stop_gap_fills_at_open() {
        let mut a = Account::new(cfg(), "Test");
        a.on_bar("X", &bar(0, 100.0, 100.0, 100.0, 100.0));
        a.place(req(Side::Buy, 1.0, OrderKind::Stop(105.0)), 0).unwrap();
        a.on_bar("X", &bar(1, 108.0, 109.0, 107.0, 108.0));
        assert_eq!(a.position("X").unwrap().avg, 108.0);
    }

    #[test]
    fn rejects_oversize_and_bad_bracket() {
        let mut a = Account::new(cfg(), "Test");
        a.on_bar("X", &bar(0, 100.0, 100.0, 100.0, 100.0));
        assert!(a.place(req(Side::Buy, 500.0, OrderKind::Market), 0).is_err());
        let mut r = req(Side::Buy, 1.0, OrderKind::Market);
        r.stop_loss = Some(120.0);
        assert!(a.place(r, 0).is_err());
        a.cfg.allow_short = false;
        assert!(a.place(req(Side::Sell, 1.0, OrderKind::Market), 0).is_err());
    }
}
