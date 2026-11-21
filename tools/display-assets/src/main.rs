//! Converts the LVGL fonts and images of the displays into the firmware's display assets:
//! `fonts.rs` and `images.rs` (formatted with `rustfmt`) and `background.rgb565`.
//!
//! ```text
//! display-assets <LVGL sources> <lv_font_unscii_16.c> [output directory]
//! ```
//!
//! `<LVGL sources>` holds `lv_font_portfolio-6x8.c`, `logo.c`, `images/fonts/ui_font_*.c` and
//! `images/generated/bitaxe_background.c`. `lv_font_unscii_16.c` is `src/font/lv_font_unscii_16.c`
//! of LVGL. The output directory defaults to `firmware/src/display` of this repository.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

mod lvgl;

const OLED: &str = "#[cfg(feature = \"display-oled\")]";
const LCD: &str = "#[cfg(feature = \"display-st7789\")]";
const ANY: &str = "#[cfg(any(feature = \"display-oled\", feature = \"display-st7789\"))]";

const BACKGROUND_WIDTH: usize = 320;
const BACKGROUND_HEIGHT: usize = 170;

/// A font to convert: LVGL source, Rust name, doc comment, fallback font, display `cfg`.
struct FontSpec {
    file: PathBuf,
    name: &'static str,
    doc: Option<&'static str>,
    fallback: Option<&'static str>,
    cfg: &'static str,
}

/// The Rust code of `font`.
fn font_code(spec: &FontSpec, font: &lvgl::Font) -> String {
    let name = spec.name;
    let cfg = spec.cfg;
    let mut code = String::new();
    let bitmap: Vec<String> = font.bitmap.iter().map(|b| format!("0x{b:02x}")).collect();
    let _ = writeln!(
        code,
        "{cfg}\nstatic {name}_BITMAP: [u8; {}] = [{}];",
        bitmap.len(),
        bitmap.join(", ")
    );
    let _ = writeln!(
        code,
        "{cfg}\nstatic {name}_GLYPHS: [Glyph; {}] = [",
        font.glyphs.len()
    );
    for [index, adv, w, h, x, y] in &font.glyphs {
        let _ = writeln!(code, "    g({index}, {adv}, {w}, {h}, {x}, {y}),");
    }
    code.push_str("];\n");
    let mut cmaps = String::new();
    for cmap in &font.cmaps {
        let list = match &cmap.list {
            Some(list) => {
                let list_name = format!("{name}_{}", list.to_uppercase());
                let values: Vec<String> = font.lists[list].iter().map(u16::to_string).collect();
                let _ = writeln!(
                    code,
                    "{cfg}\nstatic {list_name}: [u16; {}] = [{}];",
                    values.len(),
                    values.join(", ")
                );
                format!("Some(&{list_name})")
            }
            None => "None".to_owned(),
        };
        let _ = writeln!(
            cmaps,
            "    Cmap {{\n        range_start: {},\n        range_length: {},\n        \
             glyph_id_start: {},\n        list: {list},\n    }},",
            cmap.range_start, cmap.range_length, cmap.glyph_id_start
        );
    }
    let _ = write!(
        code,
        "{cfg}\nstatic {name}_CMAPS: [Cmap; {}] = [\n{cmaps}];\n",
        font.cmaps.len()
    );
    if let Some(doc) = spec.doc {
        let _ = writeln!(code, "/// {doc}");
    }
    let fallback = spec
        .fallback
        .map_or("None".to_owned(), |f| format!("Some(&{f})"));
    let _ = writeln!(
        code,
        "{cfg}\npub static {name}: Font = Font {{\n    line_height: {},\n    base_line: {},\n    \
         bitmap: &{name}_BITMAP,\n    glyphs: &{name}_GLYPHS,\n    cmaps: &{name}_CMAPS,\n    \
         fallback: {fallback},\n}};\n",
        font.line_height, font.base_line
    );
    code
}

/// The Rust code of a monochrome image.
fn mono_code(name: &str, width: usize, height: usize, data: &[u8], cfg: &str) -> String {
    let bytes: Vec<String> = data.iter().map(|b| format!("0x{b:02x}")).collect();
    format!(
        "{cfg}\nstatic {name}_DATA: [u8; {}] = [{}];\n{cfg}\npub static {name}: MonoImage = \
         MonoImage {{\n    width: {width},\n    height: {height},\n    data: &{name}_DATA,\n}};\n\n",
        bytes.len(),
        bytes.join(", ")
    )
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

/// Formats `path` with `rustfmt`, or says how to.
fn format(path: &Path) {
    let formatted = Command::new("rustfmt")
        .args(["--edition", "2021"])
        .arg(path)
        .status()
        .is_ok_and(|s| s.success());
    if !formatted {
        eprintln!("rustfmt failed: run `cargo fmt` in firmware/");
    }
}

fn run(main: &Path, unscii: &Path, out: &Path) -> Result<(), String> {
    let mut fonts = String::from(
        "//! Fonts of the status screens.\n\nuse miner_core::ui::{Cmap, Font, Glyph};\n\nconst fn \
         g(bitmap_index: u32, adv_w: u16, box_w: u8, box_h: u8, ofs_x: i8, ofs_y: i8) -> Glyph \
         {\n    Glyph {\n        bitmap_index,\n        adv_w,\n        box_w,\n        box_h,\n        \
         ofs_x,\n        ofs_y,\n    }\n}\n\n",
    );
    let stats_font = |file: &str, name| FontSpec {
        file: main.join("images/fonts").join(file),
        name,
        doc: None,
        fallback: None,
        cfg: LCD,
    };
    let specs = [
        FontSpec {
            file: main.join("lv_font_portfolio-6x8.c"),
            name: "PORTFOLIO_6X8",
            doc: Some("6 × 8 pixels."),
            fallback: None,
            cfg: ANY,
        },
        FontSpec {
            file: unscii.to_owned(),
            name: "UNSCII_16",
            doc: Some("8 × 16 pixels, ASCII, the rest from [`PORTFOLIO_6X8`]."),
            fallback: Some("PORTFOLIO_6X8"),
            cfg: LCD,
        },
        stats_font("ui_font_DigitalNumbers16.c", "DIGITAL_NUMBERS_16"),
        stats_font("ui_font_DigitalNumbers28.c", "DIGITAL_NUMBERS_28"),
        stats_font("ui_font_OpenSansBold13.c", "OPEN_SANS_BOLD_13"),
        stats_font("ui_font_OpenSansBold14.c", "OPEN_SANS_BOLD_14"),
        stats_font("ui_font_AngelWish40.c", "ANGEL_WISH_40"),
    ];
    for spec in &specs {
        let font =
            lvgl::font(&read(&spec.file)?).map_err(|e| format!("{}: {e}", spec.file.display()))?;
        eprintln!(
            "{:<20} {:>4} glyphs, {:>5} bitmap bytes, line {} base {}",
            spec.name,
            font.glyphs.len() - 1,
            font.bitmap.len(),
            font.line_height,
            font.base_line
        );
        fonts.push_str(&font_code(spec, &font));
    }
    let fonts_path = out.join("fonts.rs");
    write(&fonts_path, fonts.trim_end().to_owned() + "\n")?;
    format(&fonts_path);

    let logo = read(&main.join("logo.c"))?;
    let mut images =
        format!("//! Images of the status screens.\n\n{ANY}\nuse miner_core::ui::MonoImage;\n\n");
    // the logos are dark on white, the identify text white on black
    for (array, name, width, height, set_white, cfg) in [
        ("bitaxe_logo_map", "BITAXE_LOGO", 77, 30, false, OLED),
        ("osmu_logo_map", "OSMU_LOGO", 125, 27, false, OLED),
        ("identify_text_map", "IDENTIFY", 44, 30, true, ANY),
    ] {
        let data = lvgl::mono(&lvgl::bytes(&logo, array)?, width, height, set_white)
            .map_err(|e| format!("{array}: {e}"))?;
        images.push_str(&mono_code(name, width, height, &data, cfg));
    }
    let _ = writeln!(
        images,
        "/// Background of the statistics screen: {BACKGROUND_WIDTH} × {BACKGROUND_HEIGHT} pixels, \
         RGB565 most significant byte first.\n{LCD}\npub static BACKGROUND: &[u8; \
         {BACKGROUND_WIDTH} * {BACKGROUND_HEIGHT} * 2] = include_bytes!(\"background.rgb565\");"
    );
    let images_path = out.join("images.rs");
    write(&images_path, images)?;
    format(&images_path);

    // generated with the most significant byte first already
    let background = lvgl::bytes(
        &read(&main.join("images/generated/bitaxe_background.c"))?,
        "bitaxe_background_map",
    )?;
    if background.len() != BACKGROUND_WIDTH * BACKGROUND_HEIGHT * 2 {
        return Err(format!("background: {} bytes", background.len()));
    }
    write(&out.join("background.rgb565"), &background)?;
    eprintln!("background.rgb565 {} bytes", background.len());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (main, unscii) = match args.as_slice() {
        [main, unscii] | [main, unscii, _] => (main, unscii),
        _ => {
            eprintln!(
                "usage: display-assets <LVGL sources> <lv_font_unscii_16.c> [output directory]"
            );
            return ExitCode::FAILURE;
        }
    };
    let out = args.get(2).cloned().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../firmware/src/display")
    });
    match run(main, unscii, &out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
