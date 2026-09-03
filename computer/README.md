# Toad Computer

This image is a small Linux desktop with one Rust MCP agent. The image is a
contract: anything serving these tools at `/mcp` is a valid computer.

## Build

The crate lives outside this directory, so the repository is the build context.
`computer/Dockerfile.dockerignore` keeps that context to Cargo metadata and the
Rust crates.

```sh
docker build -t toad-computer:dev -f computer/Dockerfile .
```

## Run

```sh
TOKEN=$(openssl rand -hex 24)
docker run -d --name toad-computer-test \
  --cap-drop=ALL \
  --cap-add=CHOWN --cap-add=SETUID --cap-add=SETGID \
  --cap-add=DAC_OVERRIDE --cap-add=KILL --cap-add=NET_BIND_SERVICE \
  --security-opt no-new-privileges \
  --pids-limit 512 --memory 2g --shm-size 1g \
  -p 127.0.0.1:8787:8787 -p 127.0.0.1:5800:5800 \
  -e TOAD_COMPUTER_TOKEN="$TOKEN" \
  -v "$PWD:/workspace" \
  toad-computer:dev
```

The web desktop is at `http://127.0.0.1:5800`; native VNC is available on port
5900 when published. The six added capabilities are the minimum the jlesage
init needs for ownership changes, UID/GID transitions, cross-user process
cleanup, and its capability-bearing nginx binary. Every other capability stays
dropped, and `no-new-privileges` remains in force.

Alpine 3.22 has no `wmctrl` package. The agent uses xdotool for focus and close
and its existing EWMH connection for maximize and restore.

## Contract

- `/health` is an open liveness probe; `/mcp` uses bearer auth when configured.
- `capture`, `input`, `browser`, `shell`, `files`, `windows`, `wait`, and `state` are the entire tool surface.
- `X-Computer-Holder` identifies the teammate for leases and queued runs.
- Files stay below `/home/agent`, and visible input is serialized.
- Chromium is visible in the desktop and is driven directly over CDP.

The Docker-reported uncompressed image size is **710 MB**.
