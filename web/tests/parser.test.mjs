import assert from "node:assert/strict";
import test from "node:test";

import {
  applyTelemetryLine,
  createTelemetryState,
  parseNumericFields,
  parseTankLine,
} from "../parser.mjs";

test("parses the tank diagnostic line including invalid echoes", () => {
  const parsed = parseTankLine(
    "TANK state:NEUTRAL REQUIRED rx:1 arm:1 drive:+0.25 turn:-0.50 " +
      "left:+0.10 right:-0.20 range[F:1.23 B:!0.00 L:0.45 R:2.50 " +
      "U:3.10 D:!0.00] " +
      "gps:3 sat:11 lat:-27.4700000 lon:153.1000000",
  );

  assert.deepEqual(parsed, {
    state: "NEUTRAL REQUIRED",
    receiverConnected: true,
    armed: true,
    drive: 0.25,
    turn: -0.5,
    left: 0.1,
    right: -0.2,
    ranges: {
      front: { valid: true, metres: 1.23 },
      rear: { valid: false, metres: 0 },
      left: { valid: true, metres: 0.45 },
      right: { valid: true, metres: 2.5 },
      up: { valid: true, metres: 3.1 },
      down: { valid: false, metres: 0 },
    },
    gps: {
      fix: 3,
      satellites: 11,
      latitude: -27.47,
      longitude: 153.1,
    },
  });
});

test("rejects unrelated console output", () => {
  assert.equal(parseTankLine("IMU setup complete"), null);
});

test("extracts signed numeric CLI fields", () => {
  assert.deepEqual(
    parseNumericFields("bat.v:12.40\tbat.i:+1.25\tbat.mah:-3.00"),
    {
      "bat.v": 12.4,
      "bat.i": 1.25,
      "bat.mah": -3,
    },
  );
});

test("combines streamed GPS battery barometer and attitude values", () => {
  const state = createTelemetryState();

  assert.equal(
    applyTelemetryLine(
      state,
      "gps.time:123 fix:3 sat:9 lat:-274700000 lon:1531000000 alt:18.500",
      10,
    ),
    true,
  );
  applyTelemetryLine(state, "bat.v:8.20 bat.i:+1.30 bat.mah:+20.00", 20);
  applyTelemetryLine(state, "bar.alt:12.50 press:100800.0 temp:24.75", 30);
  applyTelemetryLine(state, "roll:+1.5 pitch:-2.0 yaw:+90.0", 40);

  assert.deepEqual(state.gps, {
    fix: 3,
    satellites: 9,
    latitude: -27.47,
    longitude: 153.1,
    altitude: 18.5,
  });
  assert.deepEqual(state.battery, { voltage: 8.2, current: 1.3 });
  assert.deepEqual(state.barometer, {
    altitude: 12.5,
    pressure: 100800,
    temperature: 24.75,
  });
  assert.deepEqual(state.attitude, {
    roll: 1.5,
    pitch: -2,
    yaw: 90,
  });
  assert.equal(state.lastUpdate, 40);
});
