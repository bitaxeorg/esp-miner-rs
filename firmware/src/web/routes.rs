//! The routes of the web interface, dispatched by a single `match`.
//!
//! A picoserve `Router` nests each route in the next, and polling a request walks down the
//! nesting: at release optimisation, each level builds the next one's future on the stack, which
//! took the first route about 190 KiB of stack. Here the handler of a request is the only one
//! whose future exists, in a box on the heap.

use alloc::boxed::Box;
use core::{future::Future, pin::Pin};

use picoserve::{
    extract::{Form, FromRequest, FromRequestParts, Query},
    io::Read,
    request::{Path, Request},
    response::{IntoResponse, ResponseWriter, StatusCode},
    routing::{PathRouterService, RequestHandlerService},
    ResponseSent,
};
use serde::de::DeserializeOwned;

use super::{api, assets, pages};

/// The future answering a request.
type Answer<'a, E> = Pin<Box<dyn Future<Output = Result<ResponseSent, E>> + 'a>>;

pub struct Routes;

impl PathRouterService for Routes {
    async fn call_path_router_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _current_path_parameters: (),
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        route(path.encoded(), request, response_writer).await
    }
}

/// The index in `path` between `prefix` and `suffix`.
fn index(path: &str, prefix: &str, suffix: &str) -> Option<usize> {
    path.strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .parse()
        .ok()
}

fn route<'a, R: Read + 'a, W: ResponseWriter<Error = R::Error> + 'a>(
    path: &'a str,
    request: Request<'a, R>,
    w: W,
) -> Answer<'a, W::Error> {
    let r = request;
    let pool_delete = index(path, "/pool/", "/delete");
    let pool = index(path, "/pool/", "");
    let api_pool = index(path, "/api/system/pools/", "");
    match (r.parts.method(), path) {
        // pages and their parts
        ("GET", "/") => Box::pin(reply(r, w, pages::home)),
        ("GET", "/scoreboard") => Box::pin(reply(r, w, pages::scoreboard)),
        ("GET", "/swarm") => Box::pin(reply(r, w, pages::swarm)),
        ("GET", "/logs") => Box::pin(reply(r, w, pages::logs)),
        ("GET", "/system") => Box::pin(reply(r, w, pages::system)),
        ("POST", "/system/identify") => Box::pin(reply(r, w, pages::system_identify)),
        ("GET", "/pool") => Box::pin(reply(r, w, pages::pool)),
        ("POST", "/pool") => Box::pin(form(r, w, pages::save_pools)),
        ("POST", "/pool/fallback") => Box::pin(form(r, w, pages::use_fallback)),
        ("GET", "/network") => Box::pin(reply(r, w, pages::network)),
        ("POST", "/network") => Box::pin(form(r, w, pages::save_network)),
        ("GET", "/design") => Box::pin(reply(r, w, pages::design)),
        ("POST", "/design") => Box::pin(form(r, w, pages::save_design)),
        ("GET", "/settings") => Box::pin(reply(r, w, pages::settings_page)),
        ("POST", "/settings") => Box::pin(form(r, w, pages::save_settings)),
        ("GET", "/update") => Box::pin(reply(r, w, pages::update)),
        ("POST", "/update/boot") => Box::pin(form(r, w, pages::boot)),
        ("GET", "/ap") => Box::pin(reply(r, w, pages::ap)),
        ("POST", "/ap") => Box::pin(form(r, w, pages::save_network)),
        ("GET", "/fragment/top") => Box::pin(reply(r, w, pages::top_live)),
        ("GET", "/fragment/home") => Box::pin(reply(r, w, pages::home_live)),
        ("GET", "/fragment/chart") => Box::pin(reply(r, w, pages::chart)),
        ("GET", "/fragment/scoreboard") => Box::pin(reply(r, w, pages::scoreboard_live)),
        ("GET", "/fragment/logs") => Box::pin(query(r, w, pages::logs_live)),
        ("GET", "/fragment/system") => Box::pin(reply(r, w, pages::system_live)),
        ("GET", "/fragment/wifi-scan") => Box::pin(reply(r, w, pages::wifi_scan)),
        // JSON API
        ("GET", "/api/system/info") => Box::pin(reply(r, w, api::info)),
        ("GET", "/api/system/asic") => Box::pin(reply(r, w, api::asic)),
        ("GET", "/api/system/statistics") => Box::pin(service(r, w, api::Statistics, ())),
        ("GET", "/api/system/scoreboard") => Box::pin(reply(r, w, api::scoreboard)),
        ("GET", "/api/system/logs") => Box::pin(reply(r, w, api::logs)),
        ("GET", "/api/system/wifi/scan") => Box::pin(reply(r, w, api::wifi_scan)),
        ("GET", "/api/system/firmware/checksum") => Box::pin(reply(r, w, api::checksum)),
        ("PATCH", "/api/system") => Box::pin(service(r, w, api::PatchSettings, ())),
        ("POST", "/api/system/restart") => Box::pin(reply(r, w, api::restart)),
        ("POST", "/api/system/pause") => Box::pin(reply(r, w, api::pause)),
        ("POST", "/api/system/resume") => Box::pin(reply(r, w, api::resume)),
        ("POST", "/api/system/identify") => Box::pin(reply(r, w, api::identify)),
        ("POST", "/api/system/boot") => Box::pin(service(r, w, api::Boot, ())),
        ("POST", "/api/system/blockFound/dismiss") => {
            Box::pin(reply(r, w, api::dismiss_block_found))
        }
        ("POST", "/api/system/OTA") => Box::pin(service(r, w, api::Ota, ())),
        ("POST", "/api/system/OTAWWW") => Box::pin(reply(r, w, api::ota_www)),
        ("GET", "/api/theme") => Box::pin(reply(r, w, api::theme)),
        ("POST", "/api/theme") => Box::pin(service(r, w, api::PostTheme, ())),
        // static files
        ("GET", "/htmx.min.js") => Box::pin(reply(r, w, assets::htmx)),
        ("GET", "/style.css") => Box::pin(reply(r, w, assets::style)),
        ("GET", "/app.js") => Box::pin(reply(r, w, assets::app)),
        ("GET", "/logo.svg") => Box::pin(reply(r, w, assets::logo)),
        ("GET", "/fonts/Nippo-Regular.woff2") => Box::pin(reply(r, w, assets::font)),
        // what operating systems ask to detect a captive portal
        (
            "GET",
            "/generate_204"
            | "/gen_204"
            | "/hotspot-detect.html"
            | "/library/test/success.html"
            | "/connecttest.txt"
            | "/ncsi.txt"
            | "/redirect"
            | "/canonical.html"
            | "/success.txt",
        ) => Box::pin(reply(r, w, pages::not_found)),
        // the pools by index
        ("POST", _) if pool_delete.is_some() => {
            let i = pool_delete.unwrap_or_default();
            Box::pin(reply(r, w, move || pages::delete_pool(i)))
        }
        ("POST", _) if pool.is_some() => {
            let i = pool.unwrap_or_default();
            Box::pin(form(r, w, move |f| pages::save_pool(i, f)))
        }
        ("PUT", _) if api_pool.is_some() => {
            let i = api_pool.unwrap_or_default();
            Box::pin(service(r, w, api::PutPool, (i,)))
        }
        ("DELETE", _) if api_pool.is_some() => {
            let i = api_pool.unwrap_or_default();
            Box::pin(reply(r, w, move || api::delete_pool(i)))
        }
        _ => Box::pin(not_found(r, w)),
    }
}

/// The answer of a handler without extractor.
async fn reply<
    R: Read,
    W: ResponseWriter<Error = R::Error>,
    T: IntoResponse,
    F: Future<Output = T>,
>(
    request: Request<'_, R>,
    w: W,
    handler: impl FnOnce() -> F,
) -> Result<ResponseSent, W::Error> {
    let answer = handler().await;
    answer
        .write_to(request.body_connection.finalize().await?, w)
        .await
}

/// The answer of a handler of a form.
async fn form<
    R: Read,
    W: ResponseWriter<Error = R::Error>,
    D: DeserializeOwned,
    T: IntoResponse,
    F: Future<Output = T>,
>(
    mut request: Request<'_, R>,
    w: W,
    handler: impl FnOnce(Form<D>) -> F,
) -> Result<ResponseSent, W::Error> {
    match Form::<D>::from_request(&(), request.parts, request.body_connection.body()).await {
        Ok(f) => {
            let answer = handler(f).await;
            answer
                .write_to(request.body_connection.finalize().await?, w)
                .await
        }
        Err(rejection) => {
            rejection
                .write_to(request.body_connection.finalize().await?, w)
                .await
        }
    }
}

/// The answer of a handler of a query.
async fn query<
    R: Read,
    W: ResponseWriter<Error = R::Error>,
    D: DeserializeOwned,
    T: IntoResponse,
    F: Future<Output = T>,
>(
    request: Request<'_, R>,
    w: W,
    handler: impl FnOnce(Query<D>) -> F,
) -> Result<ResponseSent, W::Error> {
    match Query::<D>::from_request_parts(&(), &request.parts).await {
        Ok(q) => {
            let answer = handler(q).await;
            answer
                .write_to(request.body_connection.finalize().await?, w)
                .await
        }
        Err(rejection) => {
            rejection
                .write_to(request.body_connection.finalize().await?, w)
                .await
        }
    }
}

async fn service<
    R: Read,
    W: ResponseWriter<Error = R::Error>,
    P,
    S: RequestHandlerService<(), P>,
>(
    request: Request<'_, R>,
    w: W,
    service: S,
    path_parameters: P,
) -> Result<ResponseSent, W::Error> {
    service
        .call_request_handler_service(&(), path_parameters, request, w)
        .await
}

async fn not_found<R: Read, W: ResponseWriter<Error = R::Error>>(
    request: Request<'_, R>,
    w: W,
) -> Result<ResponseSent, W::Error> {
    (
        StatusCode::NOT_FOUND,
        format_args!("{} not found\r\n", request.parts.path()),
    )
        .write_to(request.body_connection.finalize().await?, w)
        .await
}
