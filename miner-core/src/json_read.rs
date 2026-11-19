//! Reading JSON request bodies: the members of an object and the elements of an array, as raw
//! text, then each value as the type the caller expects.

use core::fmt::Write;

/// The JSON text is not what was expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

/// Skips whitespace.
fn skip_ws(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// End (exclusive) of the value starting at `i`.
fn value_end(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    match *b.get(i)? {
        b'"' => {
            let mut j = i + 1;
            while j < b.len() {
                match b[j] {
                    b'\\' => j += 2,
                    b'"' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = value_end(s, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// The members of a JSON object, as (raw key with its quotes, raw value). Stops at the first
/// syntax error, reported as an `Err` item.
pub fn members(json: &str) -> Members<'_> {
    let start = skip_ws(json, 0);
    let ok = json.as_bytes().get(start) == Some(&b'{');
    Members {
        s: json,
        i: start + 1,
        done: !ok,
        failed: !ok,
    }
}

pub struct Members<'a> {
    s: &'a str,
    i: usize,
    done: bool,
    failed: bool,
}

impl<'a> Iterator for Members<'a> {
    type Item = Result<(&'a str, &'a str), Invalid>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return if core::mem::take(&mut self.failed) {
                Some(Err(Invalid))
            } else {
                None
            };
        }
        let s = self.s;
        let mut i = skip_ws(s, self.i);
        if s.as_bytes().get(i) == Some(&b'}') {
            self.done = true;
            return None;
        }
        let item = (|| {
            if s.as_bytes().get(i) != Some(&b'"') {
                return None;
            }
            let key_end = value_end(s, i)?;
            let key = &s[i..key_end];
            i = skip_ws(s, key_end);
            if s.as_bytes().get(i) != Some(&b':') {
                return None;
            }
            i = skip_ws(s, i + 1);
            let end = value_end(s, i)?;
            let value = &s[i..end];
            i = skip_ws(s, end);
            match s.as_bytes().get(i) {
                Some(b',') => i += 1,
                Some(b'}') => {}
                _ => return None,
            }
            Some((key, value, i))
        })();
        match item {
            Some((key, value, next)) => {
                self.i = next;
                Some(Ok((key, value)))
            }
            None => {
                self.done = true;
                Some(Err(Invalid))
            }
        }
    }
}

/// The elements of a JSON array, raw. `None` when `json` is not an array.
pub fn elements(json: &str) -> Option<impl Iterator<Item = Result<&str, Invalid>>> {
    let start = skip_ws(json, 0);
    if json.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut i = start + 1;
    let mut done = false;
    Some(core::iter::from_fn(move || {
        if done {
            return None;
        }
        i = skip_ws(json, i);
        if json.as_bytes().get(i) == Some(&b']') {
            done = true;
            return None;
        }
        let Some(end) = value_end(json, i) else {
            done = true;
            return Some(Err(Invalid));
        };
        let element = &json[i..end];
        i = skip_ws(json, end);
        match json.as_bytes().get(i) {
            Some(b',') => i += 1,
            Some(b']') => {}
            _ => {
                done = true;
                return Some(Err(Invalid));
            }
        }
        Some(Ok(element))
    }))
}

/// Whether a raw key is `name`.
pub fn key_is(raw_key: &str, name: &str) -> bool {
    raw_key.len() == name.len() + 2 && &raw_key[1..raw_key.len() - 1] == name
}

/// A raw JSON string value, unescaped into `out`. `Err` if `raw` is not a string.
pub fn string_into<W: Write>(raw: &str, out: &mut W) -> Result<(), Invalid> {
    let inner = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .ok_or(Invalid)?;
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' {
            match chars.next().ok_or(Invalid)? {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'u' => {
                    let hex: heapless::String<4> = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).map_err(|_| Invalid)?;
                    char::from_u32(code).unwrap_or('\u{fffd}')
                }
                _ => return Err(Invalid),
            }
        } else {
            c
        };
        out.write_char(c).map_err(|_| Invalid)?;
    }
    Ok(())
}

/// A number, `None` if `raw` is not one.
pub fn number(raw: &str) -> Option<f64> {
    raw.parse().ok()
}

/// A boolean: `true`, `false`, or a number (non-zero being true).
pub fn boolean(raw: &str) -> Option<bool> {
    match raw {
        "true" => Some(true),
        "false" => Some(false),
        _ => number(raw).map(|n| n != 0.0),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::{string::String, vec::Vec};

    use super::*;

    #[test]
    fn object_members() {
        let body = r#" { "ssid": "my \"net\"", "frequency": 525.5, "autofanspeed": 1,
            "pools": [{"id": 0, "stratumURL": "a"}, {"id": 1}], "x": null } "#;
        let m: Vec<_> = members(body).map(|r| r.unwrap()).collect();
        assert_eq!(m.len(), 5);
        assert!(key_is(m[0].0, "ssid"));
        let mut ssid = String::new();
        string_into(m[0].1, &mut ssid).unwrap();
        assert_eq!(ssid, "my \"net\"");
        assert_eq!(number(m[1].1), Some(525.5));
        assert_eq!(boolean(m[2].1), Some(true));
        let pools: Vec<_> = elements(m[3].1).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(pools, [r#"{"id": 0, "stratumURL": "a"}"#, r#"{"id": 1}"#]);
        assert_eq!(m[4].1, "null");
    }

    #[test]
    fn errors() {
        assert!(members("not json").next().unwrap().is_err());
        assert!(members(r#"{"a" 1}"#).next().unwrap().is_err());
        assert!(members(r#"{"a": "unterminated}"#).next().unwrap().is_err());
        assert_eq!(members("{}").count(), 0);
        assert!(elements("{}").is_none());
        assert_eq!(boolean("\"yes\""), None);
        let mut s = String::new();
        assert!(string_into("1", &mut s).is_err());
        s.clear();
        string_into(r#""café""#, &mut s).unwrap();
        assert_eq!(s, "café");
    }
}
