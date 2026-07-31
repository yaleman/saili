# SAILI Tank Console

The files in this directory are the dependency-free Web Serial console for the
MadFlight FC3v2 tank firmware.

## Run locally

From the repository root:

```bash
mise run tank-console
```

Open `http://localhost:8080` in desktop Chrome or Edge, then choose **Connect
USB** and select the FC3 serial device. Web Serial requires a secure context;
`localhost` qualifies. Safari and Firefox do not currently expose Web Serial.

## Console controls

The serial console displays raw lines and parsed telemetry. **Auto-scroll** is
enabled by default. **Show TANK state rows** is disabled by default, which
hides the high-frequency lines beginning with `TANK state:` while retaining
them for parsing and log downloads. **Download** always includes the complete
captured log, including hidden rows.

The parsed dashboard continues to update while TANK state rows are hidden.
The filter only changes terminal visibility; it does not stop or alter the
firmware stream.

## Test and build

Run the parser tests and generate the deployable static site with:

```bash
mise run tank-console-build
```

The test suite is in `tests/`, the source entry point is `app.mjs`, and
`build.py` copies the source files into the ignored `dist/client/` output used
by GitHub Pages. After changing the console while it is open, reload the page
before connecting so its HTML and JavaScript are from the same build.
