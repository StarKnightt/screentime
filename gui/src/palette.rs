//! App colours, taken from each app's own icon.
//!
//! Every opaque, clearly coloured pixel votes for its hue (weighted by how
//! vivid it is). The strongest hues become the app's candidates. Each app
//! then gets one candidate re-toned to a shared lightness and a capped chroma,
//! so a neon icon and a muted one sit together on the dial. Two apps whose
//! icons share a hue (two orange icons) are pushed apart: the app with fewer
//! options picks first, and the other takes its second hue or a darker tone.
//!
//! All maths is in OKLab/OKLCH, where equal steps look equal.

use std::collections::HashMap;

use iced::Color;

/// One hue an icon is made of. `weight` is its share of the vivid pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hue {
    pub h: f32,
    pub c: f32,
    pub weight: f32,
}

/// Pixels below this OKLCH chroma count as grey.
const GREY_CHROMA: f32 = 0.045;
/// Hues closer than this (degrees) read as the same colour on the dial.
const MIN_HUE_GAP: f32 = 28.0;

// ------------------------------------------------------------ colour maths

fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB (0..1) to OKLab.
pub fn oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab to linear-free sRGB (may fall outside 0..1).
fn oklab_to_srgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        linear_to_srgb(4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3),
        linear_to_srgb(-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3),
        linear_to_srgb(-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3),
    ]
}

/// OKLCH to an sRGB colour, reducing chroma until it fits the gamut.
pub fn oklch(l: f32, c: f32, h_deg: f32) -> Color {
    let h = h_deg.to_radians();
    let mut c = c;
    loop {
        let rgb = oklab_to_srgb([l, c * h.cos(), c * h.sin()]);
        let inside = rgb.iter().all(|v| (-0.001..=1.001).contains(v));
        if inside || c <= 0.0 {
            return Color::from_rgb(rgb[0].clamp(0.0, 1.0), rgb[1].clamp(0.0, 1.0), rgb[2].clamp(0.0, 1.0));
        }
        c -= 0.005;
    }
}

fn hue_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

// ------------------------------------------------------------ icon analysis

/// The hues an RGBA icon is made of, strongest first. Empty for a grey or
/// monochrome icon.
pub fn icon_hues(rgba: &[u8]) -> Vec<Hue> {
    const BINS: usize = 24; // 15 degrees each
    let mut bins = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); BINS]; // weight, a, b, chroma sum
    let (mut opaque, mut vivid) = (0usize, 0usize);
    for p in rgba.as_chunks::<4>().0 {
        if p[3] < 200 {
            continue;
        }
        opaque += 1;
        let [_, a, b] = oklab(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let c = (a * a + b * b).sqrt();
        if c < GREY_CHROMA {
            continue;
        }
        vivid += 1;
        let h = b.atan2(a).to_degrees().rem_euclid(360.0);
        let bin = &mut bins[(h / 15.0) as usize % BINS];
        bin.0 += c;
        bin.1 += a * c;
        bin.2 += b * c;
        bin.3 += c * c;
    }
    let total: f32 = bins.iter().map(|b| b.0).sum();
    // A few tinted pixels on an otherwise grey icon don't make it coloured.
    if opaque == 0 || total <= 0.0 || vivid * 20 < opaque {
        return Vec::new();
    }
    // Peaks: a bin stronger than both neighbours, merged with them.
    let mut peaks = Vec::new();
    for i in 0..BINS {
        let (prev, next) = (bins[(i + BINS - 1) % BINS].0, bins[(i + 1) % BINS].0);
        let w = bins[i].0;
        if w <= 0.0 || w < prev || w <= next {
            continue;
        }
        let group = [(i + BINS - 1) % BINS, i, (i + 1) % BINS];
        let (gw, ga, gb, gc) = group.iter().fold((0.0, 0.0, 0.0, 0.0), |acc, &j| {
            (acc.0 + bins[j].0, acc.1 + bins[j].1, acc.2 + bins[j].2, acc.3 + bins[j].3)
        });
        peaks.push(Hue { h: gb.atan2(ga).to_degrees().rem_euclid(360.0), c: gc / gw, weight: gw / total });
    }
    peaks.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    // Only hues that are a real part of the icon.
    let strongest = peaks.first().map_or(0.0, |p| p.weight);
    peaks.retain(|p| p.weight >= 0.18 * strongest && p.weight >= 0.06);
    peaks
}

// --------------------------------------------------------------- assignment

/// What an app's slice is painted with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    /// An icon hue (degrees) and its chroma, plus a lightness step (0 or 1)
    /// used when two apps had to share a hue.
    Hue { h: f32, c: f32, step: u8 },
    /// Grey or monochrome icon: a graphite tone.
    Neutral,
}

/// Give every app a tone. `apps` is (name, candidate hues), most-used first.
pub fn assign(apps: &[(String, Vec<Hue>)]) -> HashMap<String, Tone> {
    // Apps with fewer options choose first; ties go to the most used.
    let mut order: Vec<usize> = (0..apps.len()).collect();
    order.sort_by_key(|&i| (apps[i].1.len().clamp(0, 2) == 2, i));
    let mut taken: Vec<f32> = Vec::new();
    let mut out = HashMap::new();
    for i in order {
        let (name, hues) = &apps[i];
        let tone = if hues.is_empty() {
            Tone::Neutral
        } else if let Some(h) = hues.iter().find(|h| taken.iter().all(|&t| hue_gap(t, h.h) >= MIN_HUE_GAP)) {
            taken.push(h.h);
            Tone::Hue { h: h.h, c: h.c, step: 0 }
        } else {
            let h = hues[0];
            Tone::Hue { h: h.h, c: h.c, step: 1 }
        };
        out.insert(name.clone(), tone);
    }
    out
}

/// The colour of a tone in the given theme.
pub fn color(tone: Tone, dark: bool) -> Color {
    match tone {
        Tone::Hue { h, c, step } => {
            // One lightness for every app keeps the dial calm; the chroma
            // cap keeps neon icons from shouting over the rest.
            let c = c.clamp(0.07, 0.15);
            let l = match (dark, step) {
                (false, 0) => 0.66,
                (false, _) => 0.50,
                (true, 0) => 0.74,
                (true, _) => 0.60,
            };
            oklch(l + yellow_lift(h, dark), c, h)
        }
        Tone::Neutral => {
            if dark {
                oklch(0.68, 0.012, 70.0)
            } else {
                oklch(0.50, 0.012, 70.0)
            }
        }
    }
}

/// Yellows only read as yellow when light; at the shared lightness they
/// turn olive. Lift them along a smooth bump centred on 95 degrees.
fn yellow_lift(h: f32, dark: bool) -> f32 {
    let d = hue_gap(h, 95.0);
    if d >= 45.0 {
        return 0.0;
    }
    let bump = (d / 45.0 * std::f32::consts::FRAC_PI_2).cos();
    bump * if dark { 0.08 } else { 0.13 }
}

/// The "Other" row: quieter than any app.
pub fn other(dark: bool) -> Color {
    if dark {
        oklch(0.45, 0.008, 70.0)
    } else {
        oklch(0.80, 0.008, 70.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(colors: &[([u8; 3], usize)]) -> Vec<u8> {
        let mut px = Vec::new();
        for &(rgb, n) in colors {
            for _ in 0..n {
                px.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        // A transparent margin must not vote.
        px.extend_from_slice(&[255, 0, 0, 0].repeat(50));
        px
    }

    #[test]
    fn oklch_round_trips_through_oklab() {
        let c = oklch(0.66, 0.12, 40.0);
        let [l, a, b] = oklab(c.r, c.g, c.b);
        assert!((l - 0.66).abs() < 0.01, "{l}");
        assert!(((a * a + b * b).sqrt() - 0.12).abs() < 0.01);
        assert!((b.atan2(a).to_degrees() - 40.0).abs() < 1.5);
    }

    #[test]
    fn dominant_hue_comes_first_and_grey_is_ignored() {
        // Mostly orange, some purple, lots of white and grey.
        let px = icon(&[([240, 120, 30], 300), ([120, 60, 200], 120), ([255, 255, 255], 400), ([128, 128, 128], 200)]);
        let hues = icon_hues(&px);
        assert_eq!(hues.len(), 2, "{hues:?}");
        assert!(hue_gap(hues[0].h, 55.0) < 20.0, "orange first: {hues:?}");
        assert!(hue_gap(hues[1].h, 300.0) < 25.0, "then purple: {hues:?}");
    }

    #[test]
    fn grey_icon_is_neutral() {
        let px = icon(&[([30, 30, 30], 300), ([200, 200, 200], 300)]);
        assert!(icon_hues(&px).is_empty());
        let tones = assign(&[("Terminal".into(), vec![])]);
        assert_eq!(tones["Terminal"], Tone::Neutral);
    }

    #[test]
    fn apps_sharing_a_hue_are_pushed_apart() {
        let orange = Hue { h: 50.0, c: 0.15, weight: 0.7 };
        let purple = Hue { h: 300.0, c: 0.15, weight: 0.3 };
        let coral = Hue { h: 40.0, c: 0.13, weight: 1.0 };
        // Firefox is used more, but has a second hue; Claude has only one.
        let tones = assign(&[("Firefox".into(), vec![orange, purple]), ("Claude".into(), vec![coral])]);
        assert!(matches!(tones["Claude"], Tone::Hue { h, step: 0, .. } if h == 40.0));
        assert!(matches!(tones["Firefox"], Tone::Hue { h, step: 0, .. } if h == 300.0));
        // A third orange-only app has nowhere to go: same hue, darker step.
        let tones = assign(&[("A".into(), vec![coral]), ("B".into(), vec![orange])]);
        assert!(matches!(tones["B"], Tone::Hue { step: 1, .. }));
    }

    #[test]
    fn yellow_is_lifted_out_of_olive() {
        let y = color(Tone::Hue { h: 95.0, c: 0.15, step: 0 }, false);
        let [l, _, _] = oklab(y.r, y.g, y.b);
        assert!(l > 0.76, "{l}");
    }

    #[test]
    fn toned_colours_share_one_lightness() {
        for h in [20.0, 160.0, 250.0] {
            let c = color(Tone::Hue { h, c: 0.3, step: 0 }, false);
            let [l, a, b] = oklab(c.r, c.g, c.b);
            assert!((l - 0.66).abs() < 0.01);
            assert!((a * a + b * b).sqrt() <= 0.151);
        }
    }
}
