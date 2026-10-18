//! GPT-Live on a ChatGPT login, placed the way the Codex app places it: a
//! WebRTC call created at `chatgpt.com/backend-api/codex/realtime/calls` with
//! the login's bearer, joined by a sideband socket at
//! `wss://api.openai.com/v1/live/<call id>`. No API key is involved.
//!
//! The desk is the call's peer. The person's audio goes up as Opus over RTP,
//! 20 ms packets at 48 kHz, paced by the driver and filled with silence when
//! the microphone is quiet or held, so the peer never starves. Everything
//! else rides the sideband: the model's speech as 24 kHz PCM, what each side
//! said, and the model's hand-offs. GPT-Live has no session tools: it hands
//! every request to the client as a `delegation`, which this adapter reports
//! as the `send_to_teammate` call the bridge already answers, and the caller
//! runs any desk-side tools for it ([`Realtime::delegates`]). The sign-in
//! cannot use a `responses` backend: the service refuses its validation
//! connection (HTTP 401). A line for the person
//! goes in on the `speakable` channel and is said as given, so it can never be
//! taken for the person's words. The server stops a reply itself when the
//! person talks; the adapter reports the person's new turn so the caller can
//! cut what it is playing.
//!
//! A session lasts an hour, and a call outlives it: five minutes before the
//! end, a replacement call is placed in the background with the conversation
//! so far in its instructions, and the driver switches to it between replies
//! (or after half a minute of a reply), then hangs up the old one. A sideband
//! that drops is rejoined; a call that has ended, or a peer that fails, ends
//! the session, and the caller carries on without a front.

use super::socket::Carry;
use super::{Command, Realtime, RealtimeConfig, RealtimeEvent, RealtimeInput, RealtimeSession};
use crate::voice::front::SEND_TO_TEAMMATE;
use crate::voice::speech::{SpeechError, SpeechId};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use rtc::media::Sample;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use webrtc::media_stream::MediaStreamTrack;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCIceGatheringState, RTCPeerConnectionState, RTCSessionDescription,
};

/// The ChatGPT login's provider id.
pub const PROVIDER_ID: &str = "openai-codex";
pub const MODEL: &str = "gpt-live-1-codex";
/// The voices GPT-Live speaks in, the default first.
pub const VOICES: [&str; 8] = [
    "cove", "juniper", "maple", "spruce", "ember", "vale", "breeze", "sol",
];
const CALLS_URL: &str =
    "https://chatgpt.com/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas";
const SIDEBAND_URL: &str = "wss://api.openai.com/v1/live";
/// The protocol generation the sideband speaks.
const ALPHA: &str = "quicksilver=v2";
/// The rate the call is heard and spoken at on the desk's side.
const RATE: u32 = 24_000;
const OPUS_RATE: u32 = 48_000;
const OPUS_PAYLOAD: u8 = 111;
/// One RTP packet: 20 ms at 48 kHz.
const PACKET_SAMPLES: usize = 960;
const PACKET: Duration = Duration::from_millis(20);
/// Microphone audio held for the pacer before the oldest is dropped: half a
/// second, so a stall never turns into lag.
const MIC_BACKLOG: usize = OPUS_RATE as usize / 2;
/// The sideband takes context in pieces of at most this many bytes.
const CONTEXT_BYTES: usize = 500;
/// A speech chunk whose loudest sample is below this is silence; the sideband
/// streams silence between replies, and none of it is the model speaking.
const SILENT_PEAK: i32 = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const REJOINS: u32 = 3;
/// Lines wait for the session to start, or this long at most.
const START_WAIT: Duration = Duration::from_secs(5);
/// A replacement call is placed this long into a session, or five minutes
/// before the end the server gives, whichever is sooner.
const RENEW_AFTER: Duration = Duration::from_mins(55);
const RENEW_EARLY: Duration = Duration::from_mins(5);
/// A replacement waits for the model to finish speaking, this long at most.
const SWITCH_WAIT: Duration = Duration::from_secs(30);
/// A replacement that could not be placed is tried again after this.
const RENEW_RETRY: Duration = Duration::from_secs(60);
const REJOIN_WAIT: Duration = Duration::from_millis(200);
/// One sideband event, a 200 ms speech chunk at most in practice.
const MESSAGE_LIMIT: usize = 4 << 20;
const COMMAND_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 256;

type Sideband = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Who the call is placed as: the login's bearer and its account.
struct Identity {
    token: String,
    account: Option<String>,
    session: String,
}

#[derive(Clone)]
pub struct GptLive {
    /// The ChatGPT login's directory, refreshed before each call.
    token_dir: PathBuf,
    calls_url: String,
    sideband_url: String,
    renew_after: Duration,
}

/// One placed call: who placed it, its id, its peer and its sideband.
struct Placed {
    who: Identity,
    call: String,
    peer: Peer,
    sideband: Sideband,
}

impl GptLive {
    pub fn new(token_dir: PathBuf) -> Self {
        Self {
            token_dir,
            calls_url: CALLS_URL.into(),
            sideband_url: SIDEBAND_URL.into(),
            renew_after: RENEW_AFTER,
        }
    }

    fn malformed(&self) -> SpeechError {
        SpeechError::Malformed {
            provider_id: PROVIDER_ID.into(),
        }
    }

    fn unreachable(&self) -> SpeechError {
        SpeechError::Unreachable {
            provider_id: PROVIDER_ID.into(),
        }
    }

    fn status_error(&self, status: u16) -> SpeechError {
        let provider_id = PROVIDER_ID.to_string();
        match status {
            401 => SpeechError::SignInRequired { provider_id },
            402 | 403 | 429 => SpeechError::Entitlement {
                provider_id,
                status,
            },
            _ => SpeechError::Refused {
                provider_id,
                status,
            },
        }
    }

    async fn identity(&self) -> Result<Identity, SpeechError> {
        let record = timeout(
            CONNECT_TIMEOUT,
            crate::providers::chatgpt::refreshed_auth(&self.token_dir),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| SpeechError::SignInRequired {
            provider_id: PROVIDER_ID.into(),
        })?;
        Ok(Identity {
            token: record.access_token.unwrap_or_default(),
            account: record.account_id.filter(|id| !id.is_empty()),
            session: uuid::Uuid::new_v4().to_string(),
        })
    }

    /// Creates the call from the peer's offer: the answer and the call's id.
    async fn create(
        &self,
        who: &Identity,
        offer: String,
        instructions: &str,
        voice: &str,
        delegation: Value,
    ) -> Result<(String, String), SpeechError> {
        let session = json!({
            "model": MODEL,
            "instructions": instructions,
            "audio": {"output": {"voice": voice}},
            "delegation": delegation,
        });
        let http = crate::voice::speech::http_client().map_err(|_| self.unreachable())?;
        let mut request = http
            .post(&self.calls_url)
            .bearer_auth(&who.token)
            .header("openai-alpha", ALPHA)
            .header("originator", "codex_cli_rs")
            .header("session-id", &who.session)
            .header("x-session-id", &who.session)
            .json(&json!({"sdp": offer, "session": session}));
        if let Some(account) = &who.account {
            request = request.header("ChatGPT-Account-ID", account);
        }
        let response = timeout(CONNECT_TIMEOUT, request.send())
            .await
            .map_err(|_| self.unreachable())?
            .map_err(|_| self.unreachable())?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.status_error(status.as_u16()));
        }
        let call = response
            .headers()
            .get(http::header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(call_id)
            .ok_or_else(|| self.malformed())?;
        let answer = response.text().await.map_err(|_| self.malformed())?;
        Ok((answer, call))
    }

    /// Places one call with `config`, its instructions extended by the
    /// conversation so far when it replaces an earlier one.
    async fn place(&self, config: &RealtimeConfig, carry: &Carry) -> Result<Placed, SpeechError> {
        let who = self.identity().await?;
        let peer = Peer::new().await.map_err(|()| self.unreachable())?;
        let offer = if let Ok(offer) = peer.offer().await {
            offer
        } else {
            peer.close().await;
            return Err(self.unreachable());
        };
        let instructions = carry.instructions(&config.instructions);
        let instructions = self::instructions(&instructions);
        let delegation = json!({"type": "client"});
        // A server that fails or cannot be reached is asked again, as Codex
        // does; a refused login or plan will not change.
        let mut wait = REJOIN_WAIT;
        let mut attempt = 0;
        let (answer, call) = loop {
            match self
                .create(
                    &who,
                    offer.clone(),
                    &instructions,
                    &config.voice,
                    delegation.clone(),
                )
                .await
            {
                Ok(created) => break created,
                Err(
                    SpeechError::Unreachable { .. } | SpeechError::Refused { status: 500.., .. },
                ) if attempt < REJOINS => {
                    attempt += 1;
                    sleep(wait).await;
                    wait *= 2;
                }
                Err(error) => {
                    peer.close().await;
                    return Err(error);
                }
            }
        };
        if peer.answer(answer).await.is_err() {
            peer.close().await;
            return Err(self.malformed());
        }
        match self.join(&who, &call).await {
            Ok(sideband) => Ok(Placed {
                who,
                call,
                peer,
                sideband,
            }),
            Err(error) => {
                peer.close().await;
                Err(error)
            }
        }
    }

    async fn join(&self, who: &Identity, call: &str) -> Result<Sideband, SpeechError> {
        crate::desk::install_crypto_provider();
        let mut request = format!("{}/{call}", self.sideband_url)
            .into_client_request()
            .map_err(|_| self.malformed())?;
        let headers = request.headers_mut();
        let mut bearer = http::HeaderValue::from_str(&format!("Bearer {}", who.token))
            .map_err(|_| self.malformed())?;
        bearer.set_sensitive(true);
        headers.insert(http::header::AUTHORIZATION, bearer);
        for (name, value) in [
            ("openai-alpha", Some(ALPHA)),
            ("originator", Some("codex_cli_rs")),
            ("x-session-id", Some(who.session.as_str())),
            ("chatgpt-account-id", who.account.as_deref()),
        ] {
            if let Some(value) = value {
                headers.insert(
                    name,
                    http::HeaderValue::from_str(value).map_err(|_| self.malformed())?,
                );
            }
        }
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MESSAGE_LIMIT))
            .max_frame_size(Some(MESSAGE_LIMIT));
        let (socket, _) = timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(request, Some(config), false),
        )
        .await
        .map_err(|_| self.unreachable())?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                self.status_error(response.status().as_u16())
            }
            _ => self.unreachable(),
        })?;
        Ok(socket)
    }
}

#[async_trait]
impl Realtime for GptLive {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: PROVIDER_ID.into(),
            model_id: MODEL.into(),
            voice: None,
        }
    }

    fn is_subscription(&self) -> bool {
        true
    }

    fn delegates(&self) -> bool {
        true
    }

    async fn open(&self, config: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
        for rate in [config.input_rate, config.output_rate] {
            if rate != RATE {
                return Err(SpeechError::UnsupportedFormat(format!("PCM at {rate} Hz")));
            }
        }
        let placed = self.place(&config, &Carry::default()).await?;
        let (commands, command_queue) = mpsc::channel(COMMAND_QUEUE);
        let (event_queue, events) = mpsc::channel(EVENT_QUEUE);
        let Placed {
            who,
            call,
            peer,
            sideband,
        } = placed;
        let now = Instant::now();
        let driver = Driver {
            live: self.clone(),
            config,
            who,
            call,
            peer,
            events: event_queue,
            mic: Mic::default(),
            heard: Heard::default(),
            held: Some(Vec::new()),
            start_by: now + START_WAIT,
            carry: Carry::default(),
            saying: String::new(),
            renew_at: now + self.renew_after,
            placing: None,
            ready: None,
        };
        tokio::spawn(driver.run(sideband, command_queue));
        Ok(RealtimeSession {
            input: RealtimeInput { commands },
            events,
        })
    }
}

/// The front's instructions, with how this model reaches its tools: GPT-Live
/// delegates rather than calling them, and the answer comes back to say.
fn instructions(base: &str) -> String {
    format!(
        "{base}\n\nIn this call you reach your tools and your working session by \
         delegating: every mention of send_to_teammate or of another tool means \
         delegating the person's request. What a delegation finds comes back to you \
         to tell them."
    )
}

/// The call's id out of the `Location` the call was created at: the last
/// segment that is an `rtc_` id or a UUID.
fn call_id(location: &str) -> Option<String> {
    location
        .split('?')
        .next()?
        .rsplit('/')
        .find(|segment| {
            (segment.starts_with("rtc_") && segment.len() > 4)
                || uuid::Uuid::try_parse(segment).is_ok()
        })
        .filter(|segment| {
            segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
        .map(str::to_string)
}

/// A line for the speakable channel, in pieces the sideband takes, split on
/// character boundaries.
fn context_pieces(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for character in text.chars() {
        if piece.len() + character.len_utf8() > CONTEXT_BYTES {
            pieces.push(std::mem::take(&mut piece));
        }
        piece.push(character);
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

fn silent(pcm: &[u8]) -> bool {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .all(|pair| i32::from(i16::from_le_bytes(*pair)).abs() < SILENT_PEAK)
}

/// What the sideband says, turned into the call's events.
#[derive(Default)]
struct Heard {
    /// The model is in a reply, so its silence is part of it.
    speaking: bool,
    /// The person has started a turn the caller was told of.
    person: bool,
}

impl Heard {
    fn event(&mut self, event: &Value) -> Result<Vec<RealtimeEvent>, ()> {
        let text = |value: &Value| value.as_str().map(str::to_string).ok_or(());
        Ok(match event["type"].as_str().ok_or(())? {
            "session.output_audio.delta" | "output_audio.delta" => {
                let delta = event["delta"]
                    .as_str()
                    .or(event["audio"].as_str())
                    .ok_or(())?;
                let pcm = STANDARD.decode(delta).map_err(|_| ())?;
                if !pcm.len().is_multiple_of(2) {
                    return Err(());
                }
                if pcm.is_empty() || (!self.speaking && silent(&pcm)) {
                    Vec::new()
                } else {
                    vec![RealtimeEvent::Audio(pcm)]
                }
            }
            "output_transcript.added" => {
                self.speaking = true;
                vec![RealtimeEvent::Saying(text(&event["item"]["text"])?)]
            }
            "turn.created" if event["turn"]["role"] == "assistant" => {
                self.speaking = true;
                Vec::new()
            }
            "turn.created" | "input_transcript.added" if !self.person => {
                // The person talks: the server has stopped the model, and
                // the caller stops what it is still playing.
                self.person = true;
                vec![RealtimeEvent::SpeechStarted]
            }
            "turn.done" => {
                let turn = &event["turn"];
                let words = turn["transcript"].as_str().unwrap_or_default().trim();
                if turn["role"] == "assistant" {
                    self.speaking = false;
                    vec![RealtimeEvent::ResponseDone]
                } else {
                    self.person = false;
                    if words.is_empty() {
                        Vec::new()
                    } else {
                        vec![RealtimeEvent::Heard(words.into())]
                    }
                }
            }
            "delegation.created" => {
                let item = &event["item"];
                if item["target"] != "client" {
                    return Ok(Vec::new());
                }
                let words: Vec<&str> = item["content"]
                    .as_array()
                    .ok_or(())?
                    .iter()
                    .filter(|part| part["type"] == "input_text")
                    .filter_map(|part| part["text"].as_str())
                    .collect();
                vec![RealtimeEvent::ToolCall {
                    call_id: text(&item["id"])?,
                    name: SEND_TO_TEAMMATE.into(),
                    arguments: json!({"words": words.join(" ")}).to_string(),
                }]
            }
            "error" => {
                // Which setting or field was refused, and the provider's
                // sentence about it, so a refusal can be fixed: never the
                // transcript, which the error does not carry.
                let error = &event["error"];
                eprintln!(
                    "[voice] GPT-Live refused: {} param={} {}",
                    error["code"].as_str().unwrap_or("?"),
                    error["param"].as_str().unwrap_or("-"),
                    crate::driver::clip(error["message"].as_str().unwrap_or(""), 300),
                );
                vec![RealtimeEvent::Refused {
                    code: event["error"]["code"]
                        .as_str()
                        .filter(|code| {
                            code.len() <= 64
                                && code.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric()
                                        || matches!(byte, b'_' | b'.' | b'-')
                                })
                        })
                        .map(str::to_string),
                }]
            }
            _ => Vec::new(),
        })
    }
}

/// The person's audio waiting for the pacer, at 48 kHz.
#[derive(Default)]
struct Mic {
    samples: VecDeque<i16>,
    /// The last sample of the previous frame, to interpolate across frames.
    last: i16,
}

impl Mic {
    /// 24 kHz PCM16 in, doubled to 48 kHz by interpolation.
    fn push(&mut self, pcm: &[u8]) {
        for pair in pcm.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(*pair);
            let between = ((i32::from(self.last) + i32::from(sample)) / 2) as i16;
            self.samples.push_back(between);
            self.samples.push_back(sample);
            self.last = sample;
        }
        while self.samples.len() > MIC_BACKLOG {
            self.samples.pop_front();
        }
    }

    /// The next 20 ms, or silence when there is not enough.
    fn packet(&mut self) -> [i16; PACKET_SAMPLES] {
        let mut packet = [0; PACKET_SAMPLES];
        if self.samples.len() >= PACKET_SAMPLES {
            for (slot, sample) in packet.iter_mut().zip(self.samples.drain(..PACKET_SAMPLES)) {
                *slot = sample;
            }
        }
        packet
    }
}

/// The call's WebRTC side: one Opus track up, and an `oai-events` channel the
/// server expects to see open.
struct Peer {
    connection: Arc<dyn PeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    gathered: Arc<Notify>,
    failed: Arc<Notify>,
}

struct PeerEvents {
    gathered: Arc<Notify>,
    failed: Arc<Notify>,
}

#[async_trait]
impl PeerConnectionEventHandler for PeerEvents {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.notify_one();
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if matches!(
            state,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
        ) {
            self.failed.notify_one();
        }
    }
}

impl Peer {
    async fn new() -> Result<Self, ()> {
        let codec = RTCRtpCodec {
            mime_type: "audio/opus".into(),
            clock_rate: OPUS_RATE,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
            rtcp_feedback: vec![],
        };
        let mut media = MediaEngine::default();
        media
            .register_codec(
                RTCRtpCodecParameters {
                    rtp_codec: codec.clone(),
                    payload_type: OPUS_PAYLOAD,
                },
                RtpCodecKind::Audio,
            )
            .map_err(|_| ())?;
        let ssrc: u32 = rand_ssrc();
        let track = Arc::new(
            TrackLocalStaticSample::new(MediaStreamTrack::new(
                "realtime".into(),
                format!("audio-{ssrc}"),
                "microphone".into(),
                RtpCodecKind::Audio,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec,
                    active: true,
                    ..Default::default()
                }],
            ))
            .map_err(|_| ())?,
        );
        let gathered = Arc::new(Notify::new());
        let failed = Arc::new(Notify::new());
        let connection: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_media_engine(media)
                .with_handler(Arc::new(PeerEvents {
                    gathered: gathered.clone(),
                    failed: failed.clone(),
                }))
                .with_udp_addrs(vec!["0.0.0.0:0"])
                .with_tcp_addrs(vec!["0.0.0.0:0"])
                .build()
                .await
                .map_err(|_| ())?,
        );
        let peer = Self {
            connection,
            track,
            ssrc,
            gathered,
            failed,
        };
        if peer.connection.add_track(peer.track.clone()).await.is_err()
            || peer
                .connection
                .create_data_channel("oai-events", None)
                .await
                .is_err()
        {
            peer.close().await;
            return Err(());
        }
        Ok(peer)
    }

    /// The offer with every local candidate in it: the server does not
    /// trickle.
    async fn offer(&self) -> Result<String, ()> {
        timeout(CONNECT_TIMEOUT, async {
            let offer = self.connection.create_offer(None).await.map_err(|_| ())?;
            self.connection
                .set_local_description(offer)
                .await
                .map_err(|_| ())?;
            self.gathered.notified().await;
            self.connection
                .local_description()
                .await
                .map(|offer| offer.sdp)
                .ok_or(())
        })
        .await
        .map_err(|_| ())?
    }

    async fn answer(&self, sdp: String) -> Result<(), ()> {
        let answer = RTCSessionDescription::answer(sdp).map_err(|_| ())?;
        timeout(
            CONNECT_TIMEOUT,
            self.connection.set_remote_description(answer),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
    }

    async fn send(&self, encoder: &mut opus::Encoder, pcm: &[i16]) -> Result<(), ()> {
        let mut packet = [0u8; 1275];
        let size = encoder.encode(pcm, &mut packet).map_err(|_| ())?;
        timeout(
            PACKET * 5,
            self.track.write_sample(
                self.ssrc,
                OPUS_PAYLOAD,
                &Sample {
                    data: bytes::Bytes::copy_from_slice(&packet[..size]),
                    duration: PACKET,
                    ..Default::default()
                },
                &[],
            ),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
    }

    async fn close(&self) {
        let _ = timeout(CLOSE_TIMEOUT, self.connection.close()).await;
    }
}

fn rand_ssrc() -> u32 {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    u32::from_le_bytes(bytes).max(1)
}

enum Next {
    Command(Option<Command>),
    Packet,
    Read(Option<Result<Message, tokio_tungstenite::tungstenite::Error>>),
    PeerFailed,
    Renew,
    Placed(Result<Box<Placed>, SpeechError>),
}

/// Owns one call, then each call that replaces it: its peer, its sideband and
/// the pacer that feeds the peer.
struct Driver {
    live: GptLive,
    config: RealtimeConfig,
    who: Identity,
    call: String,
    peer: Peer,
    events: mpsc::Sender<Result<RealtimeEvent, SpeechError>>,
    mic: Mic,
    heard: Heard,
    /// What was to go on the sideband before the session started; a line
    /// given before then is not spoken. `None` once it has.
    held: Option<Vec<Value>>,
    /// When held lines go out even without the session's word.
    start_by: Instant,
    /// The conversation so far, for the instructions of a replacement call.
    carry: Carry,
    /// The model's current reply so far.
    saying: String,
    renew_at: Instant,
    /// A replacement call being placed.
    placing: Option<oneshot::Receiver<Result<Box<Placed>, SpeechError>>>,
    /// A replacement ready to switch to, and since when.
    ready: Option<(Box<Placed>, Instant)>,
}

impl Driver {
    async fn run(mut self, mut sideband: Sideband, mut commands: mpsc::Receiver<Command>) {
        match self.drive(&mut sideband, &mut commands).await {
            Ok(()) | Err(SpeechError::Cancelled) => {}
            Err(error) => {
                let _ = self.events.send(Err(error)).await;
            }
        }
        let _ = timeout(CLOSE_TIMEOUT, sideband.close(None)).await;
        self.peer.close().await;
        if let Some((mut ready, _)) = self.ready.take() {
            let _ = timeout(CLOSE_TIMEOUT, ready.sideband.close(None)).await;
            ready.peer.close().await;
        }
    }

    async fn drive(
        &mut self,
        sideband: &mut Sideband,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<(), SpeechError> {
        let mut encoder =
            opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|_| self.live.malformed())?;
        let mut pacer = tokio::time::interval(PACKET);
        pacer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let failed = self.peer.failed.clone();
            let renewing = self.placing.is_some() || self.ready.is_some();
            let next = tokio::select! {
                command = commands.recv() => Next::Command(command),
                _ = pacer.tick() => Next::Packet,
                read = sideband.next() => Next::Read(read),
                () = failed.notified() => Next::PeerFailed,
                () = sleep_until(self.renew_at), if !renewing => Next::Renew,
                placed = async { self.placing.as_mut().expect("guarded").await }, if self.placing.is_some() => {
                    self.placing = None;
                    Next::Placed(placed.unwrap_or(Err(SpeechError::Cancelled)))
                }
                () = self.events.closed() => return Err(SpeechError::Cancelled),
            };
            if self.held.is_some() && Instant::now() >= self.start_by {
                self.release(sideband).await?;
            }
            match next {
                Next::Command(None) => return Ok(()),
                Next::Command(Some(command)) => self.command(sideband, command).await?,
                Next::Packet => {
                    let packet = self.mic.packet();
                    if self.peer.send(&mut encoder, &packet).await.is_err() {
                        return Err(self.live.unreachable());
                    }
                }
                Next::Read(Some(Ok(Message::Text(text)))) => {
                    let event: Value =
                        serde_json::from_str(&text).map_err(|_| self.live.malformed())?;
                    if event["type"] == "session.started" {
                        self.started(&event["session"]);
                        self.release(sideband).await?;
                    }
                    let events = self
                        .heard
                        .event(&event)
                        .map_err(|()| self.live.malformed())?;
                    for event in events {
                        self.note(&event);
                        self.emit(event).await?;
                    }
                }
                Next::Read(Some(Ok(Message::Ping(_)))) => {
                    let _ = sideband.flush().await;
                }
                Next::Read(Some(Ok(Message::Binary(_)))) => return Err(self.live.malformed()),
                Next::Read(Some(Ok(_))) => {}
                Next::Read(None | Some(Err(_))) => self.rejoin(sideband).await?,
                Next::PeerFailed => return Err(self.live.unreachable()),
                Next::Renew => self.place_replacement(),
                Next::Placed(Ok(placed)) => self.ready = Some((placed, Instant::now())),
                Next::Placed(Err(SpeechError::Cancelled)) => return Err(SpeechError::Cancelled),
                // The old call still has minutes; try again before they run out.
                Next::Placed(Err(_)) => self.renew_at = Instant::now() + RENEW_RETRY,
            }
            let due = self
                .ready
                .as_ref()
                .is_some_and(|(_, since)| !self.heard.speaking || since.elapsed() >= SWITCH_WAIT);
            if due {
                self.switch(sideband).await?;
            }
        }
    }

    /// The server's end for this session brings renewal forward to five
    /// minutes before it.
    fn started(&mut self, session: &Value) {
        let Some(expires) = session["expires_at"].as_u64() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let left = Duration::from_secs(expires.saturating_sub(now));
        self.renew_at = self
            .renew_at
            .min(Instant::now() + left.saturating_sub(RENEW_EARLY));
    }

    /// What each side said joins the transcript a replacement carries.
    fn note(&mut self, event: &RealtimeEvent) {
        match event {
            RealtimeEvent::Saying(piece) => self.saying.push_str(piece),
            RealtimeEvent::ResponseDone => {
                let words = std::mem::take(&mut self.saying);
                self.carry.push("You", &words);
            }
            RealtimeEvent::Heard(words) => self.carry.push("Caller", words),
            _ => {}
        }
    }

    /// Places the replacement in the background, so the call goes on while
    /// it connects. It carries the conversation as it stands; what is said
    /// while it connects is lost to it, as on the socket providers' renewal.
    fn place_replacement(&mut self) {
        let (done, placing) = oneshot::channel();
        let live = self.live.clone();
        let config = self.config.clone();
        let carry = self.carry.clone();
        tokio::spawn(async move {
            let _ = done.send(live.place(&config, &carry).await.map(Box::new));
        });
        self.placing = Some(placing);
    }

    /// Moves the call onto its replacement and hangs up the old one.
    async fn switch(&mut self, sideband: &mut Sideband) -> Result<(), SpeechError> {
        let Some((placed, _)) = self.ready.take() else {
            return Ok(());
        };
        let Placed {
            who,
            call,
            peer,
            sideband: fresh,
        } = *placed;
        let mut old_sideband = std::mem::replace(sideband, fresh);
        let old_peer = std::mem::replace(&mut self.peer, peer);
        self.who = who;
        self.call = call;
        tokio::spawn(async move {
            let _ = timeout(CLOSE_TIMEOUT, old_sideband.close(None)).await;
            old_peer.close().await;
        });
        let now = Instant::now();
        self.heard = Heard::default();
        self.saying.clear();
        self.held = Some(Vec::new());
        self.start_by = now + START_WAIT;
        self.renew_at = now + self.live.renew_after;
        self.emit(RealtimeEvent::Renewed).await
    }

    async fn emit(&self, event: RealtimeEvent) -> Result<(), SpeechError> {
        self.events
            .send(Ok(event))
            .await
            .map_err(|_| SpeechError::Cancelled)
    }

    async fn command(
        &mut self,
        sideband: &mut Sideband,
        command: Command,
    ) -> Result<(), SpeechError> {
        match command {
            Command::Audio(pcm) => {
                self.mic.push(&pcm);
                Ok(())
            }
            Command::Say(line) => {
                self.append(sideband, "session.context.append", None, "speakable", &line)
                    .await
            }
            // A text turn from the caller is context for the model, not
            // something to say aloud.
            Command::Text(text) => {
                self.append(
                    sideband,
                    "session.context.append",
                    None,
                    "commentary",
                    &text,
                )
                .await
            }
            // What a delegation found, or a refusal, goes back as commentary
            // on it; a plain hand-off the model already acknowledged does not.
            Command::ToolResult {
                call_id,
                output,
                reply,
            } => {
                if !reply || output.is_empty() {
                    return Ok(());
                }
                self.append(
                    sideband,
                    "delegation.context.append",
                    Some(&call_id),
                    "commentary",
                    &output,
                )
                .await
            }
            // The server stops a reply itself when the person talks.
            Command::Truncate { .. } => Ok(()),
        }
    }

    async fn append(
        &mut self,
        sideband: &mut Sideband,
        kind: &str,
        delegation: Option<&str>,
        channel: &str,
        text: &str,
    ) -> Result<(), SpeechError> {
        for piece in context_pieces(text) {
            let mut event = json!({
                "type": kind,
                "channel": channel,
                "content": [{"type": "input_text", "text": piece}],
            });
            if let Some(id) = delegation {
                event["delegation_item_id"] = json!(id);
            }
            self.send(sideband, event).await?;
        }
        Ok(())
    }

    /// Sends what waited for the session to start.
    async fn release(&mut self, sideband: &mut Sideband) -> Result<(), SpeechError> {
        for event in self.held.take().unwrap_or_default() {
            self.send(sideband, event).await?;
        }
        Ok(())
    }

    async fn send(&mut self, sideband: &mut Sideband, event: Value) -> Result<(), SpeechError> {
        if let Some(held) = &mut self.held {
            held.push(event);
            return Ok(());
        }
        let text = event.to_string();
        if sideband
            .send(Message::Text(text.clone().into()))
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.rejoin(sideband).await?;
        sideband
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| self.live.unreachable())
    }

    /// The call outlives its sideband: a dropped one is joined again. A call
    /// the server no longer knows has ended.
    async fn rejoin(&mut self, sideband: &mut Sideband) -> Result<(), SpeechError> {
        let mut wait = REJOIN_WAIT;
        for _ in 0..REJOINS {
            sleep(wait).await;
            match self.live.join(&self.who, &self.call).await {
                Ok(fresh) => {
                    let mut old = std::mem::replace(sideband, fresh);
                    let _ = timeout(CLOSE_TIMEOUT, old.close(None)).await;
                    self.heard = Heard::default();
                    return self.emit(RealtimeEvent::Renewed).await;
                }
                Err(SpeechError::Refused {
                    status: 404 | 410, ..
                }) => {
                    return Err(self.live.unreachable());
                }
                Err(SpeechError::Unreachable { .. }) => wait *= 2,
                Err(error) => return Err(error),
            }
        }
        Err(self.live.unreachable())
    }
}

#[cfg(test)]
mod tests;
