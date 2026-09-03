# Computer

The computer is a containerized Linux desktop that a teammate drives through
one MCP server. It is local infrastructure rather than a special agent type:
Toad grants the machine's `/mcp` endpoint to a teammate like any other server.
The desktop remains visible in the browser viewer while the Rust agent reads
its accessibility tree, captures pixels, and performs input.

## Image

`computer/Dockerfile` adds Chromium, X11 input and clipboard programs, AT-SPI,
D-Bus, xdotool window controls, and Noto/DejaVu fonts to
`jlesage/baseimage-gui:alpine-3.22-v4`. The base supplies the display, window
manager, browser viewer on port 5800, VNC on port 5900, process supervision,
and its non-root application user. The Docker-reported uncompressed image size
is **1.05 GB**.

Alpine 3.22 does not publish a `wmctrl` package. Focus and close therefore use
xdotool, while maximize and restore use the agent's existing EWMH connection.

Build from the repository root because the Rust crate is outside `computer/`:

```sh
docker build -t toad-computer:dev -f computer/Dockerfile .
```

## Agent

The agent listens on `0.0.0.0:8787` by default and exposes eight grouped tools:

- `capture` returns a scaled PNG and the AT-SPI tree, or writes an original PNG.
- `input` clicks, moves, drags, scrolls, types, presses keys, and uses the clipboard.
- `browser` drives the visible Chromium over CDP; element refs last for one text snapshot.
- `shell` runs bounded commands or launches a detached desktop application.
- `files` gets, puts, and lists paths confined below the computer home.
- `windows` lists, focuses, closes, sizes, and tiles EWMH windows.
- `wait` polls the accessibility tree and browser page text for a phrase.
- `state` owns control leases, browser logins, and home-directory snapshots.

`/health` never requires authentication. When `TOAD_COMPUTER_TOKEN` is set,
every method on `/mcp` requires `Authorization: Bearer <token>` and otherwise
returns a JSON 401. `X-Computer-Holder` names the teammate using a lease or run
slot; an absent header means `anonymous`.

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

The jlesage init needs only `CHOWN`, `SETUID`, `SETGID`, `DAC_OVERRIDE`, `KILL`,
and `NET_BIND_SERVICE` for ownership changes, UID/GID transitions, cross-user
process cleanup, and its capability-bearing nginx binary. Toad drops every
other capability and keeps `no-new-privileges`, memory/PID limits, and
loopback-bound published ports in force.

## Contract proof

The opt-in Rust test drives the real image through rmcp's streamable HTTP
client. It checks authentication, the exact tool list, shell, capture and
windows, a visible Chromium page, file confinement, and cross-holder control.

```sh
TOAD_COMPUTER_URL=http://127.0.0.1:8787 \
TOAD_COMPUTER_TOKEN=$TOKEN \
cargo test -p toad-computer --test contract
```

## What left the image

Node and Playwright are gone because Rust drives Chromium's CDP directly.
Tesseract is gone because `wait` reads the desktop tree and page text. The old
machine proxy is gone because the Toad side wakes the machine when a session
starts.
