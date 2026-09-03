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
  --security-opt no-new-privileges \
  --memory 2g --pids-limit 512 --shm-size 1g \
  -p 127.0.0.1:8787:8787 -p 127.0.0.1:5800:5800 \
  -e TOAD_COMPUTER_TOKEN=$TOKEN \
  toad-computer:dev
```

The web desktop is at `http://127.0.0.1:5800`; native VNC is available on port
5900 when published. The jlesage supervisor cannot start its non-root services
under `--cap-drop=ALL`; it exits before `/startapp.sh` when its `chown` and
UID/GID transitions are refused. Keep Docker's default capability set for this
base image and retain `no-new-privileges`.

Alpine 3.22 has no `wmctrl` package. The agent uses xdotool for focus and close
and its existing EWMH connection for maximize and restore.

## Contract

- `/health` is an open liveness probe; `/mcp` uses bearer auth when configured.
- `capture`, `input`, `browser`, `shell`, `files`, `windows`, `wait`, and `state` are the entire tool surface.
- `X-Computer-Holder` identifies the teammate for leases and queued runs.
- Files stay below `/home/agent`, and visible input is serialized.
- Chromium is visible in the desktop and is driven directly over CDP.

The Docker-reported uncompressed image size is **1.05 GB**.
