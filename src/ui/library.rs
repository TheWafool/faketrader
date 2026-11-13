use super::chart::{ChartData, ChartView};
use super::theme::{heading, mono, pal};
use super::{Core, Purpose};
use crate::model::{Interval, Series};
use crate::patterns::{self, Bias, Pattern, Signal};
use crate::trainer::{self, BaseRate};
use egui::{RichText, Ui};
use std::collections::BTreeMap;

pub struct PatternsScreen {
    selected: Pattern,
    rates: BTreeMap<Pattern, BaseRate>,
    pending: usize,
    scanned: Vec<Series>,
    example: Option<(Series, Signal)>,
    chart: ChartView,
}

impl Default for PatternsScreen {
    fn default() -> Self {
        Self { selected: Pattern::BullishEngulfing, rates: BTreeMap::new(), pending: 0, scanned: Vec::new(), example: None, chart: ChartView::default() }
    }
}

impl PatternsScreen {
    pub fn on_series(&mut self, core: &mut Core, s: Series) {
        self.pending = self.pending.saturating_sub(1);
        self.scanned.retain(|x| x.symbol != s.symbol);
        self.scanned.push(s);
        if self.pending == 0 {
            self.rates = trainer::base_rates(&self.scanned, &core.saved.settings.quiz);
            core.toast(format!("Scanned {} tickers.", self.scanned.len()), false);
        }
    }

    pub fn on_scan_error(&mut self, symbol: &str) {
        self.pending = self.pending.saturating_sub(1);
        let _ = symbol;
    }

    fn scan(&mut self, core: &mut Core) {
        let list = core.saved.settings.watchlist.clone();
        self.pending = list.len();
        for s in list {
            core.request(&s, Interval::D1, Purpose::Scan);
        }
    }

    fn example(&mut self, core: &mut Core) {
        let mut found: Vec<(usize, Signal)> = Vec::new();
        for (i, s) in self.scanned.iter().enumerate() {
            for sig in patterns::detect(&s.candles) {
                if sig.pattern == self.selected && sig.index + 20 < s.candles.len() && sig.index > 60 {
                    found.push((i, sig));
                }
            }
        }
        if found.is_empty() {
            self.example = None;
            core.toast(format!("No {} found in the scanned tickers.", self.selected.name().to_lowercase()), true);
            return;
        }
        let (i, sig) = found[core.rng.range(0, found.len())];
        self.example = Some((self.scanned[i].clone(), sig));
        self.chart.reset();
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        egui::Panel::left("pat_list").resizable(false).exact_size(240.0).show(ui, |ui| {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (title, candles) in [("Candles", true), ("Chart patterns", false)] {
                    ui.label(RichText::new(title).color(p.muted));
                    for pat in Pattern::ALL.iter().copied().filter(|x| x.is_candle() == candles) {
                        let col = match pat.bias() {
                            Bias::Bullish => p.up,
                            Bias::Bearish => p.down,
                            Bias::Neutral => p.muted,
                        };
                        ui.horizontal(|ui| {
                            let (r, painter) = ui.allocate_painter(egui::vec2(6.0, 16.0), egui::Sense::hover());
                            painter.rect_filled(r.rect.shrink2(egui::vec2(1.0, 2.0)), 1.0, col);
                            if ui.selectable_label(self.selected == pat, pat.name()).clicked() {
                                self.selected = pat;
                                self.example = None;
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let pat = self.selected;
            ui.label(RichText::new(pat.name()).font(egui::FontId::new(28.0, heading())));
            ui.label(RichText::new(match pat.bias() {
                Bias::Bullish => "Read as bullish",
                Bias::Bearish => "Read as bearish",
                Bias::Neutral => "No direction on its own",
            }).color(p.muted));
            ui.add_space(4.0);
            ui.label(pat.lesson());
            ui.add_space(10.0);
            let h = core.saved.settings.quiz.horizon_bars;
            ui.horizontal(|ui| {
                let label = if self.pending > 0 { format!("Scanning, {} left", self.pending) } else if self.scanned.is_empty() { "Scan watchlist".into() } else { "Rescan".into() };
                if ui.add_enabled(self.pending == 0, egui::Button::new(label)).on_hover_text("Loads daily history for every watchlist ticker and counts what followed each pattern").clicked() {
                    self.scan(core);
                }
                if ui.add_enabled(!self.scanned.is_empty(), egui::Button::new("Show a real example")).clicked() {
                    self.example(core);
                }
            });
            if let Some(r) = self.rates.get(&pat) {
                let n = r.count.max(1) as f64;
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{} cases in {} tickers. {} daily bars later: higher {:.0}%, lower {:.0}%, sideways {:.0}%. Average move {:+.2} ATR.",
                        r.count,
                        self.scanned.len(),
                        h,
                        r.higher as f64 / n * 100.0,
                        r.lower as f64 / n * 100.0,
                        (r.count - r.higher - r.lower) as f64 / n * 100.0,
                        r.avg_move_atr
                    ))
                    .font(mono(12.5)),
                );
                ui.label(RichText::new("Past results on these tickers. Small samples swing a lot; treat them as context, not a rule.").small().color(p.muted));
            } else if !self.scanned.is_empty() {
                ui.label(RichText::new("No cases in the scanned tickers.").color(p.muted));
            }

            if !self.rates.is_empty() {
                ui.add_space(10.0);
                ui.collapsing("All patterns compared", |ui| {
                    egui::Grid::new("rates").striped(true).num_columns(5).spacing([16.0, 3.0]).show(ui, |ui| {
                        for hd in ["Pattern", "Cases", "Higher", "Lower", "Avg ATR"] {
                            ui.label(RichText::new(hd).color(p.muted));
                        }
                        ui.end_row();
                        for (k, r) in &self.rates {
                            let n = r.count.max(1) as f64;
                            ui.label(k.name());
                            ui.label(r.count.to_string());
                            ui.label(format!("{:.0}%", r.higher as f64 / n * 100.0));
                            ui.label(format!("{:.0}%", r.lower as f64 / n * 100.0));
                            ui.label(RichText::new(format!("{:+.2}", r.avg_move_atr)).color(super::signed(ui, r.avg_move_atr)));
                            ui.end_row();
                        }
                    });
                });
            }

            if let Some((s, sig)) = &self.example {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} on {}", s.symbol, crate::model::fmt_date(s.candles[sig.index].t, false))).font(mono(12.5)).color(p.muted));
                let start = sig.index.saturating_sub(120);
                let end = (sig.index + h).min(s.candles.len() - 1);
                let candles = &s.candles[start..=end];
                let data = ChartData {
                    candles,
                    dim_from: Some(sig.index - start + 1),
                    curtain_slots: 0,
                    curtain_after: None,
                    curtain_lift: 1.0,
                    lines: Vec::new(),
                    markers: Vec::new(),
                    highlight: Some((sig.start - start, sig.index - start)),
                    signals: Vec::new(),
                    intraday: false,
                    hide_dates: false,
                    overlays: &core.saved.settings.overlays,
                };
                self.chart.show(ui, &data);
            }
        });
    }
}
