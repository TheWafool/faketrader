pub mod chart;
mod desk;
mod journal;
mod library;
mod quiz;
mod replay;
mod settings;
pub mod theme;
pub mod ticket;

use crate::broker::Trade;
use crate::data::{self, SearchHit};
use crate::model::{Candle, Interval, Rng, Series};
use crate::store::{self, Saved, ThemeChoice};
use egui::{RichText, Ui};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    Replay,
    Quiz,
    Desk,
    Scan,
}

pub enum Msg {
    Series { purpose: Purpose, symbol: String, interval: Interval, result: Result<Series, String> },
    Search { query: String, result: Result<Vec<SearchHit>, String> },
    Quotes(Vec<(String, Result<data::Quote, String>)>),
}

/// Background downloads. Each job runs on its own thread and reports back on a channel.
pub struct Net {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    pub busy: usize,
}

impl Net {
    fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, ctx, busy: 0 }
    }
    fn run(&mut self, job: impl FnOnce() -> Msg + Send + 'static) {
        self.busy += 1;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }
    pub fn series(&mut self, symbol: &str, interval: Interval, force: bool, purpose: Purpose) {
        let symbol = symbol.trim().to_uppercase();
        self.run(move || {
            let result = data::get_series(&symbol, interval, force).map_err(|e| format!("{e:#}"));
            Msg::Series { purpose, symbol, interval, result }
        });
    }
    pub fn search(&mut self, query: &str) {
        let query = query.to_string();
        self.run(move || {
            let result = data::search(&query).map_err(|e| format!("{e:#}"));
            Msg::Search { query, result }
        });
    }
    pub fn quotes(&mut self, symbols: Vec<String>) {
        self.run(move || Msg::Quotes(symbols.into_iter().map(|s| { let r = data::fetch_quote(&s).map_err(|e| format!("{e:#}")); (s, r) }).collect()));
    }
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// State shared by every screen.
pub struct Core {
    pub saved: Saved,
    pub dirty: bool,
    pub net: Net,
    /// Series loaded this run, by symbol and interval.
    pub library: HashMap<(String, Interval), Series>,
    pub status: Option<Status>,
    pub rng: Rng,
    /// Series that were already in memory when requested, delivered next frame.
    ready: Vec<(Purpose, Series)>,
}

impl Core {
    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status { text: text.into(), error, at: Instant::now() });
    }
    /// Get a series for `purpose`: from memory if loaded, otherwise from cache or network.
    pub fn request(&mut self, symbol: &str, interval: Interval, purpose: Purpose) {
        let key = (symbol.trim().to_uppercase(), interval);
        match self.library.get(&key) {
            Some(s) if crate::model::now_unix() - s.fetched_at < crate::config::cache_ttl_secs(interval.secs()) || purpose != Purpose::Desk => {
                self.ready.push((purpose, s.clone()))
            }
            _ => self.net.series(&key.0, interval, false, purpose),
        }
    }
    pub fn random_symbol(&mut self) -> String {
        let w = &self.saved.settings.watchlist;
        if w.is_empty() { "SPY".into() } else { w[self.rng.range(0, w.len())].clone() }
    }
    /// Move closed trades from an account into the journal with journal-wide ids.
    pub fn journal_trades(&mut self, trades: Vec<Trade>) {
        for mut t in trades {
            let next = self.saved.journal.iter().map(|t| t.id).max().unwrap_or(0) + 1;
            t.id = next;
            self.saved.journal.push(t);
            self.dirty = true;
        }
    }
    pub fn typing(ui: &Ui) -> bool {
        ui.ctx().memory(|m| m.focused().is_some())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Replay,
    Quiz,
    Desk,
    Journal,
    Patterns,
    Settings,
}

impl Screen {
    const ALL: [Screen; 6] = [Screen::Replay, Screen::Quiz, Screen::Desk, Screen::Journal, Screen::Patterns, Screen::Settings];
    fn label(self) -> &'static str {
        match self {
            Screen::Replay => "Replay",
            Screen::Quiz => "Pattern quiz",
            Screen::Desk => "Paper desk",
            Screen::Journal => "Journal",
            Screen::Patterns => "Patterns",
            Screen::Settings => "Settings",
        }
    }
}

pub struct App {
    core: Core,
    screen: Screen,
    replay: replay::ReplayScreen,
    quiz: quiz::QuizScreen,
    desk: desk::DeskScreen,
    journal: journal::JournalScreen,
    library: library::PatternsScreen,
    settings: settings::SettingsScreen,
    last_save: Instant,
    applied_theme: Option<(ThemeChoice, f32)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let (saved, err) = store::load();
        let mut core = Core {
            saved,
            dirty: false,
            net: Net::new(cc.egui_ctx.clone()),
            library: HashMap::new(),
            status: None,
            rng: Rng::seeded(),
            ready: Vec::new(),
        };
        if let Some(e) = err {
            core.toast(e, true);
        }
        Self {
            core,
            screen: Screen::Replay,
            replay: Default::default(),
            quiz: Default::default(),
            desk: Default::default(),
            journal: Default::default(),
            library: Default::default(),
            settings: Default::default(),
            last_save: Instant::now(),
            applied_theme: None,
        }
    }

    fn save_now(&mut self) {
        match store::save(&self.core.saved) {
            Ok(()) => self.core.dirty = false,
            Err(e) => self.core.toast(format!("Couldn't save progress: {e}. Check that {} is writable.", store::state_path().display()), true),
        }
        self.last_save = Instant::now();
    }

    fn dispatch(&mut self, msg: Msg) {
        let c = &mut self.core;
        match msg {
            Msg::Series { purpose, symbol, interval, result } => match result {
                Ok(s) => {
                    c.library.insert((symbol, interval), s.clone());
                    self.route(purpose, s);
                }
                Err(e) => match purpose {
                    Purpose::Scan => self.library.on_scan_error(&symbol),
                    Purpose::Quiz => self.quiz.on_error(&mut self.core, e),
                    _ => {
                        self.replay.waiting = false;
                        self.core.toast(e, true)
                    }
                },
            },
            Msg::Search { query, result } => self.desk.on_search(c, query, result),
            Msg::Quotes(q) => self.desk.on_quotes(c, q),
        }
    }

    fn route(&mut self, purpose: Purpose, s: Series) {
        match purpose {
            Purpose::Replay => self.replay.on_series(&mut self.core, s),
            Purpose::Quiz => self.quiz.on_series(&mut self.core, s),
            Purpose::Desk => self.desk.on_series(&mut self.core, s),
            Purpose::Scan => self.library.on_series(&mut self.core, s),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let theme = (self.core.saved.settings.theme, self.core.saved.settings.ui_scale);
        if self.applied_theme != Some(theme) {
            theme::apply(ctx, theme.0 == ThemeChoice::Dark, theme.1.clamp(0.7, 2.0));
            self.applied_theme = Some(theme);
        }
        while let Ok(msg) = self.core.net.rx.try_recv() {
            self.core.net.busy = self.core.net.busy.saturating_sub(1);
            self.dispatch(msg);
        }
        for (purpose, s) in std::mem::take(&mut self.core.ready) {
            self.route(purpose, s);
        }
        self.desk.tick(&mut self.core);
        if self.core.dirty && self.last_save.elapsed() > Duration::from_secs(4) {
            self.save_now();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let p = theme::pal(ui.ctx());
        egui::Panel::top("nav").frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::symmetric(14, 8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("FakeTrader").font(egui::FontId::new(20.0, theme::heading())).color(p.brass));
                ui.add_space(18.0);
                for s in Screen::ALL {
                    let on = self.screen == s;
                    let text = RichText::new(s.label()).size(16.0).color(if on { p.text } else { p.muted });
                    let r = ui.add(egui::Button::new(text).frame(false));
                    if on {
                        let rect = r.rect;
                        ui.painter().hline(rect.x_range(), rect.bottom() + 3.0, egui::Stroke::new(2.0, p.brass));
                    }
                    if r.clicked() {
                        self.screen = s;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dark = self.core.saved.settings.theme == ThemeChoice::Dark;
                    if ui.button(if dark { "Light theme" } else { "Dark theme" }).clicked() {
                        self.core.saved.settings.theme = if dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                        self.core.dirty = true;
                    }
                    if self.core.net.busy > 0 {
                        ui.spinner();
                        ui.label(RichText::new("Loading prices").color(p.muted));
                    }
                });
            });
        });

        let status = self.core.status.as_ref().filter(|s| s.at.elapsed() < Duration::from_secs(if s.error { 12 } else { 5 }));
        if let Some(s) = status {
            let (text, error) = (s.text.clone(), s.error);
            egui::Panel::bottom("status").frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::symmetric(14, 5))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).color(if error { p.down } else { p.muted }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            self.core.status = None;
                        }
                    });
                });
            });
        }

        let core = &mut self.core;
        match self.screen {
            Screen::Replay => self.replay.ui(ui, core),
            Screen::Quiz => self.quiz.ui(ui, core),
            Screen::Desk => self.desk.ui(ui, core),
            Screen::Journal => self.journal.ui(ui, core),
            Screen::Patterns => self.library.ui(ui, core),
            Screen::Settings => self.settings.ui(ui, core),
        }
    }

    fn on_exit(&mut self) {
        self.save_now();
    }
}

/// Labelled number in a stat grid.
pub fn stat(ui: &mut Ui, label: &str, value: String, color: Option<egui::Color32>) {
    let p = theme::pal(ui.ctx());
    ui.label(RichText::new(label).color(p.muted));
    ui.label(RichText::new(value).font(theme::mono(13.5)).color(color.unwrap_or(p.text)));
    ui.end_row();
}

/// Signed dollars: "+$12.50", "-$3.00".
pub fn money(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "+" };
    format!("{sign}${:.2}", v.abs())
}

/// Colour for a signed number.
pub fn signed(ui: &Ui, v: f64) -> egui::Color32 {
    let p = theme::pal(ui.ctx());
    if v > 0.0 { p.up } else if v < 0.0 { p.down } else { p.text }
}

/// Toolbar shared by every chart: tools and indicator toggles.
pub fn chart_toolbar(ui: &mut Ui, view: &mut chart::ChartView, core: &mut Core) {
    let o = &mut core.saved.settings.overlays;
    let before = o.clone();
    ui.horizontal_wrapped(|ui| {
        for t in chart::Tool::ALL {
            if ui.selectable_label(view.tool == t, t.label()).on_hover_text(t.hint()).clicked() {
                view.tool = t;
            }
        }
        if !view.drawings.is_empty() && ui.small_button("Clear drawings").clicked() {
            view.drawings.clear();
        }
        ui.separator();
        ui.toggle_value(&mut o.sma_fast, "SMA 20");
        ui.toggle_value(&mut o.sma_slow, "SMA 50");
        ui.toggle_value(&mut o.ema, "EMA 9");
        ui.toggle_value(&mut o.bollinger, "Bands");
        ui.toggle_value(&mut o.vwap, "VWAP").on_hover_text("Intraday charts only");
        ui.toggle_value(&mut o.volume, "Volume");
        ui.toggle_value(&mut o.patterns, "Pattern tags");
        ui.separator();
        use crate::store::LowerPane;
        for (v, l) in [(LowerPane::None, "No lower pane"), (LowerPane::Rsi, "RSI"), (LowerPane::Macd, "MACD")] {
            ui.selectable_value(&mut o.lower, v, l);
        }
    });
    if *o != before {
        core.dirty = true;
    }
}

/// Fill markers for a set of fills on a candle series.
pub fn fill_markers(candles: &[Candle], fills: &[crate::broker::Fill], symbol: &str) -> Vec<chart::Marker> {
    fills
        .iter()
        .filter(|f| f.symbol == symbol)
        .filter_map(|f| {
            let i = candles.partition_point(|c| c.t <= f.t).checked_sub(1)?;
            Some(chart::Marker { index: i, side: f.side, price: f.price })
        })
        .collect()
}

/// Lines for a position and its working orders.
pub fn account_lines(acct: &crate::broker::Account, symbol: &str) -> Vec<chart::PriceLine> {
    use crate::broker::OrderKind;
    use chart::{LineId, LineKind, PriceLine};
    let mut v = Vec::new();
    if let Some(p) = acct.position(symbol) {
        let label = if p.qty > 0.0 { format!("Long {:.0}", p.qty) } else { format!("Short {:.0}", -p.qty) };
        v.push(PriceLine { price: p.avg, label, kind: LineKind::Entry, id: None, preview: false });
    }
    for o in acct.orders.iter().filter(|o| o.symbol == symbol) {
        let Some(price) = o.kind.price() else { continue };
        let kind = match (o.reduce_only, o.kind) {
            (true, OrderKind::Stop(_)) => LineKind::Stop,
            (true, _) => LineKind::Target,
            _ => LineKind::Order,
        };
        let side = if o.side == crate::model::Side::Buy { "Buy" } else { "Sell" };
        v.push(PriceLine { price, label: format!("{side} {:.0}", o.qty), kind, id: Some(LineId::Order(o.id)), preview: false });
    }
    v
}

pub fn atr_last(candles: &[Candle]) -> f64 {
    crate::indicators::atr(candles, crate::config::indicators::ATR).last().copied().unwrap_or(f64::NAN)
}
