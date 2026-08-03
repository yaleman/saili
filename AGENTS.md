# SAILI OpenCode Agent Guide

## Project Overview

A Rust TUI application for reading SAILI Simulator - PhoenixRC Controllers. Works with the USB adapter (VID: 0x1781, PID: 0x0898), whose original and clone revisions expose different HID layouts. The two case selectors choose adapter mode/protocol; they are not HID inputs.

## Key Entry Points

- Library entry: `src/lib.rs` - provides `SailiDevice::connect()` for programmatic access to the adapter
- TUI entry: `src/main.rs` -> `src/app.rs::App::run()` - contains the live terminal UI
- Diagnostics: `read_saili.py` - Python script for adapter testing
- Web console: `web/index.html` and `web/app.mjs` - browser-based FC3 serial diagnostics

## Commands

### Build and Run
- `cargo run --release` - Build and run the TUI application
- `cargo build` - Build library and binaries (Debug mode)
- `cargo test` - Run library and protocol tests
- `mise run tank-console` - Serve the Web Serial console locally
- `mise run tank-console-build` - Test and build the dependency-free console

### Test & Verify
- `mise check` - Run all checks (lint, typecheck, test)

### Common Workflows
- Development: `cargo run` (Debug) or `cargo run --release` (optimized)
- Quick test: `cargo test --lib` (library tests only)
- Integration test setup: Requires SAILI adapter connected (PhoenixRC mode)

## Architecture

**One package project** (`src/lib.rs` is a library, `src/main.rs` depends on it)

**Library API**:
- `SailiDevice::connect()` -> `Result<SailiDevice, SailiError>` - discovers and opens the adapter
- `SailiDevice::spawn_reader()` - returns a typed result and drains reports independently of TUI redraws
- `RawReport`, `ReportFormat`, `Decoder`, and `DecodedState` - typed raw and semantic protocol layers

**TUI Architecture**:
- Main runs `app::run()` with continuous device polling
- Polls device every 10ms, redraws UI every 40ms
- Exits on 'q', Escape, or Ctrl-C
- Shows eight channel gauges, decoder/mux status, reader statistics, raw packet bytes, mapping status, and live/safe output state
- Starts without the HID adapter so ESPHome status and the mapping modal remain usable
- Press `m` to map all eight analogue inputs to ROLL, PITCH, THROTTLE, YAW, AUX2, AUX3, AUX4, and AUX5

## Dependencies & Path

**Dependencies** ( Cargo.toml ):
- `crossterm` - Terminal UI event handling  
- `hidapi` - HID API for USB communication
- `ratatui` - Rich TUI framework
- `thiserror` - Error handling

## Testing Quirks

**Protocol tests** (`tests/protocol.rs`):
- Tests packet decoding with `[10, 1, 20, 30, 40, 50, 60, 70]` data
- Analogue channel byte indices are non-standard: [0, 2, 3, 4, 5, 6, 7]
- Clone byte index 1 is never treated as an arm switch; it is an analogue input only in explicit Linux-demuxed mode. Legacy button behavior is display-only and available only in explicit legacy mode.
- Short reports rejected with `PacketError::UnexpectedLength`

**HID report contract**:
- Reports are exactly 8 bytes. Clone formats expose eight analogue inputs; raw clones multiplex byte 7 across alternating reports and require guided phase calibration plus cadence-loss fail-closed handling, while Linux `hid-pxrc` reports use byte 1 and byte 7 as the final two axes.
- The SAILI case selectors do not change HID reports; they only select adapter mode/protocol
- The TUI mapping modal explicitly maps all eight analogue inputs to four primary outputs and AUX2-AUX5
- CRSF AUX1/channel 5 is controlled by live/safe state: high while live, low in safe hold or failsafe

**Hardware requirements**:
- Physical SAILI adapter required for integration tests
- Adapter must be in PhoenixRC mode
- No sudo needed - uses HID interface (libusb not required)

**Debugging**:
- Check adapter connection: `system_profiler SPUSBDataType | grep -A 12 -iE 'SAILI|Phoenix'`
- Test adapter: `read_saili.py` requires Python with `uv`

## Toolchain

**Setup**:
- macOS (native HID support)
- Rust toolchain (stable)
- Python with `uv` for diagnostics
- `brew install rust` if needed

**Package order**: No order-sensitive commands
- Library: Tests first, then binary
- Binary has no test harness dependent on package order

## Code Conventions

- `std::fmt::Error` aliased to `SailiError` for termination
- `Debug` and `Error` derive always used for error types
- `Channel_COUNT` constants, `VENDOR_ID`, `PRODUCT_ID` defined as `pub const`
- `Result<, SailiError>` and `Result<, AppError>` dominate error handling
- Panic-free in library, recoverable errors everywhere

## File Ownership

**Library/API code**: `src/lib.rs` - maintains public API
**Application**: `src/app.rs` - UI and main logic  
**Diagnostics**: `read_saili.py` - lightweight Python debugging
**Web console**: `web/` - browser UI, telemetry parser, and console tests

The web console hides `TANK state:` rows by default through the unchecked
**Show TANK state rows** control. The filter only affects terminal display;
parsing and downloaded logs still include those rows. See `web/README.md` for
the local workflow and build details.

## Generated/Build Artifacts

**Build products**:
- `target/debug/saili` - Debug TUI binary
- `target/debug/build/` - Build script outputs
- `target/debug/libc-*` - Dependency build intermediates

**No source generators**: Code is hand-written, no code gen dependencies.
