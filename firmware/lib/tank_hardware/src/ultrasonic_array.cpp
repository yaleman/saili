#include "ultrasonic_array.h"

#include <algorithm>
#include <cmath>
#include <limits>

namespace tank {
namespace {

constexpr std::uint32_t kEchoTimeoutUs = 25'000;
constexpr std::uint32_t kMeasurementSlotUs = 30'000;
constexpr std::uint32_t kReadingFreshnessMs = 500;
constexpr float kMinimumDistanceMetres = 0.02F;
constexpr float kMaximumDistanceMetres = 4.0F;
constexpr float kEchoMicrosecondsToMetres = 0.0001715F;

}  // namespace

UltrasonicArray *UltrasonicArray::instance_ = nullptr;

UltrasonicArray::UltrasonicArray(
    std::array<UltrasonicPins, kRangeSensorCount> pins)
    : pins_(pins) {}

bool UltrasonicArray::begin() {
    if (instance_ != nullptr) {
        return false;
    }
    instance_ = this;

    for (const UltrasonicPins &pin : pins_) {
        pinMode(pin.trigger, OUTPUT);
        digitalWrite(pin.trigger, LOW);
        pinMode(pin.echo, INPUT);
    }

    attachInterrupt(
        digitalPinToInterrupt(pins_[0].echo),
        front_echo_interrupt,
        CHANGE);
    attachInterrupt(
        digitalPinToInterrupt(pins_[1].echo),
        back_echo_interrupt,
        CHANGE);
    attachInterrupt(
        digitalPinToInterrupt(pins_[2].echo),
        left_echo_interrupt,
        CHANGE);
    attachInterrupt(
        digitalPinToInterrupt(pins_[3].echo),
        right_echo_interrupt,
        CHANGE);

    next_measurement_us_ = micros() + kMeasurementSlotUs;
    return true;
}

void UltrasonicArray::update() {
    const std::uint32_t now_us = micros();

    if (waiting_for_echo_) {
        noInterrupts();
        const std::uint32_t rise_us = echo_rise_us_;
        const std::uint32_t fall_us = echo_fall_us_;
        interrupts();

        if (rise_us != 0 && fall_us != 0) {
            const std::uint32_t duration_us = fall_us - rise_us;
            const float distance =
                static_cast<float>(duration_us) * kEchoMicrosecondsToMetres;
            RangeReading &reading = readings_[active_sensor_];
            if (distance >= kMinimumDistanceMetres
                && distance <= kMaximumDistanceMetres) {
                reading.metres = filters_[active_sensor_].push(distance);
                reading.updated_ms = millis();
                reading.valid = true;
            } else {
                reading.valid = false;
            }
            finish_measurement(now_us);
        } else if (
            now_us - measurement_started_us_ >= kEchoTimeoutUs) {
            readings_[active_sensor_].valid = false;
            finish_measurement(now_us);
        }
        return;
    }

    if (deadline_reached(now_us, next_measurement_us_)) {
        start_measurement(now_us);
    }
}

const std::array<RangeReading, kRangeSensorCount> &
UltrasonicArray::readings() const {
    return readings_;
}

std::array<std::uint16_t, kRangeSensorCount>
UltrasonicArray::millimetres() const {
    std::array<std::uint16_t, kRangeSensorCount> result{};
    const std::uint32_t now_ms = millis();
    for (std::size_t index = 0; index < readings_.size(); ++index) {
        const RangeReading &reading = readings_[index];
        if (!reading.valid
            || now_ms - reading.updated_ms > kReadingFreshnessMs) {
            result[index] = std::numeric_limits<std::uint16_t>::max();
            continue;
        }
        const float millimetres = reading.metres * 1000.0F;
        result[index] = static_cast<std::uint16_t>(
            std::clamp(
                std::lround(millimetres),
                0L,
                static_cast<long>(
                    std::numeric_limits<std::uint16_t>::max() - 1)));
    }
    return result;
}

std::uint8_t UltrasonicArray::valid_mask() const {
    std::uint8_t mask = 0;
    const std::uint32_t now_ms = millis();
    for (std::size_t index = 0; index < readings_.size(); ++index) {
        const RangeReading &reading = readings_[index];
        if (reading.valid
            && now_ms - reading.updated_ms <= kReadingFreshnessMs) {
            mask |= static_cast<std::uint8_t>(1U << index);
        }
    }
    return mask;
}

void UltrasonicArray::handle_echo_interrupt(std::size_t sensor_index) {
    if (!waiting_for_echo_ || sensor_index != active_sensor_) {
        return;
    }

    if (digitalRead(pins_[sensor_index].echo) == HIGH) {
        echo_rise_us_ = micros();
    } else if (echo_rise_us_ != 0) {
        echo_fall_us_ = micros();
    }
}

void UltrasonicArray::front_echo_interrupt() {
    if (instance_ != nullptr) {
        instance_->handle_echo_interrupt(0);
    }
}

void UltrasonicArray::back_echo_interrupt() {
    if (instance_ != nullptr) {
        instance_->handle_echo_interrupt(1);
    }
}

void UltrasonicArray::left_echo_interrupt() {
    if (instance_ != nullptr) {
        instance_->handle_echo_interrupt(2);
    }
}

void UltrasonicArray::right_echo_interrupt() {
    if (instance_ != nullptr) {
        instance_->handle_echo_interrupt(3);
    }
}

bool UltrasonicArray::deadline_reached(
    std::uint32_t now,
    std::uint32_t deadline) {
    return static_cast<std::int32_t>(now - deadline) >= 0;
}

void UltrasonicArray::start_measurement(std::uint32_t now_us) {
    active_sensor_ = (active_sensor_ + 1) % pins_.size();

    noInterrupts();
    echo_rise_us_ = 0;
    echo_fall_us_ = 0;
    interrupts();

    digitalWrite(pins_[active_sensor_].trigger, HIGH);
    delayMicroseconds(12);
    digitalWrite(pins_[active_sensor_].trigger, LOW);

    measurement_started_us_ = now_us;
    next_measurement_us_ = now_us + kMeasurementSlotUs;
    waiting_for_echo_ = true;
}

void UltrasonicArray::finish_measurement(std::uint32_t) {
    waiting_for_echo_ = false;
}

}  // namespace tank

