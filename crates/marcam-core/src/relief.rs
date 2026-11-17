//! Relief parts from grayscale images: a heightfield mesh, flat (vise, Z up) or wrapped
//! around the rotary axis. White is high unless `invert` is set.
//!
//! Images are PNG (8 or 16-bit gray, gray + alpha, RGB, RGBA or 8-bit palette; not
//! interlaced) or PGM (P5 / P2). Colour is reduced to luma; transparent pixels count as black.

use crate::mesh::{Mesh, MeshError};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};
use std::io::Read;

/// How an image becomes a part (`"image": {...}` on a mesh model).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageRelief {
    /// Size of the image on the part (mm): along X, and along Y (flat) or around the bar
    /// measured at `base` (wrapped).
    pub width: f64,
    pub height: f64,
    /// Height of white above black (mm).
    pub depth: f64,
    /// Z of black (flat), or the radius of black (wrapped).
    #[serde(default)]
    pub base: f64,
    /// Black is high.
    #[serde(default)]
    pub invert: bool,
    /// Wrap around the rotary axis: X along the axis, the image's height around it, centred
    /// on the top of the bar at A0 (image top towards +Y).
    #[serde(default)]
    pub wrap: bool,
    /// Larger images are averaged down to this many pixels on their longer side.
    #[serde(default = "default_max_pixels")]
    pub max_pixels: usize,
}

fn default_max_pixels() -> usize {
    300
}

/// A grayscale image, values 0 (black) to 1 (white), row 0 at the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Gray {
    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.v[j * self.w + i]
    }

    /// Average down by an integer factor so the longer side is at most `max` pixels.
    pub fn shrink(&self, max: usize) -> Gray {
        let f = self.w.max(self.h).div_ceil(max.max(1)).max(1);
        if f == 1 {
            return self.clone();
        }
        let (w, h) = (self.w.div_ceil(f), self.h.div_ceil(f));
        let mut v = vec![0.0f32; w * h];
        for j in 0..h {
            for i in 0..w {
                let (mut sum, mut n) = (0.0f32, 0.0f32);
                for jj in j * f..((j + 1) * f).min(self.h) {
                    for ii in i * f..((i + 1) * f).min(self.w) {
                        sum += self.at(ii, jj);
                        n += 1.0;
                    }
                }
                v[j * w + i] = sum / n;
            }
        }
        Gray { w, h, v }
    }
}

fn bad(s: impl Into<String>) -> MeshError {
    MeshError::Format(s.into())
}

/// Decode a PNG or PGM image to gray.
pub fn decode_gray(data: &[u8]) -> Result<Gray, MeshError> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(data)
    } else if data.starts_with(b"P5") || data.starts_with(b"P2") {
        decode_pgm(data)
    } else {
        Err(bad("image: expected PNG or PGM (P5/P2)"))
    }
}

fn decode_pgm(data: &[u8]) -> Result<Gray, MeshError> {
    // header: magic, width, height, maxval, separated by whitespace and # comments
    let mut fields = Vec::new();
    let mut i = 2;
    while fields.len() < 3 {
        while i < data.len() && (data[i].is_ascii_whitespace() || data[i] == b'#') {
            if data[i] == b'#' {
                while i < data.len() && data[i] != b'\n' {
                    i += 1;
                }
            }
            i += 1;
        }
        let s = i;
        while i < data.len() && data[i].is_ascii_digit() {
            i += 1;
        }
        let n: usize = std::str::from_utf8(&data[s..i])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| bad("PGM: bad header"))?;
        fields.push(n);
    }
    let (w, h, max) = (fields[0], fields[1], fields[2].max(1));
    let n = w * h;
    let v: Vec<f32> = if data.starts_with(b"P5") {
        let px = &data[(i + 1).min(data.len())..];
        let bytes = if max > 255 { 2 } else { 1 };
        if px.len() < n * bytes {
            return Err(bad("PGM: data is short"));
        }
        (0..n)
            .map(|k| {
                let raw = if bytes == 2 {
                    u16::from_be_bytes([px[2 * k], px[2 * k + 1]]) as f32
                } else {
                    px[k] as f32
                };
                raw / max as f32
            })
            .collect()
    } else {
        let text = std::str::from_utf8(&data[i..]).map_err(|_| bad("PGM: not ASCII"))?;
        let v: Vec<f32> = text
            .split_ascii_whitespace()
            .take(n)
            .map(|t| t.parse::<f32>().map(|x| x / max as f32))
            .collect::<Result<_, _>>()
            .map_err(|_| bad("PGM: bad pixel"))?;
        if v.len() < n {
            return Err(bad("PGM: data is short"));
        }
        v
    };
    Ok(Gray { w, h, v })
}

fn decode_png(data: &[u8]) -> Result<Gray, MeshError> {
    let mut pos = 8;
    let (mut w, mut h, mut depth, mut ctype) = (0usize, 0usize, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = data
            .get(pos + 8..pos + 8 + len)
            .ok_or_else(|| bad("PNG: truncated chunk"))?;
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                h = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                depth = body[8];
                ctype = body[9];
                if body[12] != 0 {
                    return Err(bad("PNG: interlaced images are not supported"));
                }
            }
            b"PLTE" => palette = body.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(bad(format!("PNG: colour type {ctype}"))),
    };
    if !(depth == 8 || (depth == 16 && ctype != 3)) {
        return Err(bad(format!("PNG: {depth}-bit samples are not supported")));
    }
    if w == 0 || h == 0 {
        return Err(bad("PNG: empty image"));
    }
    let bpp = channels * depth as usize / 8;
    let stride = w * bpp;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .map_err(|e| bad(format!("PNG: {e}")))?;
    if raw.len() < h * (stride + 1) {
        return Err(bad("PNG: image data is short"));
    }
    // undo the scanline filters
    let mut px = vec![0u8; h * stride];
    for y in 0..h {
        let f = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp {
                px[y * stride + x - bpp]
            } else {
                0
            };
            let b = if y > 0 { px[(y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 {
                px[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = (
                        (p - a as i16).abs(),
                        (p - b as i16).abs(),
                        (p - c as i16).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return Err(bad(format!("PNG: filter {f}"))),
            };
            px[y * stride + x] = src[x].wrapping_add(pred);
        }
    }
    let sample = |k: usize, ch: usize| -> f32 {
        let o = k * bpp + ch * depth as usize / 8;
        if depth == 16 {
            u16::from_be_bytes([px[o], px[o + 1]]) as f32 / 65535.0
        } else {
            px[o] as f32 / 255.0
        }
    };
    let luma = |r: f32, g: f32, b: f32| 0.299 * r + 0.587 * g + 0.114 * b;
    let v = (0..w * h)
        .map(|k| match ctype {
            0 => sample(k, 0),
            2 => luma(sample(k, 0), sample(k, 1), sample(k, 2)),
            3 => {
                let i = px[k] as usize;
                let c = palette.get(i).copied().unwrap_or([0, 0, 0]);
                let alpha = trns.get(i).copied().unwrap_or(255) as f32 / 255.0;
                luma(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0 * alpha
            }
            4 => sample(k, 0) * sample(k, 1),
            _ => luma(sample(k, 0), sample(k, 1), sample(k, 2)) * sample(k, 3),
        })
        .collect();
    Ok(Gray { w, h, v })
}

/// The relief surface for an image: a grid of pixel-centre vertices, two triangles per
/// pixel square, normals outward (up, or away from the axis when wrapped).
pub fn relief_mesh(img: &Gray, r: &ImageRelief) -> Mesh {
    let img = img.shrink(r.max_pixels);
    let val = |i: usize, j: usize| -> f64 {
        let v = img.at(i, j) as f64;
        if r.invert {
            1.0 - v
        } else {
            v
        }
    };
    let (w, h) = (img.w, img.h);
    let px = r.width / w as f64;
    let py = r.height / h as f64;
    let mut tris = Vec::new();
    if !r.wrap {
        // image top row at +Y; lower-left corner of the image at (0, 0)
        let p = |i: usize, j: usize| {
            [
                (i as f64 + 0.5) * px,
                r.height - (j as f64 + 0.5) * py,
                r.base + val(i, j) * r.depth,
            ]
        };
        for j in 0..h.saturating_sub(1) {
            for i in 0..w.saturating_sub(1) {
                // j grows towards -Y, so (i, j+1) -> (i+1, j+1) -> (i+1, j) is counter-clockwise
                let (a, b, c, d) = (p(i, j + 1), p(i + 1, j + 1), p(i + 1, j), p(i, j));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        return Mesh { triangles: tris };
    }
    // wrapped: rings all the way round, the image over its arc and black elsewhere, so the
    // bar keeps its base radius outside the image
    let base = r.base.max(1e-6);
    let arc = (r.height / base).min(TAU);
    let n = ((TAU * base / py).ceil() as usize).clamp(16, 4096);
    let radius = |i: usize, k: usize| -> f64 {
        let phi = TAU * k as f64 / n as f64;
        // arc length from the image's top edge, increasing with the angle from top - arc/2
        let mut d = phi - (PI / 2.0 - arc / 2.0);
        d = d.rem_euclid(TAU);
        let s = d * base;
        let row = s / py - 0.5;
        if d > arc || row < -0.5 || row > h as f64 - 0.5 {
            return base;
        }
        let j = row.round().clamp(0.0, h as f64 - 1.0) as usize;
        base + val(i, j) * r.depth
    };
    let ring = |i: usize, k: usize| {
        let phi = TAU * (k % n) as f64 / n as f64;
        let rr = radius(i, k % n);
        [(i as f64 + 0.5) * px, rr * phi.cos(), rr * phi.sin()]
    };
    for i in 0..w.saturating_sub(1) {
        for k in 0..n {
            let (a, b) = (ring(i, k), ring(i, k + 1));
            let (c, d) = (ring(i + 1, k), ring(i + 1, k + 1));
            // as Mesh::revolve: (+angle) x (+x) is outward
            tris.push([a, b, c]);
            tris.push([b, d, c]);
        }
    }
    Mesh { triangles: tris }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pgm(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        let mut out = format!("P5\n# test\n{w} {h}\n255\n").into_bytes();
        for j in 0..h {
            for i in 0..w {
                out.push(f(i, j));
            }
        }
        out
    }

    #[test]
    fn pgm_and_png_decode_to_the_same_gray() {
        let g = decode_gray(&pgm(5, 3, |i, j| (i * 50 + j) as u8)).unwrap();
        assert_eq!((g.w, g.h), (5, 3));
        assert!((g.at(4, 2) - 202.0 / 255.0).abs() < 1e-6);
        // an RGB PNG written by the preview renderer: gray pixels decode to their level
        let mut cv = crate::render::Canvas::new(5, 3, [255, 255, 255]);
        cv.set(1, 2, [128, 128, 128]);
        let p = decode_gray(&cv.to_png()).unwrap();
        assert_eq!((p.w, p.h), (5, 3));
        assert!((p.at(0, 0) - 1.0).abs() < 1e-6 && (p.at(1, 2) - 128.0 / 255.0).abs() < 1e-3);
        assert!(decode_gray(b"GIF89a").is_err());
    }

    #[test]
    fn flat_relief_spans_the_image_with_white_high() {
        let g = decode_gray(&pgm(4, 2, |i, _| if i == 3 { 255 } else { 0 })).unwrap();
        let m = relief_mesh(
            &g,
            &ImageRelief {
                width: 8.0,
                height: 4.0,
                depth: 2.0,
                base: -3.0,
                invert: false,
                wrap: false,
                max_pixels: 300,
            },
        );
        let (lo, hi) = m.bounds().unwrap();
        assert_eq!((lo[0], hi[0], lo[1], hi[1]), (1.0, 7.0, 1.0, 3.0));
        assert_eq!((lo[2], hi[2]), (-3.0, -1.0));
        // upward normals
        for t in &m.triangles {
            let n = crate::mesh::cross(crate::mesh::sub(t[1], t[0]), crate::mesh::sub(t[2], t[0]));
            assert!(n[2] > 0.0);
        }
    }

    #[test]
    fn wrapped_relief_is_centred_on_the_top_and_round_elsewhere() {
        let g = decode_gray(&pgm(3, 4, |_, _| 255)).unwrap();
        let m = relief_mesh(
            &g,
            &ImageRelief {
                width: 6.0,
                height: 8.0,
                depth: 1.0,
                base: 10.0,
                invert: false,
                wrap: true,
                max_pixels: 300,
            },
        );
        let r_of = |p: [f64; 3]| p[1].hypot(p[2]);
        let top = m
            .triangles
            .iter()
            .flatten()
            .filter(|p| p[1].abs() < 0.5 && p[2] > 0.0)
            .map(|&p| r_of(p))
            .fold(0.0, f64::max);
        let bottom = m
            .triangles
            .iter()
            .flatten()
            .filter(|p| p[1].abs() < 0.5 && p[2] < 0.0)
            .map(|&p| r_of(p))
            .fold(0.0, f64::max);
        assert!(
            (top - 11.0).abs() < 1e-9 && (bottom - 10.0).abs() < 1e-9,
            "{top} {bottom}"
        );
    }
}
