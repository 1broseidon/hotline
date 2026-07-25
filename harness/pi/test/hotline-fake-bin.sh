#!/usr/bin/env bash
# Stand-in for the `hotline` binary used by the integration smoke. The extension
# invokes it as `hotline run`; we ignore the subcommand and exec the fake child,
# which speaks the real Go JSON-RPC frames over stdio.
exec node "$(dirname "$0")/fake-hotline.mjs"
