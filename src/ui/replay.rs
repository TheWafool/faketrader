use super::chart::{ChartData, ChartEvent, ChartView, LineId};
use super::theme::{heading, mono, pal};
use super::ticket::{self, Ticket, TicketCtx};
use super::{account_lines, atr_last, chart_toolbar, fill_markers, money, signed, stat, Core, Purpose};
use crate::model::{fmt_date, Interval, Side};
use crate::stats;
use crate::trainer::Replay;
use egui::{Key, RichText, Ui};

/// Bars shown after the session ends, so you can see what came next.
const AFTERMATH_BARS: usize = 40;

#[derive(Default)]
pub struct ReplayScreen {
    session: Option<Replay>,
    chart: ChartView,
    ticket: Ticket,
    playing: bool,
    play_acc: f32,
    pub waiting: bool,
    ticker: String,
    lift_from: Option<f64>,
}

impl ReplayScreen {
    pub fn on_series(&mut self, core: &mut Core, s: crate::model::Series) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        let set = &core.saved.settings;
        let sym = s.symbol.clone();
        let interval = s.interval;
        match Replay::random(s, &set.replay, &set.broker, &mut core.rng) {
            Some(r) => {
                self.session = Some(r);
                self.chart.reset();
                self.chart.drawings.clear();
                self.ticket = Ticket::default();
                self.playing = false;
                self.lift_from = None;
            }
            None => core.toast(format!("{sym} doesn't have enough {} history for a drill. Pick a longer interval or another ticker.", interval.label()), true),
        }
    }

    fn start(&mut self, core: &mut Core) {
        let sym = if self.ticker.trim().is_empty() { core.random_symbol() } else { self.ticker.trim().to_uppercase() };
        self.waiting = true;
        core.request(&sym, core.saved.settings.replay_interval, Purpose::Replay);
    }

    fn finish(&mut self, core: &mut Core) {
        let Some(r) = self.session.as_mut() else { return };
        r.finish();
        self.playing = false;
        if let Some(rep) = &r.report {
            core.journal_trades(std::mem::take(&mut r.account.trades));
            core.saved.sessions.push(rep.clone());
            core.dirty = true;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        if self.session.is_none() {
            self.setup_ui(ui, core);
            return;
        }
        let now = ui.input(|i| i.time);
        let dt = ui.input(|i| i.stable_dt).min(0.1);

        // Keys.
        if !Core::typing(ui) {
            let (space, shift, p_key, b, s, f, c, e) = ui.input(|i| {
                (i.key_pressed(Key::Space), i.modifiers.shift, i.key_pressed(Key::P), i.key_pressed(Key::B), i.key_pressed(Key::S), i.key_pressed(Key::F), i.key_pressed(Key::C), i.key_pressed(Key::E))
            });
            if space {
                for _ in 0..if shift { 10 } else { 1 } {
                    self.step(core);
                }
            }
            if p_key {
                self.playing = !self.playing;
            }
            if b || s {
                self.order_hotkey(if b { Side::Buy } else { Side::Sell });
            }
            if f {
                self.flatten();
            }
            if c
                && let Some(r) = self.session.as_mut() {
                    r.account.orders.clear();
                }
            if e {
                self.finish(core);
            }
        }
        if self.playing {
            self.play_acc += dt * core.saved.settings.replay.autoplay_bps;
            while self.play_acc >= 1.0 {
                self.play_acc -= 1.0;
                self.step(core);
            }
            ui.ctx().request_repaint();
        }

        let finished = self.session.as_ref().is_some_and(|r| r.finished());
        if finished && self.lift_from.is_none() {
            self.lift_from = Some(now);
        }

        egui::Panel::right("replay_side").resizable(true).default_size(330.0).min_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if finished {
                    self.report_ui(ui, core);
                } else {
                    self.side_ui(ui, core);
                }
            });
        });
        egui::Panel::bottom("replay_transport").show(ui, |ui| {
            ui.add_space(4.0);
            self.transport_ui(ui, core);
            ui.add_space(2.0);
            chart_toolbar(ui, &mut self.chart, core);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal(ui.ctx()).panel)).show(ui, |ui| {
            let Some(r) = self.session.as_mut() else { return };
            let lift = self.lift_from.map(|t| ((now - t) / 0.9).clamp(0.0, 1.0) as f32).unwrap_or(0.0);
            if lift > 0.0 && lift < 1.0 {
                ui.ctx().request_repaint();
            }
            let eased = 1.0 - (1.0 - lift).powi(3);
            let shown_end = if finished { (r.cursor + AFTERMATH_BARS).min(r.series.candles.len() - 1) } else { r.cursor };
            let candles = &r.series.candles[..=shown_end];
            let mut lines = account_lines(&r.account, r.symbol());
            if !finished {
                lines.extend(self.ticket.preview_lines(r.last_close(), r.account.position(r.symbol()).is_some()));
            }
            let signals = if core.saved.settings.overlays.patterns { r.visible_signals().copied().collect() } else { Vec::new() };
            let data = ChartData {
                candles,
                dim_from: finished.then_some(r.cursor + 1),
                curtain_slots: if finished { 0 } else { r.end - r.cursor + 1 },
                curtain_after: (!finished || lift < 1.0).then_some(r.cursor),
                curtain_lift: if finished { eased } else { 0.0 },
                lines,
                markers: fill_markers(candles, &r.account.fills, r.symbol()),
                highlight: None,
                signals,
                intraday: r.series.interval.intraday(),
                hide_dates: r.blind && !finished,
                overlays: &core.saved.settings.overlays,
            };
            let (_, events) = self.chart.show(ui, &data);
            for ev in events {
                let ChartEvent::LineMoved(id, price) = ev;
                match id {
                    LineId::Order(oid) => r.account.modify_price(oid, price),
                    other => {
                        self.ticket.on_line_moved(other, price);
                    }
                }
            }
        });
    }

    fn step(&mut self, core: &mut Core) {
        let Some(r) = self.session.as_mut() else { return };
        if !r.step() {
            self.finish(core);
        }
    }

    fn order_hotkey(&mut self, side: Side) {
        let Some(r) = self.session.as_mut() else { return };
        if r.finished() {
            return;
        }
        let last = r.last_close();
        let atr = atr_last(r.visible());
        let req = self.ticket.make(side, r.symbol(), last, atr);
        let t = r.series.candles[r.cursor].t;
        self.ticket.submit(req, &mut r.account, t);
    }

    fn flatten(&mut self) {
        let Some(r) = self.session.as_mut() else { return };
        if r.finished() {
            return;
        }
        // Next-open fill, like any market order.
        let sym = r.symbol().to_string();
        r.account.cancel_symbol(&sym);
        if let Some(p) = r.account.position(&sym) {
            let side = if p.qty > 0.0 { Side::Sell } else { Side::Buy };
            let qty = p.qty.abs();
            let t = r.series.candles[r.cursor].t;
            let req = crate::broker::OrderRequest { symbol: sym, side: Some(side), qty, kind: Some(crate::broker::OrderKind::Market), ..Default::default() };
            if let Err(e) = r.account.place(req, t) {
                self.ticket.error = Some(e.0);
            }
        }
    }

    fn setup_ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(560.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.label(RichText::new("Replay drill").font(egui::FontId::new(30.0, heading())));
                    ui.label(RichText::new("A real chart, cut at a random date. You see the history up to that bar and nothing after it. Trade it one bar at a time, then see how you did against just holding.").color(p.muted));
                    ui.add_space(14.0);
                    let set = &mut core.saved.settings;
                    let before = set.clone();
                    egui::Grid::new("setup").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                        ui.label("Ticker");
                        ui.add(egui::TextEdit::singleline(&mut self.ticker).hint_text("blank picks one from your watchlist").desired_width(240.0));
                        ui.end_row();
                        ui.label("Bar size");
                        ui.horizontal(|ui| {
                            for iv in Interval::ALL {
                                ui.selectable_value(&mut set.replay_interval, iv, iv.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Bars to trade");
                        ui.add(egui::DragValue::new(&mut set.replay.session_bars).range(20..=1000));
                        ui.end_row();
                        ui.label("History shown first");
                        ui.add(egui::DragValue::new(&mut set.replay.lookback_bars).range(30..=1000));
                        ui.end_row();
                        ui.label("Starting cash");
                        ui.add(egui::DragValue::new(&mut set.broker.starting_cash).range(1000.0..=10_000_000.0).prefix("$").speed(500.0));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut set.replay.blind, "Hide ticker and dates until the end");
                        ui.end_row();
                    });
                    if *set != before {
                        core.dirty = true;
                    }
                    if set.replay_interval.intraday() {
                        ui.label(RichText::new("Yahoo keeps 60 days of 5m and 15m bars and two years of hourly bars.").small().color(p.muted));
                    }
                    ui.add_space(12.0);
                    let label = if self.waiting { "Loading chart" } else { "Start drill" };
                    let btn = egui::Button::new(RichText::new(label).strong().color(p.bg)).fill(p.brass).min_size(egui::vec2(160.0, 34.0));
                    if ui.add_enabled(!self.waiting, btn).clicked() {
                        self.start(core);
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("Space next bar   Shift+Space ten bars   P play   B buy   S sell   F flatten   C cancel orders   E end").font(mono(11.5)).color(p.muted));

                    let sessions = &core.saved.sessions;
                    if !sessions.is_empty() {
                        ui.add_space(22.0);
                        ui.label(RichText::new("Recent drills").font(egui::FontId::new(17.0, heading())));
                        egui::Grid::new("recent").striped(true).num_columns(6).spacing([14.0, 4.0]).show(ui, |ui| {
                            for h in ["Grade", "Ticker", "Bars", "You", "Hold", "Trades"] {
                                ui.label(RichText::new(h).color(p.muted));
                            }
                            ui.end_row();
                            for s in sessions.iter().rev().take(10) {
                                ui.label(RichText::new(s.grade()).font(egui::FontId::new(16.0, heading())).color(p.brass));
                                ui.label(format!("{} {}", s.symbol, s.interval.label()));
                                ui.label(s.bars.to_string());
                                ui.label(RichText::new(format!("{:+.2}%", s.return_pct)).font(mono(12.5)).color(signed(ui, s.return_pct)));
                                ui.label(RichText::new(format!("{:+.2}%", s.hold_pct)).font(mono(12.5)).color(p.muted));
                                ui.label(s.trades.len().to_string());
                                ui.end_row();
                            }
                        });
                    }
                });
            });
        });
    }

    fn transport_ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        let Some(r) = self.session.as_ref() else { return };
        let finished = r.finished();
        ui.horizontal(|ui| {
            if finished {
                if ui.button(RichText::new("New drill").strong()).clicked() {
                    self.session = None;
                    self.start(core);
                    return;
                }
                if ui.button("Change settings").clicked() {
                    self.session = None;
                    return;
                }
                ui.label(RichText::new("Faded bars show what happened after your session.").color(p.muted));
                return;
            }
            if ui.button("Next bar").on_hover_text("Space").clicked() {
                self.step(core);
            }
            if ui.button("Ten bars").on_hover_text("Shift+Space").clicked() {
                for _ in 0..10 {
                    self.step(core);
                }
            }
            if ui.button(if self.playing { "Pause" } else { "Play" }).on_hover_text("P").clicked() {
                self.playing = !self.playing;
            }
            let speed = &mut core.saved.settings.replay.autoplay_bps;
            ui.add(egui::Slider::new(speed, 0.5..=20.0).logarithmic(true).suffix(" bars/s"));
            ui.separator();
            let Some(r) = self.session.as_ref() else { return };
            let done = r.bars_done();
            let total = r.bars_total();
            ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_width(160.0).text(format!("{done} of {total}")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("End session").on_hover_text("E. Closes any position at the last close.").clicked() {
                    self.finish(core);
                }
            });
        });
    }

    fn side_ui(&mut self, ui: &mut Ui, _core: &mut Core) {
        let p = pal(ui.ctx());
        let Some(r) = self.session.as_mut() else { return };
        let iv = r.series.interval;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(r.display_name()).font(egui::FontId::new(22.0, heading())));
            ui.label(RichText::new(iv.label()).color(p.muted));
            if r.blind {
                ui.label(RichText::new("blind").small().color(p.muted)).on_hover_text("Ticker and dates are hidden until the session ends");
            }
        });
        let last = r.last_close();
        let eq = r.account.equity();
        let start = r.account.cfg.starting_cash;
        ui.label(RichText::new(format!("Last {last:.2}")).font(mono(14.0)));
        ui.label(RichText::new(format!("Equity ${eq:.2}  ({:+.2}%)", (eq / start - 1.0) * 100.0)).font(mono(13.0)).color(signed(ui, eq - start)));
        ui.add_space(6.0);
        ui.label(RichText::new("Plan").strong());
        ui.add(egui::TextEdit::multiline(&mut r.plan).hint_text("What are you looking for? Where would you get in, and where are you wrong?").desired_rows(2).desired_width(f32::INFINITY));
        ui.separator();
        let sym = r.symbol().to_string();
        let display = r.display_name().to_string();
        if ticket::position_ui(ui, &r.account, &sym, &display) {
            self.flatten();
        }
        let Some(r) = self.session.as_mut() else { return };
        ui.separator();
        ui.label(RichText::new("Order").strong());
        let atr = atr_last(r.visible());
        let cx = TicketCtx {
            account: &r.account,
            symbol: &sym,
            last,
            atr,
            risk_per_trade: _core.saved.settings.replay.risk_per_trade,
            fill_note: "Market orders fill at the next bar's open.",
        };
        if let Some(req) = self.ticket.ui(ui, &cx) {
            let t = r.series.candles[r.cursor].t;
            self.ticket.submit(req, &mut r.account, t);
        }
        ui.separator();
        ui.label(RichText::new("Working orders").strong());
        for id in ticket::orders_ui(ui, &r.account, Some(&sym)) {
            r.account.cancel(id);
        }
        if !r.account.trades.is_empty() {
            ui.separator();
            let s = stats::compute(&r.account.trades);
            ui.label(RichText::new(format!("{} closed  {:.0}% won  net {}", s.trades, s.win_rate * 100.0, money(s.net))).font(mono(12.5)).color(signed(ui, s.net)));
        }
    }

    fn report_ui(&mut self, ui: &mut Ui, _core: &mut Core) {
        let p = pal(ui.ctx());
        let Some(r) = self.session.as_ref() else { return };
        let Some(rep) = r.report.as_ref() else { return };
        let s = rep.stats();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(rep.grade()).font(egui::FontId::new(54.0, heading())).color(p.brass));
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(&rep.symbol).font(egui::FontId::new(22.0, heading())));
                let intraday = rep.interval.intraday();
                ui.label(RichText::new(format!("{} to {}", fmt_date(rep.start_t, intraday), fmt_date(rep.end_t, intraday))).color(p.muted));
                if r.blind {
                    ui.label(RichText::new(format!("Played as \"{}\"", r.alias)).small().color(p.muted));
                }
            });
        });
        ui.add_space(6.0);
        egui::Grid::new("report").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            stat(ui, "Your return", format!("{:+.2}%", rep.return_pct), Some(signed(ui, rep.return_pct)));
            stat(ui, "Buy and hold", format!("{:+.2}%", rep.hold_pct), None);
            stat(ui, "Max drawdown", format!("{:.2}%", rep.max_dd * 100.0), None);
            stat(ui, "Trades", s.trades.to_string(), None);
            if s.trades > 0 {
                stat(ui, "Win rate", format!("{:.0}%", s.win_rate * 100.0), None);
                stat(ui, "Net P&L", money(s.net), Some(signed(ui, s.net)));
                stat(ui, "Profit factor", if s.profit_factor.is_finite() { format!("{:.2}", s.profit_factor) } else { "no losses".into() }, None);
                if let Some(r) = s.avg_r {
                    stat(ui, "Average R", format!("{r:+.2}"), Some(signed(ui, r)));
                }
            }
        });
        ui.add_space(6.0);
        let curve: Vec<f64> = r.account.equity_curve.iter().map(|e| e.1).collect();
        ui.label(RichText::new("Equity").strong());
        super::chart::line_chart(ui, &curve, Some(r.account.cfg.starting_cash), 90.0);
        ui.add_space(6.0);
        ui.label(RichText::new("Notes").strong());
        for line in stats::coaching(&s, rep.return_pct, rep.hold_pct) {
            ui.label(line.to_string());
            ui.add_space(2.0);
        }
        if !rep.plan.trim().is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Your plan was").color(p.muted));
            ui.label(RichText::new(&rep.plan).italics());
        }
        if !rep.trades.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("Trades").strong());
            egui::Grid::new("rtrades").striped(true).num_columns(4).show(ui, |ui| {
                for t in &rep.trades {
                    ui.label(if t.side == Side::Buy { "Long" } else { "Short" });
                    ui.label(RichText::new(format!("{:.0} @ {:.2} > {:.2}", t.qty, t.entry_px, t.exit_px)).font(mono(12.0)));
                    ui.label(RichText::new(format!("{:+.2}", t.pnl)).font(mono(12.0)).color(signed(ui, t.pnl)));
                    ui.label(RichText::new(t.r_multiple.map(|r| format!("{r:+.1}R")).unwrap_or_default()).font(mono(12.0)).color(p.muted));
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Trades are saved to your journal, where you can add notes.").small().color(p.muted));
        }
    }
}
