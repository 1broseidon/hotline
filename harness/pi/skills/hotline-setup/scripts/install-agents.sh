#!/bin/sh
# install-agents.sh - copy hotline's starter agent templates into the pi user
# agents directory, never clobbering the operator's own agents.
#
# Usage:
#   install-agents.sh                 # install all starter agents
#   install-agents.sh researcher scout
#
# Semantics (see the pi-byoh design, §D):
#   - target ~/.pi/agent/agents/<name>.md (user scope: headless, no trust gate)
#   - missing        -> install
#   - identical      -> skip silently (already installed)
#   - different      -> install as hotline-<name>.md with the frontmatter `name:`
#                       rewritten to hotline-<name>, leaving the operator's file
#                       untouched; report the conflict
#   - idempotent; prints the final agents-dir listing
#
# Overrides (used by tests):
#   HOTLINE_AGENTS_SRC   source templates dir (default: ../../../agents)
#   HOTLINE_AGENTS_DEST  destination agents dir (default: ~/.pi/agent/agents)
#
# No network access.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

SRC="${HOTLINE_AGENTS_SRC:-$SCRIPT_DIR/../../../agents}"
DEST="${HOTLINE_AGENTS_DEST:-$HOME/.pi/agent/agents}"

if [ ! -d "$SRC" ]; then
  echo "error: source templates dir not found: $SRC" >&2
  exit 1
fi

# Default to every starter template when no names are given.
if [ "$#" -eq 0 ]; then
  set -- architect researcher scout implementer
fi

mkdir -p "$DEST"

for name in "$@"; do
  # Reject path-shaped or otherwise unexpected names (review F9): `name` is used
  # raw in $SRC/$name.md and $DEST/$name.md, so a value like `../foo` would
  # traverse out of both dirs. Starter agent names are plain identifiers.
  case "$name" in
    *[!A-Za-z0-9_-]*|"")
      echo "skip: invalid agent name '$name' (allowed: letters, digits, '_', '-')" >&2
      continue
      ;;
  esac

  src="$SRC/$name.md"
  if [ ! -f "$src" ]; then
    echo "skip: no such starter agent '$name' (looked in $SRC)" >&2
    continue
  fi

  dest="$DEST/$name.md"

  if [ ! -e "$dest" ]; then
    cp "$src" "$dest"
    chmod 0644 "$dest"
    echo "installed: $name -> $dest"
    continue
  fi

  if cmp -s "$src" "$dest"; then
    echo "unchanged: $name (already installed)"
    continue
  fi

  # A different file already owns this name: install under our namespace instead
  # of overwriting the operator's agent, rewriting the frontmatter name to match.
  alt_name="hotline-$name"
  alt_dest="$DEST/$alt_name.md"

  # Render the namespaced copy to a temp file first so we can apply the same
  # never-clobber semantics to it (review F8): if a `hotline-<name>.md` already
  # exists we must NOT blindly overwrite it — the setup skill pins a model into
  # that file, and a later re-run would silently discard the pin.
  tmp_alt="$alt_dest.tmp.$$"
  awk -v nm="$alt_name" '
    BEGIN { done = 0 }
    /^name:[[:space:]]/ && done == 0 { print "name: " nm; done = 1; next }
    { print }
  ' "$src" > "$tmp_alt"

  if [ ! -e "$alt_dest" ]; then
    mv "$tmp_alt" "$alt_dest"
    chmod 0644 "$alt_dest"
    echo "conflict: '$name' already exists and differs; installed as '$alt_name' -> $alt_dest"
  elif cmp -s "$tmp_alt" "$alt_dest"; then
    rm -f "$tmp_alt"
    echo "conflict: '$name' already exists and differs; '$alt_name' already installed (unchanged)"
  else
    rm -f "$tmp_alt"
    echo "conflict: '$name' differs and '$alt_name' already exists with local edits; left untouched (remove it to reinstall)" >&2
  fi
done

echo "---"
echo "agents in $DEST:"
ls -1 "$DEST"
