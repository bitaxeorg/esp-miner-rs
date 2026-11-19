//! Who may use the web interface: clients on a private network, and pages served from one.

/// Whether `address` is in a private, loopback or link-local IPv4 range.
pub fn is_private_ipv4(address: [u8; 4]) -> bool {
    matches!(
        address,
        [10, ..] | [127, ..] | [192, 168, ..] | [169, 254, ..]
    ) || (address[0] == 172 && (16..=31).contains(&address[1]))
}

/// Host suffixes of local networks.
const LOCAL_SUFFIXES: [&str; 6] = [
    ".local",
    ".lan",
    ".internal",
    ".home",
    ".home.arpa",
    ".localdomain",
];

/// Whether a request with this `Origin` header comes from a page of a private network: no
/// origin, an origin whose host is a private IPv4 address, `localhost`, a name without dots, or
/// a name under a local network suffix.
pub fn is_allowed_origin(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let origin = origin.trim();
    if origin.is_empty() || origin == "null" {
        return true;
    }
    let rest = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    let host = host.trim_end_matches('.');
    if let Some(address) = parse_ipv4(host) {
        return is_private_ipv4(address);
    }
    let lower = |s: &str, suffix: &str| {
        s.len() >= suffix.len() && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    };
    host.eq_ignore_ascii_case("localhost")
        || (!host.is_empty() && !host.contains('.'))
        || LOCAL_SUFFIXES.iter().any(|suffix| lower(host, suffix))
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut address = [0; 4];
    let mut parts = s.split('.');
    for byte in &mut address {
        *byte = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ranges() {
        for a in [
            [10, 1, 2, 3],
            [172, 16, 0, 1],
            [172, 31, 255, 1],
            [192, 168, 4, 1],
            [127, 0, 0, 1],
            [169, 254, 1, 1],
        ] {
            assert!(is_private_ipv4(a), "{a:?}");
        }
        for a in [
            [8, 8, 8, 8],
            [172, 15, 0, 1],
            [172, 32, 0, 1],
            [192, 169, 0, 1],
            [100, 64, 0, 1],
        ] {
            assert!(!is_private_ipv4(a), "{a:?}");
        }
    }

    #[test]
    fn origins() {
        assert!(is_allowed_origin(None));
        assert!(is_allowed_origin(Some("http://192.168.1.42")));
        assert!(is_allowed_origin(Some("http://10.0.0.5:8080")));
        assert!(is_allowed_origin(Some("http://bitaxe")));
        assert!(is_allowed_origin(Some("http://bitaxe.local")));
        assert!(is_allowed_origin(Some("http://miner.home.arpa")));
        assert!(is_allowed_origin(Some("http://localhost:4200")));
        assert!(!is_allowed_origin(Some("https://evil.example")));
        assert!(!is_allowed_origin(Some("http://8.8.8.8")));
        assert!(!is_allowed_origin(Some("http://local.example.com")));
    }
}
