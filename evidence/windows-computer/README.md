# Evidence: Windows teammate computer

Hotline 0.45.0 → patched build, Windows 11 Enterprise 26300, Docker Desktop 29.9.2 (WSL2). User and folder names are replaced (`<user>`, `Projects\example`).

| | File | Shows |
|---|---|---|
| Root cause | `before-hotline.log` | 0.45.0's own log: first `\\?\C… is not a valid Windows path` (the verbatim bind source), then, once that was fixed, `has a colon in its path` for a granted `C:\` folder |
| Fix working | `after-docker-inspect.txt` | The running computer: both bind sources are plain `C:\…`; the granted folder is mounted `:ro` and a write inside it is refused; `--cap-drop=ALL`, `no-new-privileges`, 4 GB |
| Fix working | `after-computer-titlebar.png`, `after-computer-status.png` | The teammate's computer window, live (`Watching Hotline · live 2 ms`) |

The "2 failed" in the screenshots are two of the agent's own shell commands that exited non-zero, not computer failures.
