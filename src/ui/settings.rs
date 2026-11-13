use super::theme::{heading, mono, pal};
use super::Core;
use crate::model::Interval;
use crate::store::{Saved, ThemeChoice};
use egui::{RichText, Ui};

#[derive(Default)]
pub struct SettingsScreen {
    watchlist_text: Option<String>,
    import_symbol: String,
    import_interval: Interval,
    confirm_wipe: bool,
}

impl SettingsScreen {
    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(720.0);
                let s = &mut core.saved.settings;
                let before = s.clone();
                let section = |ui: &mut Ui, t: &str| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(t).font(egui::FontId::new(19.0, heading())));
                };

                section(ui, "Broker");
                ui.label(RichText::new("Used by new replay drills and when you reset the desk account.").color(p.muted));
                egui::Grid::new("broker").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    let b = &mut s.broker;
                    ui.label("Starting cash");
                    ui.add(egui::DragValue::new(&mut b.starting_cash).range(1000.0..=10_000_000.0).prefix("$").speed(500.0));
                    ui.end_row();
                    ui.label("Fee per order");
                    ui.add(egui::DragValue::new(&mut b.commission_per_order).range(0.0..=50.0).prefix("$").speed(0.05));
                    ui.end_row();
                    ui.label("Fee per share");
                    ui.add(egui::DragValue::new(&mut b.commission_per_share).range(0.0..=0.1).prefix("$").speed(0.0005).max_decimals(4));
                    ui.end_row();
                    ui.label("Slippage");
                    ui.add(egui::DragValue::new(&mut b.slippage_bps).range(0.0..=100.0).suffix(" bps").speed(0.1));
                    ui.end_row();
                    ui.label("Leverage");
                    ui.add(egui::DragValue::new(&mut b.leverage).range(1.0..=6.0).suffix("x").speed(0.05));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut b.allow_short, "Allow short selling");
                    ui.end_row();
                });

                section(ui, "Practice");
                egui::Grid::new("practice").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    ui.label("Risk per trade");
                    let mut pct = s.replay.risk_per_trade * 100.0;
                    if ui.add(egui::DragValue::new(&mut pct).range(0.1..=10.0).suffix("% of equity").speed(0.05)).changed() {
                        s.replay.risk_per_trade = pct / 100.0;
                    }
                    ui.end_row();
                    ui.label("Quiz horizon");
                    ui.add(egui::DragValue::new(&mut s.quiz.horizon_bars).range(2..=60).suffix(" bars"));
                    ui.end_row();
                    ui.label("Sideways band");
                    ui.add(egui::DragValue::new(&mut s.quiz.min_move_atr).range(0.0..=3.0).suffix(" ATR").speed(0.05));
                    ui.end_row();
                });

                section(ui, "Display");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.theme, ThemeChoice::Dark, "Dark");
                    ui.selectable_value(&mut s.theme, ThemeChoice::Light, "Light");
                    ui.add_space(16.0);
                    ui.label("Scale");
                    ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.75).step_by(0.05));
                });

                section(ui, "Watchlist");
                ui.label(RichText::new("Drills and quiz questions pick from these. Separate tickers with spaces or commas. Use Yahoo symbols, e.g. BRK-B, ^GSPC, BTC-USD, EURUSD=X.").color(p.muted));
                let text = self.watchlist_text.get_or_insert_with(|| s.watchlist.join(" "));
                ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(f32::INFINITY).font(mono(13.0)));
                let (save, restore) = ui.horizontal(|ui| (ui.button("Save watchlist").clicked(), ui.button("Restore defaults").clicked())).inner;
                if save {
                    let mut list: Vec<String> = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|x| !x.is_empty()).map(|x| x.to_uppercase()).collect();
                    list.dedup();
                    s.watchlist = list;
                    self.watchlist_text = None;
                }
                if restore {
                    s.watchlist = crate::config::DEFAULT_WATCHLIST.iter().map(|x| x.to_string()).collect();
                    self.watchlist_text = None;
                }
                let changed = *s != before;
                if changed {
                    core.dirty = true;
                }

                section(ui, "Your own data");
                ui.label(RichText::new("Import a CSV with Date, Open, High, Low, Close and Volume columns. It is saved under the ticker you give it, so replay drills can use it by name.").color(p.muted));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.import_symbol).hint_text("Ticker name").desired_width(110.0));
                    egui::ComboBox::from_id_salt("imp_iv").selected_text(self.import_interval.label()).show_ui(ui, |ui| {
                        for iv in Interval::ALL {
                            ui.selectable_value(&mut self.import_interval, iv, iv.label());
                        }
                    });
                    if ui.add_enabled(!self.import_symbol.trim().is_empty(), egui::Button::new("Import CSV")).clicked()
                        && let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv", "txt"]).pick_file() {
                            match crate::data::import_csv(&path, self.import_symbol.trim(), self.import_interval) {
                                Ok(series) => {
                                    core.toast(format!("Imported {} bars as {} {}.", series.candles.len(), series.symbol, series.interval.label()), false);
                                    core.library.insert((series.symbol.clone(), series.interval), series);
                                }
                                Err(e) => core.toast(format!("Import failed: {e:#}"), true),
                            }
                        }
                });

                section(ui, "Storage");
                let dir = crate::data::data_dir();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(dir.display().to_string()).font(mono(12.5)));
                    if ui.small_button("Copy path").clicked() {
                        ui.ctx().copy_text(dir.display().to_string());
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Clear price cache").on_hover_text("Deletes downloaded prices. Imported CSVs are removed too.").clicked() {
                        let _ = std::fs::remove_dir_all(dir.join("cache"));
                        core.library.clear();
                        core.toast("Price cache cleared.", false);
                    }
                });
                ui.add_space(6.0);
                if self.confirm_wipe {
                    ui.label(RichText::new("This deletes your journal, drill history, quiz record and desk account. It can't be undone.").color(p.down));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Erase everything").color(p.down)).clicked() {
                            core.saved = Saved::default();
                            core.dirty = true;
                            self.confirm_wipe = false;
                            self.watchlist_text = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_wipe = false;
                        }
                    });
                } else if ui.button("Erase all progress").clicked() {
                    self.confirm_wipe = true;
                }

                section(ui, "About");
                ui.label(format!("FakeTrader {}. Practice only: no real orders are ever sent.", env!("CARGO_PKG_VERSION")));
                ui.label(RichText::new("Market data comes from Yahoo Finance's public chart endpoint and is for personal, non-commercial use.").color(p.muted));
                ui.label(RichText::new("Fonts: Barlow Semi Condensed and IBM Plex Mono, SIL Open Font License.").color(p.muted));
            });
        });
    }
}
