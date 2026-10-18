use super::*;

#[cfg(unix)]
/// A chat-completions fixture that answers each request with the next of
/// `answers`, a list of stream chunks, and keeps every request.
async fn provider_scripted(
    answers: Vec<Vec<Value>>,
) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Router, body::Bytes, routing::post};
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = seen.clone();
    let answers = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(
        answers,
    )));
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let request: Value = serde_json::from_slice(&body).unwrap();
            let chunks = answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("an unscripted request: {request}"));
            requests.lock().unwrap().push(request);
            let body: String = chunks
                .into_iter()
                .map(|chunk| format!("data: {chunk}\n\n"))
                .collect();
            async move {
                (
                    [("Content-Type", "text/event-stream")],
                    format!("{body}data: [DONE]\n\n"),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, seen)
}

#[cfg(unix)]
fn reply_chunks(text: &str) -> Vec<Value> {
    vec![
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}),
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]
}

#[cfg(unix)]
fn shell_chunks(command: &str) -> Vec<Value> {
    let arguments = json!({ "command": command }).to_string();
    vec![
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_build","type":"function","function":{"name":"shell","arguments":arguments}}]},"finish_reason":null}]}),
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]
}

#[cfg(unix)]
/// The next line the front is given to say.
async fn next_said(asked: &mut tokio::sync::mpsc::Receiver<Asked>) -> String {
    loop {
        if let Asked::Say(text) = next_asked(asked).await {
            return text;
        }
    }
}

/// On a call to the teammate, a command it left running that ends on its own
/// wakes a turn, and the front is given that turn's reply to say: the person
/// hears that the build finished without asking again.
#[cfg(unix)]
#[tokio::test]
async fn running_process_that_ends_during_a_call_is_said_on_the_call() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_scripted(vec![
        shell_chunks("sleep 1; echo done"),
        reply_chunks("The build is going."),
        reply_chunks("The build finished."),
    ])
    .await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Goldie","goal":"Build what is asked.","cwd":root.to_str().unwrap(),"reach":"machine"}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(started["ok"], true);
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    until(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        while desk.front.config.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the front was opened");

    let words = "run the build";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c1".into(),
            name: "send_to_teammate".into(),
            arguments: json!({ "words": words }).to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(next_said(&mut desk.asked).await, "The build is going.");
    // Nothing the person said started the next turn: the build's end did.
    assert_eq!(next_said(&mut desk.asked).await, "The build finished.");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let woke = requests[2].to_string();
    assert!(
        woke.contains("a command you left running has ended"),
        "{woke}"
    );
    assert!(woke.contains("done"), "{woke}");
}

#[cfg(unix)]
fn tool_chunks(name: &str, arguments: Value) -> Vec<Value> {
    let arguments = arguments.to_string();
    vec![
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_tool","type":"function","function":{"name":name,"arguments":arguments}}]},"finish_reason":null}]}),
        json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]
}

/// On a call, a teammate that starts a server, opens it and replies has
/// answered: the call goes Listening at the reply, so while the server runs
/// the call is not thinking and the front is given no progress line, and
/// stopping the server from the desk says nothing on the call.
#[cfg(unix)]
#[tokio::test]
async fn running_process_a_server_started_on_a_call_leaves_the_call_listening() {
    let opened = common::browser();
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let link = format!("http://127.0.0.1:{port}/chart.html");
    let serve = format!(
        "printf '<h1>The chart</h1>' > chart.html; echo $$ > server.pid; \
         exec python3 -m http.server {port} --bind 127.0.0.1"
    );
    let (provider, requests) = provider_scripted(vec![
        tool_chunks("shell", json!({ "command": serve })),
        tool_chunks("open_link", json!({ "url": link })),
        reply_chunks("The chart is open."),
    ])
    .await;
    let socket = &mut desk.socket;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Goldie","goal":"Serve what is asked.","cwd":root.to_str().unwrap(),"reach":"machine"}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(started["ok"], true);
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    until(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        while desk.front.config.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the front was opened");

    let words = "make me a chart page and open it";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c1".into(),
            name: "send_to_teammate".into(),
            arguments: json!({ "words": words }).to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(next_said(&mut desk.asked).await, "The chart is open.");
    let socket = &mut desk.socket;
    until(socket, |f| {
        f["sub"] == 2 && f["event"]["state"] == "listening"
    })
    .await;
    let pid: i32 = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(pid) = std::fs::read_to_string(root.join("server.pid"))
                .ok()
                .and_then(|pid| pid.trim().parse().ok())
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the server wrote its pid");
    assert!(
        std::fs::read_to_string(opened)
            .unwrap_or_default()
            .lines()
            .any(|handed| handed == link),
        "the link was opened"
    );

    // A thinking call re-sends its state every 15 s, and the front's progress
    // line rides on that; a listening call sends neither while the server
    // runs. Two seconds shows the call has settled, not the full 15.
    let quiet = tokio::time::Instant::now() + Duration::from_secs(2);
    while let Ok(Some(Ok(message))) = tokio::time::timeout_at(quiet, socket.next()).await {
        let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_ne!(frame["event"]["state"], "thinking", "{frame}");
    }
    while let Ok(asked) = desk.asked.try_recv() {
        assert!(
            !matches!(asked, Asked::Say(_)),
            "the front was given a line"
        );
    }
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the server is still up");

    let job_id = {
        let requests = requests.lock().unwrap();
        requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .find_map(|message| {
                let content: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
                content["job"]["job_id"].as_str().map(str::to_owned)
            })
            .expect("the shell's launch receipt")
    };
    let stopped = answer(
        socket,
        93,
        json!({"cmd":"process.stop","params":{"personaId":persona,"id":job_id}}),
    )
    .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    tokio::time::timeout(Duration::from_secs(10), async {
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the server was stopped");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        requests.lock().unwrap().len(),
        3,
        "the teammate was not woken"
    );
    while let Ok(asked) = desk.asked.try_recv() {
        assert!(!matches!(asked, Asked::Say(_)), "the call said nothing");
    }
}
