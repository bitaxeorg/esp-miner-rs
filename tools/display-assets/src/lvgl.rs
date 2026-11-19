//! Reading the LVGL C sources: `lv_font_fmt_txt` fonts and byte arrays.

use std::collections::BTreeMap;

/// A glyph descriptor: bitmap index, advance (1/16 pixel), box width and height, box offsets.
pub type Glyph = [i64; 6];

/// A character map of a font.
#[derive(Debug, PartialEq, Eq)]
pub struct Cmap {
    pub range_start: u32,
    pub range_length: u32,
    pub glyph_id_start: u32,
    /// Name of the array of code point offsets of a sparse map.
    pub list: Option<String>,
}

/// An `lv_font_fmt_txt` font, 1 bit per pixel, plain bitmaps.
#[derive(Debug)]
pub struct Font {
    pub bitmap: Vec<u8>,
    pub glyphs: Vec<Glyph>,
    pub lists: BTreeMap<String, Vec<u16>>,
    pub cmaps: Vec<Cmap>,
    pub line_height: u32,
    pub base_line: u32,
}

/// `src` without its C comments.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
        } else if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else {
            let c = rest.chars().next().unwrap_or_default();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The range of the text between the first `{` after `from` and its matching `}`.
fn braced(src: &str, from: usize) -> Option<std::ops::Range<usize>> {
    let open = from + src[from..].find('{')?;
    let mut depth = 0;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + 1..open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The initializer of the C array `name`.
fn array<'a>(src: &'a str, name: &str) -> Result<&'a str, String> {
    let at = src
        .find(&format!("{name}[]"))
        .ok_or_else(|| format!("no array {name}"))?;
    braced(src, at)
        .map(|range| &src[range])
        .ok_or_else(|| format!("array {name} not closed"))
}

/// The `0x` numbers of `text`, in order.
fn hex_values(text: &str) -> Vec<u32> {
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("0x") {
        let digits: String = rest[at + 2..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        if let Ok(v) = u32::from_str_radix(&digits, 16) {
            values.push(v);
        }
        rest = &rest[at + 2 + digits.len()..];
    }
    values
}

/// The bytes of the C array `name`.
pub fn bytes(src: &str, name: &str) -> Result<Vec<u8>, String> {
    let src = strip_comments(src);
    hex_values(array(&src, name)?)
        .into_iter()
        .map(|v| u8::try_from(v).map_err(|_| format!("{name}: {v:#x} is not a byte")))
        .collect()
}

/// The value of the designated initializer `.name = value` in `text`, as written.
fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(".{name}");
    let mut rest = text;
    while let Some(at) = rest.find(&key) {
        let after = &rest[at + key.len()..];
        if let Some(value) = after.trim_start().strip_prefix('=') {
            let value = value.trim_start();
            let end = value.find([',', '}', '\n']).unwrap_or(value.len());
            return Some(value[..end].trim());
        }
        rest = after;
    }
    None
}

fn number_field(text: &str, name: &str) -> Result<i64, String> {
    let value = field(text, name).ok_or_else(|| format!("no field .{name}"))?;
    value
        .parse()
        .map_err(|_| format!(".{name} = {value} is not a number"))
}

/// The top-level `{ ... }` entries of an array initializer.
fn entries(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(range) = braced(body, from) {
        from = range.end + 1;
        out.push(&body[range]);
    }
    out
}

/// Reads an `lv_font_fmt_txt` font. Fails unless it has 1 bit per pixel and plain bitmaps.
pub fn font(src: &str) -> Result<Font, String> {
    let src = strip_comments(src);
    if number_field(&src, "bpp")? != 1 {
        return Err("not 1 bit per pixel".into());
    }
    if number_field(&src, "bitmap_format")? != 0 {
        return Err("compressed bitmaps".into());
    }
    let bitmap = hex_values(array(&src, "glyph_bitmap")?)
        .into_iter()
        .map(|v| v as u8)
        .collect();
    let glyphs = entries(array(&src, "glyph_dsc")?)
        .into_iter()
        .map(|e| {
            let mut g = [0; 6];
            for (value, name) in
                g.iter_mut()
                    .zip(["bitmap_index", "adv_w", "box_w", "box_h", "ofs_x", "ofs_y"])
            {
                *value = number_field(e, name)?;
            }
            Ok(g)
        })
        .collect::<Result<_, String>>()?;
    let mut lists = BTreeMap::new();
    let mut from = 0;
    while let Some(at) = src[from..].find("unicode_list_") {
        let at = from + at;
        let name: String = src[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if src[at + name.len()..].starts_with("[]") {
            let values = hex_values(array(&src, &name)?)
                .into_iter()
                .map(|v| v as u16)
                .collect();
            lists.insert(name.clone(), values);
        }
        from = at + name.len();
    }
    let cmaps = entries(array(&src, "cmaps")?)
        .into_iter()
        .map(|e| {
            let list = field(e, "unicode_list").ok_or("no .unicode_list")?;
            if field(e, "glyph_id_ofs_list").is_some_and(|ofs| ofs != "NULL") {
                return Err("character map with a glyph offset list".to_owned());
            }
            Ok(Cmap {
                range_start: number_field(e, "range_start")? as u32,
                range_length: number_field(e, "range_length")? as u32,
                glyph_id_start: number_field(e, "glyph_id_start")? as u32,
                list: (list != "NULL").then(|| list.to_owned()),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(Font {
        bitmap,
        glyphs,
        lists,
        cmaps,
        line_height: number_field(&src, "line_height")? as u32,
        base_line: number_field(&src, "base_line")? as u32,
    })
}

/// A monochrome image of a little-endian RGB565 one: rows of `width` bits, most significant bit
/// first, each row starting on a byte, set where the pixel isn't white.
pub fn mono(rgb565_le: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    if rgb565_le.len() < width * height * 2 {
        return Err(format!("{} bytes for {width} × {height}", rgb565_le.len()));
    }
    let stride = width.div_ceil(8);
    let mut out = vec![0; stride * height];
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 2;
            if u16::from_le_bytes([rgb565_le[i], rgb565_le[i + 1]]) != 0xffff {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &str = r#"
/* comment with { brace */
static LV_ATTRIBUTE_LARGE_CONST const uint8_t glyph_bitmap[] = {
    /* U+0020 " " */
    0x0,
    /* U+0041 "A" */
    0xf0, // two rows
};
static const lv_font_fmt_txt_glyph_dsc_t glyph_dsc[] = {
    {.bitmap_index = 0, .adv_w = 0, .box_w = 0, .box_h = 0, .ofs_x = 0, .ofs_y = 0} /* id = 0 reserved */,
    {.bitmap_index = 0, .adv_w = 48, .box_w = 1, .box_h = 1, .ofs_x = 0, .ofs_y = 0},
    {.bitmap_index = 1, .adv_w = 48, .box_w = 2, .box_h = 2, .ofs_x = 0, .ofs_y = -1}
};
static const uint16_t unicode_list_0[] = {
    0x0, 0x21
};
static const lv_font_fmt_txt_cmap_t cmaps[] =
{
    {
        .range_start = 32, .range_length = 34, .glyph_id_start = 1,
        .unicode_list = unicode_list_0, .glyph_id_ofs_list = NULL, .list_length = 2, .type = LV_FONT_FMT_TXT_CMAP_SPARSE_TINY
    }
};
static lv_font_fmt_txt_dsc_t font_dsc = {
    .glyph_bitmap = glyph_bitmap,
    .glyph_dsc = glyph_dsc,
    .cmaps = cmaps,
    .cmap_num = 1,
    .bpp = 1,
    .bitmap_format = 0,
};
const lv_font_t font = {
    .line_height = 4,          /*The maximum line height required by the font*/
    .base_line = 1,             /*Baseline measured from the bottom of the line*/
};
"#;

    #[test]
    fn reads_a_font() {
        let f = font(FONT).unwrap();
        assert_eq!(f.bitmap, [0x00, 0xf0]);
        assert_eq!(f.glyphs.len(), 3);
        assert_eq!(f.glyphs[2], [1, 48, 2, 2, 0, -1]);
        assert_eq!(f.lists["unicode_list_0"], [0x0, 0x21]);
        assert_eq!(
            f.cmaps,
            [Cmap {
                range_start: 32,
                range_length: 34,
                glyph_id_start: 1,
                list: Some("unicode_list_0".into()),
            }]
        );
        assert_eq!((f.line_height, f.base_line), (4, 1));
    }

    #[test]
    fn refuses_what_it_cannot_convert() {
        for (from, to) in [
            (".bitmap_format = 0", ".bitmap_format = 1"),
            (".bpp = 1", ".bpp = 4"),
            (
                ".glyph_id_ofs_list = NULL",
                ".glyph_id_ofs_list = glyph_id_ofs_list_0",
            ),
        ] {
            assert!(font(&FONT.replace(from, to)).is_err(), "{to}");
        }
    }

    #[test]
    fn reads_byte_arrays() {
        let src = "const uint8_t logo_map[] = {\n  0x00, 0xff, /* 0xaa */\n  0x12,\n};";
        assert_eq!(bytes(src, "logo_map").unwrap(), [0x00, 0xff, 0x12]);
        assert!(bytes(src, "other_map").is_err());
    }

    #[test]
    fn mono_images_set_the_non_white_pixels() {
        // 3 × 2: white, black, white / black, white, white
        let px = [0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(mono(&px, 3, 2).unwrap(), [0b0100_0000, 0b1000_0000]);
        assert!(mono(&px[..10], 3, 2).is_err());
    }
}
