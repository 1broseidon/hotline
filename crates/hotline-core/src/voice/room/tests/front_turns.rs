use super::*;
use hotline_voice::realtime::{Command, RealtimeEvent};

/// A realtime model that plays the one scripted session it was given.
struct Scripted(Mutex<Option<realtime::RealtimeSession>>);
#[async_trait]
impl realtime::Realtime for Scripted {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: "test".into(),
            model_id: "front".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        true
    }
    async fn open(
        &self,
        _: realtime::RealtimeConfig,
    ) -> Result<realtime::RealtimeSession, speech::SpeechError> {
        lock(&self.0).take().ok_or(speech::SpeechError::Cancelled)
    }
}

struct Rig {
    _root: tempfile::TempDir,
    calls: Arc<Calls>,
    id: String,
    asked: mpsc::Receiver<Command>,
    provider: mpsc::Sender<Result<RealtimeEvent, speech::SpeechError>>,
    events: broadcast::Receiver<VoiceEvent>,
}

async fn rig() -> Rig {
    let (session, asked, provider) = realtime::RealtimeSession::scripted(64);
    let mut services = with_fake(Arc::new(Fake::default()));
    services.realtime = Some(Arc::new(Scripted(Mutex::new(Some(session)))));
    let (root, desk, calls) = desk(services);
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, events) = calls.subscribe(&id).unwrap();
    Rig {
        _root: root,
        calls,
        id,
        asked,
        provider,
        events,
    }
}

impl Rig {
    async fn send(&self, event: RealtimeEvent) {
        self.provider.send(Ok(event)).await.unwrap();
    }
    async fn speaking(&self, want: bool) {
        let (calls, id) = (self.calls.clone(), self.id.clone());
        until("the caller's speech is tracked", move || {
            calls.test_caller_speaking(&id) == want
        })
        .await;
    }
    async fn tool_result(&mut self, call: &str) -> String {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(Command::ToolResult {
                    call_id, output, ..
                }) = self.asked.recv().await
                    && call_id == call
                {
                    return output;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{call} was never answered"))
    }
}

fn work(call_id: &str) -> RealtimeEvent {
    RealtimeEvent::ToolCall {
        call_id: call_id.into(),
        name: "work_on_it".into(),
        arguments: json!({"words": "Fix the build."}).to_string(),
    }
}

/// A cough opens the voice detector and closes it with no words. The call
/// must not keep waiting for a transcript that is never coming.
#[tokio::test]
async fn an_empty_transcript_after_speech_started_gives_the_floor_back() {
    let mut rig = rig().await;
    rig.send(RealtimeEvent::SpeechStarted).await;
    rig.speaking(true).await;
    rig.send(work("held-back")).await;
    rig.send(RealtimeEvent::Heard(String::new())).await;
    rig.speaking(false).await;
    rig.tool_result("held-back").await;
    // The next tool call is no longer held back for a transcript.
    rig.send(work("after")).await;
    rig.tool_result("after").await;
    while let Ok(event) = rig.events.try_recv() {
        assert!(
            !matches!(event, VoiceEvent::Heard { .. }),
            "nobody was heard: {event:?}"
        );
    }
}

#[tokio::test]
async fn a_hold_an_interrupt_and_a_dropped_connection_each_end_the_callers_turn() {
    for how in ["hold", "interrupt", "drop"] {
        let rig = rig().await;
        let socket = CancellationToken::new();
        rig.calls
            .bind_connection(
                rig.id.clone(),
                "phone",
                socket.clone(),
                CancellationToken::new(),
            )
            .unwrap();
        rig.send(RealtimeEvent::SpeechStarted).await;
        rig.speaking(true).await;
        match how {
            "hold" => rig.calls.hold(&rig.id, true).unwrap(),
            "interrupt" => rig.calls.interrupt(&rig.id).unwrap(),
            _ => socket.cancel(),
        }
        rig.speaking(false).await;
    }
}
