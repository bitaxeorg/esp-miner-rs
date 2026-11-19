//! Board number and family name of the selected `bitaxe-<number>` feature, for the firmware:
//! `BOARD_VERSION` and `BOARD_NAME`; and the web assets of `web/`, gzipped into `OUT_DIR`.

use std::{env, fs, path::Path};

/// Files of `web/` served gzipped.
const WEB_ASSETS: [&str; 3] = ["htmx.min.js", "style.css", "app.js"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = env::var("OUT_DIR").expect("OUT_DIR");
    for name in WEB_ASSETS {
        let path = Path::new("web").join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let data = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        fs::write(Path::new(&out).join(format!("{name}.gz")), gzip(&data)).expect("write");
    }

    let version = std::env::vars()
        .filter_map(|(key, _)| {
            key.strip_prefix("CARGO_FEATURE_BITAXE_")
                .filter(|rest| rest.chars().all(|c| c.is_ascii_digit()))
                .map(str::to_owned)
        })
        .next()
        .unwrap_or_default();
    let name = match version.parse::<u32>().unwrap_or(0) {
        100..=199 => "Max",
        200..=299 => "Ultra",
        300..=399 => "Hex",
        400..=499 => "Supra",
        650 => "GammaDuo",
        600..=699 => "Gamma",
        700..=799 => "SupraHex",
        800..=899 => "GammaTurbo",
        1200..=1299 => "NajaDuo",
        1300..=1399 => "GammaHex",
        _ => "",
    };
    println!("cargo:rustc-env=BOARD_VERSION={version}");
    println!("cargo:rustc-env=BOARD_NAME={name}");
}

/// `data` as a gzip file: header, deflate stream, CRC-32 and size.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 10));
    out.extend(crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |crc, &byte| {
        (0..8).fold(crc ^ u32::from(byte), |c, _| {
            if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            }
        })
    })
}
