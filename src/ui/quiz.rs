use super::chart::{ChartData, ChartView};
use super::theme::{heading, mono, pal};
use super::{Core, Purpose};
use crate::model::{Interval, Series};
use crate::patterns::{Bias, Pattern};
use crate::trainer::{self, Answer, Call, QuizItem, Question};
use egui::{Key, RichText, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pool {
    All,
    Candles,
    Structures,
}

pub struct QuizScreen {
    item: Option<QuizItem>,
    chart: ChartView,
    waiting: bool,
    retries: u8,
    pool: Pool,
    interval: Interval,
    answered_at: Option<f64>,
    rate_line: Option<String>,
    confirm_reset: bool,
}

impl Default for QuizScreen {
    fn default() -> Self {
        Self { item: None, chart: ChartView::default(), waiting: false, retries: 0, pool: Pool::All, interval: Interval::D1, answered_at: None, rate_line: None, confirm_reset: false }
    }
}

impl QuizScreen {
    fn pool(&self) -> Vec<Pattern> {
        match self.pool {
            Pool::All => Vec::new(),
            Pool::Candles => Pattern::ALL.iter().copied().filter(|p| p.is_candle()).collect(),
            Pool::Structures => Pattern::ALL.iter().copied().filter(|p| !p.is_candle()).collect(),
        }
    }

    fn next(&mut self, core: &mut Core) {
        self.waiting = true;
        let sym = core.random_symbol();
        core.request(&sym, self.interval, Purpose::Quiz);
    }

    pub fn on_error(&mut self, core: &mut Core, e: String) {
        if self.waiting && self.retries < 4 {
            self.retries += 1;
            self.next(core);
        } else {
            self.waiting = false;
            core.toast(e, true);
        }
    }

    pub fn on_series(&mut self, core: &mut Core, s: Series) {
        if !self.waiting {
            return;
        }
        let pool = self.pool();
        match trainer::make_question(&s, &core.saved.settings.quiz, &pool, &mut core.rng) {
            Some(q) => {
                self.waiting = false;
                self.retries = 0;
                self.answered_at = None;
                self.rate_line = None;
                self.chart.reset();
                // Show enough context to read the setup, not the whole history.
                self.chart.follow = true;
                self.item = Some(q);
            }
            None if self.retries < 6 => {
                self.retries += 1;
                self.next(core);
            }
            None => {
                self.waiting = false;
                self.retries = 0;
                core.toast("No matching patterns found in the tickers tried. Add more tickers to your watchlist or widen the pattern pool.", true);
            }
        }
    }

    fn answer(&mut self, core: &mut Core, a: Answer, now: f64) {
        let Some(item) = self.item.as_mut() else { return };
        if item.answer.is_some() {
            return;
        }
        item.answer = Some(a);
        core.saved.quiz.record(item);
        core.dirty = true;
        self.answered_at = Some(now);
        let rates = trainer::base_rates(std::iter::once(&item.series), &core.saved.settings.quiz);
        if let Some(r) = rates.get(&item.signal.pattern)
            && r.count >= 3 {
                self.rate_line = Some(format!(
                    "On {} {} bars, this pattern appeared {} times. {} bars later price was higher {:.0}% of the time and lower {:.0}%.",
                    item.series.symbol,
                    item.series.interval.label(),
                    r.count,
                    item.horizon,
                    r.higher as f64 / r.count as f64 * 100.0,
                    r.lower as f64 / r.count as f64 * 100.0
                ));
            }
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        let now = ui.input(|i| i.time);

        // Keys: 1-4 answer, Enter next.
        if !Core::typing(ui) {
            let keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4];
            let pressed = ui.input(|i| keys.iter().position(|k| i.key_pressed(*k)));
            let enter = ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::N));
            if let (Some(k), Some(item)) = (pressed, self.item.as_ref())
                && item.answer.is_none() {
                    let a = match &item.question {
                        Question::CallMove => [Call::Higher, Call::Lower, Call::Sideways].get(k).map(|c| Answer::Call(*c)),
                        Question::NamePattern { choices } => choices.get(k).map(|c| Answer::Name(*c)),
                    };
                    if let Some(a) = a {
                        self.answer(core, a, now);
                    }
                }
            if enter && !self.waiting && self.item.as_ref().is_none_or(|i| i.answer.is_some()) {
                self.next(core);
            }
        }

        egui::Panel::right("quiz_side").resizable(true).default_size(340.0).min_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(6.0);
                self.side_ui(ui, core, now);
                ui.add_space(14.0);
                ui.separator();
                self.stats_ui(ui, core);
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal(ui.ctx()).panel)).show(ui, |ui| {
            let Some(item) = self.item.as_ref() else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Press Start in the side panel. Each question is a real chart that just completed a pattern.").color(p.muted));
                });
                return;
            };
            let answered = item.answer.is_some();
            let lift = self.answered_at.map(|t| ((now - t) / 0.7).clamp(0.0, 1.0) as f32).unwrap_or(0.0);
            if answered && lift < 1.0 {
                ui.ctx().request_repaint();
            }
            let end = item.view_end();
            let start = item.signal.index.saturating_sub(250);
            let candles = &item.series.candles[start..=end];
            let shift = |i: usize| i - start;
            let highlight = match item.question {
                Question::CallMove => Some((shift(item.signal.start), shift(item.signal.index))),
                Question::NamePattern { .. } => Some((shift(item.signal.start), shift(item.signal.index))),
            };
            let data = ChartData {
                candles,
                dim_from: answered.then_some(shift(item.signal.index) + 1),
                curtain_slots: if answered { 0 } else { item.horizon },
                curtain_after: (!answered || lift < 1.0).then(|| shift(item.signal.index)),
                curtain_lift: if answered { 1.0 - (1.0 - lift).powi(3) } else { 0.0 },
                lines: Vec::new(),
                markers: Vec::new(),
                highlight,
                signals: Vec::new(),
                intraday: item.series.interval.intraday(),
                hide_dates: !answered,
                overlays: &core.saved.settings.overlays,
            };
            ui.allocate_ui(ui.available_size() - egui::vec2(0.0, 40.0), |ui| {
                self.chart.show(ui, &data);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                super::chart_toolbar(ui, &mut self.chart, core);
            });
        });
    }

    fn side_ui(&mut self, ui: &mut Ui, core: &mut Core, now: f64) {
        let p = pal(ui.ctx());
        ui.label(RichText::new("Pattern quiz").font(egui::FontId::new(24.0, heading())));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pool, Pool::All, "All patterns");
            ui.selectable_value(&mut self.pool, Pool::Candles, "Candles");
            ui.selectable_value(&mut self.pool, Pool::Structures, "Chart patterns");
        });
        ui.horizontal(|ui| {
            for iv in [Interval::H1, Interval::D1, Interval::W1] {
                ui.selectable_value(&mut self.interval, iv, iv.label());
            }
        });
        ui.add_space(8.0);
        let Some(item) = self.item.as_ref() else {
            let label = if self.waiting { "Finding a chart" } else { "Start" };
            let b = egui::Button::new(RichText::new(label).strong().color(p.bg)).fill(p.brass).min_size(egui::vec2(140.0, 32.0));
            if ui.add_enabled(!self.waiting, b).clicked() {
                self.next(core);
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Two kinds of question: call the next move after a named pattern, or name the pattern on the shaded bars. Keys 1 to 4 answer, Enter moves on.").color(p.muted));
            return;
        };
        let pat = item.signal.pattern;
        let mut chosen: Option<Answer> = None;
        match &item.question {
            Question::CallMove => {
                ui.label(RichText::new(pat.name()).font(egui::FontId::new(20.0, heading())).color(p.brass));
                ui.label(RichText::new(pat.lesson()).color(p.muted));
                ui.add_space(8.0);
                ui.label(format!("Where is price {} bars from now?", item.horizon));
                ui.label(RichText::new(format!("Sideways means it moved less than {:.1} ATR either way.", core.saved.settings.quiz.min_move_atr)).small().color(p.muted));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for (i, c) in [Call::Higher, Call::Lower, Call::Sideways].into_iter().enumerate() {
                        let col = match c {
                            Call::Higher => p.up,
                            Call::Lower => p.down,
                            Call::Sideways => p.muted,
                        };
                        let picked = item.answer == Some(Answer::Call(c));
                        let b = egui::Button::new(RichText::new(format!("{} {}", i + 1, c.label())).color(col)).selected(picked).min_size(egui::vec2(92.0, 30.0));
                        if ui.add_enabled(item.answer.is_none(), b).clicked() {
                            chosen = Some(Answer::Call(c));
                        }
                    }
                });
            }
            Question::NamePattern { choices } => {
                ui.label(RichText::new("Name the pattern").font(egui::FontId::new(20.0, heading())).color(p.brass));
                ui.label(RichText::new("It completes on the last shaded bar.").color(p.muted));
                ui.add_space(6.0);
                for (i, c) in choices.iter().enumerate() {
                    let picked = item.answer == Some(Answer::Name(*c));
                    let b = egui::Button::new(format!("{}  {}", i + 1, c.name())).selected(picked).min_size(egui::vec2(260.0, 28.0));
                    if ui.add_enabled(item.answer.is_none(), b).clicked() {
                        chosen = Some(Answer::Name(*c));
                    }
                }
            }
        }
        if let Some(a) = chosen {
            self.answer(core, a, now);
        }
        let Some(item) = self.item.as_ref() else { return };
        if let Some(ok) = item.correct() {
            ui.add_space(10.0);
            let (head, col) = if ok { ("Right", p.up) } else { ("Wrong", p.down) };
            ui.label(RichText::new(head).font(egui::FontId::new(22.0, heading())).color(col));
            match item.question {
                Question::CallMove => {
                    ui.label(format!("{} bars later price had moved {:+.1} ATR, which counts as {}.", item.horizon, item.move_atr, item.actual.label().to_lowercase()));
                }
                Question::NamePattern { .. } => {
                    ui.label(format!("It was a {}.", pat.name().to_lowercase()));
                    ui.label(RichText::new(pat.lesson()).color(p.muted));
                    ui.label(format!("Afterwards price moved {:+.1} ATR in {} bars.", item.move_atr, item.horizon));
                }
            }
            let bias = match pat.bias() {
                Bias::Bullish => "a bullish",
                Bias::Bearish => "a bearish",
                Bias::Neutral => "a neutral",
            };
            ui.label(RichText::new(format!("Textbook reading: {bias} signal.")).color(p.muted));
            if let Some(line) = &self.rate_line {
                ui.add_space(4.0);
                ui.label(RichText::new(line).color(p.muted));
            }
            ui.label(RichText::new(format!("{} {}", item.series.symbol, crate::model::fmt_date(item.series.candles[item.signal.index].t, item.series.interval.intraday()))).font(mono(12.5)));
            ui.add_space(8.0);
            let label = if self.waiting { "Finding a chart" } else { "Next question" };
            let b = egui::Button::new(RichText::new(label).strong().color(p.bg)).fill(p.brass).min_size(egui::vec2(150.0, 30.0));
            if ui.add_enabled(!self.waiting, b).on_hover_text("Enter").clicked() {
                self.next(core);
            }
        }
    }

    fn stats_ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        let q = &core.saved.quiz;
        let total = q.total();
        ui.label(RichText::new("Your record").strong());
        if total.attempts == 0 {
            ui.label(RichText::new("Answer a few questions to see where your read is strong.").color(p.muted));
            return;
        }
        ui.label(RichText::new(format!("{} of {} right ({:.0}%)   streak {}   best {}", total.correct, total.attempts, total.rate() * 100.0, q.streak, q.best_streak)).font(mono(12.5)));
        ui.add_space(4.0);
        egui::Grid::new("quiz_stats").striped(true).num_columns(3).spacing([12.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new("Pattern").color(p.muted));
            ui.label(RichText::new("Calls").color(p.muted));
            ui.label(RichText::new("Names").color(p.muted));
            ui.end_row();
            for pat in Pattern::ALL {
                let c = q.calls.get(&pat);
                let n = q.names.get(&pat);
                if c.is_none() && n.is_none() {
                    continue;
                }
                ui.label(pat.name());
                for t in [c, n] {
                    match t {
                        Some(t) if t.attempts > 0 => {
                            let col = if t.rate() >= 0.6 { p.up } else if t.rate() < 0.4 { p.down } else { p.text };
                            ui.label(RichText::new(format!("{}/{}", t.correct, t.attempts)).font(mono(12.0)).color(col));
                        }
                        _ => {
                            ui.label("");
                        }
                    }
                }
                ui.end_row();
            }
        });
        if self.confirm_reset {
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Erase quiz record").color(p.down)).clicked() {
                    core.saved.quiz = Default::default();
                    core.dirty = true;
                    self.confirm_reset = false;
                }
                if ui.button("Keep it").clicked() {
                    self.confirm_reset = false;
                }
            });
        } else if ui.small_button("Reset quiz record").clicked() {
            self.confirm_reset = true;
        }
    }
}
