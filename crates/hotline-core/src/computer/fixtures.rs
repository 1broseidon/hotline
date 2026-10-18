use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hotline-core-computer-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A child writes the script, not this process. A file one thread holds
/// open for writing is inherited by every fork another thread makes in
/// that moment, and until that child has exec'd, running the file fails
/// with "Text file busy"; under a full parallel test run that is one
/// run in three. `sh` holds the file instead, and has exited before
/// this returns.
pub(crate) fn write_script(dir: &Path, name: &str, body: &str) {
    use std::io::Write;
    let path = dir.join(name);
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"cat > "$1" && chmod 755 "$1""#)
        .arg("sh")
        .arg(&path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "writing {}", path.display());
}

/// The fake runtime belongs to one test: each writes it into its own
/// directory with the log it appends to, the state file it reads and the
/// port it reports baked in. Nothing about it is process-wide, so these
/// tests run beside each other instead of queueing behind a lock on the
/// environment. The paths are single quoted and a scratch path holds no
/// quote to close them with.
pub(crate) fn fake_runtime(dir: &Path, mcp_port: u16) {
    let head = format!(
        "#!/bin/sh\nLOG='{}'\nSTATE='{}'\nMCP={mcp_port}\n",
        dir.join("argv.log").display(),
        dir.join("state").display(),
    );
    let body = r#"{
  printf '%s' "$1"
  i=1
  for a in "$@"; do
if [ "$i" -gt 1 ]; then printf ' %s' "$a"; fi
i=$((i + 1))
  done
  printf '\n'
} >> "$LOG"
cmd=$1
shift || true
case "$cmd" in
  version) echo "fake 1"; exit 0 ;;
  info) echo '[]'; exit 0 ;;
  image)
if [ "$1" = inspect ] && [ -f "${STATE}.noimage" ]; then
  echo "Error: No such image: $2" >&2
  exit 1
fi
exit 0
;;
  pull)
echo "test: Pulling from hotline-computer"
echo "aaaa1111: Pulling fs layer"
echo "bbbb2222: Pulling fs layer"
# A test holds the download open for as long as this file exists.
while [ -f "${STATE}.pullgate" ]; do sleep 0.05; done
if [ -f "${STATE}.pullfail" ]; then
  echo "Error: registry unreachable" >&2
  exit 1
fi
echo "aaaa1111: Pull complete"
echo "bbbb2222: Already exists"
echo "Status: Downloaded newer image for $1"
rm -f "${STATE}.noimage"
exit 0
;;
  inspect)
state=$(cat "$STATE")
if [ "$state" = absent ]; then
  echo "Error: No such container: $1" >&2
  exit 1
fi
running=false
[ "$state" = running ] && running=true
environment='[]'
if [ -f "${STATE}.token" ]; then environment='["HOTLINE_COMPUTER_TOKEN=fixture-persisted-token"]'; fi
if [ -f "${STATE}.folders" ]; then environment="[\"$(cat "${STATE}.folders")\"]"; fi
printf '[{"Config":{"Env":%s},"State":{"Running":%s},"NetworkSettings":{"Ports":{"8787/tcp":[{"HostPort":"%s"}]}}}]\n' "$environment" "$running" "$MCP"
exit 0
;;
  create)
rm -f "${STATE}.folders"
for a in "$@"; do
  case "$a" in HOTLINE_FOLDERS=?*) printf '%s' "$a" > "${STATE}.folders" ;; esac
done
echo stopped > "$STATE"; exit 0 ;;
  start) echo running > "$STATE"; exit 0 ;;
  stop) echo stopped > "$STATE"; exit 0 ;;
  rm) echo absent > "$STATE"; exit 0 ;;
  *) echo "unknown: $cmd" >&2; exit 1 ;;
esac
"#;
    write_script(dir, "docker", &format!("{head}{body}"));
}
