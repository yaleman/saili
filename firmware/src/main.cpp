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
tank::UltrasonicArray ultrasonic({
    tank::UltrasonicPins{.trigger = 10, .echo = 11},
    tank::UltrasonicPins{.trigger = 12, .echo = 13},
    tank::UltrasonicPins{.trigger = 14, .echo = 15},
    tank::UltrasonicPins{.trigger = 16, .echo = 17},
});
tank::RangeTelemetry range_telemetry;
MF_Serial *crsf_serial = nullptr;
std::uint32_t last_range_telemetry_ms = 0;
std::uint32_t last_diagnostic_ms = 0;

void print_diagnostics(
    const tank::DriveInput &input,
    const tank::DriveOutput &output) {
    const auto &ranges = ultrasonic.readings();
    Serial.printf(
        "TANK state:%s rx:%d arm:%d drive:%+.2f turn:%+.2f "
        "left:%+.2f right:%+.2f range[F:%s%.2f B:%s%.2f "
        "L:%s%.2f R:%s%.2f] gps:%d sat:%u lat:%.7f lon:%.7f\n",
        tank::drive_state_name(output.state),
        input.receiver_connected,
        input.arm_signal,
        input.drive,
        input.turn,
        output.tracks.left,
        output.tracks.right,
        ranges[0].valid ? "" : "!",
        ranges[0].metres,
        ranges[1].valid ? "" : "!",
        ranges[1].metres,
        ranges[2].valid ? "" : "!",
        ranges[2].metres,
        ranges[3].valid ? "" : "!",
        ranges[3].metres,
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

    if (!ultrasonic.begin()) {
        madflight_panic("Ultrasonic sensor initialization failed.");
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
