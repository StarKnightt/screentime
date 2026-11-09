//! Builds the .exe icon from the procedural logo and embeds it together with
//! version info (FileDescription "Screentime", which is also how the tracker
//! names this app).

#[path = "src/logo.rs"]
mod logo;

use std::io::Write;
use std::path::Path;

fn ico(path: &Path) {
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let rgba = logo::render(s);
            let mut dib = Vec::new();
            // BITMAPINFOHEADER; height counts XOR + AND masks.
            for v in [40u32, s, s * 2] {
                dib.extend_from_slice(&v.to_le_bytes());
            }
            dib.extend_from_slice(&1u16.to_le_bytes()); // planes
            dib.extend_from_slice(&32u16.to_le_bytes()); // bpp
            dib.extend_from_slice(&[0u8; 24]); // compression .. clr important
            for y in (0..s).rev() {
                for x in 0..s {
                    let i = ((y * s + x) * 4) as usize;
                    dib.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            let mask_row = s.div_ceil(32) * 4;
            dib.extend(std::iter::repeat_n(0u8, (mask_row * s) as usize));
            dib
        })
        .collect();
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&[0, 0, 1, 0]).unwrap();
    f.write_all(&(sizes.len() as u16).to_le_bytes()).unwrap();
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        f.write_all(&[dim, dim, 0, 0]).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&32u16.to_le_bytes()).unwrap();
        f.write_all(&(img.len() as u32).to_le_bytes()).unwrap();
        f.write_all(&offset.to_le_bytes()).unwrap();
        offset += img.len() as u32;
    }
    for img in &images {
        f.write_all(img).unwrap();
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out.join("screentime.ico");
    ico(&ico_path);
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let (maj, min, pat) = {
        let v: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        (v[0], v[1], v[2])
    };
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {maj},{min},{pat},0
PRODUCTVERSION {maj},{min},{pat},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Screentime"
      VALUE "ProductName", "Screentime"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "Screentime.exe"
      VALUE "LegalCopyright", "MIT License"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico_path.display().to_string().replace('\\', "\\\\"),
    );
    let rc_path = out.join("app.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_optional().unwrap();
}
