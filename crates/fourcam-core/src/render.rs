//! Dependency-free PNG previews of toolpaths: a side view (x vs radius) and an unrolled view
//! (x vs A, coloured by radius). Meant for quick visual checks committed next to examples.

use crate::toolpath::{Move, Toolpath};

/// RGB canvas.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    px: Vec<[u8; 3]>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: [u8; 3]) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![bg; w * h],
        }
    }

    pub fn set(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }

    /// Alpha-blend `c` over the pixel.
    pub fn blend(&mut self, x: i64, y: i64, c: [u8; 3], alpha: f64) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let p = &mut self.px[y as usize * self.w + x as usize];
            for k in 0..3 {
                p[k] = (p[k] as f64 * (1.0 - alpha) + c[k] as f64 * alpha).round() as u8;
            }
        }
    }

    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: [u8; 3], alpha: f64) {
        let n = ((x1 - x0).abs().max((y1 - y0).abs()).ceil() as usize).clamp(1, 20_000);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            self.blend(x.round() as i64, y.round() as i64, c, alpha);
        }
    }

    pub fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: [u8; 3]) {
        let (xa, xb) = (x0.min(x1).round() as i64, x0.max(x1).round() as i64);
        let (ya, yb) = (y0.min(y1).round() as i64, y0.max(y1).round() as i64);
        for y in ya..=yb {
            for x in xa..=xb {
                self.set(x, y, c);
            }
        }
    }

    /// Text in a 5x7 pixel font (digits, `-`, `.`, space and a few capitals), scaled by `s`.
    pub fn text(&mut self, x: i64, y: i64, s: i64, text: &str, c: [u8; 3]) {
        for (i, ch) in text.chars().enumerate() {
            let g = glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        for dy in 0..s {
                            for dx in 0..s {
                                self.set(
                                    x + (i as i64 * 6 + col) * s + dx,
                                    y + row as i64 * s + dy,
                                    c,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Encode as PNG (8-bit RGB, "Sub" filter, zlib).
    pub fn to_png(&self) -> Vec<u8> {
        use std::io::Write;
        let mut raw = Vec::with_capacity((self.w * 3 + 1) * self.h);
        for y in 0..self.h {
            raw.push(1); // Sub filter
            let row = &self.px[y * self.w..(y + 1) * self.w];
            let mut left = [0u8; 3];
            for px in row {
                for (v, l) in px.iter().zip(&left) {
                    raw.push(v.wrapping_sub(*l));
                }
                left = *px;
            }
        }
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).expect("in-memory write");
        let z = enc.finish().expect("in-memory write");

        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(self.w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        _ => [0; 7],
    }
}

/// Colour ramp (blue -> cyan -> yellow -> red) for t in [0, 1].
pub fn ramp(t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let stops = [
        [40.0, 60.0, 200.0],
        [30.0, 190.0, 210.0],
        [240.0, 210.0, 40.0],
        [210.0, 40.0, 40.0],
    ];
    let f = t * 3.0;
    let i = (f.floor() as usize).min(2);
    let u = f - i as f64;
    let mut c = [0u8; 3];
    for k in 0..3 {
        c[k] = (stops[i][k] * (1.0 - u) + stops[i + 1][k] * u).round() as u8;
    }
    c
}

/// Maps data coordinates to a pixel rectangle.
pub struct Frame {
    pub px: [f64; 4],   // left, top, right, bottom (pixels)
    pub data: [f64; 4], // x_min, y_min, x_max, y_max
}

impl Frame {
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let [l, t, r, b] = self.px;
        let [x0, y0, x1, y1] = self.data;
        (
            l + (x - x0) / (x1 - x0) * (r - l),
            b - (y - y0) / (y1 - y0) * (b - t),
        )
    }

    /// Axes box, grid lines every `gx`/`gy` data units, numbers on the ticks.
    pub fn axes(&self, cv: &mut Canvas, gx: f64, gy: f64, xlabel: &str, ylabel: &str) {
        let [l, t, r, b] = self.px;
        let [x0, y0, x1, y1] = self.data;
        let grid = [225, 225, 225];
        let ink = [90, 90, 90];
        let mut v = (x0 / gx).ceil() * gx;
        while v <= x1 + 1e-9 {
            let (px, _) = self.map(v, y0);
            cv.line(px, t, px, b, grid, 1.0);
            let label = fmt_tick(v);
            cv.text(
                px as i64 - 3 * label.len() as i64,
                b as i64 + 6,
                1,
                &label,
                ink,
            );
            v += gx;
        }
        let mut v = (y0 / gy).ceil() * gy;
        while v <= y1 + 1e-9 {
            let (_, py) = self.map(x0, v);
            cv.line(l, py, r, py, grid, 1.0);
            let label = fmt_tick(v);
            cv.text(
                l as i64 - 6 * label.len() as i64 - 4,
                py as i64 - 3,
                1,
                &label,
                ink,
            );
            v += gy;
        }
        for (a, b2, c, d) in [(l, t, r, t), (l, b, r, b), (l, t, l, b), (r, t, r, b)] {
            cv.line(a, b2, c, d, ink, 1.0);
        }
        cv.text(((l + r) / 2.0) as i64, b as i64 + 18, 2, xlabel, ink);
        cv.text(l as i64 - 40, t as i64 - 18, 2, ylabel, ink);
    }
}

fn fmt_tick(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// Tool-tip polyline of a toolpath: `(x, r, a, is_cut)` per move once X and Z are known.
/// Indexed 3+1 moves (machine X/Y/Z with A held) come out as the part point they reach, in
/// the same form: its radius and the A that would put it under a centred tool.
pub fn polyline(tp: &Toolpath) -> Vec<(f64, f64, f64, bool)> {
    let (mut x, mut r, mut a) = (None::<f64>, None::<f64>, 0.0f64);
    let mut y = 0.0f64;
    let mut pts = Vec::new();
    for m in &tp.moves {
        let xyz = match *m {
            Move::Rapid3 {
                x: mx,
                y: my,
                z: mz,
            } => Some((mx.or(x), my.unwrap_or(y), mz.or(r), false)),
            Move::Line3 {
                x: mx,
                y: my,
                z: mz,
                ..
            } => Some((Some(mx), my, Some(mz), true)),
            _ => None,
        };
        if let Some((nx, ny, nz, cut)) = xyz {
            x = nx;
            y = ny;
            r = nz;
            if let (Some(px), Some(pz)) = (nx, nz) {
                // part frame = machine turned back by A
                let (s, c) = a.to_radians().sin_cos();
                let (yp, zp) = (ny * c + pz * s, -ny * s + pz * c);
                let phi = zp.atan2(yp).to_degrees();
                pts.push((px, yp.hypot(zp), 90.0 - phi, cut));
            }
            continue;
        }
        let cut = match m {
            Move::Rapid {
                x: nx,
                r: nr,
                a: na,
            } => {
                x = nx.or(x);
                r = nr.or(r);
                a = na.unwrap_or(a);
                y = 0.0;
                false
            }
            Move::Plunge { r: nr, .. } => {
                r = Some(*nr);
                true
            }
            Move::Cut {
                x: nx,
                r: nr,
                a: na,
                ..
            } => {
                x = Some(*nx);
                r = Some(*nr);
                a = *na;
                true
            }
            _ => continue,
        };
        if let (Some(x), Some(r)) = (x, r) {
            pts.push((x, r, a, cut));
        }
    }
    pts
}

/// True when the toolpath moves in work XYZ (3-axis) rather than X/radius/A.
pub fn is_xyz(tp: &Toolpath) -> bool {
    tp.moves
        .iter()
        .any(|m| matches!(m, Move::Rapid3 { .. } | Move::Line3 { .. }))
}

/// Tool-tip polyline of a 3-axis toolpath: `(x, y, z, is_cut)` per move once XYZ is known.
pub fn polyline3(tp: &Toolpath) -> Vec<(f64, f64, f64, bool)> {
    let mut p: [Option<f64>; 3] = [None; 3];
    let mut pts = Vec::new();
    for m in &tp.moves {
        let cut = match *m {
            Move::Rapid3 { x, y, z } => {
                for (k, v) in [x, y, z].into_iter().enumerate() {
                    if v.is_some() {
                        p[k] = v;
                    }
                }
                false
            }
            Move::Line3 { x, y, z, .. } => {
                p = [Some(x), Some(y), Some(z)];
                true
            }
            _ => continue,
        };
        if let [Some(x), Some(y), Some(z)] = p {
            pts.push((x, y, z, cut));
        }
    }
    pts
}

/// Two-panel preview PNG. `profile` (if any) is drawn as the finished part in the side view.
pub fn preview_png(tp: &Toolpath, stock_r: f64, profile: Option<&[[f64; 2]]>) -> Vec<u8> {
    let pts = polyline(tp);
    let (w, h) = (1400usize, 1000usize);
    let mut cv = Canvas::new(w, h, [255, 255, 255]);
    let mut xr = pts
        .iter()
        .filter(|p| p.3)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
    if let Some(pr) = profile {
        for p in pr {
            xr = (xr.0.min(p[0]), xr.1.max(p[0]));
        }
    }
    if !xr.0.is_finite() {
        xr = (-1.0, 1.0);
    }
    let pad = 0.05 * (xr.1 - xr.0).max(1.0);
    let (xa, xb) = (xr.0 - pad, xr.1 + pad);
    let r_top = tp.safe_r.max(stock_r) + 0.5;
    let gx = nice_step(xb - xa);
    cv.text(
        20,
        12,
        2,
        &format!("{} - {}", tp.name, tp.title),
        [40, 40, 40],
    );

    // side view
    let side = Frame {
        px: [80.0, 50.0, w as f64 - 30.0, 470.0],
        data: [xa, 0.0, xb, r_top],
    };
    side.axes(&mut cv, gx, nice_step(r_top), "X", "R");
    if let Some(pr) = profile {
        // fill the finished part, one pixel column at a time
        let (l, r) = (side.px[0] as i64, side.px[2] as i64);
        for px in l..=r {
            let x = xa + (px as f64 - side.px[0]) / (side.px[2] - side.px[0]) * (xb - xa);
            if let Some(rad) = profile_radius(pr, x) {
                let (_, py) = side.map(x, rad);
                let (_, p0) = side.map(x, 0.0);
                cv.line(px as f64, py, px as f64, p0, [205, 205, 205], 1.0);
            }
        }
    }
    let (sx0, sy) = side.map(xa, stock_r);
    let (sx1, _) = side.map(xb, stock_r);
    cv.line(sx0, sy, sx1, sy, [120, 120, 120], 1.0);
    for wv in pts.windows(2) {
        let (p, q) = (wv[0], wv[1]);
        let (x0, y0) = side.map(p.0, p.1);
        let (x1, y1) = side.map(q.0, q.1);
        if q.3 {
            cv.line(x0, y0, x1, y1, [30, 90, 200], 0.35);
        } else {
            cv.line(x0, y0, x1, y1, [220, 60, 60], 0.25);
        }
    }

    // unrolled view, A wrapped to [0, 360)
    let un = Frame {
        px: [80.0, 540.0, w as f64 - 30.0, 950.0],
        data: [xa, 0.0, xb, 360.0],
    };
    un.axes(&mut cv, gx, 60.0, "X", "A");
    let (rlo, rhi) = pts
        .iter()
        .filter(|p| p.3)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    let span = (rhi - rlo).max(1e-9);
    for wv in pts.windows(2) {
        let (p, q) = (wv[0], wv[1]);
        if !q.3 {
            continue;
        }
        // split long angular moves so the wrap is drawn correctly
        let n = (((q.2 - p.2).abs() / 2.0).ceil() as usize).clamp(1, 2000);
        for i in 0..n {
            let t0 = i as f64 / n as f64;
            let t1 = (i + 1) as f64 / n as f64;
            let a0 = p.2 + (q.2 - p.2) * t0;
            let a1 = p.2 + (q.2 - p.2) * t1;
            let (w0, w1) = (a0.rem_euclid(360.0), a1.rem_euclid(360.0));
            if (w1 - w0).abs() > 180.0 {
                continue;
            }
            let x0 = p.0 + (q.0 - p.0) * t0;
            let x1 = p.0 + (q.0 - p.0) * t1;
            let r = p.1 + (q.1 - p.1) * t1;
            let (u0, v0) = un.map(x0, w0);
            let (u1, v1) = un.map(x1, w1);
            cv.line(u0, v0, u1, v1, ramp((r - rlo) / span), 0.6);
        }
    }
    // colour bar: radius low -> high
    for i in 0..200 {
        let x = w as f64 - 260.0 + i as f64;
        cv.line(x, 975.0, x, 990.0, ramp(i as f64 / 199.0), 1.0);
    }
    cv.text(w as i64 - 330, 978, 1, &format!("R{rlo:.2}"), [60, 60, 60]);
    cv.text(w as i64 - 54, 978, 1, &format!("{rhi:.2}"), [60, 60, 60]);
    cv.to_png()
}

/// Preview PNG of a 3-axis toolpath: top view (X/Y, cuts coloured by Z) over a front view
/// (X/Z), with the stock block outlined.
pub fn preview_png3(tp: &Toolpath, stock: &crate::job::StockBox) -> Vec<u8> {
    let pts = polyline3(tp);
    let (w, h) = (1400usize, 1000usize);
    let mut cv = Canvas::new(w, h, [255, 255, 255]);
    cv.text(
        20,
        12,
        2,
        &format!("{} - {}", tp.name, tp.title),
        [40, 40, 40],
    );
    let mut lo = stock.min;
    let mut hi = stock.max;
    for p in &pts {
        for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    let pad = 0.05 * (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
    let (xa, xb) = (lo[0] - pad, hi[0] + pad);
    let (ya, yb) = (lo[1] - pad, hi[1] + pad);
    let (za, zb) = (lo[2] - pad, hi[2] + pad);
    // equal scale in the top view, fitted to its box
    let (bl, bt, br, bb) = (80.0, 50.0, w as f64 - 30.0, 640.0);
    let k = ((br - bl) / (xb - xa)).min((bb - bt) / (yb - ya));
    let top = Frame {
        px: [bl, bt, bl + k * (xb - xa), bt + k * (yb - ya)],
        data: [xa, ya, xb, yb],
    };
    let gx = nice_step(xb - xa);
    top.axes(&mut cv, gx, nice_step(yb - ya), "X", "Y");
    let front = Frame {
        px: [80.0, 700.0, w as f64 - 30.0, 950.0],
        data: [xa, za, xb, zb],
    };
    front.axes(&mut cv, gx, nice_step(zb - za), "X", "Z");
    let block = [120, 120, 120];
    let outline = |cv: &mut Canvas, f: &Frame, a: [f64; 2], b: [f64; 2]| {
        let (x0, y0) = f.map(a[0], a[1]);
        let (x1, y1) = f.map(b[0], b[1]);
        for (p, q, r, t) in [
            (x0, y0, x1, y0),
            (x1, y0, x1, y1),
            (x1, y1, x0, y1),
            (x0, y1, x0, y0),
        ] {
            cv.line(p, q, r, t, block, 1.0);
        }
    };
    outline(
        &mut cv,
        &top,
        [stock.min[0], stock.min[1]],
        [stock.max[0], stock.max[1]],
    );
    outline(
        &mut cv,
        &front,
        [stock.min[0], stock.min[2]],
        [stock.max[0], stock.max[2]],
    );
    let (zlo, zhi) = pts
        .iter()
        .filter(|p| p.3)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.2), hi.max(p.2))
        });
    let span = (zhi - zlo).max(1e-9);
    for wv in pts.windows(2) {
        let (p, q) = (wv[0], wv[1]);
        let (u0, v0) = front.map(p.0, p.2);
        let (u1, v1) = front.map(q.0, q.2);
        if q.3 {
            cv.line(u0, v0, u1, v1, [30, 90, 200], 0.35);
            let (u0, v0) = top.map(p.0, p.1);
            let (u1, v1) = top.map(q.0, q.1);
            cv.line(u0, v0, u1, v1, ramp((q.2 - zlo) / span), 0.6);
        } else {
            cv.line(u0, v0, u1, v1, [220, 60, 60], 0.25);
        }
    }
    for i in 0..200 {
        let x = w as f64 - 260.0 + i as f64;
        cv.line(x, 975.0, x, 990.0, ramp(i as f64 / 199.0), 1.0);
    }
    if zlo.is_finite() {
        cv.text(w as i64 - 330, 978, 1, &format!("Z{zlo:.2}"), [60, 60, 60]);
        cv.text(w as i64 - 54, 978, 1, &format!("{zhi:.2}"), [60, 60, 60]);
    }
    cv.to_png()
}

/// Radius of a profile at `x` (None outside it); at a step the +x side wins.
fn profile_radius(pr: &[[f64; 2]], x: f64) -> Option<f64> {
    let (first, last) = (pr.first()?, pr.last()?);
    if x < first[0] || x > last[0] {
        return None;
    }
    let i = pr.partition_point(|q| q[0] <= x).saturating_sub(1);
    let [x0, r0] = pr[i];
    match pr.get(i + 1) {
        Some(&[x1, r1]) if x1 > x0 => Some(r0 + (r1 - r0) * (x - x0) / (x1 - x0)),
        _ => Some(r0),
    }
}

fn nice_step(span: f64) -> f64 {
    let raw = span / 10.0;
    let p = 10f64.powf(raw.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if raw <= m * p {
            return m * p;
        }
    }
    10.0 * p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_well_formed() {
        let mut c = Canvas::new(3, 2, [255, 0, 0]);
        c.set(1, 1, [0, 0, 255]);
        let png = c.to_png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        // known CRC of IEND
        assert_eq!(&png[png.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
    }
}
