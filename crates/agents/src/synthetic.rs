//! Synthetic test images for evals and the offline mock: a 7-segment scale
//! display renderer and its exact decoder, plus plain photos for triage.
//! Generated programmatically, so no third-party images are needed.

use image::{ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

const DIGIT_W: u32 = 40;
const DIGIT_H: u32 = 80;
const T: u32 = 8;
const GAP: u32 = 16;
const MARGIN: u32 = 24;
const BG: Rgb<u8> = Rgb([182, 199, 168]);
const INK: Rgb<u8> = Rgb([24, 28, 24]);

// Segments a b c d e f g.
const DIGITS: [[bool; 7]; 10] = [
    [true, true, true, true, true, true, false],
    [false, true, true, false, false, false, false],
    [true, true, false, true, true, false, true],
    [true, true, true, true, false, false, true],
    [false, true, true, false, false, true, true],
    [true, false, true, true, false, true, true],
    [true, false, true, true, true, true, true],
    [true, true, true, false, false, false, false],
    [true; 7],
    [true, true, true, true, false, true, true],
];

const GLYPH_K: [&str; 7] = ["1....", "1....", "1..1.", "1.1..", "11...", "1.1..", "1..1."];
const GLYPH_G: [&str; 7] = [".....", ".111.", "1...1", "1...1", ".1111", "....1", ".111."];
const PX: u32 = 4;

fn fill(img: &mut RgbImage, x0: u32, y0: u32, w: u32, h: u32) {
    for y in y0..(y0 + h).min(img.height()) {
        for x in x0..(x0 + w).min(img.width()) {
            img.put_pixel(x, y, INK);
        }
    }
}

fn segment_rects(x: u32, y: u32) -> [(u32, u32, u32, u32); 7] {
    let (w, h) = (DIGIT_W, DIGIT_H);
    [
        (x + T, y, w - 2 * T, T),
        (x + w - T, y + T, T, h / 2 - T),
        (x + w - T, y + h / 2, T, h / 2 - T),
        (x + T, y + h - T, w - 2 * T, T),
        (x, y + h / 2, T, h / 2 - T),
        (x, y + T, T, h / 2 - T),
        (x + T, y + h / 2 - T / 2, w - 2 * T, T),
    ]
}

fn draw_glyph(img: &mut RgbImage, x: u32, y: u32, rows: &[&str; 7]) {
    for (r, row) in rows.iter().enumerate() {
        for (c, ch) in row.chars().enumerate() {
            if ch == '1' {
                fill(img, x + c as u32 * PX, y + r as u32 * PX, PX, PX);
            }
        }
    }
}

/// Renders an LCD-style scale display, e.g. `render_scale("42.60", "kg")`. Returns PNG bytes.
pub fn render_scale(reading: &str, unit: &str) -> Vec<u8> {
    let digits = reading.chars().filter(|c| c.is_ascii_digit()).count() as u32;
    let dots = reading.chars().filter(|&c| c == '.').count() as u32;
    let unit_w = unit.len() as u32 * (5 * PX + GAP);
    let width = MARGIN * 2 + digits * (DIGIT_W + GAP) + dots * (T + GAP) + unit_w;
    let height = DIGIT_H + MARGIN * 2;
    let mut img = RgbImage::from_pixel(width, height, BG);
    let mut x = MARGIN;
    for ch in reading.chars() {
        if let Some(d) = ch.to_digit(10) {
            for (on, (rx, ry, rw, rh)) in DIGITS[d as usize].iter().zip(segment_rects(x, MARGIN)) {
                if *on {
                    fill(&mut img, rx, ry, rw, rh);
                }
            }
            x += DIGIT_W + GAP;
        } else if ch == '.' {
            fill(&mut img, x, MARGIN + DIGIT_H - T, T, T);
            x += T + GAP;
        }
    }
    let unit_y = MARGIN + DIGIT_H - 7 * PX;
    for ch in unit.chars() {
        match ch {
            'k' => draw_glyph(&mut img, x, unit_y, &GLYPH_K),
            'g' => draw_glyph(&mut img, x, unit_y, &GLYPH_G),
            _ => {}
        }
        x += 5 * PX + GAP;
    }
    encode_png(&img)
}

pub fn encode_png(img: &RgbImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).expect("png encoding");
    out.into_inner()
}

/// A flat image of one colour (triage: dark / no items).
pub fn render_flat(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    encode_png(&RgbImage::from_pixel(width, height, Rgb(rgb)))
}

/// A busy, well-lit pattern standing in for a photo of items (deterministic).
pub fn render_busy(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let v = (x.wrapping_mul(31) ^ y.wrapping_mul(17) ^ seed.wrapping_mul(97)) % 200;
        *p = Rgb([(v + 40) as u8, ((v * 3) % 200 + 40) as u8, ((v * 7) % 200 + 30) as u8]);
    }
    encode_png(&img)
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedReading {
    pub text: String,
    pub unit: Option<String>,
}

/// Decodes displays produced by [`render_scale`]. Returns `None` for other images.
pub fn decode_scale(png: &[u8]) -> Option<DecodedReading> {
    let img = image::load_from_memory(png).ok()?.to_rgb8();
    let dark = |x: u32, y: u32| -> bool {
        let p = img.get_pixel(x, y);
        (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 < 90
    };
    let (w, h) = img.dimensions();
    // Column runs of ink → glyph boxes.
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut x = 0;
    while x < w {
        if (0..h).any(|y| dark(x, y)) {
            let x0 = x;
            while x < w && (0..h).any(|y| dark(x, y)) {
                x += 1;
            }
            let x1 = x;
            let ys: Vec<u32> = (0..h).filter(|&y| (x0..x1).any(|xx| dark(xx, y))).collect();
            boxes.push((x0, *ys.first()?, x1 - x0, ys.last()? - ys.first()? + 1));
        } else {
            x += 1;
        }
    }
    if boxes.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut unit_glyphs = 0;
    for (bx, by, bw, bh) in boxes {
        if bh >= DIGIT_H / 2 {
            if bw < DIGIT_W / 2 {
                text.push('1');
                continue;
            }
            let at = |fx: u32, fy: u32| dark(bx + fx, by + fy);
            let segs = [
                at(bw / 2, T / 2),
                at(bw - T / 2, bh / 4),
                at(bw - T / 2, 3 * bh / 4),
                at(bw / 2, bh - T / 2),
                at(T / 2, 3 * bh / 4),
                at(T / 2, bh / 4),
                at(bw / 2, bh / 2),
            ];
            let d = DIGITS.iter().position(|p| *p == segs)?;
            text.push(char::from_digit(d as u32, 10)?);
        } else if bh <= T + 1 && bw <= T + 1 {
            text.push('.');
        } else {
            unit_glyphs += 1;
        }
    }
    if !text.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let unit = match unit_glyphs {
        2 => Some("kg".to_string()),
        1 => Some("g".to_string()),
        _ => None,
    };
    Some(DecodedReading { text, unit })
}

/// Simple image statistics used by the offline triage mock.
pub struct ImageStats {
    pub mean_luma: f32,
    pub std_luma: f32,
}

pub fn image_stats(bytes: &[u8]) -> Option<ImageStats> {
    let img = image::load_from_memory(bytes).ok()?.to_luma8();
    let n = img.pixels().len().max(1) as f32;
    let mean = img.pixels().map(|p| p[0] as f32).sum::<f32>() / n;
    let var = img.pixels().map(|p| (p[0] as f32 - mean).powi(2)).sum::<f32>() / n;
    Some(ImageStats { mean_luma: mean, std_luma: var.sqrt() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_decodes_every_digit() {
        for s in ["0123456789", "42.60", "7.5", "1111", "980"] {
            let png = render_scale(s, "kg");
            let d = decode_scale(&png).unwrap();
            assert_eq!(d.text, s);
            assert_eq!(d.unit.as_deref(), Some("kg"));
        }
        assert_eq!(decode_scale(&render_scale("350", "g")).unwrap().unit.as_deref(), Some("g"));
    }

    #[test]
    fn non_scale_images_do_not_decode() {
        assert!(decode_scale(&render_flat(64, 64, [200, 200, 200])).is_none());
        assert!(decode_scale(b"not an image").is_none());
    }

    #[test]
    fn stats() {
        let s = image_stats(&render_flat(32, 32, [10, 10, 10])).unwrap();
        assert!(s.mean_luma < 20.0 && s.std_luma < 1.0);
        let b = image_stats(&render_busy(64, 64, 1)).unwrap();
        assert!(b.std_luma > 20.0);
    }
}
