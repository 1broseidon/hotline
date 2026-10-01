---
title: Teammates and computers
description: Add teammates from your phone, and give one a Linux desktop of its own.
---

## Add a teammate from the phone

On the team list, the **+** adds a teammate on the server: a name, an
optional goal, and a harness, model and effort. It starts with a workspace
of its own under the room, and nothing else: no computer, no reach outside
that workspace, no background work. Rename it, change its goal or remove it
from the phone too.

## Give a teammate a computer

A computer is a Linux desktop in a container on the server that the teammate
drives, and that you can watch and take over from your phone. See
[The computer](/docs/setup/computer/) for what it is.

The server needs Docker or Podman, usable by the `hotline` account: add the
account to the `docker` group, or run Podman rootless as it.

Turning a computer on is a grant, so it happens on the server, not the phone.
Find the teammate's id, then turn its computer on:

```sh
H="sudo -u $(systemctl show -p User --value hotline) HOTLINE_DATA_DIR=/var/lib/hotline/room hotline"
sudo grep -o '"id":"[^"]*","name":"[^"]*"' /var/lib/hotline/room/room.jsonl | sort -u
echo '{"id":"TEAMMATE_ID","patch":{"computer":{"enabled":true}}}' | $H wire persona.update
```

The computer starts the first time the teammate uses it: ask it to, for
example "open example.com in your browser". The first start pulls the
desktop image, which takes a minute. After that, the monitor icon in the
conversation's header turns green while the teammate is using its computer.
Tap it to watch, and take control when you want the keyboard.

:::note
Finding a teammate by grepping the room log is a stopgap. Setting a
teammate's computer from the phone is planned.
:::
