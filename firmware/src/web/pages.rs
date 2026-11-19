//! The pages of the web interface, rendered on the device and refreshed with htmx.

use alloc::string::String;
use core::fmt::Write;

use embassy_time::Instant;
use miner_core::{
    json::JsonWriter,
    pool_config::{host_and_port, PoolConfig},
    stats::Column,
    units::{date_ago, diff_suffix, hash_suffix},
};
use picoserve::{
    extract::{Form, Query},
    response::StatusCode,
};
use serde::Deserialize;

use super::{
    api,
    http::{Esc, Reply},
    patch::{DISPLAYS, HIDDEN_PASSWORD},
};
use crate::{
    board, net, ota,
    settings::{self, Settings, MAX_POOLS},
    stats,
    status::{self, Status},
};

type Text<const N: usize> = heapless::String<N>;

/// Address paying the factory default pool user.
const FACTORY_ADDRESS: &str = "bc1qnp980s5fpp8l94p5cvttmtdqy8rvrq74qly2yrfmzkdsntqzlc5qkc4rkq";
const RESTART_NEEDED: &str =
    "You must restart this device after saving for changes to take effect.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Scoreboard,
    Swarm,
    Logs,
    System,
    Pool,
    Network,
    Theme,
    Settings,
    Update,
    Ap,
}

/// Icons: 24 × 24 strokes.
mod icon {
    pub const HOME: &str =
        "<path d='M3 11l9-8 9 8'/><path d='M5 10v10h14V10'/><path d='M10 20v-6h4v6'/>";
    pub const TROPHY: &str = "<path d='M7 4h10v5a5 5 0 0 1-10 0z'/><path d='M7 6H4a3 3 0 0 0 3 4M17 6h3a3 3 0 0 1-3 4'/><path d='M12 14v4M8 21h8M9 18h6'/>";
    pub const SITEMAP: &str = "<rect x='9' y='3' width='6' height='5'/><rect x='3' y='16' width='6' height='5'/><rect x='15' y='16' width='6' height='5'/><path d='M12 8v4M6 16v-4h12v4'/>";
    pub const LIST: &str = "<path d='M9 6h12M9 12h12M9 18h12'/><circle cx='4' cy='6' r='1'/><circle cx='4' cy='12' r='1'/><circle cx='4' cy='18' r='1'/>";
    pub const PULSE: &str = "<path d='M2 12h4l3-8 4 16 3-8h6'/>";
    pub const SERVER: &str = "<rect x='3' y='4' width='18' height='7' rx='1'/><rect x='3' y='13' width='18' height='7' rx='1'/><path d='M7 7.5h.01M7 16.5h.01'/>";
    pub const WIFI: &str = "<path d='M2 9a15 15 0 0 1 20 0'/><path d='M5 12.5a10 10 0 0 1 14 0'/><path d='M8.5 16a5 5 0 0 1 7 0'/><circle cx='12' cy='19.5' r='.8'/>";
    pub const PALETTE: &str = "<path d='M12 3a9 9 0 1 0 0 18c1.5 0 2-1 2-2s-1-1.5-1-2.5S14 15 15 15h2a4 4 0 0 0 4-4c0-4.4-4-8-9-8z'/><circle cx='7.5' cy='11' r='1'/><circle cx='10' cy='7' r='1'/><circle cx='15' cy='7.5' r='1'/>";
    pub const COG: &str = "<circle cx='12' cy='12' r='3'/><path d='M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1'/>";
    pub const SYNC: &str =
        "<path d='M20 7a9 9 0 0 0-15.5 1M4 17a9 9 0 0 0 15.5-1'/><path d='M20 3v4h-4M4 21v-4h4'/>";
    pub const BITCOIN: &str = "<circle cx='12' cy='12' r='10'/><path d='M9 7h4a2.5 2.5 0 0 1 0 5H9zM9 12h5a2.5 2.5 0 0 1 0 5H9zM9 7v10M11 5v2M11 17v2'/>";
    pub const BARS: &str = "<path d='M3 6h18M3 12h18M3 18h18'/>";
    pub const PAUSE: &str = "<circle cx='12' cy='12' r='10'/><path d='M10 8v8M14 8v8'/>";
    pub const PLAY: &str = "<circle cx='12' cy='12' r='10'/><path d='M10 8l6 4-6 4z'/>";
    pub const REFRESH: &str = "<path d='M21 12a9 9 0 1 1-3-6.7L21 8'/><path d='M21 3v5h-5'/>";
    pub const TRASH: &str = "<path d='M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3'/>";
    pub const SEARCH: &str = "<circle cx='11' cy='11' r='7'/><path d='M21 21l-5-5'/>";
    pub const X: &str = "<path d='M6 6l12 12M18 6L6 18'/>";
}

fn svg(out: &mut String, paths: &str) {
    let _ = write!(out, "<svg class='icon' viewBox='0 0 24 24'>{paths}</svg>");
}

/// The menu: label, path, icon, page; `None` for a separator.
const MENU: [Option<(&str, &str, &str, Page)>; 12] = [
    Some(("Dashboard", "/", icon::HOME, Page::Home)),
    Some(("Scoreboard", "/scoreboard", icon::TROPHY, Page::Scoreboard)),
    Some(("Swarm", "/swarm", icon::SITEMAP, Page::Swarm)),
    Some(("Logs", "/logs", icon::LIST, Page::Logs)),
    Some(("System", "/system", icon::PULSE, Page::System)),
    None,
    Some(("Pool", "/pool", icon::SERVER, Page::Pool)),
    Some(("Network", "/network", icon::WIFI, Page::Network)),
    Some(("Theme", "/design", icon::PALETTE, Page::Theme)),
    Some(("Settings", "/settings", icon::COG, Page::Settings)),
    Some(("Update", "/update", icon::SYNC, Page::Update)),
    None,
];

fn theme_class(scheme: &str) -> &'static str {
    match scheme {
        "light" => "theme-light dark-mode",
        "white" => "theme-white",
        _ => "theme-dark dark-mode",
    }
}

/// A whole page: head, top bar, menu, then `content`.
fn layout(page: Page, title: &str, content: impl FnOnce(&mut String)) -> Reply {
    let (hostname, scheme, color) = settings::with(|s| {
        (
            s.hostname.clone(),
            s.theme_scheme.clone(),
            s.theme_color.clone(),
        )
    });
    let ap = page == Page::Ap;
    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        "<!doctype html><html lang='en' class='{}'><head><meta charset='utf-8'>\
         <meta name='viewport' content='width=device-width, initial-scale=1'>\
         <title>{}</title><link rel='stylesheet' href='/style.css'>\
         <style>:root{{--color-primary:{}}}</style>\
         <script src='/htmx.min.js' defer></script><script src='/app.js' defer></script>\
         </head><body{}><div class='layout-topbar'><a href='/' class='logo' aria-label='AxeOS'></a>",
        theme_class(&scheme),
        Esc(title),
        Esc(&color),
        if ap { " class='ap-mode'" } else { "" }
    );
    if !ap {
        out.push_str(
            "<button class='icon-button primary' onclick='toggleMenu()' aria-label='Menu'>",
        );
        svg(&mut out, icon::BARS);
        out.push_str("</button>");
    }
    let _ = write!(
        out,
        "<span class='hostname' title='Hostname'>{}</span>",
        Esc(&hostname)
    );
    if !ap {
        out.push_str(
            "<div class='topbar-right' hx-get='/fragment/top' hx-trigger='every 5s' hx-swap='innerHTML'>",
        );
        top_icons(&mut out);
        out.push_str("</div>");
    }
    out.push_str("</div>");
    if !ap {
        out.push_str("<nav class='layout-sidebar'>");
        for item in MENU {
            match item {
                Some((label, href, paths, p)) => {
                    let _ = write!(
                        out,
                        "<a href='{href}'{}>",
                        if p == page { " class='active'" } else { "" }
                    );
                    svg(&mut out, paths);
                    let _ = write!(out, "{label}</a>");
                }
                None => out.push_str("<hr>"),
            }
        }
        out.push_str("<a href='https://bitcoin.org/bitcoin.pdf' target='_blank'>");
        svg(&mut out, icon::BITCOIN);
        out.push_str("Whitepaper</a></nav>");
    }
    out.push_str("<main class='layout-main'>");
    content(&mut out);
    out.push_str("</main><div id='toasts'></div></body></html>");
    Reply::html(out)
}

/// The icons of the top bar: Wi-Fi strength, pause or resume, restart.
fn top_icons(out: &mut String) {
    let paused = status::with(|s| s.mining_paused);
    let rssi = net::wifi_rssi();
    let bars = match rssi {
        Some(r) if r > -50 => 4,
        Some(r) if r > -60 => 3,
        Some(r) if r > -70 => 2,
        Some(_) => 1,
        None => 0,
    };
    let _ = write!(
        out,
        "<a class='icon-button' href='/network' title='Wi-Fi RSSI: {} dBm' style='opacity:{}'>",
        rssi.unwrap_or(0),
        if bars == 0 { "0.4" } else { "1" }
    );
    svg(out, icon::WIFI);
    let _ = write!(out, "<small>{bars}</small></a>");
    if paused {
        out.push_str("<button class='icon-button' title='Resume Mining' hx-post='/api/system/resume' hx-swap='none'>");
        svg(out, icon::PLAY);
    } else {
        out.push_str("<button class='icon-button' title='Pause Mining' hx-post='/api/system/pause' hx-swap='none'>");
        svg(out, icon::PAUSE);
    }
    out.push_str("</button><button class='icon-button' title='Restart' hx-post='/api/system/restart' hx-swap='none' hx-confirm='Are you sure you want to restart the device?'>");
    svg(out, icon::REFRESH);
    out.push_str("</button>");
}

pub async fn top_live() -> Reply {
    let mut out = String::new();
    top_icons(&mut out);
    Reply::html(out)
}

fn efficiency(s: &Status, hashrate: f32) -> Option<f32> {
    let power = s.power_w?;
    (hashrate > 0.0 && power > 0.0).then(|| power / (hashrate / 1000.0))
}

fn bar(out: &mut String, fraction: f32) {
    let pct = (fraction * 100.0).clamp(0.0, 100.0);
    let _ = write!(
        out,
        "<div class='bar'><span style='width:{pct:.1}%'></span></div>"
    );
}

fn bar_with_marker(out: &mut String, fraction: f32, marker: f32, label: &str) {
    let pct = (fraction * 100.0).clamp(0.0, 100.0);
    let m = (marker * 100.0).clamp(0.0, 100.0);
    let _ = write!(
        out,
        "<div class='bar'><span style='width:{pct:.1}%'></span><i class='marker' style='left:{m:.1}%'></i></div>\
         <div class='marker-label'>{}</div>",
        Esc(label)
    );
}

pub async fn home() -> Reply {
    layout(Page::Home, "AxeOS", |out| {
        out.push_str(
            "<div hx-get='/fragment/home' hx-trigger='load, every 2s' hx-swap='innerHTML'></div>\
             <div class='card chart' hx-get='/fragment/chart' hx-trigger='load, every 10s' hx-swap='innerHTML'></div>",
        );
    })
}

/// The dashboard cards, refreshed every 2 s.
pub async fn home_live() -> Reply {
    let s = status::get();
    let set = settings::get();
    let mut out = String::with_capacity(8192);
    let temps = [s.chip_temps_c[0], s.chip_temps_c[1]];
    let temp1 = if temps[0].is_nan() { 0.0 } else { temps[0] };
    let temp2 = if temps[1].is_nan() { 0.0 } else { temps[1] };
    let power = s.power_w.unwrap_or(0.0);
    let best_ever = s.best_difficulty_ever.max(set.best_diff as f64);

    // the browser title
    let mut title = Text::<128>::new();
    let _ = write!(
        title,
        "AxeOS • {} • {} • {temp1:.1}",
        set.hostname,
        hash_suffix(s.hashrate_ghs.into())
    );
    if temp2 > 0.0 {
        let _ = write!(title, "/{temp2:.1}");
    }
    let _ = write!(title, " °C • {power:.1} W • {}", diff_suffix(best_ever));
    if s.show_new_block {
        title.clear();
        let _ = title.push_str("AxeOS • Block found 🎉");
    }
    let _ = write!(out, "<title>{}</title>", Esc(&title));

    // messages
    if s.mining_paused {
        out.push_str("<div class='message warn'>Mining is paused</div>");
    }
    if set.overheat_mode || s.overheat {
        out.push_str("<div class='message error'>Device has overheated - See settings</div>");
    }
    if board::FREQUENCY_OPTIONS
        .iter()
        .min()
        .is_some_and(|&min| set.frequency < f32::from(min))
    {
        out.push_str("<div class='message warn'>Device frequency is set low - See settings</div>");
    }
    if s.using_fallback {
        let primary = set
            .pools
            .get(usize::from(set.primary_pool_index))
            .and_then(|p| PoolConfig::parse(p));
        if let Some(p) = primary {
            let _ = write!(
                out,
                "<div class='message warn'>Primary pool ({}:{}): {} - mining on fallback pool.</div>",
                Esc(p.host()),
                p.effective_port(),
                Esc(if s.primary_pool_error.is_empty() { "Pool unreachable" } else { &s.primary_pool_error })
            );
        }
    }
    if s.show_new_block {
        let mut blocks = Text::<16>::new();
        if s.block_found == 1 {
            let _ = blocks.push_str("a block");
        } else {
            let _ = write!(blocks, "{} blocks", s.block_found);
        }
        let _ = write!(
            out,
            "<div class='message success block-found'><span>Today is your lucky day! Your device has found {} 🎉</span>\
             <button class='icon-button' title='Dismiss' hx-post='/api/system/blockFound/dismiss' hx-swap='none'>",
            blocks
        );
        svg(&mut out, icon::X);
        out.push_str("</button></div>");
    }

    // hashrate, efficiency, shares, best difficulty
    out.push_str("<div class='grid grid-4'>");
    let _ = write!(
        out,
        "<div class='card'><div class='card-title'>Hashrate</div><div class='big'>{}</div>\
         <table class='rows'><tr><td title='Invalid nonces among those returned'>Error</td><td>{:.2} %</td></tr>",
        hash_suffix(s.hashrate_ghs.into()),
        s.error_percentage
    );
    if s.expected_hashrate_ghs > 0.0 {
        let _ = write!(
            out,
            "<tr><td>Expected</td><td>{}</td></tr>",
            hash_suffix(s.expected_hashrate_ghs.into())
        );
    }
    let _ = write!(
        out,
        "</table><div class='averages'><div><div class='muted'>1m</div><div class='primary-text'>{}</div></div>\
         <div><div class='muted'>10m</div><div class='primary-text'>{}</div></div>\
         <div><div class='muted'>1h</div><div class='primary-text'>{}</div></div></div></div>",
        hash_suffix(s.hashrate_1m.into()),
        hash_suffix(s.hashrate_10m.into()),
        hash_suffix(s.hashrate_1h.into())
    );
    let fmt_eff = |v: Option<f32>| {
        let mut t = Text::<16>::new();
        let _ = write!(t, "{:.2}", v.unwrap_or(0.0));
        t
    };
    let _ = write!(
        out,
        "<div class='card'><div class='card-title'>Efficiency</div><div class='big'>{} <small class='muted'>J/Th</small></div>\
         <table class='rows'><tr><td>Expected</td><td>{} J/Th</td></tr><tr><td>Average</td><td>{} J/Th</td></tr></table></div>",
        fmt_eff(efficiency(&s, s.hashrate_ghs)),
        fmt_eff(efficiency(&s, s.expected_hashrate_ghs)),
        fmt_eff(efficiency(&s, s.hashrate_1m))
    );
    let pending = s.submitted.saturating_sub(s.accepted + s.rejected);
    let _ = write!(
        out,
        "<div class='card'><div class='card-title'>Shares</div><div class='big'>{}</div>",
        s.accepted
    );
    if pending > 0 {
        let _ = write!(out, "<div class='yellow'>{pending} pending</div>");
    }
    let _ = write!(
        out,
        "<div><span class='red'>{}</span> rejected</div></div>",
        s.rejected
    );
    let _ = write!(
        out,
        "<div class='card'><div class='card-title'>Best Difficulty</div>\
         <div class='big' title='{}'>{} <small class='muted'>all-time best</small></div>\
         <div title='{}'>{} <span class='muted'>session best</span></div></div></div>",
        best_ever as u64,
        diff_suffix(best_ever),
        s.best_difficulty as u64,
        diff_suffix(s.best_difficulty)
    );

    // power, heat, fan
    out.push_str("<div class='grid grid-3'><div class='card'><div class='card-title'>Power</div>");
    let nominal = f32::from(board::NOMINAL_VOLTAGE_V);
    let vin = s.vin_v.unwrap_or(0.0);
    let _ = write!(out, "<div class='meter'><div class='meter-head'><span>Power</span><span>{power:.1} W</span></div>");
    bar(&mut out, power / f32::from(board::MAX_POWER_W));
    let _ = write!(out, "</div><div class='meter'><div class='meter-head'><span>Input Voltage</span><span>{vin:.1} V</span></div>");
    let mut marker = Text::<16>::new();
    let _ = write!(marker, "{nominal} V");
    bar_with_marker(
        &mut out,
        vin / (nominal + 0.5),
        nominal / (nominal + 0.5),
        &marker,
    );
    if vin > 0.0 && vin < nominal * 0.949 {
        out.push_str("<div class='danger-text'>Danger: Low Voltage</div>");
    }
    let max_freq = s.frequency_mhz.max(800.0);
    let _ = write!(
        out,
        "</div><div class='meter'><div class='meter-head'><span>ASIC Frequency</span><span>{} MHz</span></div>",
        s.frequency_mhz
    );
    bar(&mut out, s.frequency_mhz / max_freq);
    let vcore = s.vcore_v.unwrap_or(0.0);
    let _ = write!(
        out,
        "</div><div class='meter'><div class='meter-head'><span>Measured ASIC Voltage</span><span>{vcore:.2} V</span></div>"
    );
    bar(&mut out, vcore / 1.8);
    out.push_str("</div></div><div class='card'><div class='card-title'>Heat</div>");
    let max_temp = 75f32.max(temp1).max(temp2).max(f32::from(set.temp_target));
    let label1 = if temp2 > 0.0 {
        "ASIC Temperature 1"
    } else {
        "ASIC Temperature"
    };
    let temp_text = |t: f32| {
        let mut x = Text::<16>::new();
        if t > 0.0 {
            let _ = write!(x, "{t:.1} °C");
        } else {
            let _ = x.push_str("--");
        }
        x
    };
    let _ = write!(
        out,
        "<div class='meter'><div class='meter-head'><span>{label1}</span><span>{}</span></div>",
        temp_text(temp1)
    );
    let mut target = Text::<16>::new();
    let _ = write!(target, "{} °C", set.temp_target);
    if set.auto_fan_speed && temp2 <= 0.0 {
        bar_with_marker(
            &mut out,
            temp1 / max_temp,
            f32::from(set.temp_target) / max_temp,
            &target,
        );
    } else {
        bar(&mut out, temp1 / max_temp);
    }
    if temp1 >= 70.0 {
        out.push_str("<div class='danger-text'>Danger: High Temperature</div>");
    }
    out.push_str("</div>");
    if temp2 > 0.0 {
        let _ = write!(out, "<div class='meter'><div class='meter-head'><span>ASIC Temperature 2</span><span>{}</span></div>", temp_text(temp2));
        if set.auto_fan_speed {
            bar_with_marker(
                &mut out,
                temp2 / max_temp,
                f32::from(set.temp_target) / max_temp,
                &target,
            );
        } else {
            bar(&mut out, temp2 / max_temp);
        }
        if temp2 >= 70.0 {
            out.push_str("<div class='danger-text'>Danger: High Temperature</div>");
        }
        out.push_str("</div>");
    }
    if let Some(vr) = s.vr_temp_c.filter(|t| *t > 0.0) {
        let _ = write!(out, "<div class='meter'><div class='meter-head'><span>Voltage Regulator Temperature</span><span>{vr:.1} °C</span></div>");
        bar(&mut out, vr / 120.0);
        if vr >= 105.0 {
            out.push_str("<div class='danger-text'>Danger: High Temperature</div>");
        }
        out.push_str("</div>");
    }
    let _ = write!(
        out,
        "</div><div class='card'><div class='card-title'>Fan Speed</div>\
         <div class='meter' title='{} RPM'><div class='meter-head'><span>Fan Speed</span><span>{:.1} %</span></div>",
        s.fan_rpm.unwrap_or(0),
        s.fan_percent
    );
    bar(&mut out, s.fan_percent / 100.0);
    let _ = write!(
        out,
        "</div><table class='rows'><tr><td>RPM</td><td>{}</td></tr></table></div></div>",
        s.fan_rpm.unwrap_or(0)
    );

    // pool, block header, misc
    out.push_str("<div class='grid grid-3'>");
    if !s.pool_user.is_empty() {
        let _ = write!(
            out,
            "<div class='card'><div class='card-title'>Pool</div>\
             <form hx-post='/pool/fallback' hx-trigger='change' hx-swap='none'><select name='useFallbackStratum'>\
             <option value='0'{}>Primary</option><option value='1'{}>Fallback</option></select></form>\
             <table class='rows'><tr><td>URL</td><td title='{}'>{}:{}</td></tr>\
             <tr><td>User</td><td class='mono'>{}</td></tr>",
            if set.use_fallback_stratum { "" } else { " selected" },
            if set.use_fallback_stratum { " selected" } else { "" },
            if s.pool_connected { "Connected" } else { "Not Connected" },
            Esc(&s.pool_url),
            s.pool_port,
            Esc(&s.pool_user)
        );
        match s.response_time_ms {
            Some(ms) => {
                let _ = write!(out, "<tr><td title='Time it takes for the pool to respond to a share'>Time</td><td>{ms:.1} ms</td></tr>");
            }
            None => out.push_str("<tr><td title='Time it takes for the pool to respond to a share'>Time</td><td>--</td></tr>"),
        }
        if s.pool_difficulty > 0.0 {
            let _ = write!(
                out,
                "<tr><td>Difficulty</td><td>{}</td></tr>",
                s.pool_difficulty as u64
            );
        } else {
            out.push_str("<tr><td>Difficulty</td><td>--</td></tr>");
        }
        out.push_str("<tr><td>Mode</td><td>SV1</td></tr></table></div>");
    }
    out.push_str(
        "<div class='card'><div class='card-title'>Block Header</div><table class='rows'>",
    );
    match &s.coinbase {
        Some(c) => {
            let _ = write!(out, "<tr><td>Height</td><td>{}</td></tr>", c.block_height);
        }
        None => out.push_str("<tr><td>Height</td><td>--</td></tr>"),
    }
    let _ = write!(
        out,
        "<tr><td>Difficulty</td><td title='{}'>{}</td></tr>",
        s.network_difficulty as u64,
        diff_suffix(s.network_difficulty)
    );
    let _ = write!(
        out,
        "<tr><td>Scriptsig</td><td class='mono'>{}</td></tr></table></div>",
        Esc(s.coinbase.as_ref().map_or("--", |c| c.scriptsig.as_str()))
    );
    let uptime = Instant::now().as_secs();
    let _ = write!(
        out,
        "<div class='card'><div class='card-title'>Misc</div><table class='rows'>\
         <tr><td>Uptime</td><td>{}</td></tr><tr><td>Total Uptime</td><td>{}</td></tr>",
        date_ago(uptime, 2, false),
        date_ago(uptime, 2, false)
    );
    if s.total_hashes > 0.0 {
        let log2 = log2(s.total_hashes);
        let _ = write!(
            out,
            "<tr><td>Hashes</td><td title='{:.0} total hashes'>2<sup>{log2:.6}</sup></td></tr>",
            s.total_hashes
        );
    }
    out.push_str("</table></div></div>");
    Reply::html(out)
}

fn log2(x: f64) -> f64 {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let t = (m - 1.0) / (m + 1.0);
    let (mut term, mut sum, mut k) = (t, 0.0, 1.0);
    for _ in 0..12 {
        sum += term / k;
        term *= t * t;
        k += 2.0;
    }
    exponent as f64 + 2.0 * sum / core::f64::consts::LN_2
}

/// The hashrate and temperature chart, from the statistics.
pub async fn chart() -> Reply {
    const W: f32 = 1000.0;
    const H: f32 = 300.0;
    const LEFT: f32 = 60.0;
    const RIGHT: f32 = 50.0;
    const TOP: f32 = 10.0;
    const BOTTOM: f32 = 25.0;
    let mut hashrate: heapless::Vec<(u32, f32), { stats::STATS_LIMIT }> = heapless::Vec::new();
    let mut temps: heapless::Vec<(u32, f32), { stats::STATS_LIMIT }> = heapless::Vec::new();
    stats::with_ring(|ring| {
        for (t, v) in ring.series(Column::Hashrate) {
            if let Some(v) = v {
                let _ = hashrate.push((t, v));
            }
        }
        for (t, v) in ring.series(Column::AsicTemp) {
            if let Some(v) = v {
                let _ = temps.push((t, v));
            }
        }
    });
    let mut out = String::with_capacity(12_000);
    out.push_str("<div class='card-title'>Chart</div>");
    if hashrate.len() < 2 {
        out.push_str("<div class='muted'>Collecting data…</div>");
        return Reply::html(out);
    }
    let t0 = hashrate[0].0;
    let t1 = hashrate[hashrate.len() - 1].0.max(t0 + 1);
    let expected = status::with(|s| s.expected_hashrate_ghs);
    let max_h = hashrate
        .iter()
        .map(|p| p.1)
        .fold(expected, f32::max)
        .max(1.0)
        * 1.1;
    let max_t = temps.iter().map(|p| p.1).fold(75.0, f32::max);
    let x = |t: u32| LEFT + (t - t0) as f32 / (t1 - t0) as f32 * (W - LEFT - RIGHT);
    let y = |v: f32, max: f32| TOP + (1.0 - v / max) * (H - TOP - BOTTOM);
    let _ = write!(
        out,
        "<svg viewBox='0 0 {W} {H}' preserveAspectRatio='none'>"
    );
    for i in 0..=4 {
        let gy = TOP + i as f32 / 4.0 * (H - TOP - BOTTOM);
        let _ = write!(
            out,
            "<line class='grid-line' x1='{LEFT}' x2='{}' y1='{gy:.1}' y2='{gy:.1}'/>\
             <text class='axis-text' x='{}' y='{:.1}' text-anchor='end'>{}</text>\
             <text class='axis-text' x='{}' y='{:.1}'>{:.0}°C</text>",
            W - RIGHT,
            LEFT - 6.0,
            gy + 4.0,
            hash_suffix(f64::from(max_h * (1.0 - i as f32 / 4.0))),
            W - RIGHT + 6.0,
            gy + 4.0,
            max_t * (1.0 - i as f32 / 4.0)
        );
    }
    let span_s = (t1 - t0) / 1000;
    let _ = write!(
        out,
        "<text class='axis-text' x='{LEFT}' y='{}'>-{}</text><text class='axis-text' x='{}' y='{}' text-anchor='end'>now</text>",
        H - 6.0,
        date_ago(u64::from(span_s), 1, true),
        W - RIGHT,
        H - 6.0
    );
    // hashrate, filled, then temperature
    out.push_str("<path class='y1-fill' d='");
    let _ = write!(out, "M{:.1},{:.1}", x(hashrate[0].0), H - BOTTOM);
    for (t, v) in &hashrate {
        let _ = write!(out, "L{:.1},{:.1}", x(*t), y(*v, max_h));
    }
    let _ = write!(
        out,
        "L{:.1},{:.1}Z'/><path class='y1' d='",
        x(hashrate[hashrate.len() - 1].0),
        H - BOTTOM
    );
    for (i, (t, v)) in hashrate.iter().enumerate() {
        let _ = write!(
            out,
            "{}{:.1},{:.1}",
            if i == 0 { 'M' } else { 'L' },
            x(*t),
            y(*v, max_h)
        );
    }
    out.push_str("'/>");
    if temps.len() > 1 {
        out.push_str("<path class='y2' d='");
        for (i, (t, v)) in temps.iter().enumerate() {
            let tx = if *t < t0 { t0 } else { *t };
            let _ = write!(
                out,
                "{}{:.1},{:.1}",
                if i == 0 { 'M' } else { 'L' },
                x(tx),
                y(*v, max_t)
            );
        }
        out.push_str("'/>");
    }
    out.push_str(
        "</svg><div class='legend'><span>Hashrate</span><span class='y2'>ASIC Temp</span></div>",
    );
    Reply::html(out)
}

pub async fn scoreboard() -> Reply {
    layout(Page::Scoreboard, "AxeOS Scoreboard", |out| {
        out.push_str(
            "<div class='card'><h2>Scoreboard</h2>\
             <div hx-get='/fragment/scoreboard' hx-trigger='load, every 5s' hx-swap='innerHTML'></div></div>",
        );
    })
}

pub async fn scoreboard_live() -> Reply {
    let mut out = String::with_capacity(4096);
    out.push_str(
        "<table class='data'><tr><th>Rank</th><th>Difficulty</th><th>Job ID</th><th>ExtraNonce2</th>\
         <th>Time</th><th>Nonce</th><th>Version Bits</th></tr>",
    );
    stats::with_scoreboard(|board| {
        for (rank, e) in board.entries().iter().enumerate() {
            let medal = match rank {
                0 => "<span style='color:gold'>🏆</span>",
                1 => "<span style='color:silver'>🏆</span>",
                2 => "<span style='color:peru'>🏆</span>",
                _ => "",
            };
            if medal.is_empty() {
                let _ = write!(out, "<tr><td>{}</td>", rank + 1);
            } else {
                let _ = write!(out, "<tr><td>{medal}</td>");
            }
            let _ = write!(
                out,
                "<td title='{}'>{}</td><td class='mono'>{}</td><td class='mono'>{}</td>\
                 <td class='utc' data-ntime='{}'>{}</td><td class='mono'>{:08X}</td><td class='mono'>{:08X}</td></tr>",
                e.difficulty as u64,
                diff_suffix(e.difficulty),
                Esc(&e.job_id),
                Esc(&e.extranonce2),
                e.ntime,
                e.ntime,
                e.nonce,
                e.version_bits
            );
        }
    });
    out.push_str("</table><script>document.querySelectorAll('td.utc').forEach(td=>{const d=new Date(td.dataset.ntime*1000);td.textContent=d.toISOString().replace('T',' ').slice(0,19)})</script>");
    Reply::html(out)
}

pub async fn swarm() -> Reply {
    let ip = super::station_ip().map(|c| {
        let mut t = Text::<16>::new();
        let _ = write!(t, "{}", c.address());
        t
    });
    layout(Page::Swarm, "AxeOS Swarm", |out| {
        let _ = write!(
            out,
            "<div id='swarm' data-ip='{}'><div class='grid grid-4'>\
             <div class='card'><div class='muted'>Total Devices</div><div class='big' id='swarm-total-devices'>0</div></div>\
             <div class='card'><div class='muted'>Total Hashrate</div><div class='big' id='swarm-total-hashrate'>0 H/s</div></div>\
             <div class='card'><div class='muted'>Total Power</div><div class='big' id='swarm-total-power'>0 W</div></div>\
             <div class='card'><div class='muted'>Best Diff</div><div class='big' id='swarm-best'>0</div></div></div>\
             <div class='card'><div class='field-row'>\
             <input class='input-text' id='swarm-address' placeholder='Add Device by Address'>\
             <button class='btn' onclick='swarmAdd()'>Add</button>\
             <button class='btn btn-secondary' onclick='swarmScan(this)'>Auto Scan</button>\
             <button class='btn btn-secondary' onclick='swarmRefresh()'>Refresh</button>\
             <input class='input-text' id='swarm-filter' placeholder='Filter' oninput='swarmRefresh()'></div></div>\
             <div class='card' style='overflow-x:auto'><table class='data'><tr><th>Device</th><th>Hashrate</th><th>Shares</th>\
             <th>Best Diff</th><th>Uptime</th><th>Power</th><th>Temp</th><th>Pool Diff</th><th>ASIC</th><th></th></tr>\
             <tbody id='swarm-rows'></tbody></table></div></div>",
            Esc(ip.as_deref().unwrap_or(""))
        );
    })
}

pub async fn logs() -> Reply {
    layout(Page::Logs, "AxeOS Logs", |out| {
        out.push_str(
            "<div class='card'><h2>Realtime Logs</h2><div class='field-row' style='margin-bottom:1rem'>\
             <input class='input-text' id='log-filter' placeholder='Case-sensitive filter'>\
             <button class='btn btn-secondary' onclick='toggleLogScroll(this)'>Stop Scrolling</button>\
             <a class='btn btn-secondary' href='/api/system/logs' download='bitaxe-logs.txt'>Download Logs</a>\
             <button class='btn btn-secondary' onclick='clearLogs()'>Clear Logs</button></div>\
             <div class='logs' id='logs' hx-get='/fragment/logs' hx-trigger='load, every 1s' \
             hx-vals='js:{after: lastLogId()}' hx-swap='beforeend'></div></div>",
        );
    })
}

#[derive(Deserialize)]
pub struct LogQuery {
    after: Option<u32>,
}

pub async fn logs_live(Query(q): Query<LogQuery>) -> Reply {
    let mut out = String::new();
    crate::weblog::for_each_after(q.after.unwrap_or(0), |n, line| {
        let level = line.chars().next().unwrap_or('I');
        let _ = write!(
            out,
            "<div data-id='{n}' class='{level}'>₿ {}</div>",
            Esc(line)
        );
    });
    Reply::html(out)
}

pub async fn system() -> Reply {
    layout(Page::System, "AxeOS System", |out| {
        out.push_str(
            "<div class='card'><h2>System</h2>\
             <div hx-get='/fragment/system' hx-trigger='load, every 5s' hx-swap='innerHTML'></div>\
             <div class='buttons'><button class='btn' hx-post='/system/identify' hx-swap='none'>Identify device</button></div></div>",
        );
    })
}

pub async fn system_live() -> Reply {
    let s = status::get();
    let set = settings::get();
    let partitions = ota::partitions().await;
    let uptime = Instant::now().as_secs();
    let rssi = net::wifi_rssi();
    let mut out = String::with_capacity(4096);
    let asic_type = {
        let mut t = Text::<32>::new();
        let count = s.asic_count.max(board::ASIC_COUNT);
        if count > 1 {
            let _ = write!(t, "{count}x ");
        }
        let _ = t.push_str(board::ASIC_MODEL);
        t
    };
    let _ = write!(
        out,
        "<table class='rows left'>\
         <tr><td>Device Model</td><td style='color:{}'>{}</td></tr>\
         <tr><td>Board Version</td><td>{}</td></tr>\
         <tr class='gap'><td>ASIC Type</td><td>{}</td></tr>\
         <tr><td>Uptime</td><td>{}</td></tr>\
         <tr><td>Total Uptime</td><td>{}</td></tr>",
        board::SWARM_COLOR,
        env!("BOARD_NAME"),
        env!("BOARD_VERSION"),
        asic_type,
        date_ago(uptime, 7, false),
        date_ago(uptime, 7, false)
    );
    if s.total_hashes > 0.0 {
        let _ = write!(
            out,
            "<tr><td>Total Log2 Work</td><td>{:.6}</td></tr><tr><td>Total Hashes</td><td>{:.0}</td></tr>",
            log2(s.total_hashes),
            s.total_hashes
        );
    }
    let ip = super::station_ip();
    let mut ip_text = Text::<16>::new();
    if let Some(c) = ip {
        let _ = write!(ip_text, "{}", c.address());
    }
    let (signal, colour) = match rssi {
        Some(r) if r > -50 => ("Excellent", "green"),
        Some(r) if r > -60 => ("Good", "blue"),
        Some(r) if r > -70 => ("Fair", "orange"),
        Some(_) => ("Weak", "red"),
        None => ("", ""),
    };
    let _ = write!(
        out,
        "<tr class='gap'><td>Reset Reason</td><td>unknown</td></tr>\
         <tr><td>Wi-Fi SSID</td><td>{}</td></tr>\
         <tr><td>Wi-Fi Status</td><td>{}</td></tr>\
         <tr><td>Wi-Fi RSSI</td><td>{} dBm <span class='{colour}'>{signal}</span></td></tr>\
         <tr><td>Wi-Fi IPv4</td><td>{}</td></tr>\
         <tr class='gap'><td>MAC Address</td><td>{}</td></tr>\
         <tr><td>Free Heap Memory</td><td>{:.1} kB</td></tr>\
         <tr class='gap'><td>• Internal</td><td>{:.1} kB</td></tr>\
         <tr><td>Firmware Version</td><td>{}</td></tr>\
         <tr><td>Running Partition</td><td>{}</td></tr></table>",
        Esc(&set.ssid),
        Esc(&s.wifi_status),
        rssi.unwrap_or(0),
        ip_text,
        super::mac_address(),
        stats::free_heap() as f32 / 1000.0,
        stats::free_heap() as f32 / 1000.0,
        api::VERSION,
        partitions
            .iter()
            .find(|p| p.current)
            .map_or("factory", |p| p.label.as_str())
    );
    Reply::html(out)
}

pub async fn system_identify() -> Reply {
    let message = if api::toggle_identify() {
        "The device says \"Hi!\" for 30 seconds."
    } else {
        "The device no longer says \"Hi!\"."
    };
    Reply::html(String::new()).toast("success", message)
}

// ---- forms

/// Applies a settings object built by `build`, and answers with a toast.
async fn save(
    build: impl FnOnce(&mut JsonWriter<&mut String>),
    saved: &str,
    restart: bool,
) -> Reply {
    let mut body = String::new();
    {
        let mut j = JsonWriter::new(&mut body);
        j.begin_object();
        build(&mut j);
        j.end_object();
        let _ = j.finish();
    }
    match api::apply_settings(&body).await {
        Ok(_) if restart => Reply::html(String::new()).toast("warning", RESTART_NEEDED),
        Ok(_) => Reply::html(String::new()).toast("success", saved),
        Err(()) => Reply::html(String::new())
            .with_status(StatusCode::BAD_REQUEST)
            .toast("error", "Invalid settings"),
    }
}

fn checked(on: bool) -> &'static str {
    if on {
        " checked"
    } else {
        ""
    }
}

fn selected(on: bool) -> &'static str {
    if on {
        " selected"
    } else {
        ""
    }
}

fn network_form(out: &mut String, set: &Settings, action: &str) {
    let _ = write!(
        out,
        "<div class='card'><h2>Network Configuration</h2><form hx-post='{action}' hx-swap='none'>\
         <div class='field'><label title='Name of the device on your local network'>Hostname</label>\
         <input class='input-text' name='hostname' required maxlength='32' value='{}'></div>\
         <div class='field'><label>Wi-Fi SSID</label><div class='field-row'>\
         <input class='input-text' id='ssid' name='ssid' required maxlength='32' value='{}'>\
         <button type='button' class='btn btn-secondary' title='Scan Wi-Fi' hx-get='/fragment/wifi-scan' \
         hx-target='#wifi-list' hx-indicator='#wifi-list'>",
        Esc(&set.hostname),
        Esc(&set.ssid)
    );
    svg(out, icon::SEARCH);
    let _ = write!(
        out,
        "</button></div><div id='wifi-list'></div></div>\
         <div class='field'><label>Wi-Fi Password</label>\
         <input class='input-text' type='password' name='wifiPass' maxlength='63' placeholder='Enter Wi-Fi password' value='{HIDDEN_PASSWORD}'></div>\
         <label class='checkbox' title='Your device will sync with a DHCP-provided NTP server, or the nearest if not provided'>\
         <input type='checkbox' name='useNTP' value='true'{}>Enable Time Sync</label>\
         <div class='buttons'><button class='btn' type='submit'>Save</button>\
         <button class='btn btn-secondary' type='button' hx-post='/api/system/restart' hx-swap='none' \
         hx-confirm='Are you sure you want to restart the device?'>Restart</button></div></form></div>",
        checked(set.use_ntp)
    );
}

pub async fn network() -> Reply {
    let set = settings::get();
    layout(Page::Network, "AxeOS Network", |out| {
        network_form(out, &set, "/network")
    })
}

pub async fn ap() -> Reply {
    let set = settings::get();
    layout(Page::Ap, "AxeOS Network", |out| {
        network_form(out, &set, "/ap")
    })
}

#[derive(Deserialize)]
pub struct NetworkForm {
    hostname: Text<40>,
    ssid: Text<32>,
    #[serde(rename = "wifiPass")]
    wifi_pass: Option<Text<64>>,
    #[serde(rename = "useNTP")]
    use_ntp: Option<bool>,
}

pub async fn save_network(Form(f): Form<NetworkForm>) -> Reply {
    let restart = settings::with(|s| {
        s.ssid != f.ssid.as_str()
            || f.wifi_pass
                .as_deref()
                .is_some_and(|p| p != HIDDEN_PASSWORD && p != s.wifi_pass)
            || s.use_ntp != f.use_ntp.unwrap_or(false)
    });
    save(
        |j| {
            j.field_str("hostname", &f.hostname)
                .field_str("ssid", &f.ssid);
            if let Some(p) = &f.wifi_pass {
                j.field_str("wifiPass", p);
            }
            j.field_bool("useNTP", f.use_ntp.unwrap_or(false));
        },
        "Saved network settings",
        restart,
    )
    .await
}

pub async fn wifi_scan() -> Reply {
    let mut out = String::new();
    match net::scan().await {
        Some(mut networks) => {
            networks.sort_unstable_by_key(|n| core::cmp::Reverse(n.rssi));
            let mut seen: heapless::Vec<Text<32>, { net::SCAN_MAX }> = heapless::Vec::new();
            out.push_str(
                "<div class='card' style='margin-top:.5rem'><h5>Select Wi-Fi Network</h5>",
            );
            for n in networks.iter().filter(|n| n.rssi >= -80) {
                if seen.contains(&n.ssid) {
                    continue;
                }
                let _ = seen.push(n.ssid.clone());
                let _ = write!(
                    out,
                    "<button type='button' class='btn btn-secondary' style='margin:.25rem' title='{0} ({1} dBm)' \
                     onclick=\"document.getElementById('ssid').value=this.dataset.ssid;this.closest('#wifi-list').innerHTML=''\" \
                     data-ssid='{0}'>{0} <small>{1} dBm</small></button>",
                    Esc(&n.ssid),
                    n.rssi
                );
            }
            out.push_str("</div>");
            Reply::html(out)
        }
        None => Reply::html(out).toast("error", "Failed to scan Wi-Fi networks"),
    }
}

pub async fn settings_page() -> Reply {
    let set = settings::get();
    layout(Page::Settings, "AxeOS Settings", |out| {
        out.push_str("<div class='card'><h2>Settings</h2>");
        if set.overclock_enabled {
            out.push_str("<div class='message warn'>Custom settings can cause damage &amp; system instability. Only modify these settings if you understand the risks of running outside designed parameters.</div>");
        }
        out.push_str("<form hx-post='/settings' hx-swap='none'><div class='grid grid-2'><div>");
        // frequency and core voltage
        let option = |out: &mut String, value: u16, current: f32, default: u16| {
            let _ = write!(
                out,
                "<option value='{value}'{}>{value}{}</option>",
                selected((current - f32::from(value)).abs() < 0.5),
                if value == default { " (Default)" } else { "" }
            );
        };
        if set.overclock_enabled {
            let _ = write!(
                out,
                "<div class='field'><label>Frequency</label><input class='input-text' type='number' name='frequency' value='{}'></div>\
                 <div class='field'><label>Core Voltage</label><input class='input-text' type='number' name='coreVoltage' value='{}'></div>",
                set.frequency, set.core_voltage
            );
        } else {
            out.push_str("<div class='field'><label>Frequency</label><select name='frequency'>");
            if !board::FREQUENCY_OPTIONS
                .iter()
                .any(|&f| (set.frequency - f32::from(f)).abs() < 0.5)
            {
                let _ = write!(
                    out,
                    "<option value='{0}' selected>{0} (Custom)</option>",
                    set.frequency
                );
            }
            for &f in board::FREQUENCY_OPTIONS {
                option(out, f, set.frequency, board::HASH_FREQ_MHZ as u16);
            }
            out.push_str("</select></div><div class='field'><label>Core Voltage</label><select name='coreVoltage'>");
            if !board::VOLTAGE_OPTIONS.contains(&set.core_voltage) {
                let _ = write!(
                    out,
                    "<option value='{0}' selected>{0} (Custom)</option>",
                    set.core_voltage
                );
            }
            let default_mv = (board::VCORE * 1000.0 + 0.5) as u16;
            for &v in board::VOLTAGE_OPTIONS {
                option(out, v, f32::from(set.core_voltage), default_mv);
            }
            out.push_str("</select></div>");
        }
        if set.overheat_mode {
            out.push_str(
                "<div class='field'><button class='btn btn-danger' type='submit' name='overheat_mode' value='0' style='width:100%'>Disable Overheat Mode</button>\
                 <div class='help'>Make sure to reset Frequency and Voltage before clicking this button.</div></div>",
            );
        }
        let _ = write!(
            out,
            "<label class='checkbox'><input type='checkbox' name='autofanspeed' value='true' id='autofan'{}\
             onchange=\"document.getElementById('fan-auto').classList.toggle('hidden',!this.checked);document.getElementById('fan-manual').classList.toggle('hidden',this.checked)\">\
             Automatic Fan Control</label>\
             <div id='fan-auto'{}><div class='field'><label>Target Temperature: <output>{}</output> °C</label>\
             <input type='range' name='temptarget' min='35' max='66' value='{2}' oninput='this.previousElementSibling.firstElementChild.value=this.value'></div>\
             <div class='field'><label>Minimum Fan Speed: <output>{}</output> %</label>\
             <input type='range' name='minFanSpeed' min='0' max='99' value='{3}' oninput='this.previousElementSibling.firstElementChild.value=this.value'></div></div>\
             <div id='fan-manual'{}><div class='field'><label>Fan Speed: <output>{}</output> %</label>\
             <input type='range' name='manualFanSpeed' min='0' max='100' value='{5}' oninput='this.previousElementSibling.firstElementChild.value=this.value'></div></div>",
            checked(set.auto_fan_speed),
            if set.auto_fan_speed { "" } else { " class='hidden'" },
            set.temp_target,
            set.min_fan_speed,
            if set.auto_fan_speed { " class='hidden'" } else { "" },
            set.manual_fan_speed
        );
        // display
        out.push_str("</div><div><h3>Display</h3><div class='field'><label>Type</label><select name='display'>");
        for d in DISPLAYS {
            let _ = write!(
                out,
                "<option{}>{}</option>",
                selected(set.display == d),
                Esc(d)
            );
        }
        out.push_str(
            "</select></div><div class='field'><label>Rotation</label><select name='rotation'>",
        );
        for r in [0u16, 90, 180, 270] {
            let _ = write!(
                out,
                "<option value='{r}'{}>{r}°</option>",
                selected(set.rotation == r)
            );
        }
        out.push_str(
            "</select></div><div class='field'><label>Sleep</label><select name='displayTimeout'>",
        );
        for m in [0i32, 1, 2, 5, 15, 30, 60, 120, 240, 480, -1] {
            let label: Text<32> = match m {
                0 => "Always Off".try_into().unwrap_or_default(),
                -1 => "Always On".try_into().unwrap_or_default(),
                m => {
                    let mut t = Text::new();
                    let _ = write!(t, "Timeout: {}", date_ago(m as u64 * 60, 1, false));
                    t
                }
            };
            let _ = write!(
                out,
                "<option value='{m}'{}>{label}</option>",
                selected(set.display_timeout == m)
            );
        }
        let _ = write!(
            out,
            "</select></div><label class='checkbox'><input type='checkbox' name='invertscreen' value='true'{}>Invert Display Colors</label>\
             <h3>Statistics</h3><div class='field'><label>Data Logging</label><select name='statsFrequency'>",
            checked(set.invert_screen)
        );
        for f in [0u16, 1, 2, 5, 10, 30, 60, 120, 360, 840, 1680, 3600] {
            let label: Text<64> = if f == 0 {
                "Disabled".try_into().unwrap_or_default()
            } else {
                let mut t = Text::new();
                let _ = write!(
                    t,
                    "Every {} for {}",
                    date_ago(u64::from(f), 1, false),
                    date_ago(u64::from(f) * stats::STATS_LIMIT as u64, 1, false)
                );
                t
            };
            let _ = write!(
                out,
                "<option value='{f}'{}>{label}</option>",
                selected(set.stats_frequency == f)
            );
        }
        out.push_str(
            "</select></div></div></div><div class='buttons'><button class='btn' type='submit'>Save</button>\
             <button class='btn btn-secondary' type='button' hx-post='/api/system/restart' hx-swap='none' \
             hx-confirm='Are you sure you want to restart the device?'>Restart</button></div></form></div>",
        );
    })
}

#[derive(Deserialize)]
pub struct SettingsForm {
    frequency: Option<f32>,
    #[serde(rename = "coreVoltage")]
    core_voltage: Option<u16>,
    autofanspeed: Option<bool>,
    temptarget: Option<u16>,
    #[serde(rename = "minFanSpeed")]
    min_fan_speed: Option<u16>,
    #[serde(rename = "manualFanSpeed")]
    manual_fan_speed: Option<u16>,
    display: Option<Text<32>>,
    rotation: Option<u16>,
    #[serde(rename = "displayTimeout")]
    display_timeout: Option<i32>,
    invertscreen: Option<bool>,
    #[serde(rename = "statsFrequency")]
    stats_frequency: Option<u16>,
    overheat_mode: Option<u8>,
}

pub async fn save_settings(Form(f): Form<SettingsForm>) -> Reply {
    let restart = settings::with(|s| {
        f.display.as_deref().is_some_and(|d| d != s.display)
            || f.rotation.is_some_and(|r| r != s.rotation)
            || s.invert_screen != f.invertscreen.unwrap_or(false)
    });
    save(
        |j| {
            if let Some(v) = f.frequency {
                j.field_float("frequency", v.into());
            }
            if let Some(v) = f.core_voltage {
                j.field_num("coreVoltage", v);
            }
            j.field_bool("autofanspeed", f.autofanspeed.unwrap_or(false));
            if let Some(v) = f.temptarget {
                j.field_num("temptarget", v);
            }
            if let Some(v) = f.min_fan_speed {
                j.field_num("minFanSpeed", v);
            }
            if let Some(v) = f.manual_fan_speed {
                j.field_num("manualFanSpeed", v);
            }
            if let Some(v) = &f.display {
                j.field_str("display", v);
            }
            if let Some(v) = f.rotation {
                j.field_num("rotation", v);
            }
            if let Some(v) = f.display_timeout {
                j.field_num("displayTimeout", v);
            }
            j.field_bool("invertscreen", f.invertscreen.unwrap_or(false));
            if let Some(v) = f.stats_frequency {
                j.field_num("statsFrequency", v);
            }
            if f.overheat_mode == Some(0) {
                j.field_num("overheat_mode", 0);
            }
        },
        "Saved settings",
        restart,
    )
    .await
}

pub async fn pool() -> Reply {
    let set = settings::get();
    layout(Page::Pool, "AxeOS Pool", |out| {
        let pools: heapless::Vec<Option<PoolConfig>, MAX_POOLS> =
            set.pools.iter().map(|p| PoolConfig::parse(p)).collect();
        if pools
            .iter()
            .flatten()
            .any(|p| p.user.contains(FACTORY_ADDRESS))
        {
            out.push_str("<div class='message warn'>Warning: You are using the default factory Bitcoin address. Please change the User field to your own Bitcoin address to receive mining rewards.</div>");
        }
        out.push_str(
            "<div class='card'><h2>Active Pool Selection</h2><form hx-post='/pool' hx-swap='none'><div class='grid grid-2'>",
        );
        for (label, name, current) in [
            ("Primary Pool", "primaryPoolIndex", set.primary_pool_index),
            (
                "Fallback Pool",
                "secondaryPoolIndex",
                set.secondary_pool_index,
            ),
        ] {
            let _ = write!(
                out,
                "<div class='field'><label>{label}</label><select name='{name}'>"
            );
            for (i, p) in pools.iter().enumerate() {
                match p {
                    Some(p) => {
                        let _ = write!(
                            out,
                            "<option value='{i}'{}>Pool {}: {} ({})</option>",
                            selected(usize::from(current) == i),
                            i + 1,
                            Esc(p.host()),
                            Esc(&p.protocol)
                        );
                    }
                    None => {
                        let _ = write!(
                            out,
                            "<option value='{i}'{}>Pool {}: (not configured)</option>",
                            selected(usize::from(current) == i),
                            i + 1
                        );
                    }
                }
            }
            out.push_str("</select></div>");
        }
        let _ = write!(
            out,
            "</div><label class='checkbox'><input type='checkbox' name='useFallbackStratum' value='true'{}>Mine on the fallback pool</label>\
             <button class='btn' type='submit'>Save</button></form></div>\
             <div class='card'><h2>Pool Configurations ({}/{MAX_POOLS})</h2>",
            checked(set.use_fallback_stratum),
            pools.iter().flatten().count()
        );
        let first_free = pools.iter().position(Option::is_none);
        for (i, p) in pools.iter().enumerate() {
            let shown = p.is_some() || Some(i) == first_free;
            if !shown {
                continue;
            }
            let primary = usize::from(set.primary_pool_index) == i;
            let fallback = usize::from(set.secondary_pool_index) == i;
            let _ = write!(
                out,
                "<details class='pool'><summary><strong>Pool {}</strong>",
                i + 1
            );
            match p {
                Some(p) => {
                    let _ = write!(out, "<span class='badge blue'>Stratum V1</span>");
                    if primary {
                        out.push_str("<span class='badge green'>Primary</span>");
                    }
                    if fallback {
                        out.push_str("<span class='badge amber'>Fallback</span>");
                    }
                    let _ = write!(
                        out,
                        "<span class='muted'>{}:{}</span>",
                        Esc(p.host()),
                        p.effective_port()
                    );
                }
                None => out.push_str("<span class='muted'>(not configured) — Add Pool</span>"),
            }
            out.push_str("</summary>");
            let (url, port, user, diff, xn) = p.as_ref().map_or(("", 3333, "", 0, false), |p| {
                (
                    p.host(),
                    p.effective_port(),
                    p.user.as_str(),
                    p.suggested_difficulty,
                    p.extranonce_subscribe,
                )
            });
            let _ = write!(
                out,
                "<form hx-post='/pool/{i}' hx-swap='none'><div class='grid grid-2'>\
                 <div class='field'><label>Stratum Host</label><input class='input-text' name='stratumURL' required value='{}'></div>\
                 <div class='field'><label>Stratum Port</label><input class='input-text' type='number' name='stratumPort' min='1' max='65535' required value='{port}'></div>\
                 <div class='field'><label>User</label><input class='input-text' name='stratumUser' required value='{}'></div>\
                 <div class='field'><label>Password</label><input class='input-text' type='password' name='stratumPassword' value='{}'></div>\
                 <div class='field'><label>Suggested Difficulty</label><input class='input-text' type='number' name='stratumSuggestedDifficulty' min='0' value='{diff}'></div>\
                 </div><label class='checkbox'><input type='checkbox' name='stratumExtranonceSubscribe' value='true'{}>Enable Extranonce Subscribe</label>\
                 <div class='buttons'><button class='btn' type='submit'>Save</button>",
                Esc(url),
                Esc(user),
                if p.is_some() { HIDDEN_PASSWORD } else { "x" },
                checked(xn)
            );
            if p.is_some() {
                let _ = write!(
                    out,
                    "<button class='btn btn-secondary' type='button' title='Delete Pool'{} hx-post='/pool/{i}/delete' \
                     hx-swap='none' hx-confirm='Delete Pool {}?'>",
                    if primary || fallback { " disabled" } else { "" },
                    i + 1
                );
                svg(out, icon::TRASH);
                out.push_str("Delete</button>");
            }
            out.push_str("</div></form></details>");
        }
        out.push_str("</div>");
    })
}

#[derive(Deserialize)]
pub struct PoolSelectionForm {
    #[serde(rename = "primaryPoolIndex")]
    primary: u16,
    #[serde(rename = "secondaryPoolIndex")]
    secondary: u16,
    #[serde(rename = "useFallbackStratum")]
    use_fallback: Option<bool>,
}

pub async fn save_pools(Form(f): Form<PoolSelectionForm>) -> Reply {
    save(
        |j| {
            j.field_num("primaryPoolIndex", f.primary)
                .field_num("secondaryPoolIndex", f.secondary)
                .field_bool("useFallbackStratum", f.use_fallback.unwrap_or(false));
        },
        "Saved pool settings",
        false,
    )
    .await
}

#[derive(Deserialize)]
pub struct FallbackForm {
    #[serde(rename = "useFallbackStratum")]
    use_fallback: u8,
}

pub async fn use_fallback(Form(f): Form<FallbackForm>) -> Reply {
    let on = f.use_fallback != 0;
    save(
        |j| {
            j.field_bool("useFallbackStratum", on);
        },
        if on {
            "Switched to Fallback pool"
        } else {
            "Switched to Primary pool"
        },
        false,
    )
    .await
}

#[derive(Deserialize)]
pub struct PoolForm {
    #[serde(rename = "stratumURL")]
    url: Text<255>,
    #[serde(rename = "stratumPort")]
    port: u16,
    #[serde(rename = "stratumUser")]
    user: Text<255>,
    #[serde(rename = "stratumPassword")]
    password: Option<Text<255>>,
    #[serde(rename = "stratumSuggestedDifficulty")]
    difficulty: Option<u64>,
    #[serde(rename = "stratumExtranonceSubscribe")]
    extranonce: Option<bool>,
}

pub async fn save_pool(index: usize, Form(f): Form<PoolForm>) -> Reply {
    // a host typed with its scheme or port
    let (host, url_port) = host_and_port(&f.url);
    let port = url_port.unwrap_or(f.port);
    let mut pools = String::new();
    {
        let mut j = JsonWriter::new(&mut pools);
        j.begin_array()
            .begin_object()
            .field_num("id", index)
            .field_str("stratumURL", host)
            .field_num("stratumPort", port)
            .field_str("stratumUser", &f.user)
            .field_str("stratumPassword", f.password.as_deref().unwrap_or("x"))
            .field_num("stratumSuggestedDifficulty", f.difficulty.unwrap_or(0))
            .field_bool("stratumExtranonceSubscribe", f.extranonce.unwrap_or(false))
            .end_object()
            .end_array();
        let _ = j.finish();
    }
    save(
        |j| {
            j.key("pools").raw(&pools);
        },
        "Saved pool settings",
        false,
    )
    .await
}

pub async fn delete_pool(index: usize) -> Reply {
    match api::clear_pool(index).await {
        Ok(()) => Reply::html(String::new())
            .header("HX-Refresh", format_args!("true"))
            .toast("success", "Pool cleared successfully"),
        Err(e) => Reply::html(String::new())
            .with_status(StatusCode::BAD_REQUEST)
            .toast("error", e),
    }
}

/// Accent colours of the theme page: name and value.
const SWATCHES: [(&str, &str); 20] = [
    ("Bright Red", "#F80421"),
    ("Orange", "#F97316"),
    ("Amber", "#F59E0B"),
    ("Yellow", "#EAB308"),
    ("Lime", "#84CC16"),
    ("Green", "#22C55E"),
    ("Emerald", "#10B981"),
    ("Teal", "#14B8A6"),
    ("Cyan", "#06B6D4"),
    ("Sky", "#0EA5E9"),
    ("Blue", "#3B82F6"),
    ("Indigo", "#6366F1"),
    ("Violet", "#8B5CF6"),
    ("Purple", "#A855F7"),
    ("Fuchsia", "#D946EF"),
    ("Pink", "#EC4899"),
    ("Rose", "#F43F5E"),
    ("Bitcoin Orange", "#F7931A"),
    ("Slate", "#64748B"),
    ("Stone", "#78716C"),
];

pub async fn design() -> Reply {
    let (scheme, color) = settings::with(|s| (s.theme_scheme.clone(), s.theme_color.clone()));
    layout(Page::Theme, "AxeOS Theme", |out| {
        out.push_str("<div class='card'><h2>Theme Configuration</h2><form hx-post='/design' hx-trigger='change' hx-swap='none'><div class='field'><div class='label'>Color Scheme</div>");
        for (value, label) in [("dark", "Dark"), ("light", "Light"), ("white", "White")] {
            let _ = write!(
                out,
                "<label class='radio'><input type='radio' name='colorScheme' value='{value}'{}>{label}</label>",
                checked(scheme == value)
            );
        }
        out.push_str(
            "</div><div class='field'><div class='label'>Theme Color</div><div class='swatches'>",
        );
        for (name, value) in SWATCHES {
            let _ = write!(
                out,
                "<button type='button' class='swatch{}' title='{name}' style='background:{value}' onclick=\"pickColor('{value}')\"></button>",
                if color.eq_ignore_ascii_case(value) { " selected" } else { "" }
            );
        }
        let _ = write!(
            out,
            "</div></div><div class='field'><label>Custom Color</label>\
             <input type='color' id='primary-color' name='primaryColor' value='{}'> <span class='mono'>{}</span></div></form></div>",
            Esc(&color.to_ascii_lowercase()),
            Esc(&color.to_ascii_uppercase())
        );
    })
}

#[derive(Deserialize)]
pub struct ThemeForm {
    #[serde(rename = "colorScheme")]
    scheme: Option<Text<16>>,
    #[serde(rename = "primaryColor")]
    color: Option<Text<16>>,
}

pub async fn save_design(Form(f): Form<ThemeForm>) -> Reply {
    let color = f.color.map(|c| {
        let mut upper = Text::<16>::new();
        for ch in c.chars() {
            let _ = upper.push(ch.to_ascii_uppercase());
        }
        upper
    });
    api::save_theme(f.scheme.as_deref(), color.as_deref()).await;
    Reply::html(String::new()).header("HX-Refresh", format_args!("true"))
}

pub async fn update() -> Reply {
    let partitions = ota::partitions().await;
    layout(Page::Update, "AxeOS Update", |out| {
        let _ = write!(
            out,
            "<div class='card'><h2>Update</h2><p>Current Version: {}</p></div>\
             <div class='card'><h3>Update Firmware</h3>\
             <label class='btn'>Browse<input type='file' accept='.bin' class='hidden' onchange='uploadFirmware(this)'></label>\
             <span class='muted'> (esp-miner-rs firmware image, .bin)</span></div>\
             <div class='card'><h3>Installed Firmware</h3><table class='data'><tr><th>Partition</th><th>Version</th>\
             <th>Build time</th><th>Space</th><th>Actions</th></tr>",
            api::VERSION
        );
        for p in &partitions {
            let _ = write!(out, "<tr><td>{}", Esc(&p.label));
            if p.current {
                out.push_str(" <span class='badge active'>Active</span>");
            }
            let _ = write!(
                out,
                "</td><td>{}</td><td>{} {}</td><td>",
                Esc(p.version.as_deref().unwrap_or("--")),
                Esc(p.compile_date.as_deref().unwrap_or("")),
                Esc(p.compile_time.as_deref().unwrap_or(""))
            );
            if let Some(u) = p.usage_percent {
                let _ = write!(out, "{u}%");
                bar(out, f32::from(u) / 100.0);
            }
            out.push_str("</td><td>");
            if !p.current && p.version.is_some() {
                let _ = write!(
                    out,
                    "<form hx-post='/update/boot' hx-swap='none' hx-confirm='Set {0} as the next boot partition? The device will restart to apply this change.'>\
                     <input type='hidden' name='partition' value='{0}'><button class='btn btn-secondary' type='submit'>Activate</button></form>",
                    Esc(&p.label)
                );
            }
            out.push_str("</td></tr>");
        }
        out.push_str(
            "</table><div class='buttons'><button class='btn btn-danger' hx-post='/api/system/restart' hx-swap='none' \
             hx-confirm='Are you sure you want to restart the device?'>Restart Device</button></div></div>\
             <div id='update-modal' class='modal hidden'><div class='card'><h3>Updating Firmware</h3>\
             <p id='update-text'>Uploading... 0%</p><div class='bar'><span id='update-bar' style='width:0'></span></div>\
             <p class='help'>Please do not disconnect the power.</p></div></div>",
        );
    })
}

#[derive(Deserialize)]
pub struct BootForm {
    partition: Text<16>,
}

pub async fn boot(Form(f): Form<BootForm>) -> Reply {
    match ota::set_boot(&f.partition).await {
        Ok(()) => {
            api::restart_soon();
            Reply::html(String::new()).toast(
                "success",
                "Next boot partition set successfully. Rebooting...",
            )
        }
        Err(e) => Reply::html(String::new())
            .with_status(StatusCode::BAD_REQUEST)
            .toast("error", e),
    }
}

/// Unknown pages go to the dashboard, as a captive portal does.
pub async fn not_found() -> Reply {
    Reply::redirect("/")
}
