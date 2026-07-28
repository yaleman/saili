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

Run the live interface:

```bash
cargo run --release
```

The interface shows connection status, all seven analogue channels as live
gauges, the digital switch, report count, update age, and the raw HID report.
Press `q`, Escape, or Control-C to exit.

### Library API

`src/lib.rs` provides the reusable interface:

- `SailiDevice::connect()` discovers and opens `1781:0898` through HIDAPI.
- `SailiDevice::read_state()` returns the typed `ReadStatus` result.
- `DeviceState` exposes the seven channels, digital switch, and raw report.
- `SailiError` and `PacketError` distinguish discovery, open, read, and
  malformed-report failures.

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
