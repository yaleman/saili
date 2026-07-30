#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
betaflight_dir=$(CDPATH='' cd -- "$script_dir/../betaflight" && pwd)
picotool=${PICOTOOL:-/opt/homebrew/bin/picotool}

cd "$betaflight_dir"

command -v tio >/dev/null
test -x "$picotool"

set -- /dev/cu.usbmodem*
if [ "$#" -ne 1 ] || [ ! -e "$1" ]; then
    echo "Expected exactly one Betaflight serial device at /dev/cu.usbmodem*" >&2
    exit 1
fi
serial_port="$1"

set -- obj/betaflight_*_RP2350B_MADFLIGHT_FC3.uf2
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    echo "Expected exactly one MADFLIGHT_FC3 UF2 in betaflight/obj" >&2
    exit 1
fi
firmware="$1"

tio --baudrate 115200 --no-reconnect --mute --script-run once \
    --script 'write("#\r"); assert(expect("Entering CLI Mode", 3000) == 1, "CLI did not respond"); write("bl\r"); assert(expect("restarting in ROM bootloader mode", 3000) == 1, "bootloader command failed")' \
    "$serial_port" || true

bootloader_ready=false
attempt=0
while [ "$attempt" -lt 10 ]; do
    if "$picotool" info >/dev/null 2>&1; then
        bootloader_ready=true
        break
    fi
    attempt=$((attempt + 1))
    sleep 0.25
done

if [ "$bootloader_ready" != true ]; then
    echo "RP2350 bootloader did not appear" >&2
    exit 1
fi

"$picotool" load --verify --execute "$firmware"
