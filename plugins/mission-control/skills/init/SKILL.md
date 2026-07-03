---
name: init
description: Scaffold a mission-control workspace (filing system, agent playbook, starter voice) into the current directory. Use when the user wants to set up mission-control or turn this project into a texting-agent control room.
---

# mission-control: init

Scaffold a mission-control workspace into the current working directory from the
canonical template files bundled with this plugin at
`${CLAUDE_PLUGIN_ROOT}/skills/init/templates/`.

<!-- Source of truth for the template contents is templates/mission-control/ in the
     hotline repo. The copies under skills/init/templates/ must stay byte-identical
     to it; verify with: diff -r templates/mission-control plugins/mission-control/skills/init/templates -->

## Step 1: Check for conflicts. Never overwrite.

Check the current directory for each target path below. If a target file already
exists, do NOT write it. Collect every conflict, report the list to the user,
and skip those files. Only create what is missing. An existing `threads/`,
`inbox/`, or `archive/` directory is fine; only same-path files count as
conflicts.

Targets:

- `CLAUDE.md`
- `HOTLINE.md`
- `INDEX.md`
- `README.md`
- `threads/example-thread/README.md`
- `inbox/.gitkeep`
- `archive/.gitkeep`

## Step 2: Create the workspace

Copy each non-conflicting file from
`${CLAUDE_PLUGIN_ROOT}/skills/init/templates/` to the same relative path in the
current directory, creating directories as needed. Copy the files as-is; do not
edit or reformat them.

For example:

```sh
cp "${CLAUDE_PLUGIN_ROOT}/skills/init/templates/CLAUDE.md" ./CLAUDE.md
```

If `${CLAUDE_PLUGIN_ROOT}/skills/init/templates/` does not exist, stop and tell
the user the plugin install looks broken; do not improvise file contents.

## Step 3: Tell the user the next steps

After scaffolding, report what was created (and any conflicts skipped), then
give the next steps from the template README:

```sh
hotline setup --telegram-token <token>   # once, machine-wide
hotline init                             # in this folder
hotline start
```

Then: DM the bot, approve the pairing code with `hotline pair <code>`, and
they're in. Point them at `README.md` in the new workspace for the full guide,
and mention that `HOTLINE.md` sets the texting voice and is meant to be edited.

Do not run `hotline` commands yourself; the user runs them.
