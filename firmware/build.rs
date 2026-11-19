//! Board number and family name of the selected `bitaxe-<number>` feature, for the firmware:
//! `BOARD_VERSION` and `BOARD_NAME`.

fn main() {
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
