//! Static files: htmx, the stylesheet and script of the pages, the logo and the font.

use super::http::Reply;

static HTMX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/htmx.min.js.gz"));
static STYLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/style.css.gz"));
static APP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.js.gz"));
static LOGO: &[u8] = include_bytes!("../../web/logo.svg");
static FONT: &[u8] = include_bytes!("../../web/Nippo-Regular.woff2");

pub async fn htmx() -> Reply {
    Reply::file("application/javascript", HTMX, true)
}

pub async fn style() -> Reply {
    Reply::file("text/css", STYLE, true)
}

pub async fn app() -> Reply {
    Reply::file("application/javascript", APP, true)
}

pub async fn logo() -> Reply {
    Reply::file("image/svg+xml", LOGO, false)
}

pub async fn font() -> Reply {
    Reply::file("font/woff2", FONT, false)
}
