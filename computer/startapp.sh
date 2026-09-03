#!/bin/sh

export DISPLAY="${DISPLAY:-:0}"
export TOAD_COMPUTER_HOME="${TOAD_COMPUTER_HOME:-/home/agent}"

# AT-SPI discovers its accessibility bus through the desktop session bus.
exec dbus-run-session -- /usr/bin/toad-computer serve
