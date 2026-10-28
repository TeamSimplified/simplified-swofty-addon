// BC7 encoding for team logo slices. The game's logo arrays are BC7, and the
// launcher's formats crate only decodes it, so an imported picture is encoded
// here. Every block is written in mode 6: one subset, RGBA endpoints of seven bits
// and a p-bit each, sixteen 4-bit indices. Logos are flat colours and soft edges,
// which one subset with sixteen steps holds well, and the mode needs no partition
// search.

const WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

type Px = [u8; 4];

fn lerp(e0: Px, e1: Px, i: usize) -> Px {
    let w = WEIGHTS[i];
    let mut out = [0u8; 4];
    for c in 0..4 {
        out[c] = (((64 - w) * u32::from(e0[c]) + w * u32::from(e1[c]) + 32) >> 6) as u8;
    }
    out
}

// Colour error counts in proportion to coverage: a transparent pixel's colour
// never shows, and the edge pixels of a logo are mostly translucent.
fn error(a: Px, b: Px) -> u32 {
    let d = |c: usize| (i32::from(a[c]) - i32::from(b[c])).unsigned_abs();
    let rgb = d(0) * d(0) + d(1) * d(1) + d(2) * d(2);
    (rgb * (u32::from(a[3]) + 1)) / 256 + 2 * d(3) * d(3)
}

fn quantise(v: f32, p: u8) -> u8 {
    (((v - f32::from(p)) / 2.0).round().clamp(0.0, 127.0)) as u8
}

fn expand(c7: u8, p: u8) -> u8 {
    (c7 << 1) | p
}

struct Fit {
    q0: [u8; 4],
    q1: [u8; 4],
    p0: u8,
    p1: u8,
    idx: [usize; 16],
    err: u32,
}

fn endpoints(q0: [u8; 4], q1: [u8; 4], p0: u8, p1: u8) -> (Px, Px) {
    (q0.map(|c| expand(c, p0)), q1.map(|c| expand(c, p1)))
}

fn assign(px: &[Px; 16], e0: Px, e1: Px) -> ([usize; 16], u32) {
    let palette: Vec<Px> = (0..16).map(|i| lerp(e0, e1, i)).collect();
    let mut idx = [0usize; 16];
    let mut total = 0;
    for (k, p) in px.iter().enumerate() {
        let (best, e) = palette
            .iter()
            .enumerate()
            .map(|(i, c)| (i, error(*p, *c)))
            .min_by_key(|(_, e)| *e)
            .expect("16 entries");
        idx[k] = best;
        total += e;
    }
    (idx, total)
}

fn quantised_fit(px: &[Px; 16], lo: [f32; 4], hi: [f32; 4]) -> Fit {
    let mut best: Option<Fit> = None;
    for p0 in 0..2u8 {
        for p1 in 0..2u8 {
            let q0 = lo.map(|v| quantise(v, p0));
            let q1 = hi.map(|v| quantise(v, p1));
            let (e0, e1) = endpoints(q0, q1, p0, p1);
            let (idx, err) = assign(px, e0, e1);
            if best.as_ref().is_none_or(|b| err < b.err) {
                best = Some(Fit {
                    q0,
                    q1,
                    p0,
                    p1,
                    idx,
                    err,
                });
            }
        }
    }
    best.expect("four tries")
}

// Least squares endpoints for fixed indices, channel by channel.
fn refit(px: &[Px; 16], idx: &[usize; 16], lo: [f32; 4], hi: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let (mut out0, mut out1) = (lo, hi);
    for c in 0..4 {
        let (mut a, mut b, mut cc, mut d0, mut d1) = (0f32, 0f32, 0f32, 0f32, 0f32);
        for (k, p) in px.iter().enumerate() {
            let wt = if c == 3 {
                1.0
            } else {
                (f32::from(p[3]) + 1.0) / 256.0
            };
            let f = WEIGHTS[idx[k]] as f32 / 64.0;
            let g = 1.0 - f;
            let x = f32::from(p[c]);
            a += wt * g * g;
            b += wt * g * f;
            cc += wt * f * f;
            d0 += wt * g * x;
            d1 += wt * f * x;
        }
        let det = a * cc - b * b;
        if det.abs() > 1e-6 {
            out0[c] = ((cc * d0 - b * d1) / det).clamp(0.0, 255.0);
            out1[c] = ((a * d1 - b * d0) / det).clamp(0.0, 255.0);
        }
    }
    (out0, out1)
}

fn principal(px: &[Px; 16]) -> ([f32; 4], [f32; 4]) {
    let v: Vec<[f32; 4]> = px.iter().map(|p| p.map(f32::from)).collect();
    let mut mean = [0f32; 4];
    for p in &v {
        for c in 0..4 {
            mean[c] += p[c] / 16.0;
        }
    }
    let mut cov = [[0f32; 4]; 4];
    for p in &v {
        for i in 0..4 {
            for j in 0..4 {
                cov[i][j] += (p[i] - mean[i]) * (p[j] - mean[j]);
            }
        }
    }
    let mut axis = [1f32, 1.0, 1.0, 1.0];
    for _ in 0..8 {
        let mut next = [0f32; 4];
        for i in 0..4 {
            for j in 0..4 {
                next[i] += cov[i][j] * axis[j];
            }
        }
        let n = next.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n < 1e-6 {
            break;
        }
        axis = next.map(|x| x / n);
    }
    let t: Vec<f32> = v
        .iter()
        .map(|p| (0..4).map(|c| (p[c] - mean[c]) * axis[c]).sum())
        .collect();
    let lo = t.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = t.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let at = |s: f32| {
        let mut e = [0f32; 4];
        for c in 0..4 {
            e[c] = (mean[c] + s * axis[c]).clamp(0.0, 255.0);
        }
        e
    };
    (at(lo), at(hi))
}

fn pack(fit: &Fit) -> [u8; 16] {
    let (mut q0, mut q1, mut p0, mut p1, mut idx) = (fit.q0, fit.q1, fit.p0, fit.p1, fit.idx);
    // The first index has an implied top bit of 0.
    if idx[0] >= 8 {
        std::mem::swap(&mut q0, &mut q1);
        std::mem::swap(&mut p0, &mut p1);
        idx = idx.map(|i| 15 - i);
    }
    let mut bits: u128 = 1 << 6;
    let mut pos = 7;
    let mut put = |v: u128, n: u32| {
        bits |= v << pos;
        pos += n;
    };
    for c in 0..4 {
        put(u128::from(q0[c]), 7);
        put(u128::from(q1[c]), 7);
    }
    put(u128::from(p0), 1);
    put(u128::from(p1), 1);
    for (k, i) in idx.iter().enumerate() {
        put(*i as u128, if k == 0 { 3 } else { 4 });
    }
    bits.to_le_bytes()
}

pub fn encode_block(px: &[Px; 16]) -> [u8; 16] {
    let (mut lo, mut hi) = principal(px);
    let mut best = quantised_fit(px, lo, hi);
    for _ in 0..3 {
        if best.err == 0 {
            break;
        }
        (lo, hi) = refit(px, &best.idx, lo, hi);
        let next = quantised_fit(px, lo, hi);
        if next.err >= best.err {
            break;
        }
        best = next;
    }
    pack(&best)
}

/// Encodes an RGBA image whose sides are multiples of 4.
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = Vec::with_capacity(w * h);
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let mut px = [[0u8; 4]; 16];
            for y in 0..4 {
                for x in 0..4 {
                    let (sx, sy) = ((bx + x).min(w - 1), (by + y).min(h - 1));
                    let at = (sy * w + sx) * 4;
                    px[y * 4 + x].copy_from_slice(&rgba[at..at + 4]);
                }
            }
            out.extend_from_slice(&encode_block(&px));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use swofty_formats::texture::{Codec, decode_pixels};

    fn roundtrip(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
        decode_pixels(Codec::Bc7, w, h, &encode(rgba, w, h)).unwrap()
    }

    #[test]
    fn flat_colours_come_back_within_a_step() {
        let mut img = Vec::new();
        for i in 0..64 {
            img.extend_from_slice(if i % 8 < 4 {
                &[225, 135, 40, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
        let back = roundtrip(&img, 8, 8);
        for (a, b) in img.chunks(4).zip(back.chunks(4)) {
            for c in 0..4 {
                assert!((i32::from(a[c]) - i32::from(b[c])).abs() <= 2, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn a_soft_edge_between_two_colours_stays_close() {
        let mut img = Vec::new();
        for _ in 0..16u32 {
            for x in 0..16u32 {
                let t = x * 17;
                img.extend_from_slice(&[
                    (225 - t * 225 / 255) as u8,
                    (135 + t * 120 / 255) as u8,
                    40,
                    (255 - t / 2) as u8,
                ]);
            }
        }
        let back = roundtrip(&img, 16, 16);
        let worst = img
            .iter()
            .zip(&back)
            .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
            .max()
            .unwrap();
        assert!(worst <= 6, "worst channel error {worst}");
    }
}
