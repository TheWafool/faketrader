//! Indicators return one value per candle; warm-up slots are `NAN` so charts skip them.

use crate::model::Candle;

pub fn closes(c: &[Candle]) -> Vec<f64> {
    c.iter().map(|c| c.c).collect()
}

pub fn sma(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    if n == 0 || v.len() < n {
        return out;
    }
    let mut sum: f64 = v[..n].iter().sum();
    out[n - 1] = sum / n as f64;
    for i in n..v.len() {
        sum += v[i] - v[i - n];
        out[i] = sum / n as f64;
    }
    out
}

pub fn ema(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    let first = v.iter().position(|x| x.is_finite());
    let Some(start) = first else { return out };
    if n == 0 || v.len() < start + n {
        return out;
    }
    let k = 2.0 / (n as f64 + 1.0);
    let mut prev: f64 = v[start..start + n].iter().sum::<f64>() / n as f64;
    out[start + n - 1] = prev;
    for i in start + n..v.len() {
        prev = v[i] * k + prev * (1.0 - k);
        out[i] = prev;
    }
    out
}

/// Wilder RSI.
pub fn rsi(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    if v.len() <= n || n == 0 {
        return out;
    }
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=n {
        let d = v[i] - v[i - 1];
        if d > 0.0 { gain += d } else { loss -= d }
    }
    gain /= n as f64;
    loss /= n as f64;
    let calc = |g: f64, l: f64| if l == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + g / l) };
    out[n] = calc(gain, loss);
    for i in n + 1..v.len() {
        let d = v[i] - v[i - 1];
        gain = (gain * (n - 1) as f64 + d.max(0.0)) / n as f64;
        loss = (loss * (n - 1) as f64 + (-d).max(0.0)) / n as f64;
        out[i] = calc(gain, loss);
    }
    out
}

pub struct Macd {
    pub line: Vec<f64>,
    pub signal: Vec<f64>,
    pub hist: Vec<f64>,
}

pub fn macd(v: &[f64], fast: usize, slow: usize, sig: usize) -> Macd {
    let f = ema(v, fast);
    let s = ema(v, slow);
    let line: Vec<f64> = f.iter().zip(&s).map(|(a, b)| a - b).collect();
    let signal = ema(&line, sig);
    let hist = line.iter().zip(&signal).map(|(a, b)| a - b).collect();
    Macd { line, signal, hist }
}

pub struct Bands {
    pub mid: Vec<f64>,
    pub upper: Vec<f64>,
    pub lower: Vec<f64>,
}

pub fn bollinger(v: &[f64], n: usize, k: f64) -> Bands {
    let mid = sma(v, n);
    let mut upper = vec![f64::NAN; v.len()];
    let mut lower = vec![f64::NAN; v.len()];
    for i in n.saturating_sub(1)..v.len() {
        if !mid[i].is_finite() {
            continue;
        }
        let w = &v[i + 1 - n..=i];
        let var = w.iter().map(|x| (x - mid[i]).powi(2)).sum::<f64>() / n as f64;
        let sd = var.sqrt();
        upper[i] = mid[i] + k * sd;
        lower[i] = mid[i] - k * sd;
    }
    Bands { mid, upper, lower }
}

/// Wilder ATR.
pub fn atr(c: &[Candle], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; c.len()];
    if c.len() <= n || n == 0 {
        return out;
    }
    let tr = |i: usize| {
        if i == 0 {
            c[0].range()
        } else {
            let pc = c[i - 1].c;
            (c[i].h - c[i].l).max((c[i].h - pc).abs()).max((c[i].l - pc).abs())
        }
    };
    let mut a: f64 = (1..=n).map(tr).sum::<f64>() / n as f64;
    out[n] = a;
    for (i, slot) in out.iter_mut().enumerate().skip(n + 1) {
        a = (a * (n - 1) as f64 + tr(i)) / n as f64;
        *slot = a;
    }
    out
}

/// Session VWAP that resets at each calendar day (UTC). Meant for intraday bars.
pub fn vwap(c: &[Candle]) -> Vec<f64> {
    let mut out = vec![f64::NAN; c.len()];
    let (mut pv, mut vol, mut day) = (0.0, 0.0, i64::MIN);
    for (i, b) in c.iter().enumerate() {
        let d = b.t.div_euclid(86_400);
        if d != day {
            day = d;
            pv = 0.0;
            vol = 0.0;
        }
        let tp = (b.h + b.l + b.c) / 3.0;
        pv += tp * b.v;
        vol += b.v;
        out[i] = if vol > 0.0 { pv / vol } else { tp };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sma_ema_basic() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        let s = sma(&v, 3);
        assert!(s[1].is_nan());
        assert_eq!(s[2], 2.0);
        assert_eq!(s[4], 4.0);
        let e = ema(&v, 3);
        assert_eq!(e[2], 2.0);
        assert!((e[3] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn rsi_bounds() {
        let up: Vec<f64> = (0..30).map(|i| i as f64).collect();
        assert_eq!(*rsi(&up, 14).last().unwrap(), 100.0);
        let zig: Vec<f64> = (0..60).map(|i| if i % 2 == 0 { 10.0 } else { 11.0 }).collect();
        let r = *rsi(&zig, 14).last().unwrap();
        assert!(r > 40.0 && r < 60.0, "{r}");
    }
}
