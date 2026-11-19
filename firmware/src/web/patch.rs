//! Settings changes from the web API and pages: checked, then applied all at once or not at all.

use alloc::string::String;
use core::fmt::Write;

use miner_core::{
    json::JsonWriter,
    json_read::{boolean, elements, key_is, members, number, string_into},
};

use crate::settings::{Settings, MAX_POOLS};

/// Displays the settings accept.
pub const DISPLAYS: [&str; 6] = [
    "NONE",
    "SSD1306 (128x32)",
    "SSD1309 (128x64)",
    "SH1107 (64x128)",
    "SH1107 (128x128)",
    "ST7789 (320x170)",
];

/// The password value meaning "unchanged".
pub const HIDDEN_PASSWORD: &str = "*****";

/// What a successful change did, for the response.
#[derive(Default)]
pub struct Changed {
    pub hostname: bool,
    pub pools: bool,
}

fn string(raw: &str, min: usize, max: usize) -> Result<String, ()> {
    let mut s = String::new();
    string_into(raw, &mut s).map_err(|_| ())?;
    let len = s.chars().count();
    (min..=max).contains(&len).then_some(s).ok_or(())
}

fn integer(raw: &str, min: i64, max: i64) -> Result<i64, ()> {
    let n = number(raw).ok_or(())?;
    (n == (n as i64) as f64 && (min as f64..=max as f64).contains(&n))
        .then_some(n as i64)
        .ok_or(())
}

fn flag(raw: &str) -> Result<bool, ()> {
    boolean(raw).ok_or(())
}

/// A pool object, checked and completed with the defaults, as stored: `(id, json)`.
/// `old` gives the stored entries, for a password left unchanged.
pub fn pool_entry(
    raw: &str,
    id: Option<usize>,
    old: &[String; MAX_POOLS],
) -> Result<(usize, String), ()> {
    let mut id = id;
    let mut url = None;
    let mut port = None;
    let mut user = None;
    let mut protocol = String::from("SV1");
    let mut password: Option<String> = None;
    let mut difficulty = 0f64;
    let mut extranonce = false;
    let mut tls = 0i64;
    let mut cert = String::new();
    let mut decode = true;
    let mut share_warning = true;
    let mut channel = String::from("extended");
    let mut pubkey = String::new();
    let mut require_auth = false;
    for member in members(raw) {
        let (key, value) = member.map_err(|_| ())?;
        let name = &key[1..key.len() - 1];
        match name {
            "id" => id = Some(integer(value, 0, MAX_POOLS as i64 - 1)? as usize),
            "stratumURL" => url = Some(string(value, 1, 255)?),
            "stratumPort" => port = Some(integer(value, 1, 65535)?),
            "stratumUser" => user = Some(string(value, 1, 255)?),
            "stratumProtocol" => {
                protocol = string(value, 3, 3)?;
                if protocol != "SV1" && protocol != "SV2" {
                    return Err(());
                }
            }
            "stratumPassword" => password = Some(string(value, 0, 255)?),
            "stratumSuggestedDifficulty" => {
                difficulty = number(value)
                    .filter(|d| (0.0..=1e18).contains(d))
                    .ok_or(())?
            }
            "stratumExtranonceSubscribe" => extranonce = flag(value)?,
            "stratumTLS" => tls = integer(value, 0, 2)?,
            "stratumCert" => cert = string(value, 0, 3000)?,
            "stratumDecodeCoinbase" => decode = flag(value)?,
            "stratumShareWarning" => share_warning = flag(value)?,
            "stratumV2ChannelType" => {
                channel = string(value, 0, 16)?;
                if channel != "standard" && channel != "extended" {
                    return Err(());
                }
            }
            "stratumV2AuthorityPubkey" => pubkey = string(value, 0, 128)?,
            "stratumV2RequireAuth" => require_auth = flag(value)?,
            _ => {}
        }
    }
    let id = id.ok_or(())?;
    let (url, port, user) = (url.ok_or(())?, port.ok_or(())?, user.ok_or(())?);
    let password = match password {
        Some(p) if p == HIDDEN_PASSWORD => stored_password(&old[id]).unwrap_or_else(|| "x".into()),
        Some(p) => p,
        None => "x".into(),
    };
    let mut out = String::new();
    let mut json = JsonWriter::new(&mut out);
    json.begin_object()
        .field_str("stratumProtocol", &protocol)
        .field_str("stratumURL", &url)
        .field_num("stratumPort", port)
        .field_str("stratumUser", &user)
        .field_str("stratumPassword", &password)
        .field_num("stratumSuggestedDifficulty", difficulty as u64)
        .field_bool("stratumExtranonceSubscribe", extranonce)
        .field_num("stratumTLS", tls)
        .field_str("stratumCert", &cert)
        .field_bool("stratumDecodeCoinbase", decode)
        .field_bool("stratumShareWarning", share_warning)
        .field_str("stratumV2ChannelType", &channel)
        .field_str("stratumV2AuthorityPubkey", &pubkey)
        .field_bool("stratumV2RequireAuth", require_auth)
        .end_object();
    json.finish().map_err(|_| ())?;
    Ok((id, out))
}

/// The password of a stored pool entry.
pub fn stored_password(entry: &str) -> Option<String> {
    members(entry).filter_map(Result::ok).find_map(|(k, v)| {
        key_is(k, "stratumPassword").then(|| {
            let mut s = String::new();
            string_into(v, &mut s).ok().map(|()| s)
        })?
    })
}

/// Applies the members of a JSON settings object to `s`, all of them or, if one is invalid,
/// none.
pub fn apply(s: &mut Settings, body: &str) -> Result<Changed, ()> {
    let mut new = s.clone();
    let mut changed = Changed::default();
    for member in members(body) {
        let (key, value) = member.map_err(|_| ())?;
        let name = &key[1..key.len() - 1];
        match name {
            "ssid" => new.ssid = string(value, 1, 32)?,
            "wifiPass" => {
                let pass = string(value, 0, 63)?;
                if pass != HIDDEN_PASSWORD {
                    new.wifi_pass = pass;
                }
            }
            "hostname" => {
                let mut h = string(value, 1, 38)?;
                if let Some(stripped) = h.strip_suffix(".local") {
                    h = stripped.into();
                }
                if h.is_empty() || h.len() > 32 {
                    return Err(());
                }
                changed.hostname = h != s.hostname;
                new.hostname = h;
            }
            "useNTP" => new.use_ntp = flag(value)?,
            "primaryPoolIndex" => new.primary_pool_index = integer(value, 0, 7)? as u16,
            "secondaryPoolIndex" => new.secondary_pool_index = integer(value, 0, 7)? as u16,
            "useFallbackStratum" => new.use_fallback_stratum = flag(value)?,
            "frequency" => {
                new.frequency = number(value)
                    .filter(|f| (1.0..=65535.0).contains(f))
                    .ok_or(())? as f32
            }
            "coreVoltage" => new.core_voltage = integer(value, 1, 65535)? as u16,
            "overclockEnabled" => new.overclock_enabled = flag(value)?,
            "display" => {
                let d = string(value, 0, 32)?;
                if !DISPLAYS.contains(&d.as_str()) {
                    return Err(());
                }
                new.display = d;
            }
            "rotation" => {
                let r = integer(value, 0, 270)?;
                if r % 90 != 0 {
                    return Err(());
                }
                new.rotation = r as u16;
            }
            "invertscreen" => new.invert_screen = flag(value)?,
            "displayOffset" => new.display_offset = integer(value, 0, 255)? as u16,
            "displayTimeout" => new.display_timeout = integer(value, -1, 65535)? as i32,
            "autofanspeed" => new.auto_fan_speed = flag(value)?,
            "manualFanSpeed" => new.manual_fan_speed = integer(value, 0, 100)? as u16,
            "minFanSpeed" => new.min_fan_speed = integer(value, 0, 99)? as u16,
            "temptarget" => new.temp_target = integer(value, 35, 66)? as u16,
            "overheat_mode" => {
                integer(value, 0, 0)?;
                new.overheat_mode = false;
            }
            "useCustomWWW" => {
                integer(value, 0, 1)?;
            }
            "statsFrequency" => new.stats_frequency = integer(value, 0, 65535)? as u16,
            "pools" => {
                for element in elements(value).ok_or(())? {
                    let (id, entry) = pool_entry(element.map_err(|_| ())?, None, &s.pools)?;
                    new.pools[id] = entry;
                }
                changed.pools = true;
            }
            _ => {}
        }
    }
    if new.primary_pool_index != s.primary_pool_index
        || new.secondary_pool_index != s.secondary_pool_index
        || new.use_fallback_stratum != s.use_fallback_stratum
    {
        changed.pools = true;
    }
    *s = new;
    Ok(changed)
}

/// The JSON object a successful change answers: empty, or the redirection to the new host name.
pub fn response(changed: &Changed, hostname: &str) -> String {
    let mut out = String::new();
    if changed.hostname {
        let mut url = heapless::String::<64>::new();
        let _ = write!(url, "http://{hostname}.local");
        let mut json = JsonWriter::new(&mut out);
        json.begin_object()
            .field_str("status", "success")
            .key("redirect")
            .begin_object()
            .field_str("url", &url)
            .field_num("delay", 2000)
            .field_str("message", "Hostname updated. Redirecting to new address...")
            .end_object()
            .end_object();
        let _ = json.finish();
    }
    out
}
