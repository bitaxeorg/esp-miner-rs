//! Stratum V2 mining client, over any byte stream (`embedded-io-async`).
//!
//! [`Client::connect`] runs the Noise handshake (the pool's certificate is checked against its
//! authority key and the time), sets the connection up for mining with version rolling, and
//! opens one channel:
//!
//! - **standard**: the pool sends a merkle root per job; the chips get each job once and roll
//!   the version bits and the nonce over it;
//! - **extended**: the pool sends the coinbase around an extranonce and the merkle path; each
//!   [`Client::roll_job`] builds the coinbase with the next extranonce, like Stratum V1's
//!   extranonce2.
//!
//! Then [`Client::poll_message`] reads what the pool sends without waiting, [`Client::roll_job`]
//! gives jobs for the chips, and [`Client::submit`] sends their shares.

#[cfg(test)]
mod tests;
pub mod work;

use alloc::vec::Vec as AllocVec;

use binary_sv2::{GetSize, Serialize, Str0255, B032, U256};
use codec_sv2::{
    Decoded, Decrypted, Handshake, InitiatorSent, MessageFrame, NoiseDecoder, NoiseEncoder,
    TransportDecryptState, TransportEncryptState,
};
use common_messages_sv2::{
    Protocol, SetupConnection, CHANNEL_BIT_SETUP_CONNECTION, MESSAGE_TYPE_SETUP_CONNECTION,
};
use embedded_io_async::{Read, ReadReady, Write};
use heapless::{String, Vec};
use mining_sv2::{
    OpenExtendedMiningChannel, OpenStandardMiningChannel, SubmitSharesExtended,
    SubmitSharesStandard, CHANNEL_BIT_OPEN_EXTENDED_MINING_CHANNEL,
    CHANNEL_BIT_OPEN_STANDARD_MINING_CHANNEL, CHANNEL_BIT_SUBMIT_SHARES_EXTENDED,
    CHANNEL_BIT_SUBMIT_SHARES_STANDARD, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
    MESSAGE_TYPE_SUBMIT_SHARES_STANDARD,
};
use noise_sv2::{Initiator, INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE};
use parsers_sv2::{CommonMessages, Mining};
use rand_core::{CryptoRng, RngCore};

use crate::{
    header::difficulty,
    job::{MiningJob, EXTRANONCE_LEN_MAX},
    share::Share,
};
use work::{Job, Merkle, PrevHash, Work};

/// Protocol version spoken (the only one there is).
const PROTOCOL_VERSION: u16 = 2;
/// `SetupConnection` flags of the mining protocol.
const REQUIRES_STANDARD_JOBS: u32 = 1 << 0;
const REQUIRES_VERSION_ROLLING: u32 = 1 << 2;
/// `SetupConnectionSuccess` flag: the pool doesn't let the version be rolled.
const REQUIRES_FIXED_VERSION: u32 = 1 << 0;
/// Extranonce asked for an extended channel: 4 bytes, 2^32 coinbases per job.
const MIN_EXTRANONCE_SIZE: u16 = 4;
/// Common messages take the types below this one, the mining protocol the ones from it.
const FIRST_MINING_MESSAGE_TYPE: u8 = 0x10;
/// Request id of the channel opening.
const OPEN_REQUEST_ID: u32 = 1;

/// The kind of mining channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChannelType {
    Standard,
    Extended,
}

impl ChannelType {
    /// From the `stratumV2ChannelType` setting: `standard`, else extended.
    pub fn from_setting(setting: &str) -> Self {
        if setting.eq_ignore_ascii_case("standard") {
            Self::Standard
        } else {
            Self::Extended
        }
    }
}

/// What the client tells the pool about the connection and the miner.
#[derive(Debug, Clone)]
pub struct Config<'a> {
    /// The pool as the miner reached it.
    pub host: &'a str,
    pub port: u16,
    /// Account and worker the shares count for.
    pub user: &'a str,
    /// The pool's authority public key (x-only), which signs its certificate; `None` takes any
    /// pool key.
    pub authority_key: Option<[u8; 32]>,
    pub channel: ChannelType,
    /// Expected hashrate, in hashes per second.
    pub nominal_hashrate: f32,
    pub vendor: &'a str,
    pub hardware_version: &'a str,
    pub firmware: &'a str,
    pub device_id: &'a str,
}

/// Decodes an authority public key given in base58 (as pools publish it).
pub fn authority_key(base58: &str) -> Option<[u8; 32]> {
    base58
        .trim()
        .parse::<key_utils::Secp256k1PublicKey>()
        .ok()
        .map(|k| k.into_bytes())
}

/// Error codes are kept up to this length.
pub type Code = String<32>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The stream failed or was closed.
    Network,
    /// The Noise handshake failed: the pool key isn't signed by the authority key, its
    /// certificate isn't valid now, or a message is broken.
    Handshake,
    /// A frame could not be decrypted or decoded: the session can't go on.
    Codec,
    /// A message could not be encoded (a field too long).
    Encode,
    /// The pool refused the connection setup.
    SetupRejected(Code),
    /// The pool refused the channel.
    ChannelRejected(Code),
    /// The pool closed the channel.
    ChannelClosed(Code),
    /// A message type the connection setup did not expect.
    Unexpected(u8),
}

/// What a message from the pool changed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    /// There is new work: the jobs rolled from now on use it. `clean` when the jobs rolled
    /// before are stale (a new block, or a new extranonce prefix).
    Work { clean: bool },
    /// The difficulty shares must reach.
    Difficulty(f64),
    /// Shares accepted since the last answer.
    Accepted(u32),
    /// A share rejected, with the pool's error code.
    Rejected(Code),
    /// The pool asks to connect again (possibly elsewhere).
    Reconnect,
}

fn code(s: &Str0255) -> Code {
    let text = core::str::from_utf8(s.as_bytes()).unwrap_or("?");
    let mut code = Code::new();
    for c in text.chars() {
        if code.push(c).is_err() {
            break;
        }
    }
    code
}

fn str0255(s: &str) -> Result<Str0255<'_>, Error> {
    Str0255::try_from(s).map_err(|_| Error::Encode)
}

fn u256(v: &U256) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(&v.as_bytes()[..32]);
    out
}

async fn read_more<T: Read>(io: &mut T, decoder: &mut NoiseDecoder) -> Result<(), Error> {
    let n = io
        .read(decoder.read_buf())
        .await
        .map_err(|_| Error::Network)?;
    if n == 0 {
        return Err(Error::Network);
    }
    decoder.advance(n).map_err(|_| Error::Codec)
}

pub struct Client<T> {
    io: T,
    encoder: NoiseEncoder,
    decoder: NoiseDecoder,
    encrypt: TransportEncryptState,
    /// Taken while a frame is decoded; `None` after a failed decode.
    decrypt: Option<TransportDecryptState>,
    channel: ChannelType,
    channel_id: u32,
    group_channel_id: u32,
    target: [u8; 32],
    extranonce_prefix: Vec<u8, EXTRANONCE_LEN_MAX>,
    extranonce_size: usize,
    version_rolling: bool,
    sequence: u32,
    work: Work,
}

impl<T: Read + ReadReady + Write> Client<T> {
    /// Runs the handshake, the connection setup and the channel opening. `now` is the Unix time,
    /// to check the pool's certificate; `rng` gives the ephemeral key.
    pub async fn connect<R: RngCore + CryptoRng>(
        mut io: T,
        config: &Config<'_>,
        rng: &mut R,
        now: u32,
    ) -> Result<Self, Error> {
        let initiator = match config.authority_key {
            Some(key) => Initiator::from_raw_k_with_rng(key, rng),
            None => Initiator::without_pk_with_rng(rng),
        }
        .map_err(|_| Error::Handshake)?;
        let (message, handshake) = Handshake::initiator(initiator)
            .step_0()
            .map_err(|_| Error::Handshake)?;
        let mut encoder = NoiseEncoder::new();
        let mut decoder = NoiseDecoder::new();
        let out = encoder.encode_handshake(message);
        io.write_all(&out).await.map_err(|_| Error::Network)?;
        io.flush().await.map_err(|_| Error::Network)?;
        let reply = loop {
            match decoder
                .next_handshake_frame::<InitiatorSent>()
                .map_err(|_| Error::Handshake)?
            {
                Decoded::Frame(reply) => break reply,
                Decoded::Incomplete => read_more(&mut io, &mut decoder).await?,
            }
        };
        let reply: [u8; INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE] =
            reply.payload().try_into().map_err(|_| Error::Handshake)?;
        let (encrypt, decrypt) = handshake
            .step_2_with_now(reply, now)
            .map_err(|_| Error::Handshake)?
            .split();
        let mut client = Client {
            io,
            encoder,
            decoder,
            encrypt,
            decrypt: Some(decrypt),
            channel: config.channel,
            channel_id: 0,
            group_channel_id: 0,
            target: [0xff; 32],
            extranonce_prefix: Vec::new(),
            extranonce_size: 0,
            version_rolling: true,
            sequence: 0,
            work: Work::default(),
        };
        client.setup(config).await?;
        client.open_channel(config).await?;
        Ok(client)
    }

    async fn setup(&mut self, config: &Config<'_>) -> Result<(), Error> {
        let mut flags = REQUIRES_VERSION_ROLLING;
        if config.channel == ChannelType::Standard {
            flags |= REQUIRES_STANDARD_JOBS;
        }
        let setup = SetupConnection {
            protocol: Protocol::MiningProtocol,
            min_version: PROTOCOL_VERSION,
            max_version: PROTOCOL_VERSION,
            flags,
            endpoint_host: str0255(config.host)?,
            endpoint_port: config.port,
            vendor: str0255(config.vendor)?,
            hardware_version: str0255(config.hardware_version)?,
            firmware: str0255(config.firmware)?,
            device_id: str0255(config.device_id)?,
        };
        self.send(
            setup,
            MESSAGE_TYPE_SETUP_CONNECTION,
            CHANNEL_BIT_SETUP_CONNECTION,
        )
        .await?;
        let (msg_type, mut payload) = self.recv().await?;
        match CommonMessages::try_from((msg_type, &mut payload[..])) {
            Ok(CommonMessages::SetupConnectionSuccess(m)) => {
                self.version_rolling = m.flags & REQUIRES_FIXED_VERSION == 0;
                Ok(())
            }
            Ok(CommonMessages::SetupConnectionError(m)) => {
                Err(Error::SetupRejected(code(&m.error_code)))
            }
            _ => Err(Error::Unexpected(msg_type)),
        }
    }

    async fn open_channel(&mut self, config: &Config<'_>) -> Result<(), Error> {
        let user = Str0255::try_from(config.user).map_err(|_| Error::Encode)?;
        let max_target = [0xff; 32];
        match config.channel {
            ChannelType::Standard => {
                let open = OpenStandardMiningChannel {
                    request_id: OPEN_REQUEST_ID,
                    user_identity: user,
                    nominal_hash_rate: config.nominal_hashrate,
                    max_target: U256::from(&max_target),
                };
                self.send(
                    open,
                    MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL,
                    CHANNEL_BIT_OPEN_STANDARD_MINING_CHANNEL,
                )
                .await?;
            }
            ChannelType::Extended => {
                let open = OpenExtendedMiningChannel {
                    request_id: OPEN_REQUEST_ID,
                    user_identity: user,
                    nominal_hash_rate: config.nominal_hashrate,
                    max_target: U256::from(&max_target),
                    min_extranonce_size: MIN_EXTRANONCE_SIZE,
                };
                self.send(
                    open,
                    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
                    CHANNEL_BIT_OPEN_EXTENDED_MINING_CHANNEL,
                )
                .await?;
            }
        }
        let (msg_type, mut payload) = self.recv().await?;
        if msg_type < FIRST_MINING_MESSAGE_TYPE {
            return Err(Error::Unexpected(msg_type));
        }
        match Mining::try_from((msg_type, &mut payload[..])) {
            Ok(Mining::OpenStandardMiningChannelSuccess(m)) => {
                self.opened(
                    m.channel_id,
                    m.group_channel_id,
                    &m.target,
                    m.extranonce_prefix.as_bytes(),
                    0,
                );
                Ok(())
            }
            Ok(Mining::OpenExtendedMiningChannelSuccess(m)) => {
                self.opened(
                    m.channel_id,
                    m.group_channel_id,
                    &m.target,
                    m.extranonce_prefix.as_bytes(),
                    usize::from(m.extranonce_size),
                );
                Ok(())
            }
            Ok(Mining::OpenMiningChannelError(m)) => {
                Err(Error::ChannelRejected(code(&m.error_code)))
            }
            _ => Err(Error::Unexpected(msg_type)),
        }
    }

    fn opened(
        &mut self,
        channel_id: u32,
        group_channel_id: u32,
        target: &U256,
        extranonce_prefix: &[u8],
        extranonce_size: usize,
    ) {
        self.channel_id = channel_id;
        self.group_channel_id = group_channel_id;
        self.target = u256(target);
        self.extranonce_prefix = Vec::from_slice(extranonce_prefix).unwrap_or_default();
        self.extranonce_size = extranonce_size;
    }

    async fn send<M: Serialize + GetSize>(
        &mut self,
        message: M,
        msg_type: u8,
        channel_msg: bool,
    ) -> Result<(), Error> {
        let frame = MessageFrame::from_message(message, msg_type, 0, channel_msg)
            .map_err(|_| Error::Encode)?;
        let out = self
            .encoder
            .encode_transport(frame, &mut self.encrypt)
            .map_err(|_| Error::Encode)?;
        self.io.write_all(&out).await.map_err(|_| Error::Network)?;
        self.io.flush().await.map_err(|_| Error::Network)
    }

    /// The next frame, waiting for it: its message type and payload.
    async fn recv(&mut self) -> Result<(u8, AllocVec<u8>), Error> {
        loop {
            let state = self.decrypt.take().ok_or(Error::Codec)?;
            match self.decoder.next_transport_frame(state) {
                Ok(Decrypted::Frame(mut frame, state)) => {
                    self.decrypt = Some(state);
                    return Ok((frame.header().msg_type(), frame.payload().to_vec()));
                }
                Ok(Decrypted::Incomplete(state)) => {
                    self.decrypt = Some(state);
                    read_more(&mut self.io, &mut self.decoder).await?;
                }
                Err(_) => return Err(Error::Codec),
            }
        }
    }

    /// Handles the messages the pool sent, until one changes something; `Ok(None)` when there
    /// is nothing more to read now.
    pub async fn poll_message(&mut self) -> Result<Option<Event>, Error> {
        loop {
            let state = self.decrypt.take().ok_or(Error::Codec)?;
            match self.decoder.next_transport_frame(state) {
                Ok(Decrypted::Frame(mut frame, state)) => {
                    self.decrypt = Some(state);
                    let msg_type = frame.header().msg_type();
                    if let Some(event) = self.handle(msg_type, frame.payload())? {
                        return Ok(Some(event));
                    }
                }
                Ok(Decrypted::Incomplete(state)) => {
                    self.decrypt = Some(state);
                    if !self.io.read_ready().map_err(|_| Error::Network)? {
                        return Ok(None);
                    }
                    read_more(&mut self.io, &mut self.decoder).await?;
                }
                Err(_) => return Err(Error::Codec),
            }
        }
    }

    fn ours(&self, channel_id: u32) -> bool {
        channel_id == self.channel_id || channel_id == self.group_channel_id
    }

    fn handle(&mut self, msg_type: u8, payload: &mut [u8]) -> Result<Option<Event>, Error> {
        if msg_type < FIRST_MINING_MESSAGE_TYPE {
            return Ok(match CommonMessages::try_from((msg_type, payload)) {
                Ok(CommonMessages::Reconnect(_))
                | Ok(CommonMessages::ChannelEndpointChanged(_)) => Some(Event::Reconnect),
                _ => None,
            });
        }
        let Ok(message) = Mining::try_from((msg_type, payload)) else {
            return Ok(None);
        };
        Ok(match message {
            Mining::NewMiningJob(m) if self.ours(m.channel_id) => {
                let job = Job {
                    id: m.job_id,
                    version: m.version,
                    version_rolling: true,
                    merkle: Merkle::Root(u256(&m.merkle_root)),
                };
                let active = self.work.new_job(job, m.ntime_start.as_ref().copied());
                active.then_some(Event::Work { clean: false })
            }
            Mining::NewExtendedMiningJob(m) if self.ours(m.channel_id) => {
                let job = Job {
                    id: m.job_id,
                    version: m.version,
                    version_rolling: m.version_rolling_allowed,
                    merkle: Merkle::Coinbase {
                        prefix: m.coinbase_tx_prefix.as_bytes().to_vec(),
                        suffix: m.coinbase_tx_suffix.as_bytes().to_vec(),
                        path: m.merkle_path.iter().map(u256).collect(),
                    },
                };
                let active = self.work.new_job(job, m.ntime_start.as_ref().copied());
                active.then_some(Event::Work { clean: false })
            }
            Mining::SetNewPrevHash(m) if self.ours(m.channel_id) => {
                let active = self.work.new_prev_hash(PrevHash {
                    job_id: m.job_id,
                    hash: u256(&m.prev_hash),
                    ntime: m.ntime_start,
                    nbits: m.nbits,
                });
                active.then_some(Event::Work { clean: true })
            }
            Mining::SetTarget(m) if self.ours(m.channel_id) => {
                self.target = u256(&m.target);
                Some(Event::Difficulty(self.difficulty()))
            }
            Mining::SetExtranoncePrefix(m) if self.ours(m.channel_id) => {
                self.extranonce_prefix =
                    Vec::from_slice(m.extranonce_prefix.as_bytes()).unwrap_or_default();
                self.work
                    .active()
                    .is_some()
                    .then_some(Event::Work { clean: true })
            }
            Mining::SubmitSharesSuccess(m) => Some(Event::Accepted(m.new_submits_accepted_count)),
            Mining::SubmitSharesError(m) => Some(Event::Rejected(code(&m.error_code))),
            Mining::CloseChannel(m) if self.ours(m.channel_id) => {
                return Err(Error::ChannelClosed(code(&m.reason_code)))
            }
            _ => None,
        })
    }

    /// The next job for the chips: on an extended channel each one has a new extranonce, on a
    /// standard one the active job comes once. `None` without work.
    pub fn roll_job(&mut self) -> Option<MiningJob> {
        self.work
            .roll(&self.extranonce_prefix, self.extranonce_size)
    }

    /// Sends a share of a job from [`Self::roll_job`].
    pub async fn submit(&mut self, share: &Share) -> Result<(), Error> {
        let job_id = share.job_id.parse::<u32>().map_err(|_| Error::Encode)?;
        self.sequence = self.sequence.wrapping_add(1);
        match self.channel {
            ChannelType::Standard => {
                let submit = SubmitSharesStandard {
                    channel_id: self.channel_id,
                    sequence_number: self.sequence,
                    job_id,
                    nonce: share.nonce,
                    ntime: share.ntime,
                    version: share.version,
                };
                self.send(
                    submit,
                    MESSAGE_TYPE_SUBMIT_SHARES_STANDARD,
                    CHANNEL_BIT_SUBMIT_SHARES_STANDARD,
                )
                .await
            }
            ChannelType::Extended => {
                let extranonce = share.extranonce2.clone();
                let submit = SubmitSharesExtended {
                    channel_id: self.channel_id,
                    sequence_number: self.sequence,
                    job_id,
                    nonce: share.nonce,
                    ntime: share.ntime,
                    version: share.version,
                    extranonce: B032::new(&extranonce).map_err(|_| Error::Encode)?,
                };
                self.send(
                    submit,
                    MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
                    CHANNEL_BIT_SUBMIT_SHARES_EXTENDED,
                )
                .await
            }
        }
    }

    /// The difficulty of the channel's target.
    pub fn difficulty(&self) -> f64 {
        difficulty(&self.target)
    }

    /// The pool lets the version bits be rolled.
    pub fn version_rolling(&self) -> bool {
        self.version_rolling
    }

    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }

    /// The extranonce prefix the pool gave, and the size of the miner's part.
    pub fn extranonce(&self) -> (&[u8], usize) {
        (&self.extranonce_prefix, self.extranonce_size)
    }

    /// The active job and the block it builds on.
    pub fn active_job(&self) -> Option<(&Job, &PrevHash)> {
        self.work.active()
    }
}
