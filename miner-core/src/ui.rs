//! Text and images of the status screens: bitmap fonts, labels and their layout.
//!
//! Drawing goes through a `plot(x, y)` callback for every set pixel, so the same code draws on a
//! monochrome frame buffer or on a band of a colour panel.

/// A glyph of a [`Font`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// First byte of the glyph bitmap in [`Font::bitmap`]: `box_w × box_h` bits, 1 bit per pixel,
    /// row after row, most significant bit first.
    pub bitmap_index: u32,
    /// Pen advance (1/16 pixel).
    pub adv_w: u16,
    pub box_w: u8,
    pub box_h: u8,
    /// Box position from the pen (x) and from the baseline (y, up).
    pub ofs_x: i8,
    pub ofs_y: i8,
}

/// Code points mapped to glyphs: from `range_start`, `range_length` of them, to the glyphs from
/// `glyph_id_start`. Every code point of the range, or only those of `list` (offsets from
/// `range_start`, in increasing order), get a glyph, in order.
#[derive(Clone, Copy, Debug)]
pub struct Cmap {
    pub range_start: u32,
    pub range_length: u32,
    pub glyph_id_start: u16,
    pub list: Option<&'static [u16]>,
}

/// A bitmap font, 1 bit per pixel.
#[derive(Debug)]
pub struct Font {
    /// Height of a line of text, the baseline `base_line` above its bottom.
    pub line_height: u8,
    pub base_line: u8,
    pub bitmap: &'static [u8],
    /// Glyph 0 is unused.
    pub glyphs: &'static [Glyph],
    pub cmaps: &'static [Cmap],
    /// Font for the code points this one lacks.
    pub fallback: Option<&'static Font>,
}

impl Font {
    /// The glyph of `c`, from this font or its fallback, with the font it comes from.
    pub fn glyph(&self, c: char) -> Option<(&Font, &Glyph)> {
        let cp = c as u32;
        for cmap in self.cmaps {
            let Some(offset) = cp.checked_sub(cmap.range_start) else {
                continue;
            };
            if offset >= cmap.range_length {
                continue;
            }
            let index = match cmap.list {
                None => Some(offset as usize),
                Some(list) => list.binary_search(&(offset as u16)).ok(),
            };
            if let Some(glyph) =
                index.and_then(|i| self.glyphs.get(cmap.glyph_id_start as usize + i))
            {
                return Some((self, glyph));
            }
        }
        self.fallback.and_then(|f| f.glyph(c))
    }

    /// Pen advance of `glyph` in pixels.
    pub fn advance(glyph: &Glyph) -> i32 {
        (i32::from(glyph.adv_w) + 8) >> 4
    }

    /// Width of `text` in pixels. Characters without a glyph take no room.
    pub fn text_width(&self, text: &str) -> i32 {
        text.chars()
            .filter_map(|c| self.glyph(c))
            .map(|(_, g)| Self::advance(g))
            .sum()
    }

    /// Draws `text` with the top of its line at `(x, y)`: calls `plot` for every set pixel.
    pub fn draw(&self, text: &str, x: i32, y: i32, plot: &mut impl FnMut(i32, i32)) {
        let mut pen = x;
        for c in text.chars() {
            let Some((font, glyph)) = self.glyph(c) else {
                continue;
            };
            // the glyphs of a fallback font sit on this font's baseline
            let top = y + i32::from(self.line_height)
                - i32::from(self.base_line)
                - i32::from(glyph.box_h)
                - i32::from(glyph.ofs_y);
            let left = pen + i32::from(glyph.ofs_x);
            let mut bit = glyph.bitmap_index as usize * 8;
            for row in 0..i32::from(glyph.box_h) {
                for col in 0..i32::from(glyph.box_w) {
                    if font.bitmap[bit / 8] & (0x80 >> (bit % 8)) != 0 {
                        plot(left + col, top + row);
                    }
                    bit += 1;
                }
            }
            pen += Self::advance(glyph);
        }
    }
}

/// Horizontal alignment of a text in its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Speed of a scrolling label (pixels per second).
pub const SCROLL_SPEED: u32 = 25;

/// A text in a box, cut at the box edges.
pub struct Label<'a> {
    pub font: &'a Font,
    pub text: &'a str,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub align: Align,
    /// Time since the label started scrolling: a text wider than its box then scrolls left in a
    /// loop, followed by a gap, instead of being cut.
    pub scroll_ms: Option<u32>,
}

impl Label<'_> {
    /// Draws the label: calls `plot` for every set pixel inside the box.
    pub fn draw(&self, plot: &mut impl FnMut(i32, i32)) {
        let text_width = self.font.text_width(self.text);
        let mut clip = |px: i32, py: i32| {
            if px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
            {
                plot(px, py);
            }
        };
        match self.scroll_ms {
            Some(ms) if text_width > self.width => {
                let period = text_width + scroll_gap(self.font);
                let offset = scroll_offset(period, ms);
                self.font
                    .draw(self.text, self.x - offset, self.y, &mut clip);
                self.font
                    .draw(self.text, self.x - offset + period, self.y, &mut clip);
            }
            _ => {
                let x = match self.align {
                    Align::Left => self.x,
                    Align::Center => self.x + (self.width - text_width) / 2,
                    Align::Right => self.x + self.width - text_width,
                };
                self.font.draw(self.text, x, self.y, &mut clip);
            }
        }
    }
}

/// Gap between the end of a scrolling text and its next copy: three spaces.
fn scroll_gap(font: &Font) -> i32 {
    3 * font.text_width(" ")
}

/// Scroll offset after `ms` of a text looping every `period` pixels.
pub fn scroll_offset(period: i32, ms: u32) -> i32 {
    if period <= 0 {
        return 0;
    }
    let pixels = u64::from(ms) * u64::from(SCROLL_SPEED) / 1000;
    (pixels % period as u64) as i32
}

/// Top of the first of `lines` lines of `line_height`, `pad` apart, centred in the area from `top`
/// of `height`.
pub fn column_top(lines: usize, line_height: i32, pad: i32, top: i32, height: i32) -> i32 {
    let n = lines as i32;
    let content = n * line_height + (n - 1).max(0) * pad;
    top + (height - content) / 2
}

/// A monochrome image: rows of `width` bits, most significant bit first, each row starting on a
/// byte.
pub struct MonoImage {
    pub width: i32,
    pub height: i32,
    pub data: &'static [u8],
}

impl MonoImage {
    /// Draws the image with its top-left corner at `(x, y)`.
    pub fn draw(&self, x: i32, y: i32, plot: &mut impl FnMut(i32, i32)) {
        let stride = (self.width as usize).div_ceil(8);
        for row in 0..self.height {
            for col in 0..self.width {
                let byte = self.data[row as usize * stride + col as usize / 8];
                if byte & (0x80 >> (col % 8)) != 0 {
                    plot(x + col, y + row);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    // A 2-glyph font: ' ' (3 px advance) and 'A', a 2×2 block on the baseline.
    static BITMAP: [u8; 1] = [0b1111_0000];
    static GLYPHS: [Glyph; 3] = [
        Glyph {
            bitmap_index: 0,
            adv_w: 0,
            box_w: 0,
            box_h: 0,
            ofs_x: 0,
            ofs_y: 0,
        },
        Glyph {
            bitmap_index: 0,
            adv_w: 3 * 16,
            box_w: 0,
            box_h: 0,
            ofs_x: 0,
            ofs_y: 0,
        },
        Glyph {
            bitmap_index: 0,
            adv_w: 3 * 16,
            box_w: 2,
            box_h: 2,
            ofs_x: 0,
            ofs_y: 0,
        },
    ];
    static LIST: [u16; 2] = [0, 33];
    static CMAPS: [Cmap; 1] = [Cmap {
        range_start: 32,
        range_length: 34,
        glyph_id_start: 1,
        list: Some(&LIST),
    }];
    static FONT: Font = Font {
        line_height: 4,
        base_line: 1,
        bitmap: &BITMAP,
        glyphs: &GLYPHS,
        cmaps: &CMAPS,
        fallback: None,
    };

    fn pixels(f: impl FnOnce(&mut dyn FnMut(i32, i32))) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        f(&mut |x, y| out.push((x, y)));
        out
    }

    #[test]
    fn glyphs_come_from_the_sparse_list() {
        assert_eq!(FONT.glyph('A').map(|(_, g)| g.box_w), Some(2));
        assert_eq!(FONT.glyph(' ').map(|(_, g)| g.box_w), Some(0));
        assert!(FONT.glyph('B').is_none());
        assert_eq!(FONT.text_width("A A"), 9);
    }

    #[test]
    fn glyphs_sit_on_the_baseline() {
        // line 4 high, baseline 1 above the bottom: the 2-pixel box covers rows 1 and 2
        let drawn = pixels(|plot| FONT.draw("A", 10, 0, &mut |x, y| plot(x, y)));
        assert_eq!(drawn, [(10, 1), (11, 1), (10, 2), (11, 2)]);
    }

    #[test]
    fn labels_align_and_clip() {
        let label = |align, width| Label {
            font: &FONT,
            text: "A",
            x: 0,
            y: 0,
            width,
            height: 2,
            align,
            scroll_ms: None,
        };
        let right = pixels(|plot| label(Align::Right, 7).draw(&mut |x, y| plot(x, y)));
        assert_eq!(right, [(4, 1), (5, 1)]); // the row 2 is out of the 2-pixel box
        let center = pixels(|plot| label(Align::Center, 7).draw(&mut |x, y| plot(x, y)));
        assert_eq!(center, [(2, 1), (3, 1)]);
    }

    #[test]
    fn scrolling_loops() {
        assert_eq!(scroll_offset(100, 0), 0);
        assert_eq!(scroll_offset(100, 2_000), 50);
        assert_eq!(scroll_offset(100, 4_000), 0);
        assert_eq!(scroll_offset(0, 1_000), 0);
    }

    #[test]
    fn columns_are_centred() {
        assert_eq!(column_top(4, 8, 0, 0, 32), 0);
        assert_eq!(column_top(2, 8, 0, 0, 32), 8);
        assert_eq!(column_top(3, 17, 1, 8, 154), 8 + (154 - 53) / 2);
    }

    #[test]
    fn mono_images_draw_their_set_bits() {
        static DATA: [u8; 2] = [0b1000_0000, 0b0100_0000];
        let image = MonoImage {
            width: 2,
            height: 2,
            data: &DATA,
        };
        let drawn = pixels(|plot| image.draw(5, 5, &mut |x, y| plot(x, y)));
        assert_eq!(drawn, [(5, 5), (6, 6)]);
    }
}
