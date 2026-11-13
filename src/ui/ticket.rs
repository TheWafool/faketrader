//! Order ticket with built-in risk math. Shared by replay and the paper desk.

use super::chart::{LineId, LineKind, PriceLine};
use super::theme::{mono, pal};
use crate::broker::{Account, OrderKind, OrderRequest};
use crate::model::Side;
use egui::{RichText, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Market,
    Limit,
    Stop,
}

pub struct Ticket {
    pub qty: f64,
    pub kind: Kind,
    pub price: f64,
    pub use_stop: bool,
    pub stop: f64,
    pub use_target: bool,
    pub target: f64,
    pub tag: String,
    /// Last side the levels were set up for, so defaults land on the right side of price.
    pub armed_for: Option<Side>,
    /// Price when levels were set. Market-order levels shift with price until placed.
    anchor: f64,
    pub error: Option<String>,
}

impl Default for Ticket {
    fn default() -> Self {
        Self { qty: 100.0, kind: Kind::Market, price: 0.0, use_stop: true, stop: 0.0, use_target: false, target: 0.0, tag: String::new(), armed_for: None, anchor: 0.0, error: None }
    }
}

pub const TAGS: &[&str] = &["breakout", "pullback", "reversal", "trend", "range", "news", "impulse"];

pub struct TicketCtx<'a> {
    pub account: &'a Account,
    pub symbol: &'a str,
    pub last: f64,
    /// Average true range, used for default stop distance.
    pub atr: f64,
    pub risk_per_trade: f64,
    pub fill_note: &'a str,
}

impl Ticket {
    fn entry(&self, last: f64) -> f64 {
        if self.kind == Kind::Market { last } else { self.price }
    }

    /// Put stop and target on sensible sides of price for `side`.
    pub fn arm(&mut self, side: Side, last: f64, atr: f64) {
        let a = if atr.is_finite() && atr > 0.0 { atr } else { last * 0.02 };
        if self.price <= 0.0 || self.armed_for.is_none() {
            self.price = round2(last);
        }
        let e = self.entry(last);
        self.stop = round2(e - side.sign() * 1.5 * a);
        self.target = round2(e + side.sign() * 3.0 * a);
        self.armed_for = Some(side);
        self.anchor = last;
    }

    /// Keep stop and target the same distance from price for market orders.
    fn follow(&mut self, last: f64) {
        if self.kind == Kind::Market && self.anchor > 0.0 && last > 0.0 && last != self.anchor {
            let d = last - self.anchor;
            self.stop = round2(self.stop + d);
            self.target = round2(self.target + d);
            self.anchor = last;
        }
    }

    /// Faint lines for the levels on the ticket. Hidden while a position is open,
    /// where they would sit on top of the real stop and target.
    pub fn preview_lines(&self, last: f64, has_position: bool) -> Vec<PriceLine> {
        let mut v = Vec::new();
        if self.armed_for.is_none() || has_position {
            return v;
        }
        if self.kind != Kind::Market {
            v.push(PriceLine { price: self.price, label: "Entry".into(), kind: LineKind::Order, id: Some(LineId::TicketEntry), preview: true });
        }
        if self.use_stop && self.stop > 0.0 {
            v.push(PriceLine { price: self.stop, label: "Stop".into(), kind: LineKind::Stop, id: Some(LineId::TicketStop), preview: true });
        }
        if self.use_target && self.target > 0.0 {
            v.push(PriceLine { price: self.target, label: "Target".into(), kind: LineKind::Target, id: Some(LineId::TicketTarget), preview: true });
        }
        let _ = last;
        v
    }

    pub fn on_line_moved(&mut self, id: LineId, price: f64) -> bool {
        let p = round2(price);
        match id {
            LineId::TicketEntry => self.price = p,
            LineId::TicketStop => self.stop = p,
            LineId::TicketTarget => self.target = p,
            LineId::Order(_) => return false,
        }
        true
    }

    fn request(&self, side: Side, symbol: &str) -> OrderRequest {
        OrderRequest {
            symbol: symbol.into(),
            side: Some(side),
            qty: self.qty,
            kind: Some(match self.kind {
                Kind::Market => OrderKind::Market,
                Kind::Limit => OrderKind::Limit(self.price),
                Kind::Stop => OrderKind::Stop(self.price),
            }),
            stop_loss: self.use_stop.then_some(self.stop),
            take_profit: self.use_target.then_some(self.target),
            tag: self.tag.clone(),
        }
    }

    /// Draw the ticket. Returns an order to place when Buy or Sell is clicked.
    pub fn ui(&mut self, ui: &mut Ui, cx: &TicketCtx) -> Option<OrderRequest> {
        let p = pal(ui.ctx());
        let mut out = None;
        if self.armed_for.is_none() && cx.last > 0.0 {
            self.arm(Side::Buy, cx.last, cx.atr);
            // First size: the risk budget, capped by buying power.
            let per_share = (cx.last - self.stop).abs();
            if per_share > 0.0 {
                let by_risk = cx.account.equity() * cx.risk_per_trade / per_share;
                let by_power = cx.account.buying_power() / cx.last;
                self.qty = by_risk.min(by_power).floor().max(1.0);
            }
        }
        self.follow(cx.last);

        ui.horizontal(|ui| {
            for (k, label) in [(Kind::Market, "Market"), (Kind::Limit, "Limit"), (Kind::Stop, "Stop entry")] {
                if ui.selectable_label(self.kind == k, label).clicked() {
                    self.kind = k;
                    if k != Kind::Market && self.price <= 0.0 {
                        self.price = round2(cx.last);
                    }
                }
            }
        });
        egui::Grid::new("ticket").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Shares");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.qty).range(1.0..=1_000_000.0).speed(1.0).max_decimals(0));
                for q in [10.0, 100.0, 500.0] {
                    if ui.small_button(format!("{q:.0}")).clicked() {
                        self.qty = q;
                    }
                }
            });
            ui.end_row();
            if self.kind != Kind::Market {
                ui.label(if self.kind == Kind::Limit { "Limit price" } else { "Trigger price" });
                ui.add(egui::DragValue::new(&mut self.price).speed(price_speed(cx.last)).range(0.01..=f64::MAX).max_decimals(2));
                ui.end_row();
            }
            ui.checkbox(&mut self.use_stop, "Stop-loss");
            ui.add_enabled(self.use_stop, egui::DragValue::new(&mut self.stop).speed(price_speed(cx.last)).range(0.0..=f64::MAX).max_decimals(2));
            ui.end_row();
            ui.checkbox(&mut self.use_target, "Target");
            ui.add_enabled(self.use_target, egui::DragValue::new(&mut self.target).speed(price_speed(cx.last)).range(0.0..=f64::MAX).max_decimals(2));
            ui.end_row();
            ui.label("Setup");
            egui::ComboBox::from_id_salt("tag").selected_text(if self.tag.is_empty() { "none" } else { &self.tag }).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.tag, String::new(), "none");
                for t in TAGS {
                    ui.selectable_value(&mut self.tag, t.to_string(), *t);
                }
            });
            ui.end_row();
        });

        // Risk math.
        let entry = self.entry(cx.last);
        let equity = cx.account.equity();
        ui.add_space(2.0);
        if self.use_stop && self.stop > 0.0 && entry > 0.0 {
            let per_share = (entry - self.stop).abs();
            let risk = per_share * self.qty;
            let pct = risk / equity * 100.0;
            let col = if pct > 2.0 { p.down } else { p.muted };
            ui.label(RichText::new(format!("Risk ${risk:.0}  ({pct:.2}% of equity)")).font(mono(12.5)).color(col));
            if self.use_target && self.target > 0.0 && per_share > 0.0 {
                let rr = (self.target - entry).abs() / per_share;
                ui.label(RichText::new(format!("Reward to risk {rr:.2} : 1")).font(mono(12.5)).color(p.muted));
            }
            ui.horizontal(|ui| {
                let label = format!("Size for {:.1}% risk", cx.risk_per_trade * 100.0);
                if ui.button(label).on_hover_text("Sets shares so a stop-out loses this share of equity").clicked() && per_share > 0.0 {
                    self.qty = ((equity * cx.risk_per_trade) / per_share).floor().max(1.0);
                }
            });
        } else {
            ui.label(RichText::new("No stop: risk on this trade is open-ended.").color(p.down).small());
        }
        ui.label(RichText::new(format!("Notional ${:.0}   Buying power ${:.0}", entry * self.qty, cx.account.buying_power())).font(mono(12.0)).color(p.muted));

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            let buy = egui::Button::new(RichText::new(format!("Buy {:.0}", self.qty)).color(egui::Color32::WHITE).strong()).fill(p.up).min_size(egui::vec2(w, 32.0));
            let sell = egui::Button::new(RichText::new(format!("Sell {:.0}", self.qty)).color(egui::Color32::WHITE).strong()).fill(p.down).min_size(egui::vec2(w, 32.0));
            if ui.add(buy).on_hover_text("B").clicked() {
                out = Some(Side::Buy);
            }
            if ui.add(sell).on_hover_text("S").clicked() {
                out = Some(Side::Sell);
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("Set levels for long").clicked() {
                self.arm(Side::Buy, cx.last, cx.atr);
            }
            if ui.small_button("for short").clicked() {
                self.arm(Side::Sell, cx.last, cx.atr);
            }
        });
        ui.label(RichText::new(cx.fill_note).small().color(p.muted));
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(p.down));
        }
        out.map(|s| self.make(s, cx.symbol, cx.last, cx.atr))
    }

    /// Build an order for `side`. Levels set up for the other direction are re-armed first.
    pub fn make(&mut self, side: Side, symbol: &str, last: f64, atr: f64) -> OrderRequest {
        self.follow(last);
        if self.armed_for != Some(side) {
            self.arm(side, last, atr);
        }
        self.request(side, symbol)
    }

    /// Place an order, reporting a rejection on the ticket.
    pub fn submit(&mut self, req: OrderRequest, account: &mut Account, t: i64) -> bool {
        match account.place(req, t) {
            Ok(_) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.0);
                false
            }
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn price_speed(last: f64) -> f64 {
    (last * 0.0005).max(0.01)
}

/// Position box: size, average, open P&L and R.
pub fn position_ui(ui: &mut Ui, account: &Account, symbol: &str, display: &str) -> bool {
    let p = pal(ui.ctx());
    let mut flatten = false;
    match account.position(symbol) {
        None => {
            ui.label(RichText::new(format!("Flat in {display}.")).color(p.muted));
        }
        Some(pos) => {
            let last = account.last_price(symbol).unwrap_or(pos.avg);
            let upl = pos.unrealized(last);
            let col = if upl >= 0.0 { p.up } else { p.down };
            let dir = if pos.qty > 0.0 { "Long" } else { "Short" };
            ui.label(RichText::new(format!("{dir} {:.0} @ {:.2}", pos.qty.abs(), pos.avg)).font(mono(14.0)).color(p.brass));
            ui.label(RichText::new(format!("Open P&L {upl:+.2}  ({:+.2}%)", (last / pos.avg - 1.0) * 100.0 * pos.dir())).font(mono(13.0)).color(col));
            if let Some(s) = pos.initial_stop {
                let risk = (pos.avg - s).abs();
                if risk > 0.0 {
                    let r = (last - pos.avg) * pos.dir() / risk;
                    ui.label(RichText::new(format!("{r:+.2}R from a {s:.2} stop")).font(mono(12.5)).color(p.muted));
                }
            }
            if ui.button("Flatten (F)").clicked() {
                flatten = true;
            }
        }
    }
    flatten
}

/// Working orders with cancel buttons. Returns ids to cancel.
pub fn orders_ui(ui: &mut Ui, account: &Account, symbol: Option<&str>) -> Vec<u64> {
    let p = pal(ui.ctx());
    let mut cancel = Vec::new();
    let orders: Vec<_> = account.orders.iter().filter(|o| symbol.is_none_or(|s| o.symbol == s)).collect();
    if orders.is_empty() {
        ui.label(RichText::new("No working orders.").color(p.muted));
    }
    for o in orders {
        ui.horizontal(|ui| {
            let side = match o.side {
                Side::Buy => "Buy",
                Side::Sell => "Sell",
            };
            let extra = if o.reduce_only { " exit" } else { "" };
            ui.label(RichText::new(format!("{side} {:.0} {}{extra}", o.qty, o.kind.label())).font(mono(12.5)));
            if symbol.is_none() {
                ui.label(RichText::new(&o.symbol).color(p.muted));
            }
            if ui.small_button("Cancel").clicked() {
                cancel.push(o.id);
            }
        });
    }
    cancel
}
