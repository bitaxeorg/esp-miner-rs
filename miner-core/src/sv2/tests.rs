//! The client against a pool made of the same crates' responder side, over TCP on the host.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use binary_sv2::{Seq0255, Sv2Option, B064K};
use codec_sv2::{
    Decoded, Decrypted, Handshake, MessageFrame, NoiseDecoder, NoiseEncoder, TransportDecryptState,
    TransportEncryptState,
};
use common_messages_sv2::{Reconnect, SetupConnectionSuccess};
use embedded_io_async::ErrorKind;
use mining_sv2::{
    NewExtendedMiningJob, NewMiningJob, OpenExtendedMiningChannelSuccess,
    OpenStandardMiningChannelSuccess, SetNewPrevHash, SetTarget, SubmitSharesError,
    SubmitSharesSuccess,
};
use noise_sv2::{Responder, ELLSWIFT_ENCODING_SIZE};
use parsers_sv2::{CommonMessages, Mining};

use super::*;
use crate::share::rolled_version;

const AUTHORITY_PUBLIC_KEY: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
const AUTHORITY_SECRET_KEY: &str = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";
const OTHER_PUBLIC_KEY: &str = "9bDuixKmZqAJnrmP746n8zU1wyAQRrus7th9dxnkPg6RzQvCnan";

const CHANNEL_ID: u32 = 7;
const PREFIX: [u8; 2] = [0xaa, 0xbb];
const COINBASE_PREFIX: [u8; 5] = [1, 2, 3, 4, 5];
const COINBASE_SUFFIX: [u8; 3] = [6, 7, 8];
const BRANCH: [u8; 32] = [9; 32];
const PREV_HASH: [u8; 32] = [0x11; 32];
const NBITS: u32 = 0x1703_0ecd;
const NTIME: u32 = 1_760_000_000;

fn now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

/// A blocking TCP stream behind the async traits.
struct Stream(TcpStream);

impl embedded_io_async::ErrorType for Stream {
    type Error = ErrorKind;
}

impl Read for Stream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ErrorKind> {
        self.0.read(buf).map_err(|_| ErrorKind::Other)
    }
}

impl Write for Stream {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, ErrorKind> {
        self.0.write(buf).map_err(|_| ErrorKind::Other)
    }

    async fn flush(&mut self) -> Result<(), ErrorKind> {
        self.0.flush().map_err(|_| ErrorKind::Other)
    }
}

impl ReadReady for Stream {
    fn read_ready(&mut self) -> Result<bool, ErrorKind> {
        self.0.set_nonblocking(true).unwrap();
        let ready = match self.0.peek(&mut [0]) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(_) => true,
        };
        self.0.set_nonblocking(false).unwrap();
        Ok(ready)
    }
}

/// The pool side of a connection, after the handshake.
struct Pool {
    stream: TcpStream,
    encoder: NoiseEncoder,
    decoder: NoiseDecoder,
    encrypt: TransportEncryptState,
    decrypt: Option<TransportDecryptState>,
}

impl Pool {
    /// Accepts a connection and answers its handshake with a certificate valid around now.
    fn accept(listener: &TcpListener) -> Option<Self> {
        let (mut stream, _) = listener.accept().unwrap();
        let public: key_utils::Secp256k1PublicKey = AUTHORITY_PUBLIC_KEY.parse().unwrap();
        let secret: key_utils::Secp256k1SecretKey = AUTHORITY_SECRET_KEY.parse().unwrap();
        let responder = Responder::from_authority_kp(
            &public.into_bytes(),
            &secret.into_bytes(),
            Duration::from_secs(3600),
        )
        .unwrap();
        let mut encoder = NoiseEncoder::new();
        let mut decoder = NoiseDecoder::new();
        let first = loop {
            match decoder.next_handshake_frame::<Responder>().unwrap() {
                Decoded::Frame(m) => break m,
                Decoded::Incomplete => {
                    let n = stream.read(decoder.read_buf()).ok()?;
                    if n == 0 {
                        return None;
                    }
                    decoder.advance(n).unwrap();
                }
            }
        };
        let first: [u8; ELLSWIFT_ENCODING_SIZE] = first.payload().try_into().unwrap();
        let (reply, transport) = Handshake::responder(responder)
            .step_1_with_now_rng(first, now(), &mut rand::thread_rng())
            .unwrap();
        stream.write_all(&encoder.encode_handshake(reply)).unwrap();
        let (encrypt, decrypt) = transport.split();
        Some(Pool {
            stream,
            encoder,
            decoder,
            encrypt,
            decrypt: Some(decrypt),
        })
    }

    fn send<M: Serialize + GetSize>(&mut self, message: M, msg_type: u8, channel_msg: bool) {
        let frame = MessageFrame::from_message(message, msg_type, 0, channel_msg).unwrap();
        let out = self
            .encoder
            .encode_transport(frame, &mut self.encrypt)
            .unwrap();
        self.stream.write_all(&out).unwrap();
    }

    /// The next message: its type and payload.
    fn recv(&mut self) -> (u8, std::vec::Vec<u8>) {
        loop {
            match self
                .decoder
                .next_transport_frame(self.decrypt.take().unwrap())
                .unwrap()
            {
                Decrypted::Frame(mut frame, state) => {
                    self.decrypt = Some(state);
                    return (frame.header().msg_type(), frame.payload().to_vec());
                }
                Decrypted::Incomplete(state) => {
                    self.decrypt = Some(state);
                    let n = self.stream.read(self.decoder.read_buf()).unwrap();
                    assert_ne!(n, 0, "the miner closed the connection");
                    self.decoder.advance(n).unwrap();
                }
            }
        }
    }

    /// Answers the setup and the channel opening, then checks they asked for `channel`.
    fn open(&mut self, channel: ChannelType) {
        let (t, mut payload) = self.recv();
        let Ok(CommonMessages::SetupConnection(setup)) =
            CommonMessages::try_from((t, &mut payload[..]))
        else {
            panic!("expected SetupConnection, got message type {t:#x}");
        };
        let standard = channel == ChannelType::Standard;
        assert_eq!(
            setup.flags,
            REQUIRES_VERSION_ROLLING | if standard { REQUIRES_STANDARD_JOBS } else { 0 }
        );
        assert_eq!(setup.endpoint_host.as_bytes(), b"pool.example");
        self.send(
            SetupConnectionSuccess {
                used_version: 2,
                flags: 0,
            },
            common_messages_sv2::MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
            false,
        );
        let (t, mut payload) = self.recv();
        let target = [0xffu8; 32];
        match Mining::try_from((t, &mut payload[..])).unwrap() {
            Mining::OpenExtendedMiningChannel(open) => {
                assert!(!standard);
                assert_eq!(open.user_identity.as_bytes(), b"worker.1");
                assert_eq!(open.min_extranonce_size, MIN_EXTRANONCE_SIZE);
                self.send(
                    OpenExtendedMiningChannelSuccess {
                        request_id: open.request_id,
                        channel_id: CHANNEL_ID,
                        target: U256::from(&target),
                        extranonce_size: 4,
                        extranonce_prefix: B032::new(&PREFIX).unwrap(),
                        group_channel_id: 1,
                    },
                    mining_sv2::MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
                    false,
                );
            }
            Mining::OpenStandardMiningChannel(open) => {
                assert!(standard);
                self.send(
                    OpenStandardMiningChannelSuccess {
                        request_id: open.request_id,
                        channel_id: CHANNEL_ID,
                        target: U256::from(&target),
                        extranonce_prefix: B032::new(&PREFIX).unwrap(),
                        group_channel_id: 1,
                    },
                    mining_sv2::MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL_SUCCESS,
                    false,
                );
            }
            _ => panic!("expected a channel opening, got message type {t:#x}"),
        }
    }

    /// A job for the next block, then the block.
    fn work(&mut self, channel: ChannelType, job_id: u32) {
        match channel {
            ChannelType::Extended => self.send(
                NewExtendedMiningJob {
                    channel_id: CHANNEL_ID,
                    job_id,
                    ntime_start: Sv2Option::new(None),
                    version: 0x2000_0000,
                    version_rolling_allowed: true,
                    merkle_path: Seq0255::new(vec![U256::from(&BRANCH)]).unwrap(),
                    coinbase_tx_prefix: B064K::new(&COINBASE_PREFIX).unwrap(),
                    coinbase_tx_suffix: B064K::new(&COINBASE_SUFFIX).unwrap(),
                },
                mining_sv2::MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
                mining_sv2::CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
            ),
            ChannelType::Standard => self.send(
                NewMiningJob {
                    channel_id: CHANNEL_ID,
                    job_id,
                    ntime_start: Sv2Option::new(None),
                    version: 0x2000_0000,
                    merkle_root: U256::from(&BRANCH),
                },
                mining_sv2::MESSAGE_TYPE_NEW_MINING_JOB,
                mining_sv2::CHANNEL_BIT_NEW_MINING_JOB,
            ),
        }
        self.send(
            SetNewPrevHash {
                channel_id: CHANNEL_ID,
                job_id,
                prev_hash: U256::from(&PREV_HASH),
                ntime_start: NTIME,
                nbits: NBITS,
            },
            mining_sv2::MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
            mining_sv2::CHANNEL_BIT_MINING_SET_NEW_PREV_HASH,
        );
    }
}

/// Polls until an event comes.
async fn event<T: Read + ReadReady + Write>(client: &mut Client<T>) -> Event {
    loop {
        if let Some(event) = client.poll_message().await.unwrap() {
            return event;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn config(channel: ChannelType, key: &str) -> Config<'static> {
    Config {
        host: "pool.example",
        port: 3333,
        user: "worker.1",
        authority_key: authority_key(key),
        channel,
        nominal_hashrate: 1.0e12,
        vendor: "test",
        hardware_version: "test",
        firmware: "test",
        device_id: "test",
    }
}

fn connect(listener: &TcpListener, config: &Config<'_>, now: u32) -> Result<Client<Stream>, Error> {
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    embassy_futures::block_on(Client::connect(
        Stream(stream),
        config,
        &mut rand::thread_rng(),
        now,
    ))
}

#[test]
fn authority_keys_are_base58() {
    assert!(authority_key(AUTHORITY_PUBLIC_KEY).is_some());
    assert!(authority_key(" 9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72\n").is_some());
    assert_eq!(authority_key("not a key"), None);
    assert_eq!(authority_key(""), None);
}

#[test]
fn an_extended_channel_mines_and_submits() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let pool = thread::spawn({
        let listener = listener.try_clone().unwrap();
        move || {
            let mut pool = Pool::accept(&listener).unwrap();
            pool.open(ChannelType::Extended);
            pool.send(
                SetTarget {
                    channel_id: CHANNEL_ID,
                    target: U256::from(&[0xff; 32]),
                },
                mining_sv2::MESSAGE_TYPE_SET_TARGET,
                mining_sv2::CHANNEL_BIT_SET_TARGET,
            );
            pool.work(ChannelType::Extended, 42);
            // two shares: the first accepted, the second rejected
            for (i, accept) in [true, false].into_iter().enumerate() {
                let (t, mut payload) = pool.recv();
                let Ok(Mining::SubmitSharesExtended(share)) =
                    Mining::try_from((t, &mut payload[..]))
                else {
                    panic!("expected SubmitSharesExtended, got message type {t:#x}");
                };
                assert_eq!(
                    (share.channel_id, share.sequence_number, share.job_id),
                    (CHANNEL_ID, i as u32 + 1, 42)
                );
                assert_eq!(share.extranonce.as_bytes(), &[i as u8, 0, 0, 0]);
                assert_eq!(
                    (share.ntime, share.nonce, share.version),
                    (NTIME, 0x1234_5678, 0x2000_e000)
                );
                if accept {
                    pool.send(
                        SubmitSharesSuccess {
                            channel_id: CHANNEL_ID,
                            last_sequence_number: share.sequence_number,
                            new_submits_accepted_count: 1,
                            new_shares_sum: 1,
                        },
                        mining_sv2::MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
                        mining_sv2::CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
                    );
                } else {
                    pool.send(
                        SubmitSharesError {
                            channel_id: CHANNEL_ID,
                            sequence_number: share.sequence_number,
                            error_code: Str0255::try_from("difficulty-too-low").unwrap(),
                        },
                        mining_sv2::MESSAGE_TYPE_SUBMIT_SHARES_ERROR,
                        mining_sv2::CHANNEL_BIT_SUBMIT_SHARES_ERROR,
                    );
                }
            }
            pool.send(
                Reconnect {
                    new_host: Str0255::try_from("").unwrap(),
                    new_port: 0,
                },
                common_messages_sv2::MESSAGE_TYPE_RECONNECT,
                common_messages_sv2::CHANNEL_BIT_RECONNECT,
            );
        }
    });

    let mut client = connect(
        &listener,
        &config(ChannelType::Extended, AUTHORITY_PUBLIC_KEY),
        now(),
    )
    .unwrap();
    assert_eq!(client.channel_id(), CHANNEL_ID);
    assert_eq!(client.extranonce(), (&PREFIX[..], 4));
    assert!(client.version_rolling());
    embassy_futures::block_on(async {
        assert_eq!(
            event(&mut client).await,
            Event::Difficulty(client.difficulty())
        );
        assert!(client.difficulty() < 1.0);
        assert_eq!(event(&mut client).await, Event::Work { clean: true });
        for i in 0..2u8 {
            let job = client.roll_job().unwrap();
            assert_eq!(job.pool_job_id.as_str(), "42");
            assert_eq!(job.extranonce2.as_slice(), &[i, 0, 0, 0]);
            assert_eq!(
                (job.prev_blockhash, job.ntime, job.nbits),
                (PREV_HASH, NTIME, NBITS)
            );
            // the merkle root the pool computes from what the share carries
            let coinbase = [
                &COINBASE_PREFIX[..],
                &PREFIX,
                &[i, 0, 0, 0],
                &COINBASE_SUFFIX,
            ]
            .concat();
            assert_eq!(
                job.merkle_root,
                work::merkle_root(&coinbase, &[], &[], &[], &[BRANCH])
            );
            let checked =
                crate::share::check_nonce(&job, 0x1234_5678, Some(0x0000_e000), 0x1fff_e000);
            assert_eq!(
                checked.version,
                rolled_version(0x2000_0000, 0xe000, 0x1fff_e000)
            );
            let share = Share {
                job_id: job.pool_job_id.clone(),
                extranonce2: job.extranonce2.clone(),
                ntime: job.ntime,
                nonce: 0x1234_5678,
                version: checked.version,
                version_bits: checked.version_bits,
            };
            client.submit(&share).await.unwrap();
        }
        assert_eq!(event(&mut client).await, Event::Accepted(1));
        assert_eq!(
            event(&mut client).await,
            Event::Rejected(Code::try_from("difficulty-too-low").unwrap())
        );
        assert_eq!(event(&mut client).await, Event::Reconnect);
    });
    pool.join().unwrap();
}

#[test]
fn a_standard_channel_gives_each_job_once() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let pool = thread::spawn({
        let listener = listener.try_clone().unwrap();
        move || {
            let mut pool = Pool::accept(&listener).unwrap();
            pool.open(ChannelType::Standard);
            pool.work(ChannelType::Standard, 5);
            let (t, mut payload) = pool.recv();
            let Ok(Mining::SubmitSharesStandard(share)) = Mining::try_from((t, &mut payload[..]))
            else {
                panic!("expected SubmitSharesStandard, got message type {t:#x}");
            };
            assert_eq!(
                (share.channel_id, share.job_id, share.nonce),
                (CHANNEL_ID, 5, 1)
            );
        }
    });

    let mut client = connect(
        &listener,
        &config(ChannelType::Standard, AUTHORITY_PUBLIC_KEY),
        now(),
    )
    .unwrap();
    embassy_futures::block_on(async {
        assert_eq!(event(&mut client).await, Event::Work { clean: true });
        let job = client.roll_job().unwrap();
        assert_eq!((job.merkle_root, job.extranonce2.len()), (BRANCH, 0));
        assert!(client.roll_job().is_none());
        let share = Share {
            job_id: job.pool_job_id.clone(),
            extranonce2: job.extranonce2.clone(),
            ntime: job.ntime,
            nonce: 1,
            version: job.version,
            version_bits: None,
        };
        client.submit(&share).await.unwrap();
    });
    pool.join().unwrap();
}

#[test]
fn the_certificate_must_be_signed_by_the_authority_key_and_valid_now() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let pool = thread::spawn({
        let listener = listener.try_clone().unwrap();
        move || {
            // each handshake fails on the miner's side, which closes the connection
            for _ in 0..2 {
                let _ = Pool::accept(&listener);
            }
        }
    });
    // a valid key, of another authority
    let other = authority_key(OTHER_PUBLIC_KEY).unwrap();
    assert!(Initiator::from_raw_k_with_rng(other, &mut rand::thread_rng()).is_ok());
    let wrong_key = connect(
        &listener,
        &config(ChannelType::Extended, OTHER_PUBLIC_KEY),
        now(),
    );
    assert_eq!(wrong_key.err(), Some(Error::Handshake));
    let later = connect(
        &listener,
        &config(ChannelType::Extended, AUTHORITY_PUBLIC_KEY),
        now() + 7200,
    );
    assert_eq!(later.err(), Some(Error::Handshake));
    pool.join().unwrap();
}
