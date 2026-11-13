use super::chart::{ChartData, ChartEvent, ChartView, LineId};
use super::theme::{heading, mono, pal};
use super::ticket::{self, Ticket, TicketCtx};
use super::{account_lines, atr_last, chart_toolbar, fill_markers, signed, stat, Core, Purpose};
use crate::broker::Account;
use crate::config::DESK_POLL_SECS;
use crate::data::{Quote, SearchHit};
use crate::model::{now_unix, Candle, Interval, Series};
use egui::{RichText, Ui};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct DeskScreen {
    chart: ChartView,
    ticket: Ticket,
    interval: Interval,
    series: Option<Series>,
    search: String,
    hits: Vec<SearchHit>,
    quotes: HashMap<String, Quote>,
    last_poll: Option<Instant>,
    polling: bool,
    open: bool,
    confirm_reset: bool,
    loaded_for: Option<(String, Interval)>,
}

impl Default for DeskScreen {
    fn default() -> Self {
        Self {
            chart: ChartView::default(),
            ticket: Ticket::default(),
            interval: Interval::D1,
            series: None,
            search: String::new(),
            hits: Vec::new(),
            quotes: HashMap::new(),
            last_poll: None,
            polling: false,
            open: false,
            confirm_reset: false,
            loaded_for: None,
        }
    }
}

impl DeskScreen {
    pub fn on_series(&mut self, core: &mut Core, s: Series) {
        if s.symbol != core.saved.desk_symbol || s.interval != self.interval {
            return;
        }
        if let Some(c) = s.candles.last() {
            let acct = &mut core.saved.desk;
            if acct.last_price(&s.symbol).is_none() {
                acct.last.insert(s.symbol.clone(), c.c);
            }
        }
        self.series = Some(s);
    }

    pub fn on_search(&mut self, _core: &mut Core, query: String, result: Result<Vec<SearchHit>, String>) {
        if query != self.search.trim() {
            return;
        }
        match result {
            Ok(h) => self.hits = h.into_iter().filter(|h| !h.symbol.is_empty()).collect(),
            Err(e) => _core.toast(e, true),
        }
    }

    pub fn on_quotes(&mut self, core: &mut Core, q: Vec<(String, Result<Quote, String>)>) {
        self.polling = false;
        let now = now_unix();
        let acct = &mut core.saved.desk;
        for (sym, r) in q {
            let Ok(quote) = r else { continue };
            self.quotes.insert(sym.clone(), quote);
            let px = quote.price;
            let tick = Candle { t: quote.t, o: px, h: px, l: px, c: px, v: 0.0 };
            acct.on_bar(&sym, &tick);
            if let Some(s) = self.series.as_mut().filter(|s| s.symbol == sym) {
                merge_tick(s, tick);
            }
        }
        acct.record_equity(now);
        core.dirty = true;
        let closed = std::mem::take(&mut core.saved.desk.trades);
        core.journal_trades(closed);
    }

    /// Keep orders working even when the desk tab is closed.
    pub fn tick(&mut self, core: &mut Core) {
        let active = !core.saved.desk.orders.is_empty() || !core.saved.desk.positions.is_empty();
        if !(self.open || active) || self.polling {
            return;
        }
        if self.last_poll.is_some_and(|t| t.elapsed() < Duration::from_secs(DESK_POLL_SECS)) {
            return;
        }
        let mut syms: Vec<String> = vec![core.saved.desk_symbol.clone()];
        syms.extend(core.saved.desk.positions.keys().cloned());
        syms.extend(core.saved.desk.orders.iter().map(|o| o.symbol.clone()));
        if self.open {
            syms.extend(core.saved.settings.watchlist.iter().cloned());
        }
        syms.sort();
        syms.dedup();
        self.polling = true;
        self.last_poll = Some(Instant::now());
        core.net.quotes(syms);
        self.open = false;
    }

    fn select(&mut self, core: &mut Core, sym: &str) {
        core.saved.desk_symbol = sym.trim().to_uppercase();
        core.dirty = true;
        self.series = None;
        self.loaded_for = None;
        self.chart.reset();
        self.ticket = Ticket::default();
        self.last_poll = None;
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        self.open = true;
        let sym = core.saved.desk_symbol.clone();
        if self.loaded_for.as_ref() != Some(&(sym.clone(), self.interval)) {
            self.loaded_for = Some((sym.clone(), self.interval));
            core.request(&sym, self.interval, Purpose::Desk);
        }
        let p = pal(ui.ctx());

        if !Core::typing(ui) {
            let (b, s, f) = ui.input(|i| (i.key_pressed(egui::Key::B), i.key_pressed(egui::Key::S), i.key_pressed(egui::Key::F)));
            if b || s {
                self.place_hotkey(core, if b { crate::model::Side::Buy } else { crate::model::Side::Sell });
            }
            if f {
                self.flatten(core);
            }
        }

        egui::Panel::left("desk_left").resizable(true).default_size(230.0).min_size(190.0).show(ui, |ui| {
            ui.add_space(6.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search ticker or company").desired_width(f32::INFINITY));
            if r.changed() && self.search.trim().len() >= 2 {
                core.net.search(self.search.trim());
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.search.trim().is_empty() {
                let s = self.search.trim().to_uppercase();
                self.search.clear();
                self.hits.clear();
                self.select(core, &s);
            }
            if !self.search.trim().is_empty() {
                let mut pick = None;
                for h in &self.hits {
                    let r = ui.selectable_label(false, format!("{}  {}", h.symbol, h.name)).on_hover_text(format!("{} {}", h.kind, h.exchange));
                    if r.clicked() {
                        pick = Some(h.symbol.clone());
                    }
                }
                if let Some(s) = pick {
                    self.search.clear();
                    self.hits.clear();
                    self.select(core, &s);
                }
                ui.separator();
            }
            ui.label(RichText::new("Watchlist").strong());
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut pick = None;
                for w in core.saved.settings.watchlist.clone() {
                    ui.horizontal(|ui| {
                        let on = w == sym;
                        if ui.selectable_label(on, RichText::new(&w).font(mono(13.0))).clicked() {
                            pick = Some(w.clone());
                        }
                        if let Some(q) = self.quotes.get(&w) {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let chg = q.change_pct();
                                ui.label(RichText::new(format!("{chg:+.2}%")).font(mono(12.0)).color(signed(ui, chg)));
                                ui.label(RichText::new(format!("{:.2}", q.price)).font(mono(12.0)));
                            });
                        }
                    });
                }
                if let Some(s) = pick {
                    self.select(core, &s);
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Prices from Yahoo Finance, refreshed every 20 seconds and may be delayed. Change is against the previous close.").small().color(p.muted));
            });
        });

        egui::Panel::right("desk_right").resizable(true).default_size(330.0).min_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_ui(ui, core));
        });

        egui::Panel::bottom("desk_bottom").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for iv in Interval::ALL {
                    if ui.selectable_label(self.interval == iv, iv.label()).clicked() {
                        self.interval = iv;
                        self.series = None;
                        self.chart.reset();
                    }
                }
                ui.separator();
                if ui.button("Reload").clicked() {
                    core.net.series(&sym, self.interval, true, Purpose::Desk);
                }
            });
            chart_toolbar(ui, &mut self.chart, core);
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal(ui.ctx()).panel)).show(ui, |ui| {
            let Some(s) = self.series.as_ref() else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(format!("Loading {sym}")).color(p.muted));
                });
                return;
            };
            let acct = &mut core.saved.desk;
            let mut lines = account_lines(acct, &sym);
            lines.extend(self.ticket.preview_lines(acct.last_price(&sym).unwrap_or(0.0), acct.position(&sym).is_some()));
            let signals = if core.saved.settings.overlays.patterns { crate::patterns::detect(&s.candles) } else { Vec::new() };
            let data = ChartData {
                candles: &s.candles,
                dim_from: None,
                curtain_slots: 0,
                curtain_after: None,
                curtain_lift: 1.0,
                lines,
                markers: fill_markers(&s.candles, &acct.fills, &sym),
                highlight: None,
                signals,
                intraday: s.interval.intraday(),
                hide_dates: false,
                overlays: &core.saved.settings.overlays,
            };
            let (_, events) = self.chart.show(ui, &data);
            for ev in events {
                let ChartEvent::LineMoved(id, price) = ev;
                match id {
                    LineId::Order(oid) => {
                        acct.modify_price(oid, price);
                        core.dirty = true;
                    }
                    other => {
                        self.ticket.on_line_moved(other, price);
                    }
                }
            }
        });
    }

    fn last(&self, acct: &Account, sym: &str) -> f64 {
        acct.last_price(sym).or_else(|| self.series.as_ref().and_then(|s| s.candles.last()).map(|c| c.c)).unwrap_or(0.0)
    }

    fn atr(&self) -> f64 {
        self.series.as_ref().map(|s| atr_last(&s.candles)).unwrap_or(f64::NAN)
    }

    /// Place and, for market or marketable orders, fill right away at the last price.
    fn submit(&mut self, core: &mut Core, req: crate::broker::OrderRequest) {
        let sym = req.symbol.clone();
        let acct = &mut core.saved.desk;
        let last = self.last(acct, &sym);
        if last <= 0.0 {
            self.ticket.error = Some("No price yet. Wait for the quote to load.".into());
            return;
        }
        acct.last.insert(sym.clone(), last);
        let now = now_unix();
        if self.ticket.submit(req, acct, now) {
            acct.on_bar(&sym, &Candle { t: now, o: last, h: last, l: last, c: last, v: 0.0 });
            acct.record_equity(now);
            let closed = std::mem::take(&mut acct.trades);
            core.journal_trades(closed);
            core.dirty = true;
        }
    }

    fn place_hotkey(&mut self, core: &mut Core, side: crate::model::Side) {
        let sym = core.saved.desk_symbol.clone();
        let last = self.last(&core.saved.desk, &sym);
        let atr = self.atr();
        let req = self.ticket.make(side, &sym, last, atr);
        self.submit(core, req);
    }

    fn flatten(&mut self, core: &mut Core) {
        let sym = core.saved.desk_symbol.clone();
        let last = self.last(&core.saved.desk, &sym);
        if last > 0.0 {
            core.saved.desk.flatten(&sym, last, now_unix());
            let closed = std::mem::take(&mut core.saved.desk.trades);
            core.journal_trades(closed);
            core.dirty = true;
        }
    }

    fn side_ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        let sym = core.saved.desk_symbol.clone();
        let last = self.last(&core.saved.desk, &sym);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&sym).font(egui::FontId::new(24.0, heading())));
            if let Some(q) = self.quotes.get(&sym) {
                ui.label(RichText::new(format!("as of {}", crate::model::fmt_date(q.t, true))).small().color(p.muted));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{last:.2}")).font(mono(20.0)));
            if let Some(q) = self.quotes.get(&sym) {
                let chg = q.change_pct();
                ui.label(RichText::new(format!("{:+.2}  {chg:+.2}%", q.price - q.prev_close)).font(mono(13.0)).color(signed(ui, chg)));
            }
        });
        ui.add_space(4.0);

        let acct = &core.saved.desk;
        let start = acct.cfg.starting_cash;
        egui::Grid::new("desk_acct").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
            let eq = acct.equity();
            stat(ui, "Equity", format!("${eq:.2}"), Some(signed(ui, eq - start)));
            stat(ui, "Since start", format!("{:+.2}%", (eq / start - 1.0) * 100.0), Some(signed(ui, eq - start)));
            stat(ui, "Cash", format!("${:.2}", acct.cash), None);
            stat(ui, "Buying power", format!("${:.2}", acct.buying_power()), None);
        });
        ui.separator();
        if ticket::position_ui(ui, &core.saved.desk, &sym, &sym) {
            self.flatten(core);
        }
        ui.separator();
        ui.label(RichText::new("Order").strong());
        let atr = self.atr();
        let req = {
            let cx = TicketCtx {
                account: &core.saved.desk,
                symbol: &sym,
                last,
                atr,
                risk_per_trade: core.saved.settings.replay.risk_per_trade,
                fill_note: "Market orders fill now at the last price. Stops and limits are checked on each refresh.",
            };
            self.ticket.ui(ui, &cx)
        };
        if let Some(req) = req {
            self.submit(core, req);
        }
        ui.separator();
        ui.label(RichText::new("All positions").strong());
        let acct = &core.saved.desk;
        if acct.positions.is_empty() {
            ui.label(RichText::new("No open positions.").color(p.muted));
        }
        let mut pick = None;
        for pos in acct.positions.values() {
            let l = acct.last_price(&pos.symbol).unwrap_or(pos.avg);
            let upl = pos.unrealized(l);
            ui.horizontal(|ui| {
                if ui.selectable_label(pos.symbol == sym, RichText::new(&pos.symbol).font(mono(12.5))).clicked() {
                    pick = Some(pos.symbol.clone());
                }
                ui.label(RichText::new(format!("{:+.0} @ {:.2}", pos.qty, pos.avg)).font(mono(12.0)).color(p.brass));
                ui.label(RichText::new(format!("{upl:+.2}")).font(mono(12.0)).color(signed(ui, upl)));
            });
        }
        if let Some(s) = pick {
            self.select(core, &s);
        }
        ui.separator();
        ui.label(RichText::new("Working orders").strong());
        for id in ticket::orders_ui(ui, &core.saved.desk, None) {
            core.saved.desk.cancel(id);
            core.dirty = true;
        }
        ui.add_space(8.0);
        let curve: Vec<f64> = core.saved.desk.equity_curve.iter().map(|e| e.1).collect();
        if curve.len() > 1 {
            ui.label(RichText::new("Desk equity").strong());
            super::chart::line_chart(ui, &curve, Some(start), 80.0);
        }
        ui.add_space(10.0);
        if self.confirm_reset {
            ui.label(RichText::new("Reset the desk account? Positions and orders are dropped. Journal entries stay.").color(p.down));
            ui.horizontal(|ui| {
                if ui.button("Reset account").clicked() {
                    core.saved.desk = Account::new(core.saved.settings.broker.clone(), "Desk");
                    core.dirty = true;
                    self.confirm_reset = false;
                }
                if ui.button("Keep it").clicked() {
                    self.confirm_reset = false;
                }
            });
        } else if ui.small_button("Reset desk account").clicked() {
            self.confirm_reset = true;
        }
    }
}

/// Fold the latest trade into the chart: extend the current bar or start a new one.
fn merge_tick(s: &mut Series, tick: Candle) {
    let Some(&last) = s.candles.last() else { return };
    let step = s.interval.secs();
    if tick.t < last.t {
        return;
    }
    if tick.t < last.t + step {
        let n = s.candles.len();
        let l = &mut s.candles[n - 1];
        l.h = l.h.max(tick.c);
        l.l = l.l.min(tick.c);
        l.c = tick.c;
    } else {
        s.candles.push(Candle { t: last.t + step * ((tick.t - last.t) / step), ..tick });
    }
}
