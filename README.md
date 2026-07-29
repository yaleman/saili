# SAILI reader

Read the raw control packets produced by a FeiYing/SAILI PhoenixRC USB
simulator adapter (`1781:0898`) on macOS.

The adapter emits eight-byte packets containing seven analogue values and one
digital switch value. The Rust application provides a live terminal UI, while
`read_saili.py` remains available as a lightweight diagnostic reader.

![first image](image1.jpg)
![second image](image2.jpg)

## Wire the Turnigy TGY 9X

For a wired connection, use the 3.5 mm audio lead supplied with the SAILI
adapter. You do not need the three receiver/servo leads.

```text
TGY 9X trainer jack ── 3.5 mm audio lead ── SAILI 3.5 mm socket
Mac USB port        ── SAILI USB cable   ── SAILI USB socket
```

1. Disconnect power from every aircraft and receiver that is bound to the
   transmitter. Remove propellers if a nearby model could possibly become
   powered.
2. On the TGY 9X, select or create a simple simulator model. Disable mixes and
   set the modulation/output mode to **PPM**, not PCM.
3. Turn the TGY 9X power switch **off**.
4. Connect the 3.5 mm lead between the trainer jack on the back of the TGY 9X
   and the 3.5 mm socket on the SAILI adapter.
5. Leave the transmitter's main power switch **off**. On an unmodified stock
   TGY 9X, inserting the trainer lead powers the transmitter logic in simulator
   mode without powering the RF module. Its display should come on.
6. Set the SAILI mode selector to the **Phoenix/PhoenixRC** position. The
   working position is the one in which macOS identifies it as
   `SAILI Simulator - PhoenixRC Controller` with USB ID `1781:0898`.
7. Connect the SAILI adapter to the Mac over USB.

The TGY 9X exposes PPM through its phone-style trainer jack, and the SAILI
adapter accepts the supplied standard 3.5 mm lead. If you ever make a cable
instead of using the supplied one, the trainer-port signal is on the **tip**,
ground is on the **sleeve**, and no power connection is required. Do not feed
voltage into the trainer jack.

### Stock RF-module caveat

Some stock TGY 9X transmitters do not produce a usable trainer-port PPM signal
while the RF module is fully seated. Test first without disturbing the module.
If the reader finds the adapter but none of the channel values move:

1. Disconnect USB and the trainer lead, and make sure the transmitter is off.
2. If your RF module is genuinely removable, unseat it and reconnect the
   trainer setup.
3. If the stock module is tethered by an antenna wire, do **not** pull it out or
   leave it hanging from that wire. Use a documented TGY 9X trainer-port
   hardware fix instead.

Removing or modifying the RF module is not needed when the channels already
move.

The transmitter's `TRAINER` menu configures the 9X as the instructor in a
two-radio setup. It is not required when using the transmitter's PPM output
with this simulator adapter.

## Rust TUI

Install Rust if it is not already available:

```bash
brew install rust
```

The TUI reads the USB controller and sends its mapped RC state directly to the
ESPHome native API action. Use the same base64 key configured as
`api_encryption_key` in `esphome/secrets.yaml`:

```bash
export SAILI_ESPHOME_KEY='paste-the-base64-api-key-here'
cargo run --release
```

It connects to `madflight-rc-bridge.local:6053` by default. Override the
address when mDNS is unavailable:

```bash
cargo run --release -- --esphome-address 192.0.2.10:6053
```

The interface shows all seven raw inputs, the mapped roll, pitch, throttle,
yaw, and arm states, ESPHome connection state, command counts, round-trip
time, and whether the bridge is receiving live or safe values.

Output starts in **SAFE HOLD**. With the controller reporting fresh input,
throttle low, and the arm switch off, press `l` to enable live forwarding.
Press `l` again to return to safe hold. Press `q`, Escape, or Control-C to send
safe values and exit.

The default primary mapping is raw channels 1-4 to AETR. The adapter's digital
switch controls CRSF channel 5 (`aux1`, normally arm); remaining analogue
inputs populate later auxiliary channels. Learn the actual raw ordering by
moving one control at a time, then override the mapping or direction as needed:

```bash
cargo run --release -- \
  --roll-channel 4 \
  --pitch-channel 2 \
  --throttle-channel 1 \
  --yaw-channel 3 \
  --invert-pitch
```

Run `cargo run --release -- --help` for all mapping, inversion, address, and
transmit-rate options.

### Library API

`src/lib.rs` provides the reusable interface:

- `SailiDevice::connect()` discovers and opens `1781:0898` through HIDAPI.
- `SailiDevice::read_state()` returns the typed `ReadStatus` result.
- `DeviceState` exposes the seven channels, digital switch, and raw report.
- `SailiError` and `PacketError` distinguish discovery, open, read, and
  malformed-report failures.
- `RcMapping` converts adapter reports to 16 bounded RC channel values.
- `EspHomeRcClient` performs the encrypted native API handshake, discovers and
  validates `set_rc_channels`, and sends typed action calls.

The library does not initialize a terminal and can be used independently of
the Ratatui application.

## Python reader

[Homebrew](https://brew.sh/) is required:

```bash
brew install uv
```

The script declares HIDAPI as an inline dependency, so `uv` installs it in an
isolated environment without a project virtual environment or manual `pip`
commands. macOS exposes this adapter as a HID device; `libusb`, `sudo`, and a
custom driver are not required.

### Read controls

From this directory:

```bash
uv run --script read_saili.py
```

Move one control at a time to learn the radio-to-packet channel mapping. Output
looks like:

```text
Found adapter: FeiYing Model SAILI Simulator - PhoenixRC Controller
channels=127 127 127 127   0 127 127  switch=False  raw=7f 00 7f 7f 7f 00 7f 7f
```

Press Control-C to stop.

## Wi-Fi to MadFlight FC3 bridge

`esphome/madflight_rc_bridge.yaml` turns an ESP32 into a Wi-Fi-controlled CRSF
receiver for bench testing a MadFlight FC3v2. It exposes the encrypted ESPHome
native API action `set_rc_channels`, validates 16 channel values, and emits
standard CRSF `RC_CHANNELS_PACKED` frames at 420000 baud and 50 Hz.

This is a test interface, not a flight-control radio link. Wi-Fi and ESPHome
task scheduling do not provide the deterministic latency or link guarantees
needed to fly an aircraft. Remove all propellers and test the complete
failsafe path before powering motors.

### Hardware

The default configuration targets a classic ESP32 DevKit and uses GPIO17 for
CRSF transmit. Change the `esp32_board` and `crsf_tx_pin` substitutions at the
top of the YAML if your board is different.

```text
ESP32 GPIO17 / TX  ── FC3 GPIO1 / SER0_RX
ESP32 GND          ── FC3 GND
```

Only TX and common ground are required. The bridge does not currently receive
CRSF telemetry, so leave FC3 GPIO0 / `SER0_TX` disconnected. Power the ESP32
from its own USB input and power the FC3 normally. Do not join the boards'
5 V or 3.3 V rails unless you have deliberately designed a shared regulated
power supply.

Both boards use 3.3 V logic. Do not insert an RS-232 adapter, inverter, or
5 V logic-level converter between them.

### Configure Betaflight

With the propellers removed:

1. Open the FC3 in the Betaflight web configurator.
2. In **Ports**, enable **Serial RX** on `UART0`, the FC3 target port backed by
   `SER0` on GPIO0/GPIO1, then save and reboot.
3. In **Receiver**, select **Serial (via UART)** and choose **CRSF** as the
   serial receiver provider.
4. Leave serial receiver inversion disabled.
5. After the ESP32 is running and receiving commands, verify channel motion in
   the Receiver tab before configuring any arm mode.

### Build and flash the ESP32

Create a local secrets file:

```bash
cp esphome/secrets.example.yaml esphome/secrets.yaml
```

Fill in the Wi-Fi and OTA values. Generate the 32-byte base64 API key without
OpenSSL:

```bash
python3 -c 'import base64, secrets; print(base64.b64encode(secrets.token_bytes(32)).decode())'
```

Paste that value into `api_encryption_key`, then validate and flash:

```bash
uvx esphome config esphome/madflight_rc_bridge.yaml
uvx esphome run esphome/madflight_rc_bridge.yaml
```

The first flash normally uses USB. Later uploads can use the device's
`madflight-rc-bridge.local` address over Wi-Fi.

### Send channel commands

The action accepts all 16 channels as integer pulse-width-style values from
`988` to `2012`. The names make the default Betaflight AETR order explicit:

| CRSF channel | API field | Safe value |
| --- | --- | ---: |
| 1 | `roll_us` | 1500 |
| 2 | `pitch_us` | 1500 |
| 3 | `throttle_us` | 988 |
| 4 | `yaw_us` | 1500 |
| 5 | `aux1_us` (normally arm) | 988 |
| 6-16 | `aux2_us` through `aux12_us` | 988 |

When the device is added to Home Assistant, the action is named
`esphome.madflight_rc_bridge_set_rc_channels`. This disarmed command is useful
for checking the link:

```yaml
action: esphome.madflight_rc_bridge_set_rc_channels
data:
  roll_us: 1500
  pitch_us: 1500
  throttle_us: 988
  yaw_us: 1500
  aux1_us: 988
  aux2_us: 988
  aux3_us: 988
  aux4_us: 988
  aux5_us: 988
  aux6_us: 988
  aux7_us: 988
  aux8_us: 988
  aux9_us: 988
  aux10_us: 988
  aux11_us: 988
  aux12_us: 988
```

Home Assistant is convenient for a one-shot test. A controller application
should call the same action through the ESPHome native API and refresh it at
20-50 Hz.

The bridge sends no receiver frames until the first valid command. If commands
stop, it sends centered roll, pitch, and yaw with low throttle and all
auxiliaries low after 250 ms. At one second it stops CRSF frames completely so
the FC3 also detects receiver loss and applies its configured Betaflight
failsafe. The device exposes command count, CRSF frame count, command age, and
failsafe diagnostic entities through ESPHome.

## Troubleshooting

### Adapter not found

Confirm the adapter is connected and selected for PhoenixRC:

```bash
system_profiler SPUSBDataType | grep -A 12 -iE 'SAILI|Phoenix'
```

Unplug and reconnect it after changing the adapter's mode selector.

### No controls move

- Make sure the 9X model output is PPM rather than PCM.
- Keep the 9X main power switch off when using its trainer output.
- Reseat both ends of the 3.5 mm lead.
- Try the stock RF-module procedure above.
- Move one stick axis at a time; byte order is adapter-specific and does not
  necessarily match the channel numbers printed on the transmitter.

### Adapter cannot be opened

Quit PhoenixRC and any controller utility that might have the adapter open,
then unplug and reconnect it. Do not run the reader with `sudo`; the adapter is
read through macOS's HID interface and does not need root access.

## References

- [FlySky FS-TH9X product page](https://www.flyskytech.com/products_detail/42.html)
  documents its phone-jack PPM data interface.
- [SAILI adapter product page](https://alexnld.com/product/wireless-10-in-1-rc-flight-simulator-adapter-for-realflight-g5-g4-phoenix-5-0-4-0-xtr-fms-aerofly/)
  documents wired operation and the supplied 3.5 mm lead.
- [Turnigy 9X service schematic](https://es.scribd.com/document/357541543/Turnigy-9X-Service-Manual-pdf)
  identifies the trainer-jack PPM and power-switching signals.
- [Turnigy 9X trainer-port investigation](https://www.desert-wolfe.com/Projects/Turnigy/default.html)
  describes the stock RF-module signal problem.
- [MadFlight FC3v2 documentation](https://madflight.com/Board-FC3-BF/#pinout-fc3v2)
  documents the `SER0_TX`/`SER0_RX` pinout.
- [Betaflight CRSF receiver implementation](https://github.com/betaflight/betaflight/blob/master/src/main/rx/crsf.c)
  is the protocol reference used by the bridge.
- [ESPHome native API actions](https://esphome.io/components/api/#user-defined-actions)
  documents the Wi-Fi command interface.
