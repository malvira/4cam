//! Engraving and V-carving geometry: a built-in single-stroke font, SVG import to polylines,
//! and the inside/distance queries V-carving needs. The strategies that cut them are in
//! `strategy::engrave`.

use crate::det::Det;
use std::f64::consts::PI;

/// A polyline in mm (closed when the first and last points coincide).
pub type Path2 = Vec<[f64; 2]>;

// ------------------------------------------------------------------ single-stroke font

/// Strokes of one glyph on a 4 x 6 grid (x right, y up, baseline 0, cap height 6).
fn glyph(c: char) -> Option<Vec<Vec<(f64, f64)>>> {
    let o: Vec<(f64, f64)> = vec![
        (1.0, 0.0),
        (0.0, 1.0),
        (0.0, 5.0),
        (1.0, 6.0),
        (3.0, 6.0),
        (4.0, 5.0),
        (4.0, 1.0),
        (3.0, 0.0),
        (1.0, 0.0),
    ];
    let p = vec![
        (0.0, 0.0),
        (0.0, 6.0),
        (3.0, 6.0),
        (4.0, 5.0),
        (4.0, 4.0),
        (3.0, 3.0),
        (0.0, 3.0),
    ];
    let g: Vec<Vec<(f64, f64)>> = match c.to_ascii_uppercase() {
        ' ' => vec![],
        'A' => vec![
            vec![(0.0, 0.0), (2.0, 6.0), (4.0, 0.0)],
            vec![(0.7, 2.0), (3.3, 2.0)],
        ],
        'B' => vec![
            p.clone(),
            vec![(3.0, 3.0), (4.0, 2.0), (4.0, 1.0), (3.0, 0.0), (0.0, 0.0)],
        ],
        'C' => vec![vec![
            (4.0, 5.0),
            (3.0, 6.0),
            (1.0, 6.0),
            (0.0, 5.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
        ]],
        'D' => vec![vec![
            (0.0, 0.0),
            (0.0, 6.0),
            (2.5, 6.0),
            (4.0, 4.5),
            (4.0, 1.5),
            (2.5, 0.0),
            (0.0, 0.0),
        ]],
        'E' => vec![
            vec![(4.0, 6.0), (0.0, 6.0), (0.0, 0.0), (4.0, 0.0)],
            vec![(0.0, 3.0), (3.0, 3.0)],
        ],
        'F' => vec![
            vec![(4.0, 6.0), (0.0, 6.0), (0.0, 0.0)],
            vec![(0.0, 3.0), (3.0, 3.0)],
        ],
        'G' => vec![vec![
            (4.0, 5.0),
            (3.0, 6.0),
            (1.0, 6.0),
            (0.0, 5.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
            (4.0, 3.0),
            (2.0, 3.0),
        ]],
        'H' => vec![
            vec![(0.0, 0.0), (0.0, 6.0)],
            vec![(4.0, 0.0), (4.0, 6.0)],
            vec![(0.0, 3.0), (4.0, 3.0)],
        ],
        'I' => vec![
            vec![(1.0, 6.0), (3.0, 6.0)],
            vec![(2.0, 6.0), (2.0, 0.0)],
            vec![(1.0, 0.0), (3.0, 0.0)],
        ],
        'J' => vec![vec![
            (4.0, 6.0),
            (4.0, 1.0),
            (3.0, 0.0),
            (1.0, 0.0),
            (0.0, 1.0),
        ]],
        'K' => vec![
            vec![(0.0, 0.0), (0.0, 6.0)],
            vec![(4.0, 6.0), (0.0, 2.0)],
            vec![(1.3, 3.3), (4.0, 0.0)],
        ],
        'L' => vec![vec![(0.0, 6.0), (0.0, 0.0), (4.0, 0.0)]],
        'M' => vec![vec![
            (0.0, 0.0),
            (0.0, 6.0),
            (2.0, 3.0),
            (4.0, 6.0),
            (4.0, 0.0),
        ]],
        'N' => vec![vec![(0.0, 0.0), (0.0, 6.0), (4.0, 0.0), (4.0, 6.0)]],
        'O' => vec![o.clone()],
        'P' => vec![p.clone()],
        'Q' => vec![o.clone(), vec![(2.5, 1.5), (4.0, 0.0)]],
        'R' => vec![p.clone(), vec![(2.0, 3.0), (4.0, 0.0)]],
        'S' => vec![vec![
            (4.0, 5.0),
            (3.0, 6.0),
            (1.0, 6.0),
            (0.0, 5.0),
            (0.0, 4.0),
            (1.0, 3.0),
            (3.0, 3.0),
            (4.0, 2.0),
            (4.0, 1.0),
            (3.0, 0.0),
            (1.0, 0.0),
            (0.0, 1.0),
        ]],
        'T' => vec![vec![(0.0, 6.0), (4.0, 6.0)], vec![(2.0, 6.0), (2.0, 0.0)]],
        'U' => vec![vec![
            (0.0, 6.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
            (4.0, 6.0),
        ]],
        'V' => vec![vec![(0.0, 6.0), (2.0, 0.0), (4.0, 6.0)]],
        'W' => vec![vec![
            (0.0, 6.0),
            (1.0, 0.0),
            (2.0, 4.0),
            (3.0, 0.0),
            (4.0, 6.0),
        ]],
        'X' => vec![vec![(0.0, 0.0), (4.0, 6.0)], vec![(0.0, 6.0), (4.0, 0.0)]],
        'Y' => vec![
            vec![(0.0, 6.0), (2.0, 3.0), (4.0, 6.0)],
            vec![(2.0, 3.0), (2.0, 0.0)],
        ],
        'Z' => vec![vec![(0.0, 6.0), (4.0, 6.0), (0.0, 0.0), (4.0, 0.0)]],
        '0' => vec![o.clone(), vec![(0.5, 1.0), (3.5, 5.0)]],
        '1' => vec![
            vec![(1.0, 5.0), (2.0, 6.0), (2.0, 0.0)],
            vec![(1.0, 0.0), (3.0, 0.0)],
        ],
        '2' => vec![vec![
            (0.0, 5.0),
            (1.0, 6.0),
            (3.0, 6.0),
            (4.0, 5.0),
            (4.0, 4.0),
            (0.0, 0.0),
            (4.0, 0.0),
        ]],
        '3' => vec![
            vec![
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
                (4.0, 4.0),
                (3.0, 3.0),
                (1.5, 3.0),
            ],
            vec![
                (3.0, 3.0),
                (4.0, 2.0),
                (4.0, 1.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
            ],
        ],
        '4' => vec![vec![(3.0, 0.0), (3.0, 6.0), (0.0, 2.0), (4.0, 2.0)]],
        '5' => vec![vec![
            (4.0, 6.0),
            (0.0, 6.0),
            (0.0, 3.5),
            (3.0, 3.5),
            (4.0, 2.5),
            (4.0, 1.0),
            (3.0, 0.0),
            (1.0, 0.0),
            (0.0, 1.0),
        ]],
        '6' => vec![vec![
            (4.0, 5.0),
            (3.0, 6.0),
            (1.0, 6.0),
            (0.0, 5.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
            (4.0, 2.5),
            (3.0, 3.5),
            (0.0, 3.5),
        ]],
        '7' => vec![vec![(0.0, 6.0), (4.0, 6.0), (1.5, 0.0)]],
        '8' => vec![vec![
            (1.0, 3.0),
            (0.0, 4.0),
            (0.0, 5.0),
            (1.0, 6.0),
            (3.0, 6.0),
            (4.0, 5.0),
            (4.0, 4.0),
            (3.0, 3.0),
            (1.0, 3.0),
            (0.0, 2.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
            (4.0, 2.0),
            (3.0, 3.0),
        ]],
        '9' => vec![vec![
            (0.0, 1.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (4.0, 1.0),
            (4.0, 5.0),
            (3.0, 6.0),
            (1.0, 6.0),
            (0.0, 5.0),
            (0.0, 3.5),
            (1.0, 2.5),
            (4.0, 2.5),
        ]],
        '-' => vec![vec![(1.0, 3.0), (3.0, 3.0)]],
        '.' => vec![vec![(2.0, 0.0), (2.0, 0.4)]],
        ',' => vec![vec![(2.0, 0.5), (1.5, -1.0)]],
        '/' => vec![vec![(0.0, 0.0), (4.0, 6.0)]],
        ':' => vec![vec![(2.0, 1.0), (2.0, 1.4)], vec![(2.0, 4.0), (2.0, 4.4)]],
        '!' => vec![vec![(2.0, 6.0), (2.0, 2.0)], vec![(2.0, 0.0), (2.0, 0.4)]],
        '?' => vec![
            vec![
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
                (4.0, 4.0),
                (2.0, 2.5),
                (2.0, 1.8),
            ],
            vec![(2.0, 0.0), (2.0, 0.4)],
        ],
        '+' => vec![vec![(2.0, 1.0), (2.0, 5.0)], vec![(0.0, 3.0), (4.0, 3.0)]],
        '=' => vec![vec![(0.0, 2.0), (4.0, 2.0)], vec![(0.0, 4.0), (4.0, 4.0)]],
        '(' => vec![vec![(3.0, 6.5), (2.0, 5.0), (2.0, 1.0), (3.0, -0.5)]],
        ')' => vec![vec![(1.0, 6.5), (2.0, 5.0), (2.0, 1.0), (1.0, -0.5)]],
        '\'' => vec![vec![(2.0, 6.0), (2.0, 4.8)]],
        _ => return None,
    };
    Some(g)
}

/// Single-stroke text as polylines: cap height `height` (mm), first baseline starting at
/// `at`, turned by `rotate` degrees about `at`. Lines (`\n`) go down by 1.5 heights.
pub fn text_paths(
    text: &str,
    height: f64,
    at: [f64; 2],
    rotate: f64,
) -> Result<Vec<Path2>, String> {
    let s = height / 6.0;
    let (sa, ca) = rotate.to_radians().dsin_cos();
    let mut out = Vec::new();
    for (line, row) in text.split('\n').enumerate() {
        for (k, c) in row.chars().enumerate() {
            let g = glyph(c).ok_or(format!(
                "{c:?} is not in the built-in font (A-Z, 0-9, - . , / : ! ? + = ( ) ')"
            ))?;
            for stroke in g {
                out.push(
                    stroke
                        .iter()
                        .map(|&(gx, gy)| {
                            let (x, y) =
                                (k as f64 * 5.0 * s + gx * s, gy * s - line as f64 * 9.0 * s);
                            [at[0] + ca * x - sa * y, at[1] + sa * x + ca * y]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(out)
}

/// Default stroke weight of outline text: the pen's width as a fraction of the cap height.
pub const OUTLINE_WEIGHT: f64 = 0.12;

/// Text as closed outlines, for V-carving: marcam's own single-stroke font (see
/// [`text_paths`]) inked with a round pen `weight` x `height` wide. Each glyph becomes its
/// outline and the outlines of its holes (O, A, B, 0, ...), traced from a distance field to
/// within about a twentieth of the pen. Even-odd, like any closed paths.
pub fn outline_text(
    text: &str,
    height: f64,
    at: [f64; 2],
    rotate: f64,
    weight: f64,
) -> Result<Vec<Path2>, String> {
    if !(weight > 0.0 && weight < 0.5) {
        return Err(format!(
            "text_weight {weight}: use 0 < weight < 0.5 (a fraction of the height)"
        ));
    }
    let strokes = text_paths(text, height, at, rotate)?;
    if strokes.is_empty() {
        return Ok(Vec::new());
    }
    let r = 0.5 * weight * height;
    let h = (r / 10.0).clamp(0.002, 0.05);
    let field = crate::offset::DistanceField::from_strokes(&strokes, r, h);
    Ok(field
        .loops(0.0)
        .into_iter()
        .map(|lp| crate::offset::simplify(&lp, 0.1 * h))
        .filter(|lp| lp.len() >= 4)
        .collect())
}

// ------------------------------------------------------------------ SVG import

/// Affine transform [a b c d e f]: (x, y) -> (a x + c y + e, b x + d y + f).
type Affine = [f64; 6];

fn mul(m: Affine, n: Affine) -> Affine {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

fn apply(m: &Affine, p: [f64; 2]) -> [f64; 2] {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}

fn numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' {
            let st = i;
            i += 1;
            let mut dot = c == '.';
            while i < b.len() {
                let d = b[i] as char;
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == '.' && !dot {
                    dot = true;
                    i += 1;
                } else if (d == 'e' || d == 'E') && i + 1 < b.len() {
                    i += 1;
                    if b[i] == b'-' || b[i] == b'+' {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(v) = s[st..i].parse() {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn parse_transform(s: &str) -> Affine {
    let mut m: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for part in s.split(')') {
        let Some((name, args)) = part.split_once('(') else {
            continue;
        };
        let v = numbers(args);
        let g = |k: usize, d: f64| v.get(k).copied().unwrap_or(d);
        let t: Affine = match name.trim().trim_start_matches(',').trim() {
            "matrix" if v.len() == 6 => [v[0], v[1], v[2], v[3], v[4], v[5]],
            "translate" => [1.0, 0.0, 0.0, 1.0, g(0, 0.0), g(1, 0.0)],
            "scale" => [g(0, 1.0), 0.0, 0.0, g(1, g(0, 1.0)), 0.0, 0.0],
            "rotate" => {
                let (s, c) = g(0, 0.0).to_radians().dsin_cos();
                let (cx, cy) = (g(1, 0.0), g(2, 0.0));
                let r = [c, s, -s, c, 0.0, 0.0];
                mul(
                    mul([1.0, 0.0, 0.0, 1.0, cx, cy], r),
                    [1.0, 0.0, 0.0, 1.0, -cx, -cy],
                )
            }
            "skewX" => [1.0, 0.0, g(0, 0.0).to_radians().dtan(), 1.0, 0.0, 0.0],
            "skewY" => [1.0, g(0, 0.0).to_radians().dtan(), 0.0, 1.0, 0.0, 0.0],
            _ => continue,
        };
        m = mul(m, t);
    }
    m
}

/// Attribute value of a tag's text (`name="..."` or `name='...'`).
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().last();
        let after = &rest[i + name.len()..];
        let after_t = after.trim_start();
        if before.is_some_and(|c| c.is_whitespace()) && after_t.starts_with('=') {
            let v = after_t[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let body = &v[1..];
                return body.find(q).map(|j| &body[..j]);
            }
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// Points of an SVG path `d`, curves flattened to about `tol` mm (in path units).
fn path_d(d: &str, tol: f64) -> Vec<Path2> {
    let mut out: Vec<Path2> = Vec::new();
    let mut cur: Path2 = Vec::new();
    let (mut p, mut start) = ([0.0f64, 0.0f64], [0.0f64, 0.0f64]);
    let mut last_ctrl: Option<[f64; 2]> = None;
    let mut last_cmd = ' ';
    // tokens: command letters and numbers
    let mut toks: Vec<(Option<char>, f64)> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, toks: &mut Vec<(Option<char>, f64)>| {
        for v in numbers(buf) {
            toks.push((None, v));
        }
        buf.clear();
    };
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            flush(&mut buf, &mut toks);
            toks.push((Some(c), 0.0));
        } else {
            buf.push(c);
        }
    }
    flush(&mut buf, &mut toks);
    let steps = |len: f64| ((len / tol.max(1e-3)).sqrt().ceil() as usize).clamp(4, 64);
    let mut i = 0;
    let mut cmd = ' ';
    let take = |i: &mut usize, n: usize| -> Option<Vec<f64>> {
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            match toks.get(*i) {
                Some((None, x)) => {
                    v.push(*x);
                    *i += 1;
                }
                _ => return None,
            }
        }
        Some(v)
    };
    while i < toks.len() {
        if let (Some(c), _) = toks[i] {
            cmd = c;
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                if !cur.is_empty() {
                    cur.push(start);
                    out.push(std::mem::take(&mut cur));
                }
                p = start;
                last_ctrl = None;
                last_cmd = 'Z';
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { p } else { [0.0, 0.0] };
        let abs = |x: f64, y: f64| [base[0] + x, base[1] + y];
        match cmd.to_ascii_uppercase() {
            'M' => {
                let Some(v) = take(&mut i, 2) else { break };
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                p = abs(v[0], v[1]);
                start = p;
                cur.push(p);
                // further pairs are line-tos
                cmd = if rel { 'l' } else { 'L' };
                last_ctrl = None;
            }
            'L' => {
                let Some(v) = take(&mut i, 2) else { break };
                p = abs(v[0], v[1]);
                cur.push(p);
                last_ctrl = None;
            }
            'H' => {
                let Some(v) = take(&mut i, 1) else { break };
                p = [if rel { p[0] + v[0] } else { v[0] }, p[1]];
                cur.push(p);
                last_ctrl = None;
            }
            'V' => {
                let Some(v) = take(&mut i, 1) else { break };
                p = [p[0], if rel { p[1] + v[0] } else { v[0] }];
                cur.push(p);
                last_ctrl = None;
            }
            'C' | 'S' => {
                let smooth = cmd.eq_ignore_ascii_case(&'S');
                let Some(v) = take(&mut i, if smooth { 4 } else { 6 }) else {
                    break;
                };
                let c1 = if smooth {
                    match (last_ctrl, last_cmd.to_ascii_uppercase()) {
                        (Some(c), 'C' | 'S') => [2.0 * p[0] - c[0], 2.0 * p[1] - c[1]],
                        _ => p,
                    }
                } else {
                    abs(v[0], v[1])
                };
                let k = if smooth { 0 } else { 2 };
                let (c2, e) = (abs(v[k], v[k + 1]), abs(v[k + 2], v[k + 3]));
                let len = (c1[0] - p[0]).dhypot(c1[1] - p[1])
                    + (c2[0] - c1[0]).dhypot(c2[1] - c1[1])
                    + (e[0] - c2[0]).dhypot(e[1] - c2[1]);
                let n = steps(len);
                for s in 1..=n {
                    let t = s as f64 / n as f64;
                    let u = 1.0 - t;
                    cur.push([
                        u * u * u * p[0]
                            + 3.0 * u * u * t * c1[0]
                            + 3.0 * u * t * t * c2[0]
                            + t * t * t * e[0],
                        u * u * u * p[1]
                            + 3.0 * u * u * t * c1[1]
                            + 3.0 * u * t * t * c2[1]
                            + t * t * t * e[1],
                    ]);
                }
                last_ctrl = Some(c2);
                p = e;
            }
            'Q' | 'T' => {
                let smooth = cmd.eq_ignore_ascii_case(&'T');
                let Some(v) = take(&mut i, if smooth { 2 } else { 4 }) else {
                    break;
                };
                let c = if smooth {
                    match (last_ctrl, last_cmd.to_ascii_uppercase()) {
                        (Some(c), 'Q' | 'T') => [2.0 * p[0] - c[0], 2.0 * p[1] - c[1]],
                        _ => p,
                    }
                } else {
                    abs(v[0], v[1])
                };
                let k = if smooth { 0 } else { 2 };
                let e = abs(v[k], v[k + 1]);
                let len = (c[0] - p[0]).dhypot(c[1] - p[1]) + (e[0] - c[0]).dhypot(e[1] - c[1]);
                let n = steps(len);
                for s in 1..=n {
                    let t = s as f64 / n as f64;
                    let u = 1.0 - t;
                    cur.push([
                        u * u * p[0] + 2.0 * u * t * c[0] + t * t * e[0],
                        u * u * p[1] + 2.0 * u * t * c[1] + t * t * e[1],
                    ]);
                }
                last_ctrl = Some(c);
                p = e;
            }
            'A' => {
                let Some(v) = take(&mut i, 7) else { break };
                let e = abs(v[5], v[6]);
                cur.extend(arc_points(
                    p,
                    e,
                    v[0],
                    v[1],
                    v[2],
                    v[3] != 0.0,
                    v[4] != 0.0,
                    tol,
                ));
                p = e;
                last_ctrl = None;
            }
            _ => {
                i += 1;
            }
        }
        last_cmd = cmd;
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// Points of an SVG elliptical arc from `p` to `e` (endpoint form, SVG spec F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc_points(
    p: [f64; 2],
    e: [f64; 2],
    rx: f64,
    ry: f64,
    rot_deg: f64,
    large: bool,
    sweep: bool,
    tol: f64,
) -> Path2 {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-12 || ry < 1e-12 || (p[0] - e[0]).dhypot(p[1] - e[1]) < 1e-12 {
        return vec![e];
    }
    let (sp, cp) = rot_deg.to_radians().dsin_cos();
    let (dx, dy) = ((p[0] - e[0]) / 2.0, (p[1] - e[1]) / 2.0);
    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = (num / den).sqrt();
    if large == sweep {
        co = -co;
    }
    let (cx1, cy1) = (co * rx * y1 / ry, -co * ry * x1 / rx);
    let (cx, cy) = (
        cp * cx1 - sp * cy1 + (p[0] + e[0]) / 2.0,
        sp * cx1 + cp * cy1 + (p[1] + e[1]) / 2.0,
    );
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).datan2(ux * vx + uy * vy);
    let t1 = ang(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = ang(
        (x1 - cx1) / rx,
        (y1 - cy1) / ry,
        (-x1 - cx1) / rx,
        (-y1 - cy1) / ry,
    );
    if !sweep && dt > 0.0 {
        dt -= 2.0 * PI;
    } else if sweep && dt < 0.0 {
        dt += 2.0 * PI;
    }
    let r = rx.max(ry);
    let n = ((dt.abs() / (2.0 * (1.0 - tol.min(r) / r).dacos().max(1e-3))).ceil() as usize)
        .clamp(2, 256);
    (1..=n)
        .map(|k| {
            let t = t1 + dt * k as f64 / n as f64;
            let (st, ct) = t.dsin_cos();
            if k == n {
                e
            } else {
                [
                    cx + cp * rx * ct - sp * ry * st,
                    cy + sp * rx * ct + cp * ry * st,
                ]
            }
        })
        .collect()
}

/// Polylines from an SVG document, in mm with Y up: shapes `path`, `polyline`, `polygon`,
/// `line`, `rect`, `circle` and `ellipse`, with `transform`s on them and their groups. One
/// user unit is taken as 1 mm unless the root `svg` gives a size in mm/cm/in with a
/// `viewBox`. Curves are flattened to about `tol` mm.
pub fn svg_paths(text: &str, tol: f64) -> Result<Vec<Path2>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<Affine> = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
    let mut unit = 1.0;
    let mut seen_svg = false;
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            if matches!(name.trim(), "g" | "svg" | "a") && stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name: String = tag
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/')
            .collect();
        let local = attr(tag, "transform").map_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], parse_transform);
        let m = mul(*stack.last().unwrap(), local);
        match name.as_str() {
            "svg" if !seen_svg => {
                seen_svg = true;
                let len_mm = |v: &str| {
                    let n = numbers(v).first().copied()?;
                    let v = v.trim();
                    Some(if v.ends_with("mm") {
                        n
                    } else if v.ends_with("cm") {
                        10.0 * n
                    } else if v.ends_with("in") {
                        25.4 * n
                    } else {
                        return None;
                    })
                };
                if let (Some(w), Some(vb)) =
                    (attr(tag, "width").and_then(len_mm), attr(tag, "viewBox"))
                {
                    let vb = numbers(vb);
                    if vb.len() == 4 && vb[2] > 0.0 {
                        unit = w / vb[2];
                    }
                }
                if !self_closing {
                    stack.push(m);
                }
                continue;
            }
            "g" | "a" | "svg" => {
                if !self_closing {
                    stack.push(m);
                }
                continue;
            }
            _ => {}
        }
        let f = |k: &str| {
            attr(tag, k)
                .and_then(|v| numbers(v).first().copied())
                .unwrap_or(0.0)
        };
        let shapes: Vec<Path2> = match name.as_str() {
            "path" => path_d(attr(tag, "d").unwrap_or(""), tol / unit),
            "polyline" | "polygon" => {
                let v = numbers(attr(tag, "points").unwrap_or(""));
                let mut pts: Path2 = v.as_chunks::<2>().0.to_vec();
                if name == "polygon" && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                vec![pts]
            }
            "line" => vec![vec![[f("x1"), f("y1")], [f("x2"), f("y2")]]],
            "rect" => {
                let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
                vec![vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]]]
            }
            "circle" | "ellipse" => {
                let (cx, cy) = (f("cx"), f("cy"));
                let (rx, ry) = if name == "circle" {
                    (f("r"), f("r"))
                } else {
                    (f("rx"), f("ry"))
                };
                let r = rx.max(ry).max(1e-9);
                let n = ((PI / (1.0 - (tol / unit).min(r) / r).dacos().max(1e-3)).ceil() as usize)
                    .clamp(12, 360);
                vec![(0..=n)
                    .map(|k| {
                        let t = 2.0 * PI * k as f64 / n as f64;
                        [cx + rx * t.dcos(), cy + ry * t.dsin()]
                    })
                    .collect()]
            }
            _ => vec![],
        };
        for s in shapes {
            if s.len() >= 2 {
                // user units -> mm, Y up
                out.push(
                    s.iter()
                        .map(|&q| {
                            let t = apply(&m, q);
                            [t[0] * unit, -t[1] * unit]
                        })
                        .collect(),
                );
            }
        }
    }
    if !seen_svg {
        return Err("not an SVG document".into());
    }
    Ok(out)
}

/// Move and scale polylines so their bounding box starts at `at` and is `height` tall
/// (`height` <= 0 keeps the size).
pub fn place(paths: &[Path2], at: [f64; 2], height: f64) -> Vec<Path2> {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in paths.iter().flatten() {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if !lo[0].is_finite() {
        return Vec::new();
    }
    let s = if height > 0.0 && hi[1] > lo[1] {
        height / (hi[1] - lo[1])
    } else {
        1.0
    };
    paths
        .iter()
        .map(|p| {
            p.iter()
                .map(|q| [at[0] + (q[0] - lo[0]) * s, at[1] + (q[1] - lo[1]) * s])
                .collect()
        })
        .collect()
}

// ------------------------------------------------------------------ regions for V-carving

/// Closed polylines as a region (even-odd fill), with a segment index for distance queries.
pub struct Region {
    segs: Vec<[[f64; 2]; 2]>,
    pub lo: [f64; 2],
    pub hi: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    bins: Vec<Vec<u32>>,
}

impl Region {
    /// `reach`: the largest distance that will be asked for (bigger is clamped).
    pub fn new(paths: &[Path2], reach: f64) -> Region {
        Region::with_cell(paths, reach.max(0.25))
    }

    /// Indexed in square cells `cell` wide (for [`Region::nearest`], any size: about 32
    /// across the region is quick).
    pub fn with_cell(paths: &[Path2], cell: f64) -> Region {
        let mut segs = Vec::new();
        for p in paths {
            if p.len() < 3 {
                continue;
            }
            for w in p.windows(2) {
                segs.push([w[0], w[1]]);
            }
            // close it if the path does not
            let (a, b) = (p[p.len() - 1], p[0]);
            if (a[0] - b[0]).dhypot(a[1] - b[1]) > 1e-9 {
                segs.push([a, b]);
            }
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for s in &segs {
            for q in s {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        if segs.is_empty() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let nx = (((hi[0] - lo[0]) / cell).ceil() as usize + 1).max(1);
        let ny = (((hi[1] - lo[1]) / cell).ceil() as usize + 1).max(1);
        let mut bins = vec![Vec::new(); nx * ny];
        for (k, s) in segs.iter().enumerate() {
            let (a, b) = (s[0], s[1]);
            let i0 = ((a[0].min(b[0]) - lo[0]) / cell).floor().max(0.0) as usize;
            let i1 = (((a[0].max(b[0]) - lo[0]) / cell).floor() as usize).min(nx - 1);
            let j0 = ((a[1].min(b[1]) - lo[1]) / cell).floor().max(0.0) as usize;
            let j1 = (((a[1].max(b[1]) - lo[1]) / cell).floor() as usize).min(ny - 1);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    bins[j * nx + i].push(k as u32);
                }
            }
        }
        Region {
            segs,
            lo,
            hi,
            cell,
            nx,
            ny,
            bins,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// X ranges where the line y = `y` is inside the region (even-odd), sorted.
    pub fn spans(&self, y: f64) -> Vec<(f64, f64)> {
        let mut xs: Vec<f64> = self
            .segs
            .iter()
            .filter_map(|s| {
                let (a, b) = (s[0], s[1]);
                // half-open rule so a vertex on the line counts once
                if (a[1] <= y) != (b[1] <= y) {
                    Some(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
                } else {
                    None
                }
            })
            .collect();
        xs.sort_by(f64::total_cmp);
        xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
    }

    /// Distance from `p` to the outline, at most `cap`.
    pub fn distance(&self, p: [f64; 2], cap: f64) -> f64 {
        let r = cap.min(self.cell);
        let i0 = ((p[0] - r - self.lo[0]) / self.cell).floor().max(0.0) as usize;
        let j0 = ((p[1] - r - self.lo[1]) / self.cell).floor().max(0.0) as usize;
        let i1 = (((p[0] + r - self.lo[0]) / self.cell).floor().max(0.0) as usize).min(self.nx - 1);
        let j1 = (((p[1] + r - self.lo[1]) / self.cell).floor().max(0.0) as usize).min(self.ny - 1);
        let mut best = cap.min(self.cell);
        for j in j0..=j1 {
            for i in i0..=i1 {
                for &k in &self.bins[j * self.nx + i] {
                    best = best.min(seg_dist(p, self.segs[k as usize]));
                }
            }
        }
        // beyond one cell the index cannot answer: fall back to all segments
        if best >= self.cell && cap > self.cell {
            best = self
                .segs
                .iter()
                .map(|s| seg_dist(p, *s))
                .fold(cap, f64::min);
        }
        best
    }
}

/// The segments that can be the nearest to some point of each cell of a grid: for each cell,
/// those whose distance from it can be no more than the farthest any point of it can be from
/// its nearest segment. The distance found among them is the distance to the whole outline,
/// bit for bit (the same minimum of the same values), at a few of them a point.
pub struct Nearest {
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    lists: Vec<Vec<u32>>,
    segs: Vec<[[f64; 2]; 2]>,
}

impl Nearest {
    /// Over `lo..hi` (points outside are clamped to the edge cells, which keeps them exact:
    /// see [`Nearest::distance`]), in cells `cell` wide.
    pub fn new(region: &Region, lo: [f64; 2], hi: [f64; 2], cell: f64) -> Nearest {
        let segs = region.segs.clone();
        let nx = (((hi[0] - lo[0]) / cell).ceil() as usize).max(1);
        let ny = (((hi[1] - lo[1]) / cell).ceil() as usize).max(1);
        let half = 0.5 * cell * std::f64::consts::SQRT_2;
        let mut lists = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                let (ax, ay) = (lo[0] + i as f64 * cell, lo[1] + j as f64 * cell);
                let corners = [
                    [ax, ay],
                    [ax + cell, ay],
                    [ax, ay + cell],
                    [ax + cell, ay + cell],
                ];
                let c = [ax + 0.5 * cell, ay + 0.5 * cell];
                // no point of the cell is farther than this from its nearest segment (the
                // distance to a segment is convex: its largest over the cell is at a corner)
                let far = segs
                    .iter()
                    .map(|&sg| corners.iter().map(|&q| seg_dist(q, sg)).fold(0.0, f64::max))
                    .fold(f64::INFINITY, f64::min);
                let list: Vec<u32> = segs
                    .iter()
                    .enumerate()
                    .filter(|(_, sg)| seg_dist(c, **sg) - half <= far + 1e-9)
                    .map(|(k, _)| k as u32)
                    .collect();
                lists.push(list);
            }
        }
        Nearest {
            x0: lo[0],
            y0: lo[1],
            cell,
            nx,
            ny,
            lists,
            segs,
        }
    }

    /// The distance from `p` to the outline (infinite without one). A point outside the
    /// grid is looked up in all segments.
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        let fi = (p[0] - self.x0) / self.cell;
        let fj = (p[1] - self.y0) / self.cell;
        if fi < 0.0 || fj < 0.0 || fi >= self.nx as f64 || fj >= self.ny as f64 {
            return self
                .segs
                .iter()
                .map(|&s| seg_dist(p, s))
                .fold(f64::INFINITY, f64::min);
        }
        let k = fj as usize * self.nx + fi as usize;
        self.lists[k]
            .iter()
            .map(|&s| seg_dist(p, self.segs[s as usize]))
            .fold(f64::INFINITY, f64::min)
    }
}

fn seg_dist(p: [f64; 2], s: [[f64; 2]; 2]) -> f64 {
    let (a, b) = (s[0], s[1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).dhypot(p[1] - a[1] - t * dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::det::Det;

    #[test]
    fn text_is_cap_height_tall_and_rejects_unknown_characters() {
        let p = text_paths("HI 42", 6.0, [10.0, 20.0], 0.0).unwrap();
        let ys: Vec<f64> = p.iter().flatten().map(|q| q[1]).collect();
        let xs: Vec<f64> = p.iter().flatten().map(|q| q[0]).collect();
        assert_eq!(ys.iter().cloned().fold(f64::INFINITY, f64::min), 20.0);
        assert_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 26.0);
        // 5 characters at 5 units (here 5 mm) each: the last glyph ('2') ends at 4 x 1 mm in
        assert_eq!(
            xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            10.0 + 4.0 * 5.0 + 4.0
        );
        assert!(text_paths("é", 5.0, [0.0, 0.0], 0.0).is_err());
        // turned 90 degrees, it runs up +Y
        let r = text_paths("I", 6.0, [0.0, 0.0], 90.0).unwrap();
        let ymax = r
            .iter()
            .flatten()
            .map(|q| q[1])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((ymax - 3.0).abs() < 1e-9, "{ymax}");
    }

    #[test]
    fn svg_shapes_become_mm_polylines_with_y_up() {
        let svg = r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 40 20">
  <g transform="translate(2,0)">
    <rect x="0" y="0" width="10" height="4"/>
    <circle cx="20" cy="10" r="5"/>
    <path d="M 0 20 C 10 10 20 10 30 20 Z"/>
    <path d="m 30 2 a 4 4 0 1 0 8 0 a 4 4 0 1 0 -8 0"/>
  </g>
</svg>"#;
        let p = svg_paths(svg, 0.01).unwrap();
        assert_eq!(p.len(), 4);
        // rect: 10 x 4 user units at half scale, shifted by 1 mm, Y flipped
        let r = &p[0];
        assert!(
            r.contains(&[1.0, -0.0]) && r.contains(&[6.0, -2.0]),
            "{r:?}"
        );
        // circle: radius 2.5 mm round (11, -5)
        for q in &p[1] {
            assert!(((q[0] - 11.0).dhypot(q[1] + 5.0) - 2.5).abs() < 1e-9);
        }
        // arcs make a closed circle of radius 2 mm round (18, -1)
        for q in &p[3] {
            assert!(
                ((q[0] - 18.0).dhypot(q[1] + 1.0) - 2.0).abs() < 1e-6,
                "{q:?}"
            );
        }
        assert!(svg_paths("<html/>", 0.1).is_err());
    }

    #[test]
    fn region_spans_and_distances() {
        // a 10 x 4 rectangle with a 2 x 2 hole
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [0.0, 4.0], [0.0, 0.0]];
        let hole = vec![[4.0, 1.0], [6.0, 1.0], [6.0, 3.0], [4.0, 3.0], [4.0, 1.0]];
        let r = Region::new(&[outer, hole], 3.0);
        assert_eq!(r.spans(2.0), vec![(0.0, 4.0), (6.0, 10.0)]);
        assert_eq!(r.spans(0.5), vec![(0.0, 10.0)]);
        assert!((r.distance([2.0, 2.0], 10.0) - 2.0).abs() < 1e-12);
        assert!((r.distance([8.0, 2.0], 1.5) - 1.5).abs() < 1e-12, "capped");
        assert!((r.distance([7.0, 2.0], 10.0) - 1.0).abs() < 1e-12);
    }
}
