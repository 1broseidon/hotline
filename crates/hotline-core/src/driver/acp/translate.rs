//! ACP's updates in Hotline's vocabulary: what the agent says becomes an
//! [`Update`] on the tape, a permission request becomes a card, and a line said
//! mid-turn is carried into that turn.

use super::*;

/// One `session/update`: a subagent's, or the conversation's own.
pub(super) async fn receive(live: &Arc<Live>, notification: RawSessionUpdate) {
    if live.replaying.load(Ordering::SeqCst) {
        return;
    }
    if let Some(announced) = announced(&notification.update) {
        live.subagent(announced).await;
        return;
    }
    let Ok(update) = serde_json::from_value::<SessionUpdate>(notification.update) else {
        return;
    };
    let child = lock(&live.children).get(&notification.session_id).cloned();
    match child {
        Some(scribe) => {
            translate_in(&Pen::child(live, &notification.session_id, &scribe), update).await;
        }
        None => between_turns(live, update).await,
    }
}

/// One `session/update`, including those the agent sends with no prompt out.
///
/// An agent may answer its own background work after its turn is over: a
/// subagent reporting back, a command it left running finishing. What it says
/// then opens an activity the session runs as it runs a turn, so the reply
/// reaches the chat, and the phone, when it is said rather than on the next
/// message. Claude closes each such cycle with a usage report that carries
/// the cycle's cost.
async fn between_turns(live: &Arc<Live>, update: SessionUpdate) {
    if live.replaying.load(Ordering::SeqCst) {
        return;
    }
    let says = matches!(
        update,
        SessionUpdate::AgentMessageChunk(_)
            | SessionUpdate::AgentThoughtChunk(_)
            | SessionUpdate::ToolCall(_)
            | SessionUpdate::ToolCallUpdate(_)
            | SessionUpdate::Plan(_)
    );
    // Claude reports usage after every model call, but only the report that
    // closes a cycle carries its cost.
    let ends = matches!(&update, SessionUpdate::UsageUpdate(usage) if usage.cost.is_some());
    if says && let Some(generation) = live.open_unprompted() {
        watch_unprompted(Arc::downgrade(live), generation);
    }
    translate(live, update).await;
    if ends {
        live.end_unprompted().await;
        live.park_for_subagents().await;
    }
}

/// Ends an unprompted activity the agent has gone quiet in.
fn watch_unprompted(live: std::sync::Weak<Live>, generation: u64) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(UNPROMPTED_QUIET / 10).await;
            let Some(live) = live.upgrade() else { return };
            let quiet = match lock(&live.unprompted_open).as_ref() {
                Some(open) if open.generation == generation => {
                    open.last.elapsed() >= UNPROMPTED_QUIET
                }
                _ => return,
            };
            if quiet {
                live.end_unprompted().await;
                return;
            }
        }
    });
}

/// One `session/update`, as the room's vocabulary sees it.
pub(super) async fn translate(live: &Live, update: SessionUpdate) {
    translate_in(&live.pen(), update).await;
}

/// One `session/update`, written where `pen` writes: the conversation's turn,
/// or a subagent's run.
async fn translate_in(pen: &Pen<'_>, update: SessionUpdate) {
    let live = pen.live;
    if live.replaying.load(Ordering::SeqCst) {
        return;
    }
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                pen.chunk(MessageKind::Agent, &text.text, chunk.message_id.as_ref())
                    .await;
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                pen.chunk(MessageKind::Thought, &text.text, chunk.message_id.as_ref())
                    .await;
            }
        }
        SessionUpdate::ToolCall(call) => {
            pen.flush().await;
            let line = ToolLine {
                files: changed_files(call.kind, &call.locations, &call.content),
                title: clip(&call.title, TITLE_CHARS),
                kind: tool_kind(call.kind, call.meta.as_ref()),
                location: first_location(&call.locations),
            };
            pen.emit(Update::ToolCall {
                call_id: call.tool_call_id.0.to_string(),
                title: line.title.clone(),
                kind: line.kind.clone(),
            })
            .await;
            if let Some(words) = own_words(call.raw_input.as_ref()) {
                pen.emit(Update::ToolSaid {
                    call_id: call.tool_call_id.0.to_string(),
                    words,
                })
                .await;
            }
            let finished = finished(call.status, &call.content);
            let files = line.files.clone();
            lock(pen.tools).insert(call.tool_call_id.0.to_string(), line);
            if let Some((ok, output, images)) = finished {
                if ok && !files.is_empty() {
                    pen.emit(Update::ToolFiles {
                        call_id: call.tool_call_id.0.to_string(),
                        files,
                    })
                    .await;
                }
                pen.emit(Update::ToolResult {
                    call_id: call.tool_call_id.0.to_string(),
                    ok,
                    output,
                    images,
                })
                .await;
            }
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let call_id = update.tool_call_id.0.to_string();
            let fields = update.fields;
            // Absent means unchanged, so anything the update left out falls
            // back to what the call was announced with.
            let line = {
                let mut tools = lock(pen.tools);
                let previous = tools.get(&call_id);
                let mut files = previous.map(|line| line.files.clone()).unwrap_or_default();
                let changes = changed_files(
                    fields.kind.unwrap_or_else(|| {
                        if previous.is_some_and(|line| line.kind == "edit") {
                            acp::ToolKind::Edit
                        } else {
                            acp::ToolKind::Other
                        }
                    }),
                    fields.locations.as_deref().unwrap_or_default(),
                    fields.content.as_deref().unwrap_or_default(),
                );
                for change in changes {
                    if let Some(known) = files.iter_mut().find(|known| known.path == change.path) {
                        known.created |= change.created;
                    } else {
                        files.push(change);
                    }
                }
                let line = ToolLine {
                    files,
                    title: fields
                        .title
                        .as_deref()
                        .map(|title| clip(title, TITLE_CHARS))
                        .or_else(|| previous.map(|line| line.title.clone()))
                        .unwrap_or_default(),
                    // A call announced as a subagent's launch stays one,
                    // whatever kind a later update gives it.
                    kind: previous
                        .map(|line| line.kind.clone())
                        .filter(|kind| kind == SUBAGENT_LAUNCH)
                        .or_else(|| {
                            launches_subagent(update.meta.as_ref())
                                .then(|| SUBAGENT_LAUNCH.to_string())
                        })
                        .or_else(|| fields.kind.map(kind_of))
                        .or_else(|| previous.map(|line| line.kind.clone()))
                        .unwrap_or_default(),
                    location: fields
                        .locations
                        .as_deref()
                        .and_then(first_location)
                        .or_else(|| previous.and_then(|line| line.location.clone())),
                };
                let announced = ToolLine {
                    files: line.files.clone(),
                    title: line.title.clone(),
                    kind: line.kind.clone(),
                    location: line.location.clone(),
                };
                tools.insert(call_id.clone(), line);
                announced
            };
            // The input often arrives after the call was announced, and with it
            // the agent's own words for what the call does.
            if let Some(words) = own_words(fields.raw_input.as_ref()) {
                pen.emit(Update::ToolSaid {
                    call_id: call_id.clone(),
                    words,
                })
                .await;
            }
            let content = fields.content.unwrap_or_default();
            match fields.status.and_then(|status| finished(status, &content)) {
                Some((ok, output, images)) => {
                    if ok && !line.files.is_empty() {
                        pen.emit(Update::ToolFiles {
                            call_id: call_id.clone(),
                            files: line.files,
                        })
                        .await;
                    }
                    pen.emit(Update::ToolResult {
                        call_id,
                        ok,
                        output,
                        images,
                    })
                    .await;
                }
                // Still running: the line is written again so a title or a
                // path the agent only learned now reaches the transcript.
                None => {
                    pen.emit(Update::ToolCall {
                        call_id,
                        title: line.title,
                        kind: line.kind,
                    })
                    .await;
                }
            }
        }
        // TranscriptEvent::Plan exists and the window does not draw it yet, so
        // the plan arrives as the one thing the window does draw. When the
        // window grows a plan panel this becomes an Update of its own.
        SessionUpdate::Plan(plan) => {
            pen.flush().await;
            let lines: Vec<String> = plan
                .entries
                .iter()
                .map(|entry| format!("- {} ({:?})", entry.content, entry.status))
                .collect();
            if lines.is_empty() {
                return;
            }
            pen.emit(Update::Notice {
                level: NoticeLevel::Info,
                text: format!("Plan:\n{}", lines.join("\n")),
            })
            .await;
        }
        // A subagent's own pickers are not the conversation's.
        SessionUpdate::CurrentModeUpdate(_) | SessionUpdate::ConfigOptionUpdate(_)
            if pen.child.is_some() => {}
        SessionUpdate::CurrentModeUpdate(mode) => {
            lock(&live.session).info.current_mode_id = Some(mode.current_mode_id.0.to_string());
            live.publish_info();
        }
        SessionUpdate::ConfigOptionUpdate(update) => {
            let mut session = lock(&live.session);
            apply_configs_to_session(&mut session, &update.config_options);
            drop(session);
            live.publish_info();
        }
        _ => {}
    }
}

/// The agent is asking to be allowed to do something. The card goes on the
/// tape now; the answer arrives later, over the wire, as its own command.
pub(super) async fn ask_permission(
    live: Arc<Live>,
    request: RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
) {
    if lock(&live.auth).as_ref().is_some_and(|a| a.running()) {
        let _ = responder.respond(RequestPermissionResponse::new(
            RequestPermissionOutcome::Cancelled,
        ));
        return;
    }
    let request_id = new_id();
    let title = describe_request(&live, &request.tool_call);
    let options: Vec<CardOption> = request
        .options
        .iter()
        .map(|option| CardOption {
            option_id: option.option_id.0.to_string(),
            name: option.name.clone(),
            kind: serde_json::to_value(option.kind)
                .ok()
                .and_then(|kind| kind.as_str().map(str::to_string)),
        })
        .collect();

    let (answer, answered) = oneshot::channel();
    lock(&live.pending).insert(request_id.clone(), answer);
    live.flush().await;
    live.emit(Update::Permission {
        request_id,
        title,
        options,
    })
    .await;

    tokio::spawn(async move {
        let chosen = match tokio::time::timeout(PERMISSION_TIMEOUT, answered).await {
            Ok(Ok(chosen)) => chosen,
            _ => None,
        };
        let outcome = match chosen {
            Some(option_id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                acp::PermissionOptionId::new(option_id.as_str()),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        let _ = responder.respond(RequestPermissionResponse::new(outcome));
    });
}

/// What the agent says a call does in its own few words, when its input
/// carries them: Claude's shell and subagent tools ask the model for a short
/// `description` of every call.
fn own_words(input: Option<&Value>) -> Option<String> {
    input?
        .get("description")?
        .as_str()
        .map(str::trim)
        .filter(|words| !words.is_empty())
        .map(str::to_string)
}

/// What the agent is actually asking to be allowed to do.
///
/// The tool call on a permission request is a partial that points back at a
/// call already announced, so on its own it can carry nothing but an id and a
/// kind. "The agent is asking for permission" is not a question anyone can
/// answer, so this recovers the detail: the command if there is one, otherwise
/// the announced title, otherwise at least the kind of thing being attempted.
fn describe_request(live: &Live, call: &acp::ToolCallUpdate) -> String {
    let tools = lock(&live.tools);
    let known = tools.get(call.tool_call_id.0.as_ref());

    if let Some(command) = call
        .fields
        .raw_input
        .as_ref()
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    {
        return format!("Run {}", clip(command, TITLE_CHARS));
    }

    // Agents title an edit "Editing files" and leave which file to the
    // locations, which is the one detail the answer turns on.
    let where_ = call
        .fields
        .locations
        .as_deref()
        .and_then(first_location)
        .or_else(|| known.and_then(|line| line.location.clone()));
    let named = |what: String| match &where_ {
        Some(file) if !what.contains(file.as_str()) => format!("{what} — {file}"),
        _ => what,
    };

    let title = call
        .fields
        .title
        .clone()
        .or_else(|| known.map(|line| line.title.clone()))
        .filter(|title| !title.is_empty());
    if let Some(title) = title {
        return named(clip(&title, TITLE_CHARS));
    }

    let kind = call
        .fields
        .kind
        .map(kind_of)
        .or_else(|| known.map(|line| line.kind.clone()))
        .filter(|kind| !kind.is_empty());
    match kind {
        None => "The agent is asking for permission".to_string(),
        Some(kind) => named(format!(
            "Allow the agent to {}",
            PERMISSION_VERBS
                .iter()
                .find(|(name, _)| *name == kind)
                .map_or_else(|| format!("use {kind}"), |(_, verb)| (*verb).to_string())
        )),
    }
}

// -- translation helpers ----------------------------------------------------

/// The room's view of a tool call's disposition, or `None` while it is still
/// running.
pub(super) fn finished(
    status: acp::ToolCallStatus,
    content: &[acp::ToolCallContent],
) -> Option<(bool, String, Vec<ToolImage>)> {
    let ok = match status {
        acp::ToolCallStatus::Completed => true,
        acp::ToolCallStatus::Failed => false,
        _ => return None,
    };
    let (output, images) = result_of(content);
    Some((ok, output, images))
}

/// A tool's output as the transcript keeps it: the text it produced, a
/// placeholder for each image, and for an edit the file and what it now says.
fn result_of(content: &[acp::ToolCallContent]) -> (String, Vec<ToolImage>) {
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for item in content {
        match item {
            acp::ToolCallContent::Content(inner) => match &inner.content {
                ContentBlock::Text(text) => texts.push(text.text.clone()),
                ContentBlock::Image(image) => images.push(ToolImage {
                    data: image.data.clone(),
                    mime_type: image.mime_type.clone(),
                }),
                _ => {}
            },
            acp::ToolCallContent::Diff(diff) => {
                texts.push(format!("{}\n{}", diff.path.display(), diff.new_text));
            }
            _ => {}
        }
    }
    let output = with_image_placeholders(&texts.join("\n"), &images);
    (output, images)
}

fn first_location(locations: &[acp::ToolCallLocation]) -> Option<String> {
    locations
        .first()
        .and_then(|location| location.path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
}

/// An ACP enum as the string the wire spells it with, which is the string the
/// window's icons and the tape are keyed by.
fn kind_of(kind: acp::ToolKind) -> String {
    word_of(&kind)
}

/// The kind a launched subagent's call is kept under, the same as Hotline
/// Agent's: a turn left open only for such calls reads as done.
pub(super) const SUBAGENT_LAUNCH: &str = crate::session::jobs::SUBAGENT;

/// A call's kind, or a subagent's launch when the agent says so. ACP has no
/// kind for it; Claude's adapter names the tool behind each call in
/// `_meta.claudeCode.toolName`, and a subagent is launched by `Agent` (once
/// `Task`), whose call stays open, kind-less, while the subagent works.
pub(super) fn tool_kind(kind: acp::ToolKind, meta: Option<&acp::Meta>) -> String {
    if launches_subagent(meta) {
        SUBAGENT_LAUNCH.to_string()
    } else {
        kind_of(kind)
    }
}

fn launches_subagent(meta: Option<&acp::Meta>) -> bool {
    meta.and_then(|meta| meta.get("claudeCode"))
        .and_then(|claude| claude.get("toolName"))
        .and_then(Value::as_str)
        .is_some_and(|name| matches!(name, "Agent" | "Task"))
}

pub(super) fn stop_reason_of(reason: acp::StopReason) -> String {
    word_of(&reason)
}

fn word_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub(super) fn usage_of(usage: &acp::Usage) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(usage.input_tokens as i64),
        output_tokens: Some(usage.output_tokens as i64),
        total_tokens: Some(usage.total_tokens as i64),
        cache_read_tokens: usage.cached_read_tokens.map(|tokens| tokens as i64),
        cache_write_tokens: usage.cached_write_tokens.map(|tokens| tokens as i64),
    }
}

/// Carries the lines sent while a turn runs into that turn, one at a time and
/// in the order they were said.
///
/// A line the agent does not take comes back, and so does every line after
/// it, so the order survives when they are said again as prompts of their
/// own. Nothing is offered once the prompt is answered: the turn is over and
/// the line belongs to the next one. The steering extension asks an idle
/// agent to refuse (`promptRequired`) rather than start a turn nobody is
/// listening to; one that starts it anyway (`startedNewTurn`) has that turn
/// cancelled, because the line is said again as a prompt of its own.
pub(super) async fn steer_lane(
    live: Arc<Live>,
    connection: ConnectionTo<Agent>,
    session_id: SessionId,
    mut lines: mpsc::UnboundedReceiver<Line>,
    over: CancellationToken,
    stop: CancellationToken,
) -> Vec<Line> {
    let mut refused = Vec::new();
    while let Some((text, attachments)) = lines.recv().await {
        if !refused.is_empty() || over.is_cancelled() || stop.is_cancelled() {
            refused.push((text, attachments));
            continue;
        }
        let request = agent_client_protocol::UntypedMessage::new(
            STEERING_METHOD,
            serde_json::json!({
                "sessionId": session_id,
                "prompt": operator_blocks(&text, &attachments),
                "_meta": { "steering": { "idleBehavior": "promptRequired" } },
            }),
        );
        let answer = match request {
            Ok(request) => tokio::select! {
                biased;
                // Stop drops this line with the rest, so there is nothing
                // left to learn from the agent's answer.
                () = stop.cancelled() => break,
                answer = tokio::time::timeout(
                    STEER_REPLY,
                    connection.send_request(request).block_task(),
                ) => answer.ok(),
            },
            Err(_) => None,
        };
        let outcome = match &answer {
            Some(Ok(reply)) => reply.get("outcome").and_then(Value::as_str),
            _ => None,
        };
        match outcome {
            Some("injected") => lock(&live.history).push(operator_fact(&text, &attachments)),
            Some("startedNewTurn") => {
                let _ = connection.send_notification(CancelNotification::new(session_id.clone()));
                refused.push((text, attachments));
            }
            _ => {
                if matches!(&answer, Some(Err(error)) if error.code == acp::ErrorCode::MethodNotFound)
                {
                    live.steerable.store(false, Ordering::SeqCst);
                    lock(&live.session).info.capabilities.active_input = false;
                    live.publish_info();
                }
                refused.push((text, attachments));
            }
        }
    }
    refused
}

/// One operator line as ACP content. Attachments lead the message the way
/// they do in a mail client, and travel as links: a coding agent already has
/// the filesystem, and a path costs nothing to send.
pub(super) fn operator_blocks(text: &str, attachments: &[Attachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = attachments
        .iter()
        .map(|attachment| {
            ContentBlock::ResourceLink(acp::ResourceLink::new(
                attachment.name.clone(),
                file_uri(&attachment.path),
            ))
        })
        .collect();
    blocks.push(text_block(text));
    blocks
}

/// One operator line as the checkpoint a fresh session is briefed with.
pub(super) fn operator_fact(text: &str, attachments: &[Attachment]) -> String {
    format!(
        "Operator: {text}\nAttachments: {}",
        attachments
            .iter()
            .map(|a| a.path.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn text_block(text: &str) -> ContentBlock {
    ContentBlock::Text(acp::TextContent::new(text))
}

/// A path as the `file:` URI a resource link carries.
fn file_uri(path: &str) -> String {
    format!("file://{path}")
}

/// Diff payloads distinguish creation from replacement; location-only edit
/// notifications still identify edited files without guessing from the title.
fn changed_files(
    kind: acp::ToolKind,
    locations: &[acp::ToolCallLocation],
    content: &[acp::ToolCallContent],
) -> Vec<crate::contract::ChangedFile> {
    let mut files: Vec<_> = content
        .iter()
        .filter_map(|item| match item {
            acp::ToolCallContent::Diff(diff) => Some(crate::contract::ChangedFile {
                path: diff.path.to_string_lossy().into_owned(),
                created: diff.old_text.is_none(),
            }),
            _ => None,
        })
        .collect();
    if kind == acp::ToolKind::Edit {
        for location in locations {
            let path = location.path.to_string_lossy().into_owned();
            if !files.iter().any(|file| file.path == path) {
                files.push(crate::contract::ChangedFile {
                    path,
                    created: false,
                });
            }
        }
    }
    files
}

#[cfg(test)]
mod own_words_tests {
    use super::own_words;
    use serde_json::json;

    #[test]
    fn a_calls_own_description_is_its_few_words_and_nothing_else_is() {
        let input =
            json!({"command": "cargo test -p core", "description": "  Run the core tests "});
        assert_eq!(
            own_words(Some(&input)).as_deref(),
            Some("Run the core tests")
        );
        assert_eq!(own_words(Some(&json!({"command": "ls"}))), None);
        assert_eq!(own_words(Some(&json!({"description": "   "}))), None);
        assert_eq!(own_words(None), None);
    }
}
