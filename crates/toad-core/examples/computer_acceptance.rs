//! Replays desktop acceptance calls through the exact adapter used by Toad Agent.
//! stdin/stdout are one JSON object per line; the bearer stays in a local file.
use serde_json::{Value, json};
use std::io::{BufRead, Write};
#[cfg(unix)]
use toad_core::contract::Persona;
use toad_core::{computer, mcp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if matches!(args.len(), 4 | 5) && args[1] == "--provision" {
        return provision(
            &args[2],
            std::path::Path::new(&args[3]),
            args.get(4).map(String::as_str),
        )
        .await;
    }
    if args.len() != 3 {
        return Err("usage: computer_acceptance URL TOKEN_FILE | --provision IMAGE OUTPUT [PASSIVE_QA_PERSONA_ID]".into());
    }
    let ready = computer::Ready {
        url: format!("{}/mcp", args[1].trim_end_matches('/')),
        token: std::fs::read_to_string(&args[2])?.trim().into(),
    };
    let connected = mcp::connect("release-acceptance-toad", &[computer::mcp_server(&ready)]).await;
    if !connected.failed.is_empty() || connected.tools.len() != 8 {
        return Err(format!("computer connection failed: {:?}", connected.failed).into());
    }
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let name = format!(
            "computer__{}",
            request["name"].as_str().ok_or("tool name is required")?
        );
        let tool = connected
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or("computer did not advertise this tool")?;
        let response = match tool.call(request["arguments"].clone()).await {
            Ok(result) => {
                let mut content = vec![json!({"type":"text", "text":result.text})];
                content.extend(result.images.into_iter().map(
                    |image| json!({"type":"image", "data":image.data, "mimeType":image.mime_type}),
                ));
                json!({"content":content, "isError":false})
            }
            Err(error) => {
                json!({"content":[{"type":"text", "text":error.to_string()}], "isError":true})
            }
        };
        println!("{response}");
        std::io::stdout().flush()?;
    }
    Ok(())
}

/// Exercise computer creation, stop and wake without starting an agent session.
#[cfg(unix)]
async fn provision(
    image: &str,
    output: &std::path::Path,
    passive_persona_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::OpenOptionsExt;
    let workspace = output.join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let workspace = workspace.canonicalize()?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let id = passive_persona_id
        .map(str::to_owned)
        .unwrap_or_else(|| format!("acceptance-{}-{nonce}", std::process::id()));
    let persona: Persona = serde_json::from_value(json!({
        "id": id, "name": "Release acceptance", "goal": "", "backendId": "toad",
        "cwd": workspace, "mcpPolicy": {"mode":"all", "serverIds":[]},
        "backgroundWork": false, "allowedSenders": [], "sessionCheckpoints": [],
        "computer": {"enabled":true,"image":image}, "createdAt":1,"updatedAt":1
    }))?;
    let computers = computer::Computer::new();
    let ready = computers
        .ensure_running(
            &persona,
            workspace.to_str().ok_or("workspace is not UTF-8")?,
            Some(computer::runtime::Runtime::Docker),
            None,
            |message| eprintln!("{message}"),
        )
        .await?;
    let name = computer::container_name(&persona.id);
    std::fs::write(output.join("container-name.txt"), &name)?;
    let mut token = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(output.join("token"))?;
    token.write_all(ready.token.as_bytes())?;
    let url = ready.url.trim_end_matches("/mcp");
    std::fs::write(output.join("url.txt"), url)?;
    computers
        .stop(&persona.id, Some(computer::runtime::Runtime::Docker))
        .await?;
    let resumed = computers
        .ensure_running(
            &persona,
            workspace.to_str().ok_or("workspace is not UTF-8")?,
            Some(computer::runtime::Runtime::Docker),
            None,
            |message| eprintln!("{message}"),
        )
        .await?;
    assert!(
        ready.token == resumed.token,
        "waking preserves the existing token"
    );
    // Docker may choose another ephemeral published port when restarting.
    let url = resumed.url.trim_end_matches("/mcp");
    std::fs::write(output.join("url.txt"), url)?;
    println!(
        "{}",
        json!({"url":url,"container":name,"image":image,"created_and_resumed":true})
    );
    Ok(())
}

#[cfg(not(unix))]
async fn provision(
    _image: &str,
    _output: &std::path::Path,
    _passive_persona_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err("local provisioning acceptance requires a Unix host".into())
}
