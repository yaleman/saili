const NUMBER = "([+-]?\\d+(?:\\.\\d+)?)";

const TANK_PATTERN = new RegExp(
  `^TANK state:(.+?) rx:(\\d+) arm:(\\d+) drive:${NUMBER} turn:${NUMBER} ` +
    `left:${NUMBER} right:${NUMBER} range\\[` +
    `F:(!?)${NUMBER} B:(!?)${NUMBER} L:(!?)${NUMBER} R:(!?)${NUMBER}\\] ` +
    `gps:(-?\\d+) sat:(\\d+) lat:${NUMBER} lon:${NUMBER}$`,
);

export function createTelemetryState() {
  return {
    tank: null,
    gps: {
      fix: null,
      satellites: null,
      latitude: null,
      longitude: null,
      altitude: null,
    },
    battery: {
      voltage: null,
      current: null,
    },
    barometer: {
      altitude: null,
      pressure: null,
      temperature: null,
    },
    attitude: {
      roll: null,
      pitch: null,
      yaw: null,
    },
    lastUpdate: null,
  };
}

function rangeReading(invalid, value) {
  return {
    valid: invalid !== "!",
    metres: Number.parseFloat(value),
  };
}

export function parseTankLine(line) {
  const match = TANK_PATTERN.exec(line.trim());
  if (!match) {
    return null;
  }

  return {
    state: match[1],
    receiverConnected: match[2] === "1",
    armed: match[3] === "1",
    drive: Number.parseFloat(match[4]),
    turn: Number.parseFloat(match[5]),
    left: Number.parseFloat(match[6]),
    right: Number.parseFloat(match[7]),
    ranges: {
      front: rangeReading(match[8], match[9]),
      back: rangeReading(match[10], match[11]),
      left: rangeReading(match[12], match[13]),
      right: rangeReading(match[14], match[15]),
    },
    gps: {
      fix: Number.parseInt(match[16], 10),
      satellites: Number.parseInt(match[17], 10),
      latitude: Number.parseFloat(match[18]),
      longitude: Number.parseFloat(match[19]),
    },
  };
}

export function parseNumericFields(line) {
  const fields = {};
  const pattern = /([A-Za-z][A-Za-z0-9_.%]*):([+-]?\d+(?:\.\d+)?)/g;
  for (const match of line.matchAll(pattern)) {
    fields[match[1]] = Number.parseFloat(match[2]);
  }
  return fields;
}

export function applyTelemetryLine(state, rawLine, now = Date.now()) {
  const line = rawLine.trim();
  if (!line) {
    return false;
  }

  const tank = parseTankLine(line);
  if (tank) {
    state.tank = tank;
    state.gps.fix = tank.gps.fix;
    state.gps.satellites = tank.gps.satellites;
    state.gps.latitude = tank.gps.latitude;
    state.gps.longitude = tank.gps.longitude;
    state.lastUpdate = now;
    return true;
  }

  const fields = parseNumericFields(line);
  let updated = false;

  if ("gps.time" in fields) {
    state.gps.fix = fields.fix ?? state.gps.fix;
    state.gps.satellites = fields.sat ?? state.gps.satellites;
    state.gps.latitude =
      fields.lat === undefined ? state.gps.latitude : fields.lat / 10_000_000;
    state.gps.longitude =
      fields.lon === undefined ? state.gps.longitude : fields.lon / 10_000_000;
    state.gps.altitude = fields.alt ?? state.gps.altitude;
    updated = true;
  }

  if ("bat.v" in fields) {
    state.battery.voltage = fields["bat.v"];
    state.battery.current = fields["bat.i"] ?? state.battery.current;
    updated = true;
  }

  if ("bar.alt" in fields) {
    state.barometer.altitude = fields["bar.alt"];
    state.barometer.pressure = fields.press ?? state.barometer.pressure;
    state.barometer.temperature = fields.temp ?? state.barometer.temperature;
    updated = true;
  }

  if ("rcl.throttle" in fields && state.tank) {
    state.tank.receiverConnected =
      fields.connected === undefined
        ? state.tank.receiverConnected
        : fields.connected === 1;
    state.tank.armed =
      fields.armed === undefined ? state.tank.armed : fields.armed === 1;
    state.tank.drive =
      fields.pitch === undefined ? state.tank.drive : -fields.pitch;
    state.tank.turn = fields.roll ?? state.tank.turn;
    updated = true;
  } else if (
    "roll" in fields &&
    "pitch" in fields &&
    "yaw" in fields
  ) {
    state.attitude.roll = fields.roll;
    state.attitude.pitch = fields.pitch;
    state.attitude.yaw = fields.yaw;
    updated = true;
  }

  if (updated) {
    state.lastUpdate = now;
  }
  return updated;
}
