//! The JSON API of the stock firmware.

use alloc::string::String;
use core::fmt::Write as _;

use embassy_time::{Instant, Timer};
use miner_core::{
    json::JsonWriter,
    json_read::{key_is, members, string_into},
    stats::Column,
    web::is_allowed_origin,
};
use picoserve::{
    io::Read,
    request::{Request, RequestParts},
    response::{IntoResponse, ResponseWriter, StatusCode},
    routing::{Layer, Next, RequestHandlerService},
    ResponseSent,
};

use super::{
    http::Reply,
    patch::{self, HIDDEN_PASSWORD},
};
use crate::{
    board, net, ota,
    power::{Command, COMMAND},
    settings::{self, Settings, MAX_POOLS},
    stats::{self, STATS_LIMIT},
    status::{self, Status},
};

/// Largest request body read.
const BODY_MAX: usize = 4096;
/// Time to answer before restarting.
const RESTART_DELAY_MS: u64 = 1000;

/// Firmware version, as the API reports it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Restarts the chip after a moment, so that the response gets out first.
pub fn restart_soon() {
    RESTART.signal(());
}

pub static RESTART: embassy_sync::signal::Signal<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    (),
> = embassy_sync::signal::Signal::new();

/// Restarts the chip when asked to, a moment after.
#[embassy_executor::task]
pub async fn restart_task() {
    RESTART.wait().await;
    Timer::after_millis(RESTART_DELAY_MS).await;
    esp_hal::system::software_reset();
}

/// The access check of the API: requests from pages of other networks are refused, preflights
/// answered.
pub struct OriginCheck;

impl<State, PathParameters> Layer<State, PathParameters> for OriginCheck {
    type NextState = State;
    type NextPathParameters = PathParameters;

    async fn call_layer<
        'a,
        R: Read + 'a,
        NextLayer: Next<'a, R, Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: NextLayer,
        state: &State,
        path_parameters: PathParameters,
        request_parts: RequestParts<'_>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let mut path = heapless::String::<64>::new();
        let _ = write!(path, "{}", request_parts.path());
        let api = path.starts_with("/api");
        if api {
            let ap = status::with(|s| s.ap_enabled);
            let origin = request_parts
                .headers()
                .get("Origin")
                .and_then(|v| v.as_str().ok());
            if !ap && !is_allowed_origin(origin) {
                let connection = next.into_connection().await?;
                return Reply::text(StatusCode::UNAUTHORIZED, "Unauthorized")
                    .write_to(connection, response_writer)
                    .await;
            }
            if request_parts.method() == "OPTIONS" {
                let connection = next.into_connection().await?;
                return Reply::text(StatusCode::OK, "")
                    .write_to(connection, response_writer)
                    .await;
            }
        }
        next.run(state, path_parameters, response_writer).await
    }
}

/// Writes a pool entry as the API shows it: with its id, the password hidden.
fn write_pool<W: core::fmt::Write>(json: &mut JsonWriter<W>, id: usize, entry: &str) {
    json.begin_object().field_num("id", id);
    for (key, value) in members(entry).filter_map(Result::ok) {
        let name = &key[1..key.len() - 1];
        if name == "stratumPassword" {
            json.field_str(name, HIDDEN_PASSWORD);
        } else {
            json.key(name).raw(value);
        }
    }
    json.end_object();
}

/// The legacy fields of the primary and fallback pools: `prefix` + member name, e.g.
/// `fallbackStratumURL`.
fn write_legacy_pool<W: core::fmt::Write>(json: &mut JsonWriter<W>, prefix: &str, entry: &str) {
    for (key, value) in members(entry).filter_map(Result::ok) {
        let name = &key[1..key.len() - 1];
        if name == "stratumPassword" || name == "stratumV2RequireAuth" {
            continue;
        }
        let mut full = heapless::String::<48>::new();
        match prefix {
            "" => {
                let _ = full.push_str(name);
            }
            _ => {
                let _ = write!(
                    full,
                    "{prefix}{}",
                    name.strip_prefix("stratum").unwrap_or(name)
                );
            }
        }
        json.key(&full).raw(value);
    }
}

fn ipv4(stack_ip: Option<embassy_net::Ipv4Cidr>) -> heapless::String<16> {
    let mut s = heapless::String::new();
    if let Some(cidr) = stack_ip {
        let _ = write!(s, "{}", cidr.address());
    }
    s
}

/// The system information document.
pub async fn info_json() -> String {
    let s: Status = status::get();
    let set: Settings = settings::get();
    let partitions = ota::partitions().await;
    let ip = ipv4(status::with(|s| s.ipv4));
    let rssi = net::wifi_rssi();
    let uptime = Instant::now().as_secs();
    let mut out = String::with_capacity(6144);
    let mut j = JsonWriter::new(&mut out);
    let opt = |v: Option<f32>| v.map_or(0.0, f64::from);
    j.begin_object()
        .field_float("power", opt(s.power_w))
        .field_float("voltage", opt(s.vin_v) * 1000.0)
        .field_float("current", opt(s.current_a) * 1000.0)
        .field_float(
            "temp",
            if s.chip_temps_c[0].is_nan() {
                0.0
            } else {
                f64::from(s.chip_temps_c[0])
            },
        )
        .field_float(
            "temp2",
            if s.chip_temps_c[1].is_nan() {
                0.0
            } else {
                f64::from(s.chip_temps_c[1])
            },
        )
        .field_float("vrTemp", opt(s.vr_temp_c))
        .field_float("coreVoltageActual", opt(s.vcore_v) * 1000.0)
        .field_float("actualFrequency", f64::from(s.frequency_mhz))
        .field_float("expectedHashrate", f64::from(s.expected_hashrate_ghs))
        .field_float("fanspeed", f64::from(s.fan_percent))
        .field_num("fanrpm", s.fan_rpm.unwrap_or(0))
        .field_num("fan2rpm", 0)
        .field_float("hashRate", f64::from(s.hashrate_ghs))
        .field_float("hashRate_1m", f64::from(s.hashrate_1m))
        .field_float("hashRate_10m", f64::from(s.hashrate_10m))
        .field_float("hashRate_1h", f64::from(s.hashrate_1h))
        .field_float("errorPercentage", f64::from(s.error_percentage))
        .field_num("sharesAccepted", s.accepted)
        .field_num("sharesRejected", s.rejected)
        .field_num(
            "sharesPending",
            s.submitted.saturating_sub(s.accepted + s.rejected),
        )
        .field_num(
            "bestDiff",
            s.best_difficulty_ever.max(set.best_diff as f64) as u64,
        )
        .field_num("bestSessionDiff", s.best_difficulty as u64)
        .field_float("poolDifficulty", s.pool_difficulty)
        .field_float("responseTime", opt(s.response_time_ms))
        .field_float("responseTimeMin", opt(s.response_time_min_ms))
        .field_float("responseTimeMax", opt(s.response_time_max_ms))
        .field_num("responseShareBatch", 1)
        .field_num("workReceived", s.work_received)
        .field_num("blockFound", s.block_found)
        .field_bool("showNewBlock", s.show_new_block)
        .field_num(
            "blockHeight",
            s.coinbase.as_ref().map_or(0, |c| c.block_height),
        )
        .field_str(
            "scriptsig",
            s.coinbase.as_ref().map_or("", |c| c.scriptsig.as_str()),
        )
        .field_num("networkDifficulty", s.network_difficulty as u64)
        .field_num("coinbaseValueTotalSatoshis", 0)
        .field_num("coinbaseValueUserSatoshis", 0)
        .field_str("coinbasePayoutStatus", "unknown")
        .field_num("freeHeap", stats::free_heap())
        .field_num("freeHeapInternal", stats::free_heap())
        .field_num("freeHeapSpiram", 0)
        .field_num("minFreeHeap", 0)
        .field_num("maxAllocHeap", 0)
        .field_num("uptimeSeconds", uptime)
        .field_num("totalUptimeSeconds", uptime)
        .field_float("totalHashes", s.total_hashes)
        .field_float(
            "totalLog2Work",
            if s.total_hashes > 0.0 {
                libm_log2(s.total_hashes)
            } else {
                0.0
            },
        )
        .field_num("cpuUsage", 0)
        .field_bool("miningPaused", s.mining_paused)
        .field_num("overheat_mode", u8::from(set.overheat_mode))
        .field_str("wifiStatus", &s.wifi_status)
        .field_num("wifiRSSI", rssi.unwrap_or(0));
    j.field_str("version", VERSION)
        .field_str("axeOSVersion", "Unified")
        .field_str("idfVersion", "none (Rust)")
        .field_str("boardVersion", env!("BOARD_VERSION"))
        .field_num("maxPower", board::MAX_POWER_W)
        .field_num("nominalVoltage", board::NOMINAL_VOLTAGE_V)
        .field_num("smallCoreCount", board::SMALL_CORE_COUNT)
        .field_str("ASICModel", board::ASIC_MODEL)
        .field_num("isPSRAMAvailable", 0)
        .field_str("resetReason", "unknown")
        .field_str(
            "runningPartition",
            partitions
                .iter()
                .find(|p| p.current)
                .map_or("factory", |p| p.label.as_str()),
        )
        .field_str("macAddr", &super::mac_address())
        .field_str("hostname", &set.hostname)
        .field_str("ssid", &set.ssid)
        .field_str("ipv4", &ip)
        .field_str("ipv6", "")
        .field_num("apEnabled", u8::from(s.ap_enabled))
        .field_num("useNTP", u8::from(set.use_ntp))
        .field_str(
            "poolConnectionInfo",
            if s.pool_connected {
                "Connected"
            } else {
                "Not Connected"
            },
        )
        .field_str("primaryPoolError", &s.primary_pool_error)
        .field_str("poolBanner", "")
        .field_num("isUsingFallbackStratum", u8::from(s.using_fallback))
        .field_num("primaryPoolIndex", set.primary_pool_index)
        .field_num("secondaryPoolIndex", set.secondary_pool_index)
        .field_num("useFallbackStratum", u8::from(set.use_fallback_stratum));
    j.key("pools").begin_array();
    for (id, entry) in set.pools.iter().enumerate() {
        if miner_core::pool_config::PoolConfig::parse(entry).is_some() {
            write_pool(&mut j, id, entry);
        }
    }
    j.end_array();
    if let Some(primary) = set.pools.get(usize::from(set.primary_pool_index)) {
        write_legacy_pool(&mut j, "", primary);
    }
    if let Some(secondary) = set.pools.get(usize::from(set.secondary_pool_index)) {
        write_legacy_pool(&mut j, "fallbackStratum", secondary);
    }
    j.field_num("useCustomWWW", 0)
        .field_num("overclockEnabled", u8::from(set.overclock_enabled))
        .field_str("display", &set.display)
        .field_num("rotation", set.rotation)
        .field_num("invertscreen", u8::from(set.invert_screen))
        .field_num("displayTimeout", set.display_timeout)
        .field_num("autofanspeed", u8::from(set.auto_fan_speed))
        .field_num("manualFanSpeed", set.manual_fan_speed)
        .field_num("minFanSpeed", set.min_fan_speed)
        .field_num("temptarget", set.temp_target)
        .field_num("coreVoltage", set.core_voltage)
        .field_float("frequency", f64::from(set.frequency))
        .field_num("statsFrequency", set.stats_frequency)
        .field_num("statsLimit", STATS_LIMIT);
    j.key("sharesRejectedReasons").begin_array().end_array();
    j.key("blockSignals").begin_array().end_array();
    j.key("coinbaseOutputs").begin_array().end_array();
    j.field_num("coinbaseOthersCount", 0)
        .field_num("coinbaseOthersValueSatoshis", 0);
    j.key("partitions").begin_array();
    for p in &partitions {
        j.begin_object()
            .field_str("label", &p.label)
            .field_bool("isCurrent", p.current)
            .field_bool("isFactory", p.factory);
        match &p.version {
            Some(v) => j.field_str("version", v),
            None => j.key("version").null(),
        };
        match &p.compile_date {
            Some(v) => j.field_str("compileDate", v),
            None => j.key("compileDate").null(),
        };
        match &p.compile_time {
            Some(v) => j.field_str("compileTime", v),
            None => j.key("compileTime").null(),
        };
        match p.usage_percent {
            Some(v) => j.field_num("usagePercent", v),
            None => j.key("usagePercent").null(),
        };
        j.end_object();
    }
    j.end_array();
    let mut full = heapless::String::<48>::new();
    let _ = write!(full, "{}.local", set.hostname);
    j.field_str("fullHostname", &full);
    // per-chip and per-domain hashrate from the chips' hash counters (GH/s)
    j.key("hashrateMonitor")
        .begin_object()
        .key("asics")
        .begin_array();
    for chip in &s.chip_hashrates {
        j.begin_object()
            .field_float("total", f64::from(chip.total))
            .key("domains")
            .begin_array();
        for domain in &chip.domains {
            j.float(f64::from(*domain));
        }
        j.end_array().end_object();
    }
    j.end_array().end_object();
    j.end_object();
    let _ = j.finish();
    out
}

/// log2 without the float maths library: exponent from the bits, mantissa by a short series.
fn libm_log2(x: f64) -> f64 {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    // ln(m) for m in [1, 2) by atanh series, then to base 2
    let t = (mantissa - 1.0) / (mantissa + 1.0);
    let t2 = t * t;
    let mut term = t;
    let mut sum = 0.0;
    let mut k = 1.0;
    for _ in 0..12 {
        sum += term / k;
        term *= t2;
        k += 2.0;
    }
    exponent as f64 + 2.0 * sum / core::f64::consts::LN_2
}

pub async fn info() -> Reply {
    Reply::json(info_json().await)
}

pub async fn asic() -> Reply {
    let mut out = String::new();
    let mut j = JsonWriter::new(&mut out);
    j.begin_object()
        .field_str("ASICModel", board::ASIC_MODEL)
        .field_str("deviceModel", env!("BOARD_NAME"))
        .field_str("swarmColor", board::SWARM_COLOR)
        .field_num(
            "asicCount",
            status::with(|s| s.asic_count).max(board::ASIC_COUNT),
        )
        .field_num("hashDomains", board::HASH_DOMAINS)
        .field_num("defaultFrequency", board::HASH_FREQ_MHZ)
        .key("frequencyOptions")
        .begin_array();
    for f in board::FREQUENCY_OPTIONS {
        j.num(f);
    }
    j.end_array()
        .field_num("defaultVoltage", (board::VCORE * 1000.0 + 0.5) as u16)
        .key("voltageOptions")
        .begin_array();
    for v in board::VOLTAGE_OPTIONS {
        j.num(v);
    }
    j.end_array().end_object();
    let _ = j.finish();
    Reply::json(out)
}

/// `GET /api/system/statistics?columns=a,b`.
pub struct Statistics;

impl RequestHandlerService for Statistics {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: (),
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let mut columns = heapless::Vec::<Column, 19>::new();
        if let Some(query) = request.parts.query() {
            let mut q = heapless::String::<512>::new();
            let _ = q.push_str(query.0);
            for pair in q.split('&') {
                if let Some(list) = pair.strip_prefix("columns=") {
                    for name in list.split([',', '%']) {
                        // "%2C" is an encoded comma
                        let name = name.strip_prefix("2C").unwrap_or(name);
                        if let Some(c) = Column::from_name(name) {
                            if !columns.contains(&c) {
                                let _ = columns.push(c);
                            }
                        }
                    }
                }
            }
        }
        let enabled = settings::with(|s| s.stats_frequency) != 0;
        let mut out = String::with_capacity(2048);
        let mut j = JsonWriter::new(&mut out);
        let now = Instant::now().as_millis() as u32;
        if enabled {
            stats::with_ring(|ring| ring.write_json(&mut j, &columns, now));
        } else {
            miner_core::stats::StatsRing::<1>::new().write_json(&mut j, &columns, now);
        }
        let _ = j.finish();
        let connection = request.body_connection.finalize().await?;
        Reply::json(out).write_to(connection, response_writer).await
    }
}

pub async fn scoreboard() -> Reply {
    let mut out = String::new();
    let mut j = JsonWriter::new(&mut out);
    j.begin_array();
    stats::with_scoreboard(|board| {
        for e in board.entries() {
            let mut nonce = heapless::String::<8>::new();
            let _ = write!(nonce, "{:08X}", e.nonce);
            let mut bits = heapless::String::<8>::new();
            let _ = write!(bits, "{:08X}", e.version_bits);
            j.begin_object()
                .field_float("difficulty", e.difficulty)
                .field_str("job_id", &e.job_id)
                .field_str("extranonce2", &e.extranonce2)
                .field_num("ntime", e.ntime)
                .field_str("nonce", &nonce)
                .field_str("version_bits", &bits)
                .end_object();
        }
    });
    j.end_array();
    let _ = j.finish();
    Reply::json(out)
}

pub async fn logs() -> Reply {
    let mut out = String::new();
    crate::weblog::for_each_after(0, |_, line| {
        out.push_str(line);
        out.push('\n');
    });
    Reply::text(StatusCode::OK, &out).header(
        "Content-Disposition",
        format_args!("attachment; filename=\"bitaxe-logs.txt\""),
    )
}

pub async fn wifi_scan() -> Reply {
    match net::scan().await {
        Some(networks) => {
            let mut out = String::new();
            let mut j = JsonWriter::new(&mut out);
            j.begin_object().key("networks").begin_array();
            for n in &networks {
                j.begin_object()
                    .field_str("ssid", &n.ssid)
                    .field_num("rssi", n.rssi)
                    .field_num("authmode", n.authmode)
                    .end_object();
            }
            j.end_array().end_object();
            let _ = j.finish();
            Reply::json(out)
        }
        None => Reply::text(StatusCode::INTERNAL_SERVER_ERROR, "WiFi scan failed"),
    }
}

pub async fn checksum() -> Reply {
    match ota::checksum().await {
        Some(c) => {
            let mut out = String::new();
            let mut j = JsonWriter::new(&mut out);
            j.begin_object()
                .field_str("partition", &c.partition)
                .field_str("version", &c.version)
                .field_num("size", c.size)
                .field_str("sha256", &c.sha256)
                .end_object();
            let _ = j.finish();
            Reply::json(out)
        }
        None => Reply::text(StatusCode::INTERNAL_SERVER_ERROR, "Checksum unavailable"),
    }
}

/// Reads a request body of at most [`BODY_MAX`] bytes.
async fn read_body<R: Read>(request: &mut Request<'_, R>) -> Result<String, ()> {
    let body = request.body_connection.body();
    if body.content_length() > BODY_MAX {
        return Err(());
    }
    let mut reader = body.reader();
    let mut bytes = alloc::vec::Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        let n = reader.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > BODY_MAX {
            return Err(());
        }
    }
    String::from_utf8(bytes).map_err(|_| ())
}

/// Applies a settings change, saves it and reconnects the pool when it changed.
pub async fn apply_settings(body: &str) -> Result<patch::Changed, ()> {
    let mut outcome = Err(());
    let mut copy = settings::get();
    if let Ok(changed) = patch::apply(&mut copy, body) {
        let saved = settings::update(|s| *s = copy.clone()).await;
        if changed.pools {
            crate::pool::RECONNECT.signal(());
        }
        outcome = saved.map(|()| changed);
    }
    outcome
}

/// `PATCH /api/system`.
pub struct PatchSettings;

impl RequestHandlerService for PatchSettings {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: (),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let reply = match read_body(&mut request).await {
            Err(()) => Reply::text(
                StatusCode::BAD_REQUEST,
                "Content length invalid or too large",
            ),
            Ok(body) => match apply_settings(&body).await {
                Ok(changed) => {
                    let hostname = settings::with(|s| s.hostname.clone());
                    let response = patch::response(&changed, &hostname);
                    Reply::json(response)
                }
                Err(()) => Reply::text(StatusCode::BAD_REQUEST, "Wrong API input"),
            },
        };
        let connection = request.body_connection.finalize().await?;
        reply.write_to(connection, response_writer).await
    }
}

/// `PUT /api/system/pools/{index}`.
pub struct PutPool;

impl RequestHandlerService<(), (usize,)> for PutPool {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        (index,): (usize,),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let reply = if index >= MAX_POOLS {
            Reply::text(StatusCode::BAD_REQUEST, "Invalid pool index")
        } else {
            match read_body(&mut request).await {
                Err(()) => Reply::text(StatusCode::BAD_REQUEST, "Invalid request length"),
                Ok(body) => {
                    let old = settings::with(|s| s.pools.clone());
                    match patch::pool_entry(&body, Some(index), &old) {
                        Ok((id, entry)) => {
                            let _ = settings::update(|s| s.pools[id] = entry).await;
                            crate::pool::RECONNECT.signal(());
                            Reply::message(StatusCode::OK, "Pool updated successfully")
                        }
                        Err(()) => Reply::text(
                            StatusCode::BAD_REQUEST,
                            "Invalid pool configuration payload",
                        ),
                    }
                }
            }
        };
        let connection = request.body_connection.finalize().await?;
        reply.write_to(connection, response_writer).await
    }
}

/// Clears a pool slot, unless it is the primary or the fallback.
pub async fn clear_pool(index: usize) -> Result<(), &'static str> {
    if index >= MAX_POOLS {
        return Err("Invalid pool index");
    }
    let selected = settings::with(|s| {
        usize::from(s.primary_pool_index) == index || usize::from(s.secondary_pool_index) == index
    });
    if selected {
        return Err("Cannot delete a pool that is currently selected as primary or fallback");
    }
    let _ = settings::update(|s| s.pools[index].clear()).await;
    Ok(())
}

pub async fn delete_pool(index: usize) -> Reply {
    match clear_pool(index).await {
        Ok(()) => Reply::message(StatusCode::OK, "Pool cleared successfully"),
        Err(e) => Reply::text(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn restart() -> Reply {
    restart_soon();
    Reply::message(StatusCode::OK, "System will restart shortly.")
}

pub async fn pause() -> Reply {
    COMMAND.signal(Command::Pause);
    Reply::message(StatusCode::OK, "Mining paused")
}

pub async fn resume() -> Reply {
    COMMAND.signal(Command::Resume);
    Reply::message(StatusCode::OK, "Mining resumed")
}

/// Starts saying hello on the display for 30 s, or stops when it already does.
pub fn toggle_identify() -> bool {
    let now = Instant::now().as_millis();
    let mut on = false;
    status::update(|s| {
        if s.identify_until_ms > now {
            s.identify_until_ms = 0;
        } else {
            s.identify_until_ms = now + 30_000;
            on = true;
        }
    });
    on
}

pub async fn identify() -> Reply {
    if toggle_identify() {
        Reply::message(StatusCode::OK, "The device says \"Hi!\" for 30 seconds.")
    } else {
        Reply::message(StatusCode::OK, "The device no longer says \"Hi!\".")
    }
}

pub async fn dismiss_block_found() -> Reply {
    let count = status::with(|s| s.block_found);
    status::update(|s| s.show_new_block = false);
    let mut out = String::new();
    let mut j = JsonWriter::new(&mut out);
    j.begin_object()
        .field_num("blockFound", count)
        .field_bool("showNewBlock", false)
        .field_str("message", "Block found notification dismissed")
        .end_object();
    let _ = j.finish();
    Reply::json(out)
}

/// `POST /api/system/boot` with `{"partition": "ota_0"}`.
pub struct Boot;

impl RequestHandlerService for Boot {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: (),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let body = read_body(&mut request).await.unwrap_or_default();
        let mut label = heapless::String::<16>::new();
        for (key, value) in members(&body).filter_map(Result::ok) {
            if key_is(key, "partition") {
                let _ = string_into(value, &mut label);
            }
        }
        let reply = if label.is_empty() {
            Reply::text(StatusCode::BAD_REQUEST, "Missing partition label")
        } else {
            match ota::set_boot(&label).await {
                Ok(()) => {
                    restart_soon();
                    Reply::message(
                        StatusCode::OK,
                        "Next boot partition set successfully. Rebooting...",
                    )
                }
                Err(e) => Reply::text(StatusCode::BAD_REQUEST, e),
            }
        };
        let connection = request.body_connection.finalize().await?;
        reply.write_to(connection, response_writer).await
    }
}

/// `POST /api/system/OTA`: the raw image in the body.
pub struct Ota;

impl RequestHandlerService for Ota {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: (),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let reply = if status::with(|s| s.ap_enabled) {
            Reply::message(StatusCode::INTERNAL_SERVER_ERROR, "Not allowed in AP mode")
        } else {
            let mut body = request.body_connection.body().reader();
            match ota::update(&mut body).await {
                Ok(()) => {
                    restart_soon();
                    Reply::text(StatusCode::OK, "Firmware update complete, rebooting now!\n")
                }
                Err(e) => Reply::text(StatusCode::INTERNAL_SERVER_ERROR, e),
            }
        };
        let connection = request.body_connection.finalize().await?;
        reply.write_to(connection, response_writer).await
    }
}

pub async fn ota_www() -> Reply {
    Reply::message(
        StatusCode::BAD_REQUEST,
        "The web interface is part of the firmware: update the firmware instead",
    )
}

fn theme_json() -> String {
    let (scheme, color) = settings::with(|s| (s.theme_scheme.clone(), s.theme_color.clone()));
    let mut out = String::new();
    let mut j = JsonWriter::new(&mut out);
    j.begin_object()
        .field_str("colorScheme", &scheme)
        .field_str("primaryColor", &color)
        .end_object();
    let _ = j.finish();
    out
}

pub async fn theme() -> Reply {
    Reply::json(theme_json())
}

/// Saves the colour scheme and the accent colour.
pub async fn save_theme(scheme: Option<&str>, color: Option<&str>) {
    let scheme: Option<String> = scheme
        .filter(|s| ["dark", "light", "white"].contains(s))
        .map(Into::into);
    let color: Option<String> = color
        .filter(|c| {
            c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(Into::into);
    let _ = settings::update(|s| {
        if let Some(scheme) = scheme {
            s.theme_scheme = scheme;
        }
        if let Some(color) = color {
            s.theme_color = color;
        }
    })
    .await;
}

/// `POST /api/theme`.
pub struct PostTheme;

impl RequestHandlerService for PostTheme {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: (),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let body = read_body(&mut request).await.unwrap_or_default();
        let mut scheme = heapless::String::<16>::new();
        let mut color = heapless::String::<16>::new();
        for (key, value) in members(&body).filter_map(Result::ok) {
            if key_is(key, "colorScheme") {
                let _ = string_into(value, &mut scheme);
            } else if key_is(key, "primaryColor") {
                let _ = string_into(value, &mut color);
            }
        }
        save_theme(
            (!scheme.is_empty()).then_some(scheme.as_str()),
            (!color.is_empty()).then_some(color.as_str()),
        )
        .await;
        let mut out = String::new();
        let _ = out.write_str(r#"{"status":"ok"}"#);
        let connection = request.body_connection.finalize().await?;
        Reply::json(out).write_to(connection, response_writer).await
    }
}
