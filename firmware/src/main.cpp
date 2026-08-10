#include "madflight_config.h"

#define VEH_TYPE VEH_TYPE_ROVER
#define VEH_FLIGHTMODE_AP_IDS \
    {AP_ROVER_FLIGHTMODE_MANUAL, AP_ROVER_FLIGHTMODE_MANUAL, \
     AP_ROVER_FLIGHTMODE_MANUAL, AP_ROVER_FLIGHTMODE_MANUAL, \
     AP_ROVER_FLIGHTMODE_MANUAL, AP_ROVER_FLIGHTMODE_MANUAL}
#define VEH_FLIGHTMODE_NAMES \
    {"MANUAL", "MANUAL", "MANUAL", "MANUAL", "MANUAL", "MANUAL"}

#include <madflight.h>

#include <hardware/watchdog.h>

#include <array>
#include <cstdint>

#include "range_telemetry.h"
#include "reversible_esc.h"
#include "tank_core.h"
#include "ultrasonic_array.h"

namespace {

constexpr int kLeftEscPin = 6;
constexpr int kRightEscPin = 7;
constexpr bool kLeftEscReversed = false;
constexpr bool kRightEscReversed = false;
constexpr std::uint32_t kControlLoopDelayMs = 5;
constexpr std::uint32_t kRangeTelemetryIntervalMs = 200;
constexpr std::uint32_t kDiagnosticIntervalMs = 1000;
constexpr std::uint32_t kHardwareWatchdogMs = 500;
constexpr std::uint32_t kLedArmedRgb = 0xFF0000;
constexpr std::uint32_t kLedFailsafeRgb = 0xFF6000;
constexpr std::uint32_t kLedSafeRgb = 0x00FF00;

tank::DriveConfig drive_config = {
    .input_deadband = 0.05F,
    .maximum_command = 0.60F,
    .acceleration_per_second = 2.0F,
    .deceleration_per_second = 4.0F,
    .reverse_neutral_ms = 150,
};
tank::DriveController drive_controller(drive_config);
tank::ReversibleEsc esc;
constexpr std::array range_sensor_config = {
    tank::RangeSensorConfig{
        .direction = tank::RangeDirection::Front,
        .type = tank::RangeSensorType::Tk50,
        .interface = tank::SensorInterface::tca9548a(2, 3, 0),
    },
    tank::RangeSensorConfig{
        .direction = tank::RangeDirection::Rear,
        .type = tank::RangeSensorType::Tk50,
        .interface = tank::SensorInterface::tca9548a(2, 3, 1),
    },
    tank::RangeSensorConfig{
        .direction = tank::RangeDirection::Left,
        .type = tank::RangeSensorType::Tk50,
        .interface = tank::SensorInterface::tca9548a(2, 3, 2),
    },
    tank::RangeSensorConfig{
        .direction = tank::RangeDirection::Right,
        .type = tank::RangeSensorType::Tk50,
        .interface = tank::SensorInterface::tca9548a(2, 3, 3),
    },
};
tank::UltrasonicArray ultrasonic(range_sensor_config);
tank::RangeTelemetry range_telemetry;
MF_Serial *crsf_serial = nullptr;
std::uint32_t last_range_telemetry_ms = 0;
std::uint32_t last_diagnostic_ms = 0;

void print_diagnostics(
    const tank::DriveInput &input,
    const tank::DriveOutput &output) {
    const tank::RangeReading &front =
        ultrasonic.reading(tank::RangeDirection::Front);
    const tank::RangeReading &rear =
        ultrasonic.reading(tank::RangeDirection::Rear);
    const tank::RangeReading &left =
        ultrasonic.reading(tank::RangeDirection::Left);
    const tank::RangeReading &right =
        ultrasonic.reading(tank::RangeDirection::Right);
    const tank::RangeReading &up =
        ultrasonic.reading(tank::RangeDirection::Up);
    const tank::RangeReading &down =
        ultrasonic.reading(tank::RangeDirection::Down);
    Serial.printf(
        "TANK state:%s rx:%d arm:%d drive:%+.2f turn:%+.2f "
        "left:%+.2f right:%+.2f range[F:%s%.2f B:%s%.2f "
        "L:%s%.2f R:%s%.2f U:%s%.2f D:%s%.2f] gps:%d sat:%u "
        "lat:%.7f lon:%.7f\n",
        tank::drive_state_name(output.state),
        input.receiver_connected,
        input.arm_signal,
        input.drive,
        input.turn,
        output.tracks.left,
        output.tracks.right,
        front.status == tank::RangeReadingStatus::Valid ? "" : "!",
        front.metres,
        rear.status == tank::RangeReadingStatus::Valid ? "" : "!",
        rear.metres,
        left.status == tank::RangeReadingStatus::Valid ? "" : "!",
        left.metres,
        right.status == tank::RangeReadingStatus::Valid ? "" : "!",
        right.metres,
        up.status == tank::RangeReadingStatus::Valid ? "" : "!",
        up.metres,
        down.status == tank::RangeReadingStatus::Valid ? "" : "!",
        down.metres,
        static_cast<int>(gps.fix),
        gps.sat,
        static_cast<double>(gps.lat) / 10'000'000.0,
        static_cast<double>(gps.lon) / 10'000'000.0);
}

}  // namespace

void setup() {
    madflight_setup();

    if (!esc.begin(
            kLeftEscPin,
            kRightEscPin,
            kLeftEscReversed,
            kRightEscReversed)) {
        madflight_panic("Tank ESC output initialization failed.");
    }
    esc.neutral();

    const tank::RangeSensorInitResult range_init = ultrasonic.begin();
    if (!range_init.ok()) {
        Serial.printf(
            "Range sensor %u initialization failed: %s\n",
            static_cast<unsigned>(range_init.sensor_index),
            tank::range_sensor_init_error_name(range_init.error));
        madflight_panic("Range sensor initialization failed.");
    }

    crsf_serial = hal_get_ser_bus(0, 420000);
    if (crsf_serial == nullptr) {
        madflight_panic("CRSF telemetry serial port unavailable.");
    }

    watchdog_enable(kHardwareWatchdogMs, false);
    Serial.println(
        "Tank firmware ready: CRSF pitch/roll arcade drive, AUX1 arm.");
}

void loop() {
    const std::uint32_t now_ms = millis();
    ultrasonic.update();

    // madflight normalizes pitch opposite to the desired ground-vehicle
    // direction, so negate it to make forward stick produce forward motion.
    const tank::DriveInput input = {
        .receiver_connected = rcl.connected(),
        .arm_signal = rcl.armed,
        .drive = -rcl.pitch,
        .turn = rcl.roll,
    };
    const tank::DriveOutput output = drive_controller.update(input, now_ms);
    esc.write(output.tracks);

    if (now_ms - last_range_telemetry_ms
        >= kRangeTelemetryIntervalMs) {
        last_range_telemetry_ms = now_ms;
        range_telemetry.send(*crsf_serial, ultrasonic);
    }

    if (now_ms - last_diagnostic_ms >= kDiagnosticIntervalMs) {
        last_diagnostic_ms = now_ms;
        print_diagnostics(input, output);
    }

    if (output.state == tank::DriveState::Armed) {
        led.color(kLedArmedRgb);
    } else if (output.state == tank::DriveState::Failsafe) {
        led.color(kLedFailsafeRgb);
    } else {
        led.color(kLedSafeRgb);
    }

    watchdog_update();
    delay(kControlLoopDelayMs);
}

void imu_loop() {
    ahr.update();
}
