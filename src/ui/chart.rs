//! Candlestick chart: pan by dragging, zoom with the wheel, drag order lines to move them.

use super::theme::{mono, pal, Pal};
use crate::config::indicators as ind;
use crate::indicators;
use crate::model::{fmt_date, Candle, Side};
use crate::patterns::{Bias, Signal};
use crate::store::{LowerPane, Overlays};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pan,
    HLine,
    Trend,
    Measure,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Pan, Tool::HLine, Tool::Trend, Tool::Measure];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pan => "Pan",
            Tool::HLine => "Level",
            Tool::Trend => "Trendline",
            Tool::Measure => "Measure",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Pan => "Drag to scroll, wheel to zoom, double-click to jump to the latest bar",
            Tool::HLine => "Click to mark a price level",
            Tool::Trend => "Click two points to draw a line",
            Tool::Measure => "Drag to measure a move in % and bars",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drawing {
    HLine(f64),
    /// (bar index, price) pairs.
    Trend((f64, f64), (f64, f64)),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineId {
    Order(u64),
    TicketEntry,
    TicketStop,
    TicketTarget,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Entry,
    Order,
    Stop,
    Target,
}

#[derive(Clone, Debug)]
pub struct PriceLine {
    pub price: f64,
    pub label: String,
    pub kind: LineKind,
    pub id: Option<LineId>,
    /// Faint line for ticket previews (not yet placed).
    pub preview: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Marker {
    pub index: usize,
    pub side: Side,
    pub price: f64,
}

pub enum ChartEvent {
    LineMoved(LineId, f64),
}

pub struct ChartView {
    /// Index of the right-most bar slot in view (may be fractional or past the data).
    right: f64,
    /// Bars across the view.
    span: f64,
    pub follow: bool,
    pub tool: Tool,
    pub drawings: Vec<Drawing>,
    pending: Option<(f64, f64)>,
    measure: Option<((f64, f64), (f64, f64))>,
    drag_line: Option<LineId>,
}

impl Default for ChartView {
    fn default() -> Self {
        Self { right: 0.0, span: 120.0, follow: true, tool: Tool::Pan, drawings: Vec::new(), pending: None, measure: None, drag_line: None }
    }
}

pub struct ChartData<'a> {
    pub candles: &'a [Candle],
    /// Bars at or after this index are drawn faded (what happened after a session).
    pub dim_from: Option<usize>,
    /// Empty slots kept to the right of the last bar, covered by the replay curtain.
    pub curtain_slots: usize,
    /// Curtain covers everything right of this bar index.
    pub curtain_after: Option<usize>,
    /// 0 = curtain closed, 1 = fully lifted.
    pub curtain_lift: f32,
    pub lines: Vec<PriceLine>,
    pub markers: Vec<Marker>,
    pub highlight: Option<(usize, usize)>,
    pub signals: Vec<Signal>,
    pub intraday: bool,
    pub hide_dates: bool,
    pub overlays: &'a Overlays,
}

const AXIS_W: f32 = 72.0;
const TIME_H: f32 = 22.0;
const RIGHT_PAD_BARS: f64 = 4.0;

impl ChartView {
    pub fn reset(&mut self) {
        self.follow = true;
        self.span = 120.0;
        self.pending = None;
        self.measure = None;
    }

    pub fn show(&mut self, ui: &mut Ui, d: &ChartData) -> (Response, Vec<ChartEvent>) {
        let p = pal(ui.ctx());
        let mut events = Vec::new();
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.bg);
        let n = d.candles.len();
        if n == 0 || rect.width() < 120.0 || rect.height() < 120.0 {
            painter.text(rect.center(), Align2::CENTER_CENTER, "No bars loaded", mono(13.0), p.muted);
            return (resp, events);
        }

        let plot = Rect::from_min_max(rect.min, pos2(rect.right() - AXIS_W, rect.bottom() - TIME_H));
        let lower_h = if d.overlays.lower == LowerPane::None { 0.0 } else { (plot.height() * 0.22).max(60.0) };
        let main = Rect::from_min_max(plot.min, pos2(plot.right(), plot.bottom() - lower_h));
        let lower = Rect::from_min_max(pos2(plot.left(), main.bottom() + 6.0), plot.max);

        // View window.
        let last_slot = (n - 1 + d.curtain_slots) as f64 + RIGHT_PAD_BARS;
        self.span = self.span.clamp(15.0, (n as f64 + 40.0).max(15.0));
        // Following keeps the newest bar in view with a slice of curtain beside it.
        let follow_slot = (n - 1) as f64 + RIGHT_PAD_BARS + (d.curtain_slots as f64).min(self.span * 0.2);
        if self.follow {
            self.right = follow_slot;
        }
        let bw = (plot.width() as f64 / self.span) as f32;

        // Input: zoom.
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (-(scroll as f64) * 0.0025).exp() / zoom as f64;
            if (factor - 1.0).abs() > 1e-4
                && let Some(hp) = resp.hover_pos() {
                    let anchor = self.index_at(plot, hp.x);
                    let new_span = (self.span * factor).clamp(15.0, (n as f64 + 40.0).max(15.0));
                    let frac = (self.right - anchor) / self.span;
                    self.span = new_span;
                    self.right = anchor + frac * new_span;
                    self.follow = (self.right - follow_slot).abs() < 0.5;
                }
        }

        let bw = if bw.is_finite() { (plot.width() as f64 / self.span) as f32 } else { bw };
        let left_idx = self.right - self.span + 1.0;
        let x_of = |i: f64| plot.left() + ((i - left_idx) as f32 + 0.5) * bw;
        let lo = (left_idx.floor().max(0.0)) as usize;
        let hi = ((self.right.ceil()) as usize).min(n - 1);

        // Indicators on everything drawable (callers pass only bars that are allowed to be seen).
        let close = indicators::closes(d.candles);
        let ov = d.overlays;
        let sma_f = ov.sma_fast.then(|| indicators::sma(&close, ind::SMA_FAST));
        let sma_s = ov.sma_slow.then(|| indicators::sma(&close, ind::SMA_SLOW));
        let ema = ov.ema.then(|| indicators::ema(&close, ind::EMA));
        let bb = ov.bollinger.then(|| indicators::bollinger(&close, ind::BB_PERIOD, ind::BB_STDDEV));
        let vwap = (ov.vwap && d.intraday).then(|| indicators::vwap(d.candles));

        // Price range over visible bars.
        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
        if lo <= hi {
            for c in &d.candles[lo..=hi] {
                ymin = ymin.min(c.l);
                ymax = ymax.max(c.h);
            }
        }
        if let Some(bb) = &bb {
            for i in lo..=hi.min(n - 1) {
                if bb.upper[i].is_finite() {
                    ymax = ymax.max(bb.upper[i]);
                    ymin = ymin.min(bb.lower[i]);
                }
            }
        }
        if ymin > ymax {
            let c = d.candles[n - 1];
            ymin = c.l;
            ymax = c.h;
        }
        let r = (ymax - ymin).max(ymax.abs() * 0.002).max(1e-6);
        for l in &d.lines {
            if l.price > ymin - r && l.price < ymax + r {
                ymin = ymin.min(l.price);
                ymax = ymax.max(l.price);
            }
        }
        let pad = (ymax - ymin).max(1e-6) * 0.07;
        let (ymin, ymax) = (ymin - pad, ymax + pad);
        let y_of = |v: f64| main.bottom() - ((v - ymin) / (ymax - ymin)) as f32 * main.height();
        let v_at = |y: f32| ymin + (main.bottom() - y) as f64 / main.height() as f64 * (ymax - ymin);

        // Input: line dragging, tools, panning.
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        if resp.drag_started()
            && let Some(pp) = pointer {
                self.drag_line = d
                    .lines
                    .iter()
                    .filter(|l| l.id.is_some() && main.contains(pp))
                    .map(|l| ((y_of(l.price) - pp.y).abs(), l.id.unwrap()))
                    .filter(|(dist, _)| *dist < 6.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|x| x.1);
                if self.drag_line.is_none() && self.tool == Tool::Measure {
                    let pt = (self.index_at(plot, pp.x), v_at(pp.y));
                    self.measure = Some((pt, pt));
                }
            }
        if resp.dragged() {
            if let (Some(id), Some(pp)) = (self.drag_line, pointer) {
                events.push(ChartEvent::LineMoved(id, v_at(pp.y)));
            } else if let (true, Some(pp)) = (self.measure.is_some(), pointer) {
                let pt = (self.index_at(plot, pp.x), v_at(pp.y));
                if let Some(m) = self.measure.as_mut() {
                    m.1 = pt;
                }
            } else if self.tool != Tool::Measure {
                let dx = resp.drag_delta().x as f64;
                if dx != 0.0 {
                    self.right -= dx / bw as f64;
                    self.right = self.right.clamp(10.0, last_slot);
                    self.follow = (self.right - follow_slot).abs() < 0.5;
                }
            }
        }
        if resp.drag_stopped() {
            self.drag_line = None;
            self.measure = None;
        }
        if resp.double_clicked() {
            self.follow = true;
        } else if resp.clicked()
            && let Some(pp) = pointer.filter(|pp| main.contains(*pp)) {
                let pt = (self.index_at(plot, pp.x), v_at(pp.y));
                match self.tool {
                    Tool::HLine => self.drawings.push(Drawing::HLine(pt.1)),
                    Tool::Trend => match self.pending.take() {
                        None => self.pending = Some(pt),
                        Some(a) => self.drawings.push(Drawing::Trend(a, pt)),
                    },
                    _ => {}
                }
            }
        if resp.hovered() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending = None;
        }
        if resp.hovered() {
            let cursor = if self.drag_line.is_some() {
                egui::CursorIcon::ResizeVertical
            } else if let Some(pp) = resp.hover_pos() {
                let near = d.lines.iter().any(|l| l.id.is_some() && (y_of(l.price) - pp.y).abs() < 6.0);
                match self.tool {
                    _ if near => egui::CursorIcon::ResizeVertical,
                    Tool::Pan => egui::CursorIcon::Grab,
                    _ => egui::CursorIcon::Crosshair,
                }
            } else {
                egui::CursorIcon::Default
            };
            ui.ctx().set_cursor_icon(cursor);
        }

        let mp = painter.with_clip_rect(main);
        // Grid and price axis.
        let step = nice_step((ymax - ymin) / (main.height() as f64 / 48.0).max(2.0));
        let mut v = (ymin / step).ceil() * step;
        let decimals = if step >= 1.0 { 2 } else { ((-step.log10()).ceil() as usize + 1).min(6) };
        while v < ymax {
            let y = y_of(v);
            mp.hline(main.x_range(), y, Stroke::new(1.0, p.line.gamma_multiply(0.45)));
            painter.text(pos2(main.right() + 8.0, y), Align2::LEFT_CENTER, format!("{v:.decimals$}"), mono(11.5), p.muted);
            v += step;
        }
        // Time axis.
        let label_every = ((90.0 / bw).ceil() as usize).max(1);
        let mut i = lo - lo % label_every;
        while i <= hi {
            let x = x_of(i as f64);
            if x > plot.left() && x < plot.right() {
                mp.vline(x, main.y_range(), Stroke::new(1.0, p.line.gamma_multiply(0.25)));
                let text = if d.hide_dates { format!("{:+}", i as i64 - (n as i64 - 1)) } else { fmt_date(d.candles[i].t, d.intraday) };
                painter.text(pos2(x, plot.bottom() + 4.0), Align2::CENTER_TOP, text, mono(11.0), p.muted);
            }
            i += label_every;
        }

        // Pattern highlight: a box around the pattern's bars.
        if let Some((a, b)) = d.highlight {
            let (a, b) = (a.min(n - 1), b.min(n - 1));
            let hi_p = d.candles[a..=b].iter().map(|c| c.h).fold(f64::MIN, f64::max);
            let lo_p = d.candles[a..=b].iter().map(|c| c.l).fold(f64::MAX, f64::min);
            let r = Rect::from_min_max(pos2(x_of(a as f64) - bw * 0.5 - 3.0, y_of(hi_p) - 6.0), pos2(x_of(b as f64) + bw * 0.5 + 3.0, y_of(lo_p) + 6.0));
            mp.rect_filled(r, CornerRadius::same(3), p.brass.gamma_multiply(0.12));
            mp.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.2, p.brass), StrokeKind::Outside);
        }

        // Bollinger fill.
        if let Some(bb) = &bb {
            for i in lo.max(1)..=hi {
                if bb.upper[i - 1].is_finite() && bb.upper[i].is_finite() {
                    let pts = vec![
                        pos2(x_of(i as f64 - 1.0), y_of(bb.upper[i - 1])),
                        pos2(x_of(i as f64), y_of(bb.upper[i])),
                        pos2(x_of(i as f64), y_of(bb.lower[i])),
                        pos2(x_of(i as f64 - 1.0), y_of(bb.lower[i - 1])),
                    ];
                    mp.add(Shape::convex_polygon(pts, p.band.gamma_multiply(0.08), Stroke::NONE));
                }
            }
        }

        // Volume.
        if ov.volume && lo <= hi {
            let vmax = d.candles[lo..=hi].iter().map(|c| c.v).fold(0.0, f64::max);
            if vmax > 0.0 {
                let vh = main.height() * 0.16;
                for (i, c) in d.candles.iter().enumerate().take(hi + 1).skip(lo) {
                    let h = (c.v / vmax) as f32 * vh;
                    let x = x_of(i as f64);
                    let col = if c.bullish() { p.up } else { p.down };
                    let r = Rect::from_min_max(pos2(x - bw * 0.35, main.bottom() - h), pos2(x + bw * 0.35, main.bottom()));
                    mp.rect_filled(r, CornerRadius::ZERO, col.gamma_multiply(0.22));
                }
            }
        }

        // Candles.
        for (i, c) in d.candles.iter().enumerate().take(hi + 1).skip(lo) {
            let x = x_of(i as f64);
            let mut col = if c.bullish() { p.up } else { p.down };
            if d.dim_from.is_some_and(|f| i >= f) {
                col = col.gamma_multiply(0.4);
            }
            mp.vline(x, y_of(c.h)..=y_of(c.l), Stroke::new(1.0_f32.max(bw * 0.08).min(2.0), col));
            let (top, bot) = (y_of(c.o.max(c.c)), y_of(c.o.min(c.c)));
            let half = (bw * 0.36).max(0.5);
            let body = Rect::from_min_max(pos2(x - half, top), pos2(x + half, bot.max(top + 1.0)));
            mp.rect_filled(body, CornerRadius::ZERO, col);
        }

        // Overlay lines.
        let series_line = |vals: &[f64], col: Color32, w: f32| {
            let mut seg: Vec<Pos2> = Vec::new();
            for (i, &v) in vals.iter().enumerate().take(hi + 1).skip(lo) {
                if v.is_finite() {
                    seg.push(pos2(x_of(i as f64), y_of(v)));
                } else if seg.len() > 1 {
                    mp.add(Shape::line(std::mem::take(&mut seg), Stroke::new(w, col)));
                } else {
                    seg.clear();
                }
            }
            if seg.len() > 1 {
                mp.add(Shape::line(seg, Stroke::new(w, col)));
            }
        };
        if let Some(bb) = &bb {
            series_line(&bb.upper, p.band, 1.0);
            series_line(&bb.lower, p.band, 1.0);
            series_line(&bb.mid, p.band.gamma_multiply(0.6), 1.0);
        }
        if let Some(v) = &sma_f {
            series_line(v, p.sma_fast, 1.5);
        }
        if let Some(v) = &sma_s {
            series_line(v, p.sma_slow, 1.5);
        }
        if let Some(v) = &ema {
            series_line(v, p.ema, 1.3);
        }
        if let Some(v) = &vwap {
            series_line(v, p.vwap, 1.3);
        }

        // Pattern tags.
        if bw > 3.0 {
            for s in d.signals.iter().filter(|s| s.index >= lo && s.index <= hi) {
                let c = d.candles[s.index];
                let x = x_of(s.index as f64);
                let (y, align, col) = match s.pattern.bias() {
                    Bias::Bearish => (y_of(c.h) - 6.0, Align2::CENTER_BOTTOM, p.down),
                    _ => (y_of(c.l) + 6.0, Align2::CENTER_TOP, if s.pattern.bias() == Bias::Bullish { p.up } else { p.muted }),
                };
                mp.text(pos2(x, y), align, s.pattern.name(), egui::FontId::proportional(11.5), col);
            }
        }

        // Drawings.
        let draw_stroke = Stroke::new(1.3, p.text.gamma_multiply(0.75));
        for dr in &self.drawings {
            match *dr {
                Drawing::HLine(v) => {
                    mp.hline(main.x_range(), y_of(v), draw_stroke);
                    painter.text(pos2(main.right() + 8.0, y_of(v)), Align2::LEFT_CENTER, format!("{v:.2}"), mono(11.5), p.text);
                }
                Drawing::Trend(a, b) => {
                    // Extend to the right edge so the line projects forward.
                    let slope = if (b.0 - a.0).abs() > 1e-9 { (b.1 - a.1) / (b.0 - a.0) } else { 0.0 };
                    let end_i = self.right;
                    let end_v = a.1 + slope * (end_i - a.0);
                    mp.line_segment([pos2(x_of(a.0), y_of(a.1)), pos2(x_of(b.0), y_of(b.1))], draw_stroke);
                    mp.add(Shape::dashed_line(&[pos2(x_of(b.0), y_of(b.1)), pos2(x_of(end_i), y_of(end_v))], Stroke::new(1.0, p.text.gamma_multiply(0.4)), 5.0, 4.0));
                }
            }
        }
        if let (Some(a), Some(hp)) = (self.pending, resp.hover_pos()) {
            mp.line_segment([pos2(x_of(a.0), y_of(a.1)), hp], Stroke::new(1.0, p.text.gamma_multiply(0.5)));
        }

        // Replay curtain over the bars you haven't seen yet.
        if let Some(after) = d.curtain_after.filter(|_| d.curtain_lift < 1.0) {
            let edge = x_of(after as f64) + bw * 0.5 + 1.0;
            let left = edge + (plot.right() - edge) * d.curtain_lift;
            if left < plot.right() {
                let cr = Rect::from_min_max(pos2(left, plot.top()), pos2(plot.right(), main.bottom()));
                let cp = painter.with_clip_rect(cr);
                cp.rect_filled(cr, CornerRadius::ZERO, p.panel);
                let hatch = Stroke::new(1.0, p.line.gamma_multiply(0.7));
                let mut x = cr.left() - cr.height();
                while x < cr.right() {
                    cp.line_segment([pos2(x, cr.bottom()), pos2(x + cr.height(), cr.top())], hatch);
                    x += 9.0;
                }
                cp.vline(cr.left(), cr.y_range(), Stroke::new(2.0, p.brass));
                if cr.width() > 90.0 {
                    let g = painter.layout_no_wrap("Next bar".into(), egui::FontId::new(13.0, super::theme::heading()), p.brass);
                    let pos = pos2(cr.left() + 10.0, cr.top() + 10.0);
                    cp.rect_filled(Rect::from_min_size(pos - vec2(5.0, 3.0), g.size() + vec2(10.0, 6.0)), CornerRadius::same(2), p.panel);
                    cp.galley(pos, g, p.brass);
                }
            }
        }

        // Price lines (yours, in brass family).
        for l in &d.lines {
            let col = match l.kind {
                LineKind::Entry | LineKind::Order => p.brass,
                LineKind::Stop => p.down,
                LineKind::Target => p.up,
            };
            let col = if l.preview { col.gamma_multiply(0.55) } else { col };
            let y = y_of(l.price);
            if y < main.top() || y > main.bottom() {
                continue;
            }
            let st = Stroke::new(if l.preview { 1.0 } else { 1.4 }, col);
            if l.kind == LineKind::Entry {
                mp.hline(main.x_range(), y, st);
            } else {
                mp.add(Shape::dashed_line(&[pos2(main.left(), y), pos2(main.right(), y)], st, 6.0, 4.0));
            }
            let g = painter.layout_no_wrap(format!("{} {:.2}", l.label, l.price), mono(11.0), p.bg);
            let tag = Rect::from_min_size(pos2(main.right() - g.size().x - 10.0, y - g.size().y / 2.0 - 2.0), g.size() + vec2(8.0, 4.0));
            painter.rect_filled(tag, CornerRadius::same(2), col);
            painter.galley(tag.min + vec2(4.0, 2.0), g, p.bg);
        }

        // Fill markers.
        for m in &d.markers {
            if m.index < lo || m.index > hi {
                continue;
            }
            let x = x_of(m.index as f64);
            let c = d.candles[m.index];
            let s = (bw * 0.45).clamp(4.0, 8.0);
            let (pts, _) = match m.side {
                Side::Buy => {
                    let y = y_of(c.l) + 4.0;
                    (vec![pos2(x, y), pos2(x - s, y + s * 1.4), pos2(x + s, y + s * 1.4)], ())
                }
                Side::Sell => {
                    let y = y_of(c.h) - 4.0;
                    (vec![pos2(x, y), pos2(x + s, y - s * 1.4), pos2(x - s, y - s * 1.4)], ())
                }
            };
            mp.add(Shape::convex_polygon(pts, p.brass, Stroke::new(1.0, p.bg)));
            mp.hline(x - bw * 0.6..=x + bw * 0.6, y_of(m.price), Stroke::new(1.5, p.brass));
        }

        // Lower pane.
        if lower_h > 0.0 {
            let lp = painter.with_clip_rect(lower);
            lp.rect_stroke(lower, CornerRadius::ZERO, Stroke::new(1.0, p.line.gamma_multiply(0.6)), StrokeKind::Inside);
            match ov.lower {
                LowerPane::Rsi => {
                    let r = indicators::rsi(&close, ind::RSI);
                    let yl = |v: f64| lower.bottom() - (v / 100.0) as f32 * lower.height();
                    for lvl in [30.0, 70.0] {
                        lp.add(Shape::dashed_line(&[pos2(lower.left(), yl(lvl)), pos2(lower.right(), yl(lvl))], Stroke::new(1.0, p.muted.gamma_multiply(0.6)), 4.0, 4.0));
                        painter.text(pos2(lower.right() + 8.0, yl(lvl)), Align2::LEFT_CENTER, format!("{lvl:.0}"), mono(11.0), p.muted);
                    }
                    let pts: Vec<Pos2> = (lo..=hi).filter(|&i| r[i].is_finite()).map(|i| pos2(x_of(i as f64), yl(r[i]))).collect();
                    lp.add(Shape::line(pts, Stroke::new(1.4, p.sma_fast)));
                    if let Some(v) = r.get(hi).filter(|v| v.is_finite()) {
                        painter.text(pos2(lower.left() + 6.0, lower.top() + 4.0), Align2::LEFT_TOP, format!("RSI {}  {v:.1}", ind::RSI), mono(11.0), p.muted);
                    }
                }
                LowerPane::Macd => {
                    let m = indicators::macd(&close, ind::MACD_FAST, ind::MACD_SLOW, ind::MACD_SIGNAL);
                    let mut amp: f64 = 1e-9;
                    for i in lo..=hi {
                        for v in [m.line[i], m.signal[i], m.hist[i]] {
                            if v.is_finite() {
                                amp = amp.max(v.abs());
                            }
                        }
                    }
                    let yl = |v: f64| lower.center().y - (v / amp) as f32 * lower.height() * 0.45;
                    lp.hline(lower.x_range(), yl(0.0), Stroke::new(1.0, p.line));
                    for i in lo..=hi {
                        if m.hist[i].is_finite() {
                            let x = x_of(i as f64);
                            let col = if m.hist[i] >= 0.0 { p.up } else { p.down };
                            lp.rect_filled(Rect::from_two_pos(pos2(x - bw * 0.35, yl(0.0)), pos2(x + bw * 0.35, yl(m.hist[i]))), CornerRadius::ZERO, col.gamma_multiply(0.45));
                        }
                    }
                    for (vals, col) in [(&m.line, p.sma_fast), (&m.signal, p.sma_slow)] {
                        let pts: Vec<Pos2> = (lo..=hi).filter(|&i| vals[i].is_finite()).map(|i| pos2(x_of(i as f64), yl(vals[i]))).collect();
                        lp.add(Shape::line(pts, Stroke::new(1.3, col)));
                    }
                    painter.text(pos2(lower.left() + 6.0, lower.top() + 4.0), Align2::LEFT_TOP, "MACD 12 26 9", mono(11.0), p.muted);
                }
                LowerPane::None => {}
            }
        }

        // Measure box.
        if let Some((a, b)) = self.measure {
            let r = Rect::from_two_pos(pos2(x_of(a.0), y_of(a.1)), pos2(x_of(b.0), y_of(b.1)));
            let col = if b.1 >= a.1 { p.up } else { p.down };
            mp.rect_filled(r, CornerRadius::ZERO, col.gamma_multiply(0.12));
            mp.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, col), StrokeKind::Inside);
            let pct = (b.1 / a.1 - 1.0) * 100.0;
            let bars = (b.0 - a.0).round() as i64;
            mp.text(r.center_top() - vec2(0.0, 4.0), Align2::CENTER_BOTTOM, format!("{pct:+.2}%  {:+.2}  {bars} bars", b.1 - a.1), mono(12.0), col);
        }

        // Crosshair and readout.
        let hover_i = resp.hover_pos().filter(|hp| plot.contains(*hp)).map(|hp| (self.index_at(plot, hp.x).round().max(0.0) as usize, hp));
        let readout_i = hover_i.map(|(i, _)| i.min(n - 1)).unwrap_or(n - 1);
        if let Some((i, hp)) = hover_i {
            let cs = Stroke::new(1.0, p.muted.gamma_multiply(0.7));
            let x = x_of(i as f64);
            painter.add(Shape::dashed_line(&[pos2(x, plot.top()), pos2(x, plot.bottom())], cs, 3.0, 3.0));
            if main.contains(hp) {
                painter.add(Shape::dashed_line(&[pos2(main.left(), hp.y), pos2(main.right(), hp.y)], cs, 3.0, 3.0));
                let g = painter.layout_no_wrap(format!("{:.2}", v_at(hp.y)), mono(11.5), p.bg);
                let tag = Rect::from_min_size(pos2(main.right() + 2.0, hp.y - g.size().y / 2.0 - 2.0), g.size() + vec2(8.0, 4.0));
                painter.rect_filled(tag, CornerRadius::same(2), p.muted);
                painter.galley(tag.min + vec2(4.0, 2.0), g, p.bg);
            }
        }
        let c = d.candles[readout_i];
        let prev = if readout_i > 0 { d.candles[readout_i - 1].c } else { c.o };
        let chg = (c.c / prev - 1.0) * 100.0;
        let when = if d.hide_dates { format!("bar {:+}", readout_i as i64 - (n as i64 - 1)) } else { fmt_date(c.t, d.intraday) };
        let text = format!("{when}   O {:.2}  H {:.2}  L {:.2}  C {:.2}  {chg:+.2}%   Vol {}", c.o, c.h, c.l, c.c, fmt_volume(c.v));
        let g = painter.layout_no_wrap(text, mono(12.0), if chg >= 0.0 { p.up } else { p.down });
        let pos = main.min + vec2(8.0, 6.0);
        painter.rect_filled(Rect::from_min_size(pos - vec2(4.0, 2.0), g.size() + vec2(8.0, 4.0)), CornerRadius::same(2), p.bg.gamma_multiply(0.85));
        painter.galley(pos, g, p.text);
        legend(&painter, &p, ov, d.intraday, main.min + vec2(8.0, 26.0));

        (resp, events)
    }

    fn index_at(&self, plot: Rect, x: f32) -> f64 {
        let bw = plot.width() as f64 / self.span;
        let left_idx = self.right - self.span + 1.0;
        (x - plot.left()) as f64 / bw - 0.5 + left_idx
    }
}

fn legend(painter: &egui::Painter, p: &Pal, ov: &Overlays, intraday: bool, mut pos: Pos2) {
    let items = [
        (ov.sma_fast, format!("SMA {}", ind::SMA_FAST), p.sma_fast),
        (ov.sma_slow, format!("SMA {}", ind::SMA_SLOW), p.sma_slow),
        (ov.ema, format!("EMA {}", ind::EMA), p.ema),
        (ov.bollinger, format!("BB {} {}", ind::BB_PERIOD, ind::BB_STDDEV), p.band),
        (ov.vwap && intraday, "VWAP".to_string(), p.vwap),
    ];
    for (on, label, col) in items {
        if on {
            let r = painter.text(pos, Align2::LEFT_TOP, label, mono(11.0), col);
            pos.x = r.right() + 12.0;
        }
    }
}

pub fn fmt_volume(v: f64) -> String {
    match v {
        v if v >= 1e9 => format!("{:.2}B", v / 1e9),
        v if v >= 1e6 => format!("{:.2}M", v / 1e6),
        v if v >= 1e3 => format!("{:.1}K", v / 1e3),
        v => format!("{v:.0}"),
    }
}

fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let n = if f < 1.5 { 1.0 } else if f < 3.5 { 2.0 } else if f < 7.5 { 5.0 } else { 10.0 };
    n * mag
}

/// Simple line chart for equity and P&L curves.
pub fn line_chart(ui: &mut Ui, values: &[f64], baseline: Option<f64>, height: f32) {
    let p = pal(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(3), p.bg);
    if values.len() < 2 {
        painter.text(rect.center(), Align2::CENTER_CENTER, "Not enough data yet", egui::FontId::proportional(13.0), p.muted);
        return;
    }
    let inner = rect.shrink2(vec2(8.0, 10.0));
    let mut lo = values.iter().copied().fold(f64::MAX, f64::min);
    let mut hi = values.iter().copied().fold(f64::MIN, f64::max);
    if let Some(b) = baseline {
        lo = lo.min(b);
        hi = hi.max(b);
    }
    let span = (hi - lo).max(1e-9);
    let x = |i: usize| inner.left() + i as f32 / (values.len() - 1) as f32 * inner.width();
    let y = |v: f64| inner.bottom() - ((v - lo) / span) as f32 * inner.height();
    if let Some(b) = baseline {
        painter.add(Shape::dashed_line(&[pos2(inner.left(), y(b)), pos2(inner.right(), y(b))], Stroke::new(1.0, p.muted), 4.0, 4.0));
    }
    let last = *values.last().unwrap();
    let col = if baseline.is_none_or(|b| last >= b) { p.brass } else { p.down };
    let pts: Vec<Pos2> = values.iter().enumerate().map(|(i, v)| pos2(x(i), y(*v))).collect();
    painter.add(Shape::line(pts, Stroke::new(1.8, col)));
    painter.text(rect.right_top() + vec2(-8.0, 6.0), Align2::RIGHT_TOP, format!("{last:.2}"), mono(11.5), col);
    if let Some(hp) = resp.hover_pos() {
        let i = (((hp.x - inner.left()) / inner.width()) * (values.len() - 1) as f32).round().clamp(0.0, (values.len() - 1) as f32) as usize;
        painter.circle_filled(pos2(x(i), y(values[i])), 3.5, col);
        painter.text(pos2(x(i), y(values[i]) - 8.0), Align2::CENTER_BOTTOM, format!("#{i}  {:.2}", values[i]), mono(11.0), p.text);
    }
}
