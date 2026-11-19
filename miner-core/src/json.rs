//! A JSON writer over any [`core::fmt::Write`]: objects, arrays and values, commas included.

use core::fmt::{self, Display, Write};

/// Writes JSON to `W`. The first write error is kept and returned by [`JsonWriter::finish`];
/// later writes are skipped.
pub struct JsonWriter<W: Write> {
    out: W,
    /// Bit `n`: a value was already written at nesting level `n`.
    written: u64,
    depth: u32,
    /// A key was just written: the next value needs no comma.
    after_key: bool,
    result: fmt::Result,
}

impl<W: Write> JsonWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            written: 0,
            depth: 0,
            after_key: false,
            result: Ok(()),
        }
    }

    /// The output, or the first write error.
    pub fn finish(self) -> Result<W, fmt::Error> {
        self.result.map(|()| self.out)
    }

    fn write(&mut self, args: fmt::Arguments) {
        if self.result.is_ok() {
            self.result = self.out.write_fmt(args);
        }
    }

    /// Writes the comma a new value or key needs at the current level.
    fn separate(&mut self) {
        if self.after_key {
            self.after_key = false;
            return;
        }
        let bit = 1u64 << self.depth.min(63);
        if self.written & bit != 0 {
            self.write(format_args!(","));
        }
        self.written |= bit;
    }

    fn open(&mut self, bracket: char) -> &mut Self {
        self.separate();
        self.write(format_args!("{bracket}"));
        self.depth += 1;
        self.written &= !(1u64 << self.depth.min(63));
        self
    }

    fn close(&mut self, bracket: char) -> &mut Self {
        self.depth = self.depth.saturating_sub(1);
        self.write(format_args!("{bracket}"));
        self
    }

    pub fn begin_object(&mut self) -> &mut Self {
        self.open('{')
    }

    pub fn end_object(&mut self) -> &mut Self {
        self.close('}')
    }

    pub fn begin_array(&mut self) -> &mut Self {
        self.open('[')
    }

    pub fn end_array(&mut self) -> &mut Self {
        self.close(']')
    }

    /// Writes an object member's key; the next call writes its value.
    pub fn key(&mut self, key: &str) -> &mut Self {
        self.separate();
        self.write_escaped(key);
        self.write(format_args!(":"));
        self.after_key = true;
        self
    }

    pub fn str(&mut self, value: &str) -> &mut Self {
        self.separate();
        self.write_escaped(value);
        self
    }

    /// An integer, or anything else whose `Display` is a JSON number.
    pub fn num(&mut self, value: impl Display) -> &mut Self {
        self.separate();
        self.write(format_args!("{value}"));
        self
    }

    /// A float, `null` when it is not finite.
    pub fn float(&mut self, value: f64) -> &mut Self {
        self.separate();
        if value.is_finite() {
            self.write(format_args!("{value}"));
        } else {
            self.write(format_args!("null"));
        }
        self
    }

    /// A float with `decimals` decimals, `null` when it is not finite.
    pub fn fixed(&mut self, value: f64, decimals: usize) -> &mut Self {
        self.separate();
        if value.is_finite() {
            self.write(format_args!("{value:.decimals$}"));
        } else {
            self.write(format_args!("null"));
        }
        self
    }

    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.separate();
        self.write(format_args!("{value}"));
        self
    }

    pub fn null(&mut self) -> &mut Self {
        self.separate();
        self.write(format_args!("null"));
        self
    }

    /// Text that is already JSON, written as is.
    pub fn raw(&mut self, json: &str) -> &mut Self {
        self.separate();
        self.write(format_args!("{json}"));
        self
    }

    pub fn field_str(&mut self, key: &str, value: &str) -> &mut Self {
        self.key(key).str(value)
    }

    pub fn field_num(&mut self, key: &str, value: impl Display) -> &mut Self {
        self.key(key).num(value)
    }

    pub fn field_float(&mut self, key: &str, value: f64) -> &mut Self {
        self.key(key).float(value)
    }

    pub fn field_bool(&mut self, key: &str, value: bool) -> &mut Self {
        self.key(key).bool(value)
    }

    fn write_escaped(&mut self, s: &str) {
        self.write(format_args!("\""));
        let mut start = 0;
        for (i, c) in s.char_indices() {
            let escape = match c {
                '"' => Some("\\\""),
                '\\' => Some("\\\\"),
                '\n' => Some("\\n"),
                '\r' => Some("\\r"),
                '\t' => Some("\\t"),
                c if (c as u32) < 0x20 => None,
                _ => continue,
            };
            self.write(format_args!("{}", &s[start..i]));
            match escape {
                Some(e) => self.write(format_args!("{e}")),
                None => self.write(format_args!("\\u{:04x}", c as u32)),
            }
            start = i + c.len_utf8();
        }
        self.write(format_args!("{}\"", &s[start..]));
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::String;

    use super::*;

    #[test]
    fn nested_objects_and_arrays_get_their_commas() {
        let mut j = JsonWriter::new(String::new());
        j.begin_object()
            .field_num("a", 1)
            .field_str("b", "x")
            .key("c")
            .begin_array()
            .num(1)
            .begin_object()
            .field_bool("d", true)
            .end_object()
            .raw(r#"{"e":null}"#)
            .end_array()
            .field_float("f", 1.5)
            .field_float("g", f64::NAN)
            .key("h")
            .begin_object()
            .end_object()
            .end_object();
        assert_eq!(
            j.finish().unwrap(),
            r#"{"a":1,"b":"x","c":[1,{"d":true},{"e":null}],"f":1.5,"g":null,"h":{}}"#
        );
    }

    #[test]
    fn strings_are_escaped() {
        let mut j = JsonWriter::new(String::new());
        j.str("a\"b\\c\nd\u{1}é");
        assert_eq!(j.finish().unwrap(), r#""a\"b\\c\nd\u0001é""#);
    }

    #[test]
    fn the_first_error_is_kept() {
        let mut j = JsonWriter::new(heapless::String::<4>::new());
        j.begin_object().field_str("long", "value").end_object();
        assert!(j.finish().is_err());
    }
}
