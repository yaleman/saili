import { applyTelemetryLine, createTelemetryState } from "./parser.mjs";

const BAUD_RATE = 115200;
const MAX_LOG_LINES = 1200;
const DEFAULT_STREAM_COMMANDS = [
  "prcl",
  "ppwm",
  "pgps",
  "pbar",
  "pbat",
  "pahr",
];
const DANGEROUS_COMMANDS = new Map([
  ["defaults", "This clears saved MadFlight configuration and reboots the FC3."],
  ["save", "This writes the current configuration and reboots the FC3."],
  ["reboot", "This immediately reboots the FC3 and interrupts all control and telemetry."],
  ["spinmotors", "This is a motor command. The tank driver does not support it, but it should not be sent casually."],
  ["calimu", "This starts an interactive IMU calibration and requires the vehicle to be positioned as instructed."],
  ["calmag", "This starts an interactive magnetometer calibration."],
  ["calradio", "This changes receiver calibration values."],
  ["serial", "This takes over a hardware UART and can interrupt CRSF or GPS reception."],
]);

const elements = Object.fromEntries(
  [
    "connection-dot",
    "connection-status",
    "port-identity",
    "connect-button",
    "browser-warning",
    "drive-state",
    "receiver-state",
    "arm-state",
    "drive-bar",
    "turn-bar",
    "left-bar",
    "right-bar",
    "drive-value",
    "turn-value",
    "left-value",
    "right-value",
    "range-front",
    "range-back",
    "range-left",
    "range-right",
    "range-age",
    "gps-fix",
    "gps-satellites",
    "gps-latitude",
    "gps-longitude",
    "gps-altitude",
    "map-link",
    "battery",
    "barometer",
    "temperature",
    "attitude",
    "led-current",
    "led-current-label",
    "last-update",
    "terminal-output",
    "autoscroll",
    "show-tank-state",
    "download-log",
    "clear-log",
    "command-form",
    "command-input",
    "start-streams",
    "stop-streams",
    "byte-count",
    "command-warning",
    "warning-title",
    "warning-message",
  ].map((id) => [id, document.getElementById(id)]),
);

const commandButtons = [...document.querySelectorAll(".command")];
const toggleButtons = [...document.querySelectorAll(".command.toggle")];
const sendButton = elements["command-form"].querySelector('button[type="submit"]');

let port = null;
let reader = null;
let writer = null;
let keepReading = false;
let bytesReceived = 0;
let lineBuffer = "";
let telemetry = createTelemetryState();
let logLines = [];
let commandHistory = [];
let historyIndex = 0;
let writeQueue = Promise.resolve();

function setConnectionState(state, detail = "") {
  const connected = state === "connected";
  elements["connection-dot"].className = `status-dot ${state}`;
  elements["connection-status"].textContent =
    state === "connected"
      ? "Connected"
      : state === "connecting"
        ? "Connecting"
        : "Disconnected";
  elements["port-identity"].textContent = detail;
  elements["connect-button"].textContent = connected ? "Disconnect" : "Connect USB";
  elements["connect-button"].disabled = state === "connecting";
  elements["command-input"].disabled = !connected;
  sendButton.disabled = !connected;
  elements["start-streams"].disabled = !connected;
  elements["stop-streams"].disabled = !connected;
  for (const button of commandButtons) {
    button.disabled = !connected;
  }
  if (!connected) {
    for (const button of toggleButtons) {
      button.classList.remove("active");
      button.setAttribute("aria-pressed", "false");
    }
  }
}

function formatUsbIdentity(selectedPort) {
  const info = selectedPort.getInfo?.() ?? {};
  if (info.usbVendorId === undefined || info.usbProductId === undefined) {
    return "115200 baud";
  }
  const vendor = info.usbVendorId.toString(16).padStart(4, "0").toUpperCase();
  const product = info.usbProductId.toString(16).padStart(4, "0").toUpperCase();
  return `${vendor}:${product} · 115200 baud`;
}

function appendTerminal(line, kind = "serial") {
  const entry = {
    timestamp: new Date(),
    line,
    kind,
  };
  logLines.push(entry);
  if (logLines.length > MAX_LOG_LINES) {
    logLines = logLines.slice(-MAX_LOG_LINES);
    elements["terminal-output"].firstElementChild?.remove();
  }

  const row = document.createElement("div");
  const isTankState = line.trimStart().startsWith("TANK state:");
  row.className = `terminal-line ${kind}${isTankState ? " tank-state" : ""}`;
  row.hidden =
    isTankState && !(elements["show-tank-state"]?.checked ?? false);
  const time = entry.timestamp.toLocaleTimeString([], {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  row.textContent = `${time}  ${line}`;
  elements["terminal-output"].append(row);

  if (elements.autoscroll.checked) {
    elements["terminal-output"].scrollTop =
      elements["terminal-output"].scrollHeight;
  }
}

function handleLine(line) {
  appendTerminal(line);
  if (applyTelemetryLine(telemetry, line)) {
    renderTelemetry();
  }
}

function handleChunk(chunk) {
  lineBuffer += chunk.replaceAll("\r", "");
  const lines = lineBuffer.split("\n");
  lineBuffer = lines.pop() ?? "";
  for (const line of lines) {
    handleLine(line);
  }
}

async function readSerial() {
  const decoder = new TextDecoder();
  keepReading = true;
  try {
    while (port?.readable && keepReading) {
      reader = port.readable.getReader();
      try {
        while (keepReading) {
          const { value, done } = await reader.read();
          if (done) {
            break;
          }
          if (value) {
            bytesReceived += value.byteLength;
            elements["byte-count"].textContent =
              `${bytesReceived.toLocaleString()} bytes received`;
            handleChunk(decoder.decode(value, { stream: true }));
          }
        }
      } finally {
        reader.releaseLock();
        reader = null;
      }
    }
  } catch (error) {
    if (keepReading) {
      appendTerminal(`Serial read failed: ${error.message}`, "error");
    }
  } finally {
    if (keepReading) {
      await disconnectSerial();
    }
  }
}

async function connectSerial() {
  if (port) {
    await disconnectSerial();
    return;
  }

  setConnectionState("connecting");
  try {
    const authorized = await navigator.serial.getPorts();
    port =
      authorized.length === 1
        ? authorized[0]
        : await navigator.serial.requestPort();
    await port.open({
      baudRate: BAUD_RATE,
      dataBits: 8,
      stopBits: 1,
      parity: "none",
      flowControl: "none",
    });
    writer = port.writable.getWriter();
    bytesReceived = 0;
    lineBuffer = "";
    setConnectionState("connected", formatUsbIdentity(port));
    appendTerminal("USB serial connected.", "local");
    void readSerial();
    requestAllSensorStreams();
    elements["command-input"].focus();
  } catch (error) {
    port = null;
    writer = null;
    setConnectionState("disconnected");
    if (error.name !== "NotFoundError") {
      appendTerminal(`Connection failed: ${error.message}`, "error");
    }
  }
}

async function disconnectSerial() {
  keepReading = false;
  try {
    await reader?.cancel();
  } catch {
    // A disconnected USB device may already have invalidated the reader.
  }
  reader?.releaseLock();
  reader = null;
  writer?.releaseLock();
  writer = null;
  try {
    await port?.close();
  } catch {
    // Closing an already-disconnected port is harmless.
  }
  port = null;
  setConnectionState("disconnected");
  appendTerminal("USB serial disconnected.", "local");
}

function commandKey(command) {
  return command.trim().split(/\s+/, 1)[0].toLowerCase();
}

async function confirmCommand(command) {
  const warning = DANGEROUS_COMMANDS.get(commandKey(command));
  if (!warning) {
    return true;
  }
  elements["warning-title"].textContent = `Send “${command}”?`;
  elements["warning-message"].textContent = warning;
  elements["command-warning"].showModal();
  const result = await new Promise((resolve) => {
    elements["command-warning"].addEventListener(
      "close",
      () => resolve(elements["command-warning"].returnValue),
      { once: true },
    );
  });
  return result === "confirm";
}

async function sendCommand(command, requireConfirmation = true) {
  const normalized = command.trim();
  if (!normalized || !writer) {
    return;
  }
  if (requireConfirmation && !(await confirmCommand(normalized))) {
    appendTerminal(`Cancelled: ${normalized}`, "local");
    return;
  }

  const payload = new TextEncoder().encode(`${normalized}\n`);
  writeQueue = writeQueue.then(() => writer?.write(payload));
  await writeQueue;
  appendTerminal(`> ${normalized}`, "command");
}

async function enableDefaultStreams() {
  await sendCommand("poff", false);
  for (const command of DEFAULT_STREAM_COMMANDS) {
    await sendCommand(command, false);
  }
  for (const button of toggleButtons) {
    const active = DEFAULT_STREAM_COMMANDS.includes(button.dataset.command);
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  }
  appendTerminal("All identified sensor streams enabled.", "local");
}

function requestAllSensorStreams() {
  void enableDefaultStreams().catch((error) => {
    appendTerminal(
      `Could not enable sensor streams: ${error.message}`,
      "error",
    );
  });
}

function setAxis(element, value) {
  const bounded = Math.max(-1, Math.min(1, value ?? 0));
  element.style.setProperty("--value", `${Math.abs(bounded) * 50}%`);
  element.style.setProperty("--direction", bounded < 0 ? "-1" : "1");
  element.classList.toggle("negative", bounded < 0);
}

function renderRange(id, reading) {
  const element = elements[`range-${id}`];
  if (!reading?.valid) {
    element.textContent = "NO ECHO";
    element.parentElement.classList.add("invalid");
    return;
  }
  element.textContent = `${reading.metres.toFixed(2)} m`;
  element.parentElement.classList.remove("invalid");
}

function stateClass(state) {
  if (state === "MANUAL") {
    return "armed";
  }
  if (state === "FAILSAFE") {
    return "danger";
  }
  if (state === "NEUTRAL REQUIRED") {
    return "warning";
  }
  return "neutral";
}

function renderExpectedLed(state) {
  const expected =
    state === "MANUAL"
      ? { key: "armed", colour: "RED" }
      : state === "FAILSAFE"
        ? { key: "failsafe", colour: "ORANGE" }
        : { key: "safe", colour: "GREEN" };
  elements["led-current"].className = `led-current ${expected.key}`;
  elements["led-current-label"].textContent =
    `${expected.colour} · ${state}`;
}

function present(value, suffix = "", digits = 2) {
  return value === null || Number.isNaN(value)
    ? "—"
    : `${value.toFixed(digits)}${suffix}`;
}

function renderTelemetry() {
  const { tank, gps, battery, barometer, attitude } = telemetry;
  if (tank) {
    elements["drive-state"].textContent = tank.state;
    elements["drive-state"].className =
      `state-badge ${stateClass(tank.state)}`;
    elements["receiver-state"].textContent =
      tank.receiverConnected ? "Connected" : "Lost";
    elements["receiver-state"].className =
      tank.receiverConnected ? "good" : "bad";
    elements["arm-state"].textContent = tank.armed ? "Armed" : "Disarmed";
    elements["arm-state"].className = tank.armed ? "bad" : "good";
    renderExpectedLed(tank.state);

    for (const key of ["drive", "turn", "left", "right"]) {
      elements[`${key}-value`].textContent = tank[key].toFixed(2);
      setAxis(elements[`${key}-bar`], tank[key]);
    }
    for (const direction of ["front", "back", "left", "right"]) {
      renderRange(direction, tank.ranges[direction]);
    }
    elements["range-age"].textContent = "Live";
  }

  const hasFix = (gps.fix ?? 0) > 0 && (gps.satellites ?? 0) > 0;
  elements["gps-fix"].textContent = hasFix ? `FIX ${gps.fix}` : "NO FIX";
  elements["gps-fix"].className =
    `state-badge ${hasFix ? "good-badge" : "neutral"}`;
  elements["gps-satellites"].textContent =
    gps.satellites === null ? "—" : String(gps.satellites);
  elements["gps-latitude"].textContent = present(gps.latitude, "", 7);
  elements["gps-longitude"].textContent = present(gps.longitude, "", 7);
  elements["gps-altitude"].textContent = present(gps.altitude, " m", 1);

  if (hasFix && gps.latitude !== null && gps.longitude !== null) {
    elements["map-link"].href =
      `https://www.openstreetmap.org/?mlat=${gps.latitude}&mlon=${gps.longitude}` +
      `#map=18/${gps.latitude}/${gps.longitude}`;
    elements["map-link"].classList.remove("disabled");
    elements["map-link"].classList.remove("hidden");
    elements["map-link"].setAttribute("aria-disabled", "false");
  } else {
    elements["map-link"].href = "#no-gps-fix";
    elements["map-link"].hidden = true;
    elements["map-link"].classList.add("disabled");
    elements["map-link"].classList.add("hidden");
    elements["map-link"].setAttribute("aria-disabled", "true");
  }

  const voltage = present(battery.voltage, " V", 2);
  const current = present(battery.current, " A", 2);
  elements.battery.textContent =
    battery.voltage === null ? "—" : `${voltage} · ${current}`;
  elements.barometer.textContent =
    barometer.pressure === null
      ? present(barometer.altitude, " m", 2)
      : `${present(barometer.altitude, " m", 2)} · ${present(barometer.pressure, " Pa", 0)}`;
  elements.temperature.textContent = present(
    barometer.temperature,
    " °C",
    2,
  );
  elements.attitude.textContent =
    attitude.roll === null
      ? "—"
      : `R ${present(attitude.roll, "°", 1)} · P ${present(attitude.pitch, "°", 1)} · Y ${present(attitude.yaw, "°", 1)}`;
  updateAge();
}

function updateAge() {
  if (telemetry.lastUpdate === null) {
    elements["last-update"].textContent = "No frames";
    return;
  }
  const age = Date.now() - telemetry.lastUpdate;
  elements["last-update"].textContent =
    age < 1000 ? "Live" : `${Math.floor(age / 1000)} s ago`;
  elements["range-age"].textContent =
    age < 2500 ? "Live" : `${Math.floor(age / 1000)} s stale`;
  elements["range-age"].classList.toggle("stale", age >= 2500);
}

elements["connect-button"].addEventListener("click", () => {
  void connectSerial();
});

elements["command-form"].addEventListener("submit", (event) => {
  event.preventDefault();
  const command = elements["command-input"].value;
  if (!command.trim()) {
    return;
  }
  commandHistory.push(command);
  historyIndex = commandHistory.length;
  elements["command-input"].value = "";
  void sendCommand(command);
});

elements["command-input"].addEventListener("keydown", (event) => {
  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") {
    return;
  }
  event.preventDefault();
  if (event.key === "ArrowUp" && historyIndex > 0) {
    historyIndex -= 1;
  } else if (
    event.key === "ArrowDown" &&
    historyIndex < commandHistory.length
  ) {
    historyIndex += 1;
  }
  elements["command-input"].value =
    commandHistory[historyIndex] ?? "";
});

for (const button of commandButtons) {
  button.addEventListener("click", () => {
    const command = button.dataset.command;
    if (button.classList.contains("toggle")) {
      const active = !button.classList.contains("active");
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
    void sendCommand(command, false);
  });
}

elements["stop-streams"].addEventListener("click", () => {
  for (const button of toggleButtons) {
    button.classList.remove("active");
    button.setAttribute("aria-pressed", "false");
  }
  void sendCommand("poff", false);
});

elements["start-streams"].addEventListener("click", () => {
  requestAllSensorStreams();
});

elements["clear-log"].addEventListener("click", () => {
  logLines = [];
  elements["terminal-output"].replaceChildren();
  appendTerminal("Console cleared.", "local");
});

elements["show-tank-state"]?.addEventListener("change", () => {
  for (const row of elements["terminal-output"].querySelectorAll(".tank-state")) {
    row.hidden = !elements["show-tank-state"].checked;
  }
});

elements["download-log"].addEventListener("click", () => {
  const content = logLines
    .map(
      ({ timestamp, line, kind }) =>
        `${timestamp.toISOString()} [${kind}] ${line}`,
    )
    .join("\n");
  const url = URL.createObjectURL(new Blob([content], { type: "text/plain" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `saili-tank-console-${new Date().toISOString().replaceAll(":", "-")}.log`;
  link.click();
  URL.revokeObjectURL(url);
});

elements["map-link"].addEventListener("click", (event) => {
  if (elements["map-link"].getAttribute("aria-disabled") === "true") {
    event.preventDefault();
  }
});

if ("serial" in navigator) {
  navigator.serial.addEventListener("disconnect", (event) => {
    if (event.target === port) {
      void disconnectSerial();
    }
  });
  setConnectionState("disconnected");
} else {
  elements["browser-warning"].hidden = false;
  elements["connect-button"].disabled = true;
  elements["connect-button"].textContent = "Web Serial unavailable";
}

setInterval(updateAge, 500);
