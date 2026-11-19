//! A pool of the settings, from its JSON object.

use heapless::String;
use serde::Deserialize;

/// Stratum v1, the only protocol supported.
pub const PROTOCOL_V1: &str = "SV1";

/// The parts of a pool entry the miner uses. Other members of the object are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PoolConfig {
    #[serde(rename = "stratumProtocol", default = "protocol_v1")]
    pub protocol: String<8>,
    #[serde(rename = "stratumURL")]
    pub url: String<128>,
    #[serde(rename = "stratumPort")]
    pub port: u16,
    #[serde(rename = "stratumUser", default)]
    pub user: String<128>,
    #[serde(rename = "stratumPassword", default)]
    pub password: String<64>,
    #[serde(rename = "stratumSuggestedDifficulty", default)]
    pub suggested_difficulty: u32,
    #[serde(rename = "stratumExtranonceSubscribe", default)]
    pub extranonce_subscribe: bool,
    #[serde(rename = "stratumTLS", default)]
    pub tls: u16,
}

fn protocol_v1() -> String<8> {
    String::try_from(PROTOCOL_V1).unwrap_or_default()
}

impl PoolConfig {
    /// Reads a pool entry. `None` for an empty slot, invalid JSON or a pool without host.
    pub fn parse(json: &str) -> Option<Self> {
        if json.trim().is_empty() {
            return None;
        }
        let (pool, _) = serde_json_core::from_str::<Self>(json).ok()?;
        (!pool.host().is_empty()).then_some(pool)
    }

    /// Host name or address of the URL, without a scheme, port or path.
    pub fn host(&self) -> &str {
        host_and_port(&self.url).0
    }

    /// Port: from the URL if it has one, else `port`.
    pub fn effective_port(&self) -> u16 {
        host_and_port(&self.url).1.unwrap_or(self.port)
    }
}

/// Splits a pool URL such as `stratum+tcp://pool.example:3333/path` into host and port.
pub fn host_and_port(url: &str) -> (&str, Option<u16>) {
    let url = url.trim();
    let url = url.split_once("://").map_or(url, |(_, rest)| rest);
    let url = url.split(['/', '?']).next().unwrap_or("");
    match url.rsplit_once(':') {
        Some((host, port)) => match port.parse() {
            Ok(port) => (host, Some(port)),
            Err(_) => (url, None),
        },
        None => (url, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_entry_and_ignores_unknown_members() {
        let pool = PoolConfig::parse(
            r#"{"stratumProtocol":"SV1","stratumURL":"public-pool.io","stratumPort":21496,
                "stratumUser":"bc1qexample.worker","stratumPassword":"x",
                "stratumSuggestedDifficulty":1000,"stratumExtranonceSubscribe":false,
                "stratumTLS":0,"stratumCert":"","stratumV2ChannelType":"extended",
                "stratumDecodeCoinbase":true}"#,
        )
        .unwrap();
        assert_eq!(pool.host(), "public-pool.io");
        assert_eq!(pool.effective_port(), 21496);
        assert_eq!(pool.user, "bc1qexample.worker");
        assert_eq!(pool.password, "x");
        assert_eq!(pool.suggested_difficulty, 1000);
        assert_eq!(pool.protocol, PROTOCOL_V1);
    }

    #[test]
    fn missing_optional_members_take_defaults() {
        let pool =
            PoolConfig::parse(r#"{"stratumURL":"solo.ckpool.org","stratumPort":3333}"#).unwrap();
        assert_eq!(pool.protocol, PROTOCOL_V1);
        assert_eq!(pool.user, "");
        assert!(!pool.extranonce_subscribe);
    }

    #[test]
    fn empty_or_hostless_entries_are_none() {
        assert_eq!(PoolConfig::parse(""), None);
        assert_eq!(PoolConfig::parse("not json"), None);
        assert_eq!(
            PoolConfig::parse(r#"{"stratumURL":"","stratumPort":3333}"#),
            None
        );
    }

    #[test]
    fn urls_with_scheme_port_or_path() {
        assert_eq!(host_and_port("public-pool.io"), ("public-pool.io", None));
        assert_eq!(
            host_and_port("stratum+tcp://solo.ckpool.org:3333"),
            ("solo.ckpool.org", Some(3333))
        );
        assert_eq!(
            host_and_port(" 192.168.1.10:3334/x "),
            ("192.168.1.10", Some(3334))
        );
        let pool = PoolConfig::parse(
            r#"{"stratumURL":"stratum+tcp://h.example:4444","stratumPort":3333}"#,
        )
        .unwrap();
        assert_eq!((pool.host(), pool.effective_port()), ("h.example", 4444));
    }
}
