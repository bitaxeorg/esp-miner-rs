//! SNTP (RFC 4330): the request a client sends over UDP, and the time in a server's answer.

/// Length of an NTP packet without extensions.
pub const PACKET_LEN: usize = 48;
/// The NTP port.
pub const PORT: u16 = 123;
/// Seconds from 1900, where NTP counts from, to 1970.
const UNIX_EPOCH: u32 = 2_208_988_800;
/// Mode of a server's answer.
const MODE_SERVER: u8 = 4;
/// Offset of the transmit timestamp (seconds).
const TRANSMIT_SECONDS: usize = 40;

/// A client request: no leap indicator, version 4, mode 3 (client).
pub fn request() -> [u8; PACKET_LEN] {
    let mut packet = [0; PACKET_LEN];
    packet[0] = 0b00_100_011;
    packet
}

/// The Unix time, in seconds, when the server sent `answer`. `None` for a packet that isn't a
/// server's answer, a kiss-of-death (stratum 0), or one without a time.
///
/// NTP seconds wrap in 2036; counted modulo 2^32 from 1970, the result stays right until 2106.
pub fn unix_time(answer: &[u8]) -> Option<u32> {
    let packet = answer.get(..PACKET_LEN)?;
    if packet[0] & 0b111 != MODE_SERVER || packet[1] == 0 {
        return None;
    }
    let seconds = u32::from_be_bytes(
        packet[TRANSMIT_SECONDS..TRANSMIT_SECONDS + 4]
            .try_into()
            .ok()?,
    );
    (seconds != 0).then(|| seconds.wrapping_sub(UNIX_EPOCH))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(stratum: u8, seconds: u32) -> [u8; PACKET_LEN] {
        let mut p = [0; PACKET_LEN];
        // version 4, server
        p[0] = 0b00_100_100;
        p[1] = stratum;
        p[TRANSMIT_SECONDS..TRANSMIT_SECONDS + 4].copy_from_slice(&seconds.to_be_bytes());
        p
    }

    #[test]
    fn a_request_is_a_version_4_client_packet() {
        let r = request();
        assert_eq!(r[0], 0x23);
        assert!(r[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn the_answer_gives_unix_time() {
        // 2026-11-21 12:00:00 UTC
        assert_eq!(unix_time(&answer(2, 0xeeac_0640)), Some(1_795_262_400));
        // past the NTP wrap of 2036-02-07 06:28:16 UTC
        assert_eq!(unix_time(&answer(2, 10)), Some(2_085_978_506));
        assert_eq!(unix_time(&answer(0, 0xeeac_0640)), None, "kiss-of-death");
        assert_eq!(unix_time(&answer(2, 0)), None);
        assert_eq!(unix_time(&request()), None, "not an answer");
        assert_eq!(unix_time(&answer(2, 0xeeac_0640)[..40]), None);
    }
}
