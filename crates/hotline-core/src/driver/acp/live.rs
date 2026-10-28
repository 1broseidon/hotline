//! What one connection to the child holds, and the pen that writes the agent's
//! words onto the tape. Every handler on the connection shares one [`Live`].

use super::*;

/// Everything one connection owns, shared with the handlers running on it.
#[derive(Default)]
pub(super) struct Live {
    pub(super) auth_succeeded: AtomicBool,
    pub(super) auth_methods: Mutex<Vec<acp::AuthMethod>>,
    pub(super) auth: Mutex<Option<Arc<crate::driver::auth::Attempt>>>,
    pub(super) startup_failure: Mutex<Option<String>>,
    pub(super) terminal_done: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(super) connection: Mutex<Option<ConnectionTo<Agent>>>,
    pub(super) connection_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(super) session: Mutex<Session>,
    /// The latest picker metadata the harness advertised. A watch keeps
    /// notifications separate from the transcript update stream; the room
    /// decides whether the snapshot still belongs to its current session.
    pub(super) info_changes: Mutex<Option<watch::Sender<DriverInfo>>>,
    /// Where the running turn's updates go. `None` between turns.
    pub(super) updates: Mutex<Option<mpsc::Sender<Update>>>,
    /// Where the session takes work the agent starts by itself between
    /// turns, once it has asked to (see [`Driver::subscribe_unprompted`]).
    unprompted: Mutex<Option<mpsc::UnboundedSender<mpsc::Receiver<Update>>>>,
    /// Set while `updates` belongs to such work rather than to a prompt.
    pub(super) unprompted_open: Mutex<Option<Unprompted>>,
    /// The message being streamed, buffered so the tape gets whole messages.
    pub(super) open: Mutex<Option<OpenMessage>>,
    /// The last state written for each tool call. An update carries only what
    /// changed, so without somewhere to merge into, a status change arrives as
    /// a payload of blanks and erases the title.
    pub(super) tools: Mutex<HashMap<String, ToolLine>>,
    /// Where the agent's subagents are reported, once the session has asked
    /// (see [`Driver::subscribe_subagents`]).
    subagent_reports: Mutex<Option<mpsc::UnboundedSender<SubagentReport>>>,
    /// The subagent sessions the agent has announced and not yet ended, by
    /// their session id, each with its own open message and tool calls.
    pub(super) children: Mutex<HashMap<String, Arc<Scribe>>>,
    /// Permission requests waiting on a person, by the id their card carries.
    pub(super) pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// `session/load` replays the whole history; nothing is written while it
    /// does, or every restart would duplicate the conversation onto the tape.
    pub(super) replaying: AtomicBool,
    /// Whether this connection has been told what kind of room it is in.
    /// Per-connection, so a restarted backend hears it again and a resumed one
    /// does not hear it twice in the same conversation.
    pub(super) briefed: AtomicBool,
    pub(super) failed: AtomicBool,
    pub(super) cancelled: AtomicBool,
    /// Whether the agent takes a line into a running turn. Offered at
    /// `initialize`, and withdrawn if the agent then refuses the method.
    pub(super) steerable: AtomicBool,
    /// Where a line sent during the running turn goes. Open only while a
    /// `session/prompt` is out and not stopped; the rest of the time `steer`
    /// says no and the session keeps the line queued.
    pub(super) steering: Mutex<Option<Lane>>,
    /// Lines let into a turn that the agent did not take, handed back to the
    /// session to be said as prompts of their own.
    pub(super) unconsumed: Mutex<Vec<Line>>,
    pub(super) history: Mutex<Vec<String>>,
    pub(super) stderr: Mutex<VecDeque<String>>,
}

/// Work the agent took up without being prompted: which one, and when it
/// last said anything.
pub(super) struct Unprompted {
    pub(super) generation: u64,
    pub(super) last: std::time::Instant,
}

/// What the agent said about the conversation it opened.
#[derive(Default)]
pub(super) struct Session {
    pub(super) id: Option<SessionId>,
    pub(super) info: DriverInfo,
    /// The config option ids the model and mode pickers came from, when they
    /// arrived as generic config options rather than as ACP's dedicated
    /// `modes` field. Switching then goes through `session/set_config_option`.
    pub(super) model_config: Option<String>,
    pub(super) mode_config: Option<String>,
    /// Whether the current mode came from ACP's dedicated `modes` field.
    /// Generic config updates must not let a ThoughtLevel or hidden Mode
    /// selector replace that identity.
    pub(super) dedicated_modes: bool,
}

pub(super) struct OpenMessage {
    id: String,
    kind: MessageKind,
    text: String,
    /// The agent's own id for the message, when it sends one.
    from: Option<String>,
}

/// A tool call as the transcript last saw it.
pub(super) struct ToolLine {
    pub(super) files: Vec<crate::contract::ChangedFile>,
    pub(super) title: String,
    pub(super) kind: String,
    /// The first path the call named, which is the detail a permission answer
    /// usually turns on.
    pub(super) location: Option<String>,
}

/// What a subagent's session has open: the conversation's own lives on
/// [`Live`].
#[derive(Default)]
pub(super) struct Scribe {
    open: Mutex<Option<OpenMessage>>,
    tools: Mutex<HashMap<String, ToolLine>>,
}

/// Where translated updates are written: the conversation's running turn, or
/// one of the agent's subagents, each with the message and tool calls it has
/// open.
pub(super) struct Pen<'a> {
    pub(super) live: &'a Live,
    pub(super) open: &'a Mutex<Option<OpenMessage>>,
    pub(super) tools: &'a Mutex<HashMap<String, ToolLine>>,
    pub(super) child: Option<&'a str>,
}

impl Live {
    /// The message still being said, closed as it stands, for an activity
    /// ended where it cannot be flushed through `emit`: its words are already
    /// on screen, so its line is written as a stopped turn's is.
    pub(super) fn take_open(&self) -> Option<Update> {
        let message = lock(&self.open).take()?;
        (!message.text.is_empty()).then_some(Update::Message {
            kind: message.kind,
            id: message.id,
            text: message.text,
        })
    }

    pub(super) fn subscribe_unprompted(&self) -> mpsc::UnboundedReceiver<mpsc::Receiver<Update>> {
        let (sender, receiver) = mpsc::unbounded_channel();
        *lock(&self.unprompted) = Some(sender);
        receiver
    }

    /// Gives what the agent says between turns somewhere to go: a new
    /// activity handed to the session, or the one already open. `None`
    /// unless this opened one, when the generation is its to watch.
    pub(super) fn open_unprompted(&self) -> Option<u64> {
        let mut updates = lock(&self.updates);
        let mut open = lock(&self.unprompted_open);
        if let Some(open) = open.as_mut() {
            open.last = std::time::Instant::now();
            return None;
        }
        if updates.is_some() {
            return None;
        }
        let subscriber = lock(&self.unprompted).clone()?;
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        if subscriber.send(receiver).is_err() {
            return None;
        }
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
        *updates = Some(sender);
        *open = Some(Unprompted {
            generation,
            last: std::time::Instant::now(),
        });
        Some(generation)
    }

    /// Ends the unprompted activity, if one is open: what it was saying is
    /// closed into it, and it ends the way a turn does.
    pub(super) async fn end_unprompted(&self) {
        if lock(&self.unprompted_open).is_none() {
            return;
        }
        self.flush().await;
        let sender = {
            let mut updates = lock(&self.updates);
            if lock(&self.unprompted_open).take().is_none() {
                return;
            }
            updates.take()
        };
        if let Some(sender) = sender {
            let _ = sender
                .send(Update::Turn {
                    stop_reason: "end_turn".into(),
                    usage: None,
                })
                .await;
        }
    }

    pub(super) fn subscribe_info(&self) -> watch::Receiver<DriverInfo> {
        let mut changes = lock(&self.info_changes);
        changes
            .get_or_insert_with(|| watch::channel(DriverInfo::default()).0)
            .subscribe()
    }

    pub(super) fn publish_info(&self) -> DriverInfo {
        let session = lock(&self.session);
        if let Some(changes) = lock(&self.info_changes).as_ref() {
            // Keep the session snapshot guard through publication so a
            // setter and an ACP notification cannot publish out of order.
            changes.send_replace(session.info.clone());
        }
        session.info.clone()
    }

    /// Hands one update to the running turn. An update with no turn behind it
    /// is dropped, which is what a `session/update` arriving between turns is.
    pub(super) async fn emit(&self, update: Update) {
        let sender = lock(&self.updates).clone();
        if let Some(sender) = sender {
            let fact = match &update {
                Update::Message {
                    kind: MessageKind::Agent,
                    text,
                    ..
                } => Some(format!(
                    "Assistant (may be partial if the turn failed): {}",
                    super::clip(text, 4000)
                )),
                Update::ToolCall { call_id, title, .. } => Some(format!(
                    "Tool {call_id}: {title}. Execution is uncertain until its result is recorded."
                )),
                Update::ToolResult {
                    call_id,
                    ok,
                    output,
                    ..
                } => Some(format!(
                    "Tool result {call_id}, success={ok}: {}",
                    super::clip(output, 4000)
                )),
                _ => None,
            };
            if let Some(fact) = fact {
                lock(&self.history).push(fact);
            }
            let _ = sender.send(update).await;
        }
    }

    pub(super) fn pen(&self) -> Pen<'_> {
        Pen {
            live: self,
            open: &self.open,
            tools: &self.tools,
            child: None,
        }
    }

    pub(super) async fn flush(&self) {
        self.pen().flush().await;
    }

    pub(super) fn subscribe_subagents(&self) -> mpsc::UnboundedReceiver<SubagentReport> {
        let (sender, receiver) = mpsc::unbounded_channel();
        *lock(&self.subagent_reports) = Some(sender);
        receiver
    }

    fn report(&self, report: SubagentReport) {
        if let Some(reports) = lock(&self.subagent_reports).as_ref() {
            let _ = reports.send(report);
        }
    }

    /// Ends every subagent still open, which is what a session that ended
    /// or was replaced does to them: nobody will hear how they finished.
    pub(super) fn forget_subagents(&self) {
        let children: Vec<String> = lock(&self.children)
            .drain()
            .map(|(child, _)| child)
            .collect();
        for child in children {
            self.report(SubagentReport::Ended {
                child,
                status: SubagentStatus::Cancelled,
            });
        }
    }

    /// A subagent the agent announced, or one it says has ended. Anything the
    /// session it names says from now until it ends is the subagent's.
    pub(super) async fn subagent(&self, announced: Announced) {
        match announced {
            Announced::Spawned { child, title, task } => {
                let fresh = {
                    let mut children = lock(&self.children);
                    let fresh = !children.contains_key(&child);
                    children.entry(child.clone()).or_default();
                    fresh
                };
                if fresh {
                    self.report(SubagentReport::Started { child, title, task });
                }
            }
            Announced::Ended { child, status } => {
                let Some(scribe) = lock(&self.children).remove(&child) else {
                    return;
                };
                Pen::child(self, &child, &scribe).flush().await;
                self.report(SubagentReport::Ended { child, status });
            }
        }
    }

    /// The agent closed a cycle while the prompt stays open for subagents it
    /// started. Claude's adapter holds `session/prompt` until its background
    /// subagents finish, and marks the end of each cycle with the usage report
    /// that carries its cost: the one word an agent gives that its message is
    /// whole. So the message is closed, and the turn told it has nothing more
    /// to say until a subagent reports or the person speaks. An agent whose
    /// last words were not a message is still working, and one with no
    /// subagents open is about to answer the prompt, so neither parks.
    pub(super) async fn park_for_subagents(&self) {
        if lock(&self.updates).is_none()
            || lock(&self.unprompted_open).is_some()
            || lock(&self.children).is_empty()
        {
            return;
        }
        let speaking = lock(&self.open)
            .as_ref()
            .is_some_and(|message| message.kind == MessageKind::Agent);
        if !speaking {
            return;
        }
        self.flush().await;
        self.emit(Update::Parked).await;
    }

    /// Answers every permission still waiting, which is what the end of a turn
    /// and the end of a session both are: nobody is behind those buttons now.
    pub(super) fn settle_permissions(&self) {
        for (_, waiting) in lock(&self.pending).drain() {
            let _ = waiting.send(None);
        }
    }

    pub(super) fn stderr_hint(&self) -> String {
        let tail: Vec<String> = lock(&self.stderr)
            .iter()
            .rev()
            .take(3)
            .rev()
            .cloned()
            .collect();
        let tail = tail.join(" ");
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!(" The backend said: {tail}")
        }
    }
}

impl<'a> Pen<'a> {
    pub(super) fn child(live: &'a Live, child: &'a str, scribe: &'a Scribe) -> Self {
        Pen {
            live,
            open: &scribe.open,
            tools: &scribe.tools,
            child: Some(child),
        }
    }

    /// Hands one update to where this pen writes. A subagent's run keeps only
    /// whole messages, so its deltas go nowhere.
    pub(super) async fn emit(&self, update: Update) {
        match self.child {
            None => self.live.emit(update).await,
            Some(_) if matches!(update, Update::Delta { .. }) => {}
            Some(child) => self.live.report(SubagentReport::Update {
                child: child.to_string(),
                update,
            }),
        }
    }

    /// Adds text to the message being streamed, opening one — and closing any
    /// message of the other kind — when the agent changes voice.
    ///
    /// A new id from the agent is a new message too. A reply cut off by a
    /// steered line and the reply to that line are two messages, and without
    /// the id they would run together into one.
    pub(super) async fn chunk(&self, kind: MessageKind, text: &str, from: Option<&acp::MessageId>) {
        if text.is_empty() {
            return;
        }
        let from = from.map(|id| id.0.to_string());
        // The message of the other voice is closed first, so the tape never
        // holds a message that changed halfway through from speech to thought.
        let closing = {
            let mut open = lock(self.open);
            let changed = open.as_ref().is_none_or(|message| {
                message.kind != kind
                    || (message.from.is_some() && from.is_some() && message.from != from)
            });
            changed.then(|| open.take()).flatten()
        };
        self.close(closing).await;
        let id = {
            let mut open = lock(self.open);
            let message = open.get_or_insert_with(|| OpenMessage {
                id: new_id(),
                kind,
                text: String::new(),
                from,
            });
            message.text.push_str(text);
            message.id.clone()
        };
        self.emit(Update::Delta {
            kind,
            message_id: id,
            text: text.to_string(),
        })
        .await;
    }

    /// Closes the streamed message: one durable [`Update::Message`] holding
    /// everything its deltas carried.
    pub(super) async fn flush(&self) {
        let open = lock(self.open).take();
        self.close(open).await;
    }

    async fn close(&self, open: Option<OpenMessage>) {
        let Some(message) = open else { return };
        if message.text.is_empty() {
            return;
        }
        self.emit(Update::Message {
            kind: message.kind,
            id: message.id,
            text: message.text,
        })
        .await;
    }
}
