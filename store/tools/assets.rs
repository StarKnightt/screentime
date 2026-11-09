//! Writes the MSIX visual assets from the app's own procedural logo
//! (gui/src/logo.rs), so the Store icons are the exact same mark as the exe's.
//!
//!   rustc -O --edition 2021 store/tools/assets.rs -o target/store-assets.exe
//!   target/store-assets.exe store/Assets
//!
//! Every file is a PNG named with the MRT qualifiers MakePri understands.

#[path = "../../gui/src/logo.rs"]
#[allow(dead_code)]
mod logo;

use std::io::Write;
use std::path::Path;

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A valid PNG with stored (uncompressed) deflate blocks: no dependencies, and
/// MakeAppx compresses the package anyway.
fn png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let mut raw = Vec::with_capacity((w * h * 4 + h) as usize);
    for row in rgba.chunks((w * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        let n = block.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::File::create(path).unwrap().write_all(&out).unwrap();
}

/// The logo at `mark` px, centred on a transparent `w` x `h` canvas.
fn canvas(w: u32, h: u32, mark: u32) -> Vec<u8> {
    let icon = logo::render(mark);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let (ox, oy) = ((w - mark) / 2, (h - mark) / 2);
    for y in 0..mark {
        for x in 0..mark {
            let s = ((y * mark + x) * 4) as usize;
            let d = (((y + oy) * w + x + ox) * 4) as usize;
            px[d..d + 4].copy_from_slice(&icon[s..s + 4]);
        }
    }
    px
}

fn main() {
    let dir = std::env::args().nth(1).expect("output dir");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let write = |name: &str, w: u32, h: u32, mark: u32| png(&dir.join(name), w, h, &canvas(w, h, mark));
    // Tiles keep a margin (Windows' guidance: the mark at about 2/3 of a square
    // tile, half the height of a wide one); the 44 px app icon fills its box.
    for (scale, k) in [(100, 1.0), (125, 1.25), (150, 1.5), (200, 2.0), (400, 4.0)] {
        let s = |v: f32| (v * k as f32).round() as u32;
        write(&format!("Square150x150Logo.scale-{scale}.png"), s(150.0), s(150.0), s(96.0));
        write(&format!("Wide310x150Logo.scale-{scale}.png"), s(310.0), s(150.0), s(80.0));
        write(&format!("Square44x44Logo.scale-{scale}.png"), s(44.0), s(44.0), s(44.0));
        write(&format!("StoreLogo.scale-{scale}.png"), s(50.0), s(50.0), s(50.0));
        write(&format!("SmallTile.scale-{scale}.png"), s(71.0), s(71.0), s(48.0));
        write(&format!("LargeTile.scale-{scale}.png"), s(310.0), s(310.0), s(200.0));
    }
    // Taskbar, Start and Explorer sizes, plated and unplated.
    for t in [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256] {
        write(&format!("Square44x44Logo.targetsize-{t}.png"), t, t, t);
        write(&format!("Square44x44Logo.altform-unplated_targetsize-{t}.png"), t, t, t);
        write(&format!("Square44x44Logo.altform-lightunplated_targetsize-{t}.png"), t, t, t);
    }
    println!("assets written to {}", dir.display());
}
