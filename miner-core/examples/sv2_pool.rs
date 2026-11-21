//! A Stratum V2 pool to test miners against: the Noise handshake with the SRI test authority key,
//! a standard or extended channel per connection, a new job every 20 s and a new block every
//! third job, and every share checked against the channel target.
//!
//! ```text
//! cargo run -p miner-core --example sv2_pool -- [listen address, default 0.0.0.0:34254] [difficulty, default 0.001]
//! ```
//!
//! Its authority public key is `9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use binary_sv2::{GetSize, Seq0255, Serialize, Str0255, Sv2Option, B032, B064K, U256};
use codec_sv2::{
    Decoded, Decrypted, Handshake, MessageFrame, NoiseDecoder, NoiseEncoder, TransportDecryptState,
    TransportEncryptState,
};
use common_messages_sv2::SetupConnectionSuccess;
use miner_core::{
    header::{difficulty, BlockHeader},
    sv2::work::merkle_root,
};
use mining_sv2::{
    NewExtendedMiningJob, NewMiningJob, OpenExtendedMiningChannelSuccess,
    OpenStandardMiningChannelSuccess, SetNewPrevHash, SetTarget, SubmitSharesError,
    SubmitSharesSuccess,
};
use noise_sv2::{Responder, ELLSWIFT_ENCODING_SIZE};
use parsers_sv2::{CommonMessages, Mining};

const AUTHORITY_PUBLIC_KEY: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
const AUTHORITY_SECRET_KEY: &str = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";
const CHANNEL_ID: u32 = 1;
const EXTRANONCE_PREFIX: [u8; 4] = [0xe5, 0x2a, 0x00, 0x01];
const EXTRANONCE_SIZE: u16 = 8;
const JOB_INTERVAL: Duration = Duration::from_secs(20);
const JOBS_PER_BLOCK: u32 = 3;
const VERSION: u32 = 0x2000_0000;
const NBITS: u32 = 0x1703_0ecd;
/// A coinbase spending to nobody: version, one input with block height 900000 and a tag in its
/// scriptsig, room for the extranonce, then one output and lock time.
const COINBASE_PREFIX: &[u8] = &[
    1, 0, 0, 0, 1, // version, input count
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, // previous output
    0xff, 0xff, 0xff, 0xff, // index
    28,   // scriptsig length: height (4), tag (12), extranonce (12)
    3, 0xa0, 0xbb, 0x0d, // height 900000
    b's', b'v', b'2', b'_', b'p', b'o', b'o', b'l', b' ', b't', b'a', b'g',
];
const COINBASE_SUFFIX: &[u8] = &[
    0xff, 0xff, 0xff, 0xff, // sequence
    1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x6a, // one output: 0 sat, OP_RETURN
    0, 0, 0, 0, // lock time
];

fn now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

/// The target of `difficulty`, little endian.
fn target(difficulty: f64) -> [u8; 32] {
    // difficulty 1: 0xffff × 2^208
    let mut value = 65535.0 * 2f64.powi(208) / difficulty;
    let mut out = [0u8; 32];
    for byte in out.iter_mut() {
        *byte = (value % 256.0) as u8;
        value = (value / 256.0).floor();
    }
    if value >= 1.0 {
        out = [0xff; 32];
    }
    out
}

struct Conn {
    stream: TcpStream,
    encoder: NoiseEncoder,
    decoder: NoiseDecoder,
    encrypt: TransportEncryptState,
    decrypt: Option<TransportDecryptState>,
}

impl Conn {
    fn handshake(mut stream: TcpStream) -> Option<Self> {
        let public: key_utils::Secp256k1PublicKey = AUTHORITY_PUBLIC_KEY.parse().ok()?;
        let secret: key_utils::Secp256k1SecretKey = AUTHORITY_SECRET_KEY.parse().ok()?;
        let responder = Responder::from_authority_kp(
            &public.into_bytes(),
            &secret.into_bytes(),
            Duration::from_secs(24 * 3600),
        )
        .ok()?;
        let mut encoder = NoiseEncoder::new();
        let mut decoder = NoiseDecoder::new();
        let first = loop {
            match decoder.next_handshake_frame::<Responder>().ok()? {
                Decoded::Frame(m) => break m,
                Decoded::Incomplete => {
                    let n = stream.read(decoder.read_buf()).ok()?;
                    if n == 0 {
                        return None;
                    }
                    decoder.advance(n).ok()?;
                }
            }
        };
        let first: [u8; ELLSWIFT_ENCODING_SIZE] = first.payload().try_into().ok()?;
        let (reply, transport) = Handshake::responder(responder)
            .step_1_with_now_rng(first, now(), &mut rand::thread_rng())
            .ok()?;
        stream.write_all(&encoder.encode_handshake(reply)).ok()?;
        let (encrypt, decrypt) = transport.split();
        Some(Conn {
            stream,
            encoder,
            decoder,
            encrypt,
            decrypt: Some(decrypt),
        })
    }

    fn send<M: Serialize + GetSize>(
        &mut self,
        message: M,
        msg_type: u8,
        channel_msg: bool,
    ) -> Option<()> {
        let frame = MessageFrame::from_message(message, msg_type, 0, channel_msg).ok()?;
        let out = self
            .encoder
            .encode_transport(frame, &mut self.encrypt)
            .ok()?;
        self.stream.write_all(&out).ok()
    }

    /// The next message, waiting at most until `deadline`: `Some(None)` on timeout.
    fn recv(&mut self, deadline: Instant) -> Option<Option<(u8, Vec<u8>)>> {
        loop {
            match self
                .decoder
                .next_transport_frame(self.decrypt.take()?)
                .ok()?
            {
                Decrypted::Frame(mut frame, state) => {
                    self.decrypt = Some(state);
                    return Some(Some((frame.header().msg_type(), frame.payload().to_vec())));
                }
                Decrypted::Incomplete(state) => {
                    self.decrypt = Some(state);
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Some(None);
                    }
                    self.stream.set_read_timeout(Some(left)).ok()?;
                    match self.stream.read(self.decoder.read_buf()) {
                        Ok(0) => return None,
                        Ok(n) => self.decoder.advance(n).ok()?,
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            self.decoder.advance(0).ok()?;
                            return Some(None);
                        }
                        Err(_) => return None,
                    }
                }
            }
        }
    }
}

struct Job {
    id: u32,
    merkle_root: Option<[u8; 32]>,
    path: Vec<[u8; 32]>,
    prev_hash: [u8; 32],
    ntime: u32,
}

fn serve(stream: TcpStream, share_difficulty: f64) -> Option<()> {
    let peer = stream.peer_addr().ok()?;
    let mut conn = Conn::handshake(stream)?;
    println!("{peer}: handshake done");
    let forever = Instant::now() + Duration::from_secs(3600);
    let (t, mut payload) = conn.recv(forever)??;
    let Ok(CommonMessages::SetupConnection(setup)) =
        CommonMessages::try_from((t, &mut payload[..]))
    else {
        println!("{peer}: expected SetupConnection, got {t:#x}");
        return None;
    };
    println!(
        "{peer}: setup flags {:#x}, vendor {:?}, hardware {:?}, firmware {:?}",
        setup.flags,
        String::from_utf8_lossy(setup.vendor.as_bytes()),
        String::from_utf8_lossy(setup.hardware_version.as_bytes()),
        String::from_utf8_lossy(setup.firmware.as_bytes())
    );
    conn.send(
        SetupConnectionSuccess {
            used_version: 2,
            flags: 0,
        },
        common_messages_sv2::MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
        false,
    )?;
    let target = target(share_difficulty);
    let (t, mut payload) = conn.recv(forever)??;
    let extended = match Mining::try_from((t, &mut payload[..])).ok()? {
        Mining::OpenExtendedMiningChannel(m) => {
            println!(
                "{peer}: extended channel for {:?}, {:.0} H/s",
                String::from_utf8_lossy(m.user_identity.as_bytes()),
                m.nominal_hash_rate
            );
            conn.send(
                OpenExtendedMiningChannelSuccess {
                    request_id: m.request_id,
                    channel_id: CHANNEL_ID,
                    target: U256::from(&target),
                    extranonce_size: EXTRANONCE_SIZE,
                    extranonce_prefix: B032::new(&EXTRANONCE_PREFIX).ok()?,
                    group_channel_id: 0,
                },
                mining_sv2::MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
                false,
            )?;
            true
        }
        Mining::OpenStandardMiningChannel(m) => {
            println!(
                "{peer}: standard channel for {:?}, {:.0} H/s",
                String::from_utf8_lossy(m.user_identity.as_bytes()),
                m.nominal_hash_rate
            );
            conn.send(
                OpenStandardMiningChannelSuccess {
                    request_id: m.request_id,
                    channel_id: CHANNEL_ID,
                    target: U256::from(&target),
                    extranonce_prefix: B032::new(&EXTRANONCE_PREFIX).ok()?,
                    group_channel_id: 0,
                },
                mining_sv2::MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL_SUCCESS,
                false,
            )?;
            false
        }
        _ => {
            println!("{peer}: expected a channel opening, got {t:#x}");
            return None;
        }
    };
    conn.send(
        SetTarget {
            channel_id: CHANNEL_ID,
            target: U256::from(&target),
        },
        mining_sv2::MESSAGE_TYPE_SET_TARGET,
        mining_sv2::CHANNEL_BIT_SET_TARGET,
    )?;

    let mut jobs: Vec<Job> = Vec::new();
    let mut job_id = 0u32;
    let mut prev_hash = [0u8; 32];
    let mut next_job = Instant::now();
    loop {
        if Instant::now() >= next_job {
            next_job += JOB_INTERVAL;
            job_id += 1;
            let new_block = (job_id - 1).is_multiple_of(JOBS_PER_BLOCK);
            let path = vec![[job_id as u8; 32], [0x5a; 32]];
            let merkle = (!extended).then(|| {
                let extranonce = [0u8; EXTRANONCE_SIZE as usize];
                merkle_root(
                    COINBASE_PREFIX,
                    &EXTRANONCE_PREFIX,
                    &extranonce,
                    COINBASE_SUFFIX,
                    &path,
                )
            });
            let ntime_start = (!new_block).then(now);
            let start = Sv2Option::new(ntime_start);
            if extended {
                let path_u256: Vec<U256> = path.iter().map(U256::from).collect();
                conn.send(
                    NewExtendedMiningJob {
                        channel_id: CHANNEL_ID,
                        job_id,
                        ntime_start: start,
                        version: VERSION,
                        version_rolling_allowed: true,
                        merkle_path: Seq0255::new(path_u256).ok()?,
                        coinbase_tx_prefix: B064K::new(COINBASE_PREFIX).ok()?,
                        coinbase_tx_suffix: B064K::new(COINBASE_SUFFIX).ok()?,
                    },
                    mining_sv2::MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
                    mining_sv2::CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
                )?;
            } else {
                conn.send(
                    NewMiningJob {
                        channel_id: CHANNEL_ID,
                        job_id,
                        ntime_start: start,
                        version: VERSION,
                        merkle_root: U256::from(merkle.as_ref()?),
                    },
                    mining_sv2::MESSAGE_TYPE_NEW_MINING_JOB,
                    mining_sv2::CHANNEL_BIT_NEW_MINING_JOB,
                )?;
            }
            if new_block {
                prev_hash = [job_id as u8; 32];
                prev_hash[31] = 0;
                jobs.clear();
                conn.send(
                    SetNewPrevHash {
                        channel_id: CHANNEL_ID,
                        job_id,
                        prev_hash: U256::from(&prev_hash),
                        ntime_start: now(),
                        nbits: NBITS,
                    },
                    mining_sv2::MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
                    mining_sv2::CHANNEL_BIT_MINING_SET_NEW_PREV_HASH,
                )?;
            }
            println!(
                "{peer}: job {job_id}{}",
                if new_block { " on a new block" } else { "" }
            );
            jobs.push(Job {
                id: job_id,
                merkle_root: merkle,
                path,
                prev_hash,
                ntime: now(),
            });
        }
        let Some((t, mut payload)) = conn.recv(next_job)? else {
            continue;
        };
        let (sequence, job, nonce, ntime, version, extranonce) =
            match Mining::try_from((t, &mut payload[..])) {
                Ok(Mining::SubmitSharesExtended(s)) => (
                    s.sequence_number,
                    s.job_id,
                    s.nonce,
                    s.ntime,
                    s.version,
                    s.extranonce.as_bytes().to_vec(),
                ),
                Ok(Mining::SubmitSharesStandard(s)) => (
                    s.sequence_number,
                    s.job_id,
                    s.nonce,
                    s.ntime,
                    s.version,
                    Vec::new(),
                ),
                _ => {
                    println!("{peer}: message {t:#x} ignored");
                    continue;
                }
            };
        let verdict = match jobs.iter().find(|j| j.id == job) {
            None => Err("stale-share"),
            Some(j) => {
                let root = j.merkle_root.unwrap_or_else(|| {
                    merkle_root(
                        COINBASE_PREFIX,
                        &EXTRANONCE_PREFIX,
                        &extranonce,
                        COINBASE_SUFFIX,
                        &j.path,
                    )
                });
                let header = BlockHeader {
                    version,
                    prev_blockhash: j.prev_hash,
                    merkle_root: root,
                    ntime,
                    nbits: NBITS,
                    nonce,
                };
                let d = difficulty(&header.hash());
                println!(
                    "{peer}: share for job {job}: difficulty {d:.4}, ntime {} s from the job",
                    ntime as i64 - j.ntime as i64
                );
                if d >= share_difficulty {
                    Ok(())
                } else {
                    Err("difficulty-too-low")
                }
            }
        };
        match verdict {
            Ok(()) => conn.send(
                SubmitSharesSuccess {
                    channel_id: CHANNEL_ID,
                    last_sequence_number: sequence,
                    new_submits_accepted_count: 1,
                    new_shares_sum: share_difficulty as u64,
                },
                mining_sv2::MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
                mining_sv2::CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
            )?,
            Err(code) => {
                println!("{peer}: share rejected: {code}");
                conn.send(
                    SubmitSharesError {
                        channel_id: CHANNEL_ID,
                        sequence_number: sequence,
                        error_code: Str0255::try_from(code).ok()?,
                    },
                    mining_sv2::MESSAGE_TYPE_SUBMIT_SHARES_ERROR,
                    mining_sv2::CHANNEL_BIT_SUBMIT_SHARES_ERROR,
                )?
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let address = args.next().unwrap_or_else(|| "0.0.0.0:34254".into());
    let share_difficulty: f64 = args.next().and_then(|d| d.parse().ok()).unwrap_or(0.001);
    let listener = TcpListener::bind(&address).expect("listen address");
    println!("stratum v2 pool on {address}, share difficulty {share_difficulty}, authority key {AUTHORITY_PUBLIC_KEY}");
    for stream in listener.incoming().flatten() {
        thread::spawn(move || {
            let peer = stream
                .peer_addr()
                .map(|p| p.to_string())
                .unwrap_or_default();
            serve(stream, share_difficulty);
            println!("{peer}: closed");
        });
    }
}
