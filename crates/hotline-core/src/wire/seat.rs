//! What a socket may do, decided by the token it presented.

use super::Parts;
use super::socket::PHONE_CAPABILITIES;
use crate::contract::ThreadKind;
use crate::contract::{Command, Target, ViewName};

/// Reasons a harness is unavailable that say nothing about the machine's
/// accounts or folders, which a companion may read as they are.
pub(super) const PLAIN_REASONS: &[&str] = &["Not installed", "is not published for this computer"];
/// What a companion reads in place of any other reason.
pub(super) const PRIVATE_REASON: &str = "Not available on this desk. Its owner can see why.";

/// What a socket may do, decided by the token it presented.
///
/// Owners have the same authority as the local desk. Companion sockets keep
/// the narrow phone command set; transport bounds and revocation are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Desk,
    /// An authenticated owner has the local desk command set.
    Owner,
    /// A companion retains the existing phone permissions.
    Phone,
    /// A linked desk (ADR 0003): its roster and messages for the exchange
    /// queue, and nothing else. Never seated on a socket of the wire; its
    /// frames are answered by [`federation`](super::federation).
    Link,
}

impl Seat {
    /// A relay grant has no `WireRole`, so there is no seat to ask for.
    pub(super) fn for_phone(role: crate::remote::WireRole) -> Self {
        match role {
            crate::remote::WireRole::Owner => Self::Owner,
            crate::remote::WireRole::Companion => Self::Phone,
        }
    }

    pub(super) fn is_remote(self) -> bool {
        matches!(self, Self::Owner | Self::Phone | Self::Link)
    }

    pub fn permits(self, command: &Command) -> bool {
        match self {
            // The federation commands speak for a linked desk, which only
            // the link's own authenticated channel can name.
            Seat::Desk | Seat::Owner => !federation_command(command),
            Seat::Link => matches!(command, Command::Ping {}) || federation_command(command),
            // A phone answers for the person: a permission card, a
            // human, credential or passkey card is a teammate waiting on
            // someone. Capture is the one value-bearing answer: its live
            // card binds the origin and recipient before it can store and
            // grant a login or arm a passkey. Arbitrary secret management
            // and passkey arming remain at the desk.
            // Which model and how hard it thinks are settings the person
            // owns anywhere.
            //
            // Who a teammate is — its name and its goal — the person owns
            // anywhere too, through the narrow `mobile.persona_update`; and
            // letting one go takes nothing a teammate could use, so the
            // phone may `persona.delete` as the desk does.
            //
            // What stays at the desk is the standing grant rather than the
            // single answer: how far a teammate reaches, which tools and
            // servers it has, whether it keeps a computer — all of that is
            // `persona.update`. A harness mode goes with them, because for
            // some harnesses the mode is the standing permission posture
            // under another name, and `session.set_config` is narrowed to
            // effort for the same reason.
            //
            // A resting teammate's session reports no models and no configs,
            // so a phone reads what this desk can reach the way the window
            // does. Both are lists of what exists, not grants of anything.
            //
            // A file a teammate sent is part of the conversation the phone
            // already reads, so the phone reads the file too. `file.read`
            // names a message, never a path, and serves only what the desk
            // kept for that message.
            //
            // A turn that failed may be tried again from the phone too:
            // `session.retry` sends a line already on the tape once more and
            // writes nothing new, so it is no more than the phone already did.
            Seat::Phone => {
                phone_thread_command(command)
                    || matches!(
                        command,
                        Command::MobilePrompt { .. }
                    | Command::MobileAttachment { .. }
                    | Command::MobilePushRegister { .. }
                    // A narrow create, confined in core rather than by
                    // leaving fields out of the phone's form: see
                    // `mobile_persona_create` in `wire/commands/persona.rs`.
                    | Command::MobilePersonaCreate { .. }
                    // Name and goal only, confined by the command's shape.
                    | Command::MobilePersonaUpdate { .. }
                    | Command::PersonaDelete { .. }
                    // Which harnesses this desk can run and whether each is
                    // ready. Read-only, and carries no credential.
                    | Command::BackendsList { .. }
                    | Command::SessionCancel { .. }
                    | Command::SessionRetry { .. }
                    | Command::HumanAnswer { .. }
                    | Command::SecretsPasskeyAnswer { .. }
                    | Command::SecretsUseAnswer { .. }
                    | Command::ComputerCookieOffers { .. }
                    | Command::ComputerCookieAnswer { .. }
                    | Command::ComputerCookieRemove { .. }
                    | Command::SecretsCaptureAnswer(..)
                    | Command::SessionAnswerPermission { .. }
                    | Command::SessionSetModel { .. }
                    | Command::SessionSetConfig { .. }
                    | Command::ModelsList { .. }
                    | Command::ModelsEfforts { .. }
                    | Command::ComputerCapacity { .. }
                    | Command::ComputerStatus { .. }
                    | Command::ComputerStop { .. }
                    // Proof the link is alive, and a card for a link already
                    // in a conversation the phone reads.
                    | Command::Ping { .. }
                    | Command::LinkPreview { .. }
                    | Command::FileRead { .. }
                    // A picture is no more private than the name beside it.
                    | Command::AvatarRead { .. }
                    // Older lines of a tape the phone already reads.
                    | Command::TapePage { .. }
                    | Command::TeammatesExchangeStop { .. }
                    | Command::TeammatesExchangeResume { .. }
                    // A side thread is a conversation with a teammate the
                    // person already talks to from the phone; starting,
                    // speaking in, stopping and archiving one grant nothing
                    // a prompt and a cancel do not.
                    | Command::SideStart { .. }
                    | Command::SidePrompt { .. }
                    | Command::SideCancel { .. }
                    | Command::SideArchive { .. }
                    | Command::SideContinue { .. }
                    | Command::SideList { .. }
                    | Command::SideAnswerPermission { .. }
                    )
            }
        }
    }

    /// What this seat's hello says it can do: the phone's list, and
    /// `personaAccess` and `personaComputer` for the owner alone.
    pub(super) fn capabilities(self) -> Vec<&'static str> {
        let mut capabilities = PHONE_CAPABILITIES.to_vec();
        if self == Seat::Owner {
            capabilities.push("personaAccess");
            capabilities.push("personaComputer");
        }
        capabilities
    }

    pub(super) fn capabilities_for(self, parts: &Parts) -> Vec<&'static str> {
        let mut capabilities = self.capabilities();
        if matches!(self, Seat::Owner | Seat::Desk)
            && let Some(voice) = &parts.voice
        {
            let audio = voice.status().direct_available;
            let text = voice
                .status_for(crate::contract::VoiceInputMode::Text)
                .direct_available;
            if audio || text {
                capabilities.push("voice");
                capabilities.push("voiceDirectCalls");
                capabilities.push(crate::voice::LISTEN_WHILE_THINKING);
            }
            if text {
                capabilities.push("voiceTextInput");
            }
            if voice.front_available() {
                capabilities.push(crate::voice::VOICE_FRONT);
            }
        }
        capabilities
    }

    pub fn permits_sub(self, target: &Target) -> bool {
        match self {
            Seat::Desk | Seat::Owner => true,
            Seat::Link => false,
            // A teammate's schedules are a read of one list, not the room
            // stream they are kept on, which also carries every setting.
            // A thread between two teammates is read the way a tape is: the
            // phone already reads the marker for it on either tape, and the
            // thread holds what was said, never a setting. A subagent's run
            // is the same: its line is on the tape, and the run holds the
            // task, its steps and what it said.
            Seat::Phone => {
                !matches!(target, Target::Tape(id) if id == "voice-dispatcher")
                    && match target {
                        Target::ThreadId(thread) => phone_may_read(thread),
                        _ => matches!(
                            target,
                            Target::Tape(_)
                                | Target::Thread(_)
                                | Target::Run(_)
                                | Target::Side(_)
                                | Target::View(ViewName::Roster)
                                | Target::Schedules(_)
                        ),
                    }
            }
        }
    }
}

/// The commands only a linked desk sends.
fn federation_command(command: &Command) -> bool {
    matches!(
        command,
        Command::FederationRoster { .. }
            | Command::FederationDeliver { .. }
            | Command::FederationUnlink {}
    )
}

/// The `thread.*` commands a companion may run, each as what it already may do
/// with the same thing under the older name: the person speaks to a teammate
/// from the phone through `mobile.prompt`, so `thread.prompt` is for a work
/// thread only; a pair raises no card, so there is none there for it to
/// answer; a job is cancelled, and its firing stopped,
/// only where `schedule.cancel` is run; and a thread it could not subscribe to it
/// cannot page. `thread.list` leaves out what it could not read, and pairs for
/// the same reason `peers.list` is refused.
fn phone_thread_command(command: &Command) -> bool {
    match command {
        Command::ThreadPrompt { thread, .. } => thread.kind == ThreadKind::Side,
        Command::ThreadAnswer {
            answer: crate::contract::ThreadAnswer::Credential(_),
            ..
        } => false,
        Command::ThreadAnswer { thread, .. } => thread.kind != ThreadKind::Pair,
        // Closing a job cancels it, which `schedule.cancel` never let a
        // companion do, and stopping its firing is the same owner's call.
        Command::ThreadClose { thread } | Command::ThreadCancel { thread, .. } => {
            thread.kind != ThreadKind::Job
        }
        Command::ThreadPage { thread, .. } => phone_may_read(thread),
        Command::ClientHello { .. }
        | Command::ThreadList { .. }
        | Command::ThreadOpen { .. }
        | Command::ThreadPark { .. }
        | Command::ThreadContinue { .. } => true,
        _ => false,
    }
}

/// Whether a companion may read this thread: the way it reads a tape, a pair,
/// a work thread, a run and a job, but never the voice dispatcher's tape, and never a
/// call, which is a voice surface it is not seated on.
pub(super) fn phone_may_read(thread: &crate::contract::ThreadId) -> bool {
    match thread.kind {
        ThreadKind::Dm => thread.key != "voice-dispatcher",
        ThreadKind::Call => false,
        ThreadKind::Side | ThreadKind::Pair | ThreadKind::Run | ThreadKind::Job => true,
    }
}
