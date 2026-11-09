//! The app mark, drawn procedurally at any size: a warm ink tile holding a
//! small day dial. A faint track goes all the way round, three app colours own
//! stretches of the evening, and a light tick marks "now". It is the app's
//! main view, shrunk to an icon.
//! Pure std so build.rs can include it to make the .exe icon.

use std::f32::consts::{FRAC_PI_2, TAU};

/// Fractions of the day (0 = midnight at the top, clockwise) and an sRGB colour.
const ARCS: [(f32, f32, [f32; 3]); 3] = [
    (0.36, 0.52, [233.0, 178.0, 62.0]), // amber
    (0.53, 0.66, [222.0, 112.0, 84.0]), // coral
    (0.67, 0.79, [86.0, 170.0, 160.0]), // teal
];
const NOW: f32 = 0.80;
const TILE: [f32; 3] = [32.0, 30.0, 27.0];
const TRACK: [f32; 3] = [66.0, 62.0, 56.0];
const MARK: [f32; 3] = [244.0, 242.0, 237.0];

/// Straight-alpha RGBA, `size * size * 4` bytes.
pub fn render(size: u32) -> Vec<u8> {
    let s = size as f32;
    let n = size as usize;
    let mut px = vec![0u8; n * n * 4];
    const SS: usize = 4; // 4x4 supersampling
    let corner = 0.23 * s;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let ring_r = 0.29 * s;
    // Thick enough to read at 16 px.
    let half_w = (0.075 * s).max(1.3);
    let now_a = -FRAC_PI_2 + TAU * NOW;
    let now_dir = (now_a.cos(), now_a.sin());

    for y in 0..n {
        for x in 0..n {
            let mut cover = 0.0f32;
            let mut rgb = [0.0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let px_ = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py_ = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    // Rounded square.
                    let qx = (px_ - cx).abs() - (s / 2.0 - corner);
                    let qy = (py_ - cy).abs() - (s / 2.0 - corner);
                    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - corner;
                    if outside > 0.0 {
                        continue;
                    }
                    cover += 1.0;
                    let (dx, dy) = (px_ - cx, py_ - cy);
                    let d = (dx * dx + dy * dy).sqrt();
                    // Fraction of the day at this angle.
                    let f = ((dy.atan2(dx) + FRAC_PI_2).rem_euclid(TAU)) / TAU;
                    // "Now": a short radial tick across the band.
                    let along = dx * now_dir.0 + dy * now_dir.1;
                    let across = (dx * now_dir.1 - dy * now_dir.0).abs();
                    let color = if across <= (0.022 * s).max(0.7) && along >= ring_r - half_w * 1.6 && along <= ring_r + half_w * 1.6 {
                        MARK
                    } else if (d - ring_r).abs() <= half_w {
                        ARCS.iter().find(|a| f >= a.0 && f < a.1).map_or(TRACK, |a| a.2)
                    } else {
                        TILE
                    };
                    for c in 0..3 {
                        rgb[c] += color[c];
                    }
                }
            }
            if cover > 0.0 {
                let i = (y * n + x) * 4;
                for c in 0..3 {
                    px[i + c] = (rgb[c] / cover).round() as u8;
                }
                px[i + 3] = (cover / (SS * SS) as f32 * 255.0).round() as u8;
            }
        }
    }
    px
}
