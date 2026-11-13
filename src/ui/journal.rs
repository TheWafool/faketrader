use super::theme::{heading, mono, pal};
use super::ticket::TAGS;
use super::{money, signed, stat, Core};
use crate::broker::Trade;
use crate::model::{fmt_date, Side};
use crate::stats;
use egui::{RichText, Ui};

#[derive(Default)]
pub struct JournalScreen {
    mode: Option<String>,
    tag: Option<String>,
    symbol: String,
    selected: Option<u64>,
    confirm_delete: Option<u64>,
}

impl JournalScreen {
    fn keep(&self, t: &Trade) -> bool {
        self.mode.as_ref().is_none_or(|m| &t.mode == m)
            && self.tag.as_ref().is_none_or(|g| if g == "untagged" { t.tag.is_empty() } else { &t.tag == g })
            && (self.symbol.trim().is_empty() || t.symbol.eq_ignore_ascii_case(self.symbol.trim()))
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        if self.selected.is_some() {
            egui::Panel::right("journal_detail").resizable(true).default_size(320.0).show(ui, |ui| self.detail_ui(ui, core));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Journal").font(egui::FontId::new(26.0, heading())));
                ui.add_space(16.0);
                egui::ComboBox::from_id_salt("jmode").selected_text(self.mode.clone().unwrap_or("All modes".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.mode, None, "All modes");
                    for m in ["Replay", "Desk"] {
                        ui.selectable_value(&mut self.mode, Some(m.to_string()), m);
                    }
                });
                egui::ComboBox::from_id_salt("jtag").selected_text(self.tag.clone().unwrap_or("All setups".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.tag, None, "All setups");
                    ui.selectable_value(&mut self.tag, Some("untagged".into()), "untagged");
                    for t in TAGS {
                        ui.selectable_value(&mut self.tag, Some(t.to_string()), *t);
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut self.symbol).hint_text("Ticker").desired_width(80.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Export CSV").clicked() {
                        self.export(core);
                    }
                });
            });

            let trades: Vec<&Trade> = core.saved.journal.iter().filter(|t| self.keep(t)).collect();
            if trades.is_empty() {
                ui.add_space(30.0);
                ui.label(RichText::new("No trades here yet. Finish a replay drill or close a paper desk trade and it lands in this journal.").color(p.muted));
                return;
            }
            let s = stats::compute(trades.iter().copied());
            ui.add_space(6.0);
            ui.columns(3, |cols| {
                egui::Grid::new("js1").num_columns(2).spacing([14.0, 3.0]).show(&mut cols[0], |ui| {
                    stat(ui, "Trades", s.trades.to_string(), None);
                    stat(ui, "Win rate", format!("{:.1}%", s.win_rate * 100.0), None);
                    stat(ui, "Net P&L", money(s.net), Some(signed(ui, s.net)));
                    stat(ui, "Fees paid", format!("${:.2}", s.fees), None);
                });
                egui::Grid::new("js2").num_columns(2).spacing([14.0, 3.0]).show(&mut cols[1], |ui| {
                    stat(ui, "Profit factor", if s.profit_factor.is_finite() { format!("{:.2}", s.profit_factor) } else { "no losses".into() }, None);
                    stat(ui, "Per trade", money(s.expectancy), Some(signed(ui, s.expectancy)));
                    stat(ui, "Average R", s.avg_r.map(|r| format!("{r:+.2} ({} with stop)", s.with_stop)).unwrap_or("no stops set".into()), s.avg_r.map(|r| signed(ui, r)));
                    stat(ui, "Exit capture", s.capture.map(|c| format!("{:.0}% of best move", c * 100.0)).unwrap_or("-".into()), None);
                });
                egui::Grid::new("js3").num_columns(2).spacing([14.0, 3.0]).show(&mut cols[2], |ui| {
                    stat(ui, "Avg win", format!("${:.2}", s.avg_win), Some(signed(ui, s.avg_win)));
                    stat(ui, "Avg loss", money(s.avg_loss), Some(signed(ui, s.avg_loss)));
                    stat(ui, "Largest", format!("{} / {}", money(s.largest_win), money(s.largest_loss)), None);
                    stat(ui, "Streaks", format!("{} won / {} lost", s.max_win_streak, s.max_loss_streak), None);
                });
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Cumulative P&L by trade").strong());
            super::chart::line_chart(ui, &stats::pnl_curve(&trades), Some(0.0), 120.0);

            ui.add_space(8.0);
            ui.label(RichText::new("By setup").strong());
            egui::Grid::new("bytag").striped(true).num_columns(5).spacing([18.0, 3.0]).show(ui, |ui| {
                for h in ["Setup", "Trades", "Win rate", "Net", "Avg R"] {
                    ui.label(RichText::new(h).color(p.muted));
                }
                ui.end_row();
                for (tag, st) in stats::by_tag(trades.iter().copied()) {
                    ui.label(tag);
                    ui.label(st.trades.to_string());
                    ui.label(format!("{:.0}%", st.win_rate * 100.0));
                    ui.label(RichText::new(format!("{:+.2}", st.net)).font(mono(12.5)).color(signed(ui, st.net)));
                    ui.label(st.avg_r.map(|r| format!("{r:+.2}")).unwrap_or_default());
                    ui.end_row();
                }
            });

            ui.add_space(8.0);
            ui.label(RichText::new("Trades").strong());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("jtrades").striped(true).num_columns(11).spacing([14.0, 3.0]).show(ui, |ui| {
                    for h in ["Closed", "Mode", "Ticker", "Side", "Qty", "Entry", "Exit", "P&L", "R", "Setup", "Grade"] {
                        ui.label(RichText::new(h).color(p.muted));
                    }
                    ui.end_row();
                    for t in trades.iter().rev() {
                        let sel = self.selected == Some(t.id);
                        if ui.selectable_label(sel, RichText::new(fmt_date(t.exit_t, false)).font(mono(12.0))).clicked() {
                            self.selected = if sel { None } else { Some(t.id) };
                        }
                        ui.label(&t.mode);
                        ui.label(RichText::new(&t.symbol).font(mono(12.0)));
                        ui.label(if t.side == Side::Buy { "Long" } else { "Short" });
                        ui.label(RichText::new(format!("{:.0}", t.qty)).font(mono(12.0)));
                        ui.label(RichText::new(format!("{:.2}", t.entry_px)).font(mono(12.0)));
                        ui.label(RichText::new(format!("{:.2}", t.exit_px)).font(mono(12.0)));
                        ui.label(RichText::new(format!("{:+.2}", t.pnl)).font(mono(12.0)).color(signed(ui, t.pnl)));
                        ui.label(RichText::new(t.r_multiple.map(|r| format!("{r:+.2}")).unwrap_or_default()).font(mono(12.0)));
                        ui.label(&t.tag);
                        ui.label(t.grade.map(|g| "●".repeat(g as usize)).unwrap_or_default());
                        ui.end_row();
                    }
                });
            });
        });
    }

    fn detail_ui(&mut self, ui: &mut Ui, core: &mut Core) {
        let p = pal(ui.ctx());
        let Some(id) = self.selected else { return };
        let Some(t) = core.saved.journal.iter_mut().find(|t| t.id == id) else {
            self.selected = None;
            return;
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", if t.side == Side::Buy { "Long" } else { "Short" }, t.symbol)).font(egui::FontId::new(22.0, heading())));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Close").clicked() {
                    self.selected = None;
                }
            });
        });
        ui.label(RichText::new(format!("{} to {}", fmt_date(t.entry_t, true), fmt_date(t.exit_t, true))).color(p.muted));
        egui::Grid::new("jd").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
            stat(ui, "Shares", format!("{:.0}", t.qty), None);
            stat(ui, "Entry / exit", format!("{:.2} / {:.2}", t.entry_px, t.exit_px), None);
            stat(ui, "Net P&L", format!("{} ({:+.2}%)", money(t.pnl), t.return_pct()), Some(signed(ui, t.pnl)));
            stat(ui, "R multiple", t.r_multiple.map(|r| format!("{r:+.2}")).unwrap_or("no stop".into()), None);
            stat(ui, "Best move", format!("{:+.2} per share", t.mfe), None);
            stat(ui, "Worst move", format!("{:.2} per share", -t.mae), None);
            stat(ui, "Fees", format!("${:.2}", t.fees), None);
        });
        if t.pnl > 0.0 && t.mfe > 0.0 {
            let kept = ((t.exit_px - t.entry_px) * t.side.sign() / t.mfe * 100.0).clamp(0.0, 100.0);
            ui.label(RichText::new(format!("You kept {kept:.0}% of the best price this trade reached.")).color(p.muted));
        }
        ui.add_space(8.0);
        let mut changed = false;
        ui.label(RichText::new("Setup").strong());
        egui::ComboBox::from_id_salt("jd_tag").selected_text(if t.tag.is_empty() { "none" } else { &t.tag }).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut t.tag, String::new(), "none").changed();
            for g in TAGS {
                changed |= ui.selectable_value(&mut t.tag, g.to_string(), *g).changed();
            }
        });
        ui.label(RichText::new("How well did you follow your plan?").strong());
        ui.horizontal(|ui| {
            for g in 1..=5u8 {
                if ui.selectable_label(t.grade == Some(g), g.to_string()).clicked() {
                    t.grade = if t.grade == Some(g) { None } else { Some(g) };
                    changed = true;
                }
            }
        });
        ui.label(RichText::new("Notes").strong());
        changed |= ui
            .add(egui::TextEdit::multiline(&mut t.notes).hint_text("Why did you take it? What would you do differently?").desired_rows(6).desired_width(f32::INFINITY))
            .changed();
        if changed {
            core.dirty = true;
        }
        ui.add_space(12.0);
        if self.confirm_delete == Some(id) {
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Delete trade").color(p.down)).clicked() {
                    core.saved.journal.retain(|t| t.id != id);
                    core.dirty = true;
                    self.selected = None;
                    self.confirm_delete = None;
                }
                if ui.button("Keep").clicked() {
                    self.confirm_delete = None;
                }
            });
        } else if ui.small_button("Delete from journal").clicked() {
            self.confirm_delete = Some(id);
        }
    }

    fn export(&self, core: &mut Core) {
        let Some(path) = rfd::FileDialog::new().set_file_name("faketrader-journal.csv").add_filter("CSV", &["csv"]).save_file() else { return };
        let res = (|| -> anyhow::Result<()> {
            let mut w = csv::Writer::from_path(&path)?;
            w.write_record(["id", "mode", "symbol", "side", "qty", "entry_time", "entry", "exit_time", "exit", "pnl", "fees", "r", "mfe", "mae", "setup", "grade", "notes"])?;
            for t in core.saved.journal.iter().filter(|t| self.keep(t)) {
                let ts = |x: i64| chrono::DateTime::from_timestamp(x, 0).map(|d| d.to_rfc3339()).unwrap_or_default();
                w.write_record([
                    t.id.to_string(),
                    t.mode.clone(),
                    t.symbol.clone(),
                    if t.side == Side::Buy { "long".into() } else { "short".into() },
                    t.qty.to_string(),
                    ts(t.entry_t),
                    format!("{:.4}", t.entry_px),
                    ts(t.exit_t),
                    format!("{:.4}", t.exit_px),
                    format!("{:.2}", t.pnl),
                    format!("{:.2}", t.fees),
                    t.r_multiple.map(|r| format!("{r:.3}")).unwrap_or_default(),
                    format!("{:.4}", t.mfe),
                    format!("{:.4}", t.mae),
                    t.tag.clone(),
                    t.grade.map(|g| g.to_string()).unwrap_or_default(),
                    t.notes.clone(),
                ])?;
            }
            w.flush()?;
            Ok(())
        })();
        match res {
            Ok(()) => core.toast(format!("Exported to {}", path.display()), false),
            Err(e) => core.toast(format!("Export failed: {e}"), true),
        }
    }
}
