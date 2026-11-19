//! Responses: one type for pages, fragments, JSON and files, and HTML escaping.

use alloc::string::String;
use core::fmt::{self, Display, Write as _};

use picoserve::{
    io::{Read, Write},
    response::{Connection, Content, IntoResponse, Response, ResponseWriter, StatusCode},
    ResponseSent,
};

/// Header values a reply can carry.
pub type HeaderValue = heapless::String<160>;

enum Data {
    Owned(String),
    Static(&'static [u8]),
}

/// A response: status, content type, body and extra headers.
pub struct Reply {
    status: StatusCode,
    content_type: &'static str,
    data: Data,
    headers: heapless::Vec<(&'static str, HeaderValue), 6>,
}

impl Reply {
    fn new(status: StatusCode, content_type: &'static str, data: Data) -> Self {
        Self {
            status,
            content_type,
            data,
            headers: heapless::Vec::new(),
        }
    }

    pub fn html(body: String) -> Self {
        Self::new(
            StatusCode::OK,
            "text/html; charset=utf-8",
            Data::Owned(body),
        )
    }

    /// A JSON body, readable from pages of other origins.
    pub fn json(body: String) -> Self {
        Self::new(StatusCode::OK, "application/json", Data::Owned(body)).cors()
    }

    pub fn text(status: StatusCode, body: &str) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            Data::Owned(body.into()),
        )
        .cors()
    }

    /// `{"message": ...}` with `status`.
    pub fn message(status: StatusCode, message: &str) -> Self {
        let mut body = String::new();
        let mut json = miner_core::json::JsonWriter::new(&mut body);
        json.begin_object()
            .field_str("message", message)
            .end_object();
        let _ = json.finish();
        Self::json(body).with_status(status)
    }

    /// A static file, gzipped when `gzipped`, cached by the browser.
    pub fn file(content_type: &'static str, data: &'static [u8], gzipped: bool) -> Self {
        let reply = Self::new(StatusCode::OK, content_type, Data::Static(data))
            .header("Cache-Control", format_args!("max-age=2592000"));
        if gzipped {
            reply.header("Content-Encoding", format_args!("gzip"))
        } else {
            reply
        }
    }

    pub fn redirect(location: &str) -> Self {
        Self::text(StatusCode::FOUND, "Redirect to the captive portal")
            .header("Location", format_args!("{location}"))
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Adds a header; dropped when there is no room left or the value is too long.
    pub fn header(mut self, name: &'static str, value: fmt::Arguments) -> Self {
        let mut v = HeaderValue::new();
        if v.write_fmt(value).is_ok() {
            let _ = self.headers.push((name, v));
        }
        self
    }

    /// Adds the CORS headers of the API.
    pub fn cors(self) -> Self {
        self.header("Access-Control-Allow-Origin", format_args!("*"))
            .header(
                "Access-Control-Allow-Methods",
                format_args!("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            )
            .header("Access-Control-Allow-Headers", format_args!("Content-Type"))
    }

    /// Makes htmx show a toast: `level` is success, error, warning or info.
    pub fn toast(self, level: &str, message: &str) -> Self {
        let mut detail = HeaderValue::new();
        let mut json = miner_core::json::JsonWriter::new(&mut detail);
        json.begin_object()
            .key("toast")
            .begin_object()
            .field_str("level", level)
            .field_str("message", message)
            .end_object()
            .end_object();
        match json.finish() {
            Ok(_) => self.header("HX-Trigger", format_args!("{detail}")),
            Err(_) => self,
        }
    }
}

struct Body {
    content_type: &'static str,
    data: Data,
}

impl Content for Body {
    fn content_type(&self) -> &'static str {
        self.content_type
    }

    fn content_length(&self) -> usize {
        match &self.data {
            Data::Owned(s) => s.len(),
            Data::Static(b) => b.len(),
        }
    }

    async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        match &self.data {
            Data::Owned(s) => writer.write_all(s.as_bytes()).await,
            Data::Static(b) => writer.write_all(b).await,
        }
    }
}

impl IntoResponse for Reply {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let Self {
            status,
            content_type,
            data,
            headers,
        } = self;
        let response =
            Response::new(status, Body { content_type, data }).with_headers(headers.as_slice());
        response_writer.write_response(connection, response).await
    }
}

/// Text escaped for HTML content and attribute values.
pub struct Esc<'a>(pub &'a str);

impl Display for Esc<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut start = 0;
        for (i, c) in self.0.char_indices() {
            let escape = match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&#39;",
                _ => continue,
            };
            f.write_str(&self.0[start..i])?;
            f.write_str(escape)?;
            start = i + c.len_utf8();
        }
        f.write_str(&self.0[start..])
    }
}
