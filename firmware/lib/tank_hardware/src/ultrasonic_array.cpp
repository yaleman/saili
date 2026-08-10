#include "ultrasonic_array.h"

#include <algorithm>
#include <cmath>
#include <limits>

namespace tank {
namespace {

constexpr float kMinimumDistanceMetres = 0.02F;
constexpr float kMaximumDistanceMetres = 4.0F;
constexpr std::uint32_t kMinimumFreshnessMs = 1000;
constexpr std::uint32_t kFreshnessPerSensorMs = 300;
constexpr std::uint32_t kI2cHalfPeriodUs = 5;
constexpr std::uint32_t kClockStretchTimeoutUs = 1000;
constexpr std::uint8_t kMaximumGpio = 47;

std::size_t direction_index(RangeDirection direction) {
    return static_cast<std::size_t>(direction);
}

bool same_bus(const SensorInterface &left, const SensorInterface &right) {
    return left.sda_pin == right.sda_pin
        && left.scl_pin == right.scl_pin;
}

}  // namespace

const char *range_sensor_init_error_name(RangeSensorInitError error) {
    switch (error) {
        case RangeSensorInitError::None:
            return "none";
        case RangeSensorInitError::NoSensors:
            return "no sensors configured";
        case RangeSensorInitError::TooManySensors:
            return "too many sensors configured";
        case RangeSensorInitError::InvalidPins:
            return "invalid or identical SDA/SCL pins";
        case RangeSensorInitError::PinConflict:
            return "I2C buses reuse only one pin or reverse SDA/SCL";
        case RangeSensorInitError::InvalidMultiplexerIndex:
            return "TCA9548A channel must be between 0 and 7";
        case RangeSensorInitError::DirectAddressConflict:
            return "multiple direct TK50 sensors share one bus";
        case RangeSensorInitError::DirectMultiplexerConflict:
            return "direct TK50 and TCA9548A share one bus";
        case RangeSensorInitError::DuplicateMultiplexerChannel:
            return "multiple TK50 sensors use the same TCA9548A channel";
        case RangeSensorInitError::BusUnavailable:
            return "I2C bus did not become idle";
    }
    return "unknown sensor initialization error";
}

UltrasonicArray::SoftwareI2c::SoftwareI2c(
    std::uint8_t sda_pin,
    std::uint8_t scl_pin)
    : sda_pin_(sda_pin), scl_pin_(scl_pin) {}

bool UltrasonicArray::SoftwareI2c::begin() {
    digitalWrite(sda_pin_, LOW);
    digitalWrite(scl_pin_, LOW);
    pinMode(sda_pin_, INPUT);
    pinMode(scl_pin_, INPUT);
    delayMicroseconds(kI2cHalfPeriodUs);
    return digitalRead(sda_pin_) == HIGH
        && digitalRead(scl_pin_) == HIGH;
}

bool UltrasonicArray::SoftwareI2c::write(
    std::uint8_t address,
    const std::uint8_t *data,
    std::size_t size) {
    if (!start()) {
        return false;
    }
    if (!write_byte(static_cast<std::uint8_t>(address << 1U))) {
        stop();
        return false;
    }
    for (std::size_t index = 0; index < size; ++index) {
        if (!write_byte(data[index])) {
            stop();
            return false;
        }
    }
    stop();
    return true;
}

bool UltrasonicArray::SoftwareI2c::read(
    std::uint8_t address,
    std::uint8_t *data,
    std::size_t size) {
    if (size == 0 || !start()) {
        return false;
    }
    if (!write_byte(static_cast<std::uint8_t>((address << 1U) | 1U))) {
        stop();
        return false;
    }
    for (std::size_t index = 0; index < size; ++index) {
        if (!read_byte(data[index], index + 1 < size)) {
            stop();
            return false;
        }
    }
    stop();
    return true;
}

bool UltrasonicArray::SoftwareI2c::start() {
    release_sda();
    pinMode(scl_pin_, INPUT);
    if (!raise_scl() || digitalRead(sda_pin_) != HIGH) {
        return false;
    }
    delayMicroseconds(kI2cHalfPeriodUs);
    pull_sda_low();
    delayMicroseconds(kI2cHalfPeriodUs);
    pull_scl_low();
    return true;
}

void UltrasonicArray::SoftwareI2c::stop() {
    pull_sda_low();
    delayMicroseconds(kI2cHalfPeriodUs);
    if (raise_scl()) {
        delayMicroseconds(kI2cHalfPeriodUs);
        release_sda();
        delayMicroseconds(kI2cHalfPeriodUs);
    } else {
        release_sda();
    }
}

bool UltrasonicArray::SoftwareI2c::write_byte(std::uint8_t value) {
    for (std::uint8_t mask = 0x80; mask != 0; mask >>= 1U) {
        if ((value & mask) != 0) {
            release_sda();
        } else {
            pull_sda_low();
        }
        delayMicroseconds(kI2cHalfPeriodUs);
        if (!raise_scl()) {
            return false;
        }
        delayMicroseconds(kI2cHalfPeriodUs);
        pull_scl_low();
    }

    release_sda();
    delayMicroseconds(kI2cHalfPeriodUs);
    if (!raise_scl()) {
        return false;
    }
    const bool acknowledged = digitalRead(sda_pin_) == LOW;
    delayMicroseconds(kI2cHalfPeriodUs);
    pull_scl_low();
    return acknowledged;
}

bool UltrasonicArray::SoftwareI2c::read_byte(
    std::uint8_t &value,
    bool acknowledge) {
    value = 0;
    release_sda();
    for (std::uint8_t bit = 0; bit < 8; ++bit) {
        delayMicroseconds(kI2cHalfPeriodUs);
        if (!raise_scl()) {
            return false;
        }
        value = static_cast<std::uint8_t>(
            (value << 1U) | (digitalRead(sda_pin_) == HIGH ? 1U : 0U));
        delayMicroseconds(kI2cHalfPeriodUs);
        pull_scl_low();
    }

    if (acknowledge) {
        pull_sda_low();
    } else {
        release_sda();
    }
    delayMicroseconds(kI2cHalfPeriodUs);
    if (!raise_scl()) {
        return false;
    }
    delayMicroseconds(kI2cHalfPeriodUs);
    pull_scl_low();
    release_sda();
    return true;
}

bool UltrasonicArray::SoftwareI2c::raise_scl() {
    pinMode(scl_pin_, INPUT);
    const std::uint32_t started_us = micros();
    while (digitalRead(scl_pin_) != HIGH) {
        if (micros() - started_us >= kClockStretchTimeoutUs) {
            return false;
        }
    }
    return true;
}

void UltrasonicArray::SoftwareI2c::release_sda() {
    pinMode(sda_pin_, INPUT);
}

void UltrasonicArray::SoftwareI2c::pull_sda_low() {
    pinMode(sda_pin_, OUTPUT);
}

void UltrasonicArray::SoftwareI2c::pull_scl_low() {
    pinMode(scl_pin_, OUTPUT);
}

UltrasonicArray::UltrasonicArray(
    const RangeSensorConfig *configs,
    std::size_t config_count)
    : configs_(configs), config_count_(config_count) {}

RangeSensorInitResult UltrasonicArray::begin() {
    const RangeSensorInitResult validation = validate_config();
    if (!validation.ok()) {
        return validation;
    }
    const RangeSensorInitResult buses = begin_buses();
    if (!buses.ok()) {
        return buses;
    }
    return {};
}

void UltrasonicArray::update() {
    const std::uint32_t now_ms = millis();
    const RangeSensorConfig &config = configs_[active_sensor_];

    if (measurement_state_ == MeasurementState::Starting) {
        if (start_measurement(config)) {
            measurement_started_ms_ = now_ms;
            measurement_state_ = MeasurementState::Waiting;
        } else {
            store_reading(
                active_sensor_,
                RangeReadingStatus::BusError,
                0.0F,
                now_ms);
            advance_sensor();
        }
        rebuild_direction_readings(now_ms);
        return;
    }

    if (now_ms - measurement_started_ms_ < kTk50MeasurementMs) {
        rebuild_direction_readings(now_ms);
        return;
    }

    float metres = 0.0F;
    if (!finish_measurement(config, metres)) {
        store_reading(
            active_sensor_,
            RangeReadingStatus::BusError,
            0.0F,
            now_ms);
    } else if (
        metres < kMinimumDistanceMetres
        || metres > kMaximumDistanceMetres) {
        store_reading(
            active_sensor_,
            RangeReadingStatus::OutOfRange,
            metres,
            now_ms);
    } else {
        store_reading(
            active_sensor_,
            RangeReadingStatus::Valid,
            metres,
            now_ms);
    }
    rebuild_direction_readings(now_ms);
    advance_sensor();
}

const RangeReading &UltrasonicArray::reading(
    RangeDirection direction) const {
    return direction_readings_[direction_index(direction)];
}

std::array<std::uint16_t, kRangeDirectionCount>
UltrasonicArray::millimetres() const {
    std::array<std::uint16_t, kRangeDirectionCount> result{};
    const std::uint32_t now_ms = millis();
    const std::uint32_t maximum_age_ms = freshness_ms();
    for (std::size_t index = 0; index < result.size(); ++index) {
        const RangeReading &current = direction_readings_[index];
        if (current.status != RangeReadingStatus::Valid
            || now_ms - current.updated_ms > maximum_age_ms) {
            result[index] = std::numeric_limits<std::uint16_t>::max();
            continue;
        }
        result[index] = static_cast<std::uint16_t>(
            std::clamp(
                std::lround(current.metres * 1000.0F),
                0L,
                static_cast<long>(
                    std::numeric_limits<std::uint16_t>::max() - 1)));
    }
    return result;
}

std::uint8_t UltrasonicArray::valid_mask() const {
    std::uint8_t mask = 0;
    const std::uint32_t now_ms = millis();
    const std::uint32_t maximum_age_ms = freshness_ms();
    for (std::size_t index = 0; index < direction_readings_.size(); ++index) {
        const RangeReading &current = direction_readings_[index];
        if (current.status == RangeReadingStatus::Valid
            && now_ms - current.updated_ms <= maximum_age_ms) {
            mask |= static_cast<std::uint8_t>(1U << index);
        }
    }
    return mask;
}

RangeSensorInitResult UltrasonicArray::validate_config() const {
    if (config_count_ == 0 || configs_ == nullptr) {
        return {.error = RangeSensorInitError::NoSensors};
    }
    if (config_count_ > kMaximumRangeSensors) {
        return {.error = RangeSensorInitError::TooManySensors};
    }

    for (std::size_t index = 0; index < config_count_; ++index) {
        const SensorInterface &current = configs_[index].interface;
        if (current.sda_pin > kMaximumGpio
            || current.scl_pin > kMaximumGpio
            || current.sda_pin == current.scl_pin) {
            return {
                .error = RangeSensorInitError::InvalidPins,
                .sensor_index = index,
            };
        }
        if (current.type == SensorInterfaceType::Tca9548a
            && current.multiplexer_index > 7) {
            return {
                .error = RangeSensorInitError::InvalidMultiplexerIndex,
                .sensor_index = index,
            };
        }

        for (std::size_t previous = 0; previous < index; ++previous) {
            const SensorInterface &other = configs_[previous].interface;
            const bool reuses_pin =
                current.sda_pin == other.sda_pin
                || current.sda_pin == other.scl_pin
                || current.scl_pin == other.sda_pin
                || current.scl_pin == other.scl_pin;
            if (!same_bus(current, other) && reuses_pin) {
                return {
                    .error = RangeSensorInitError::PinConflict,
                    .sensor_index = index,
                };
            }
            if (!same_bus(current, other)) {
                continue;
            }
            if (current.type == SensorInterfaceType::Direct
                && other.type == SensorInterfaceType::Direct) {
                return {
                    .error = RangeSensorInitError::DirectAddressConflict,
                    .sensor_index = index,
                };
            }
            if (current.type != other.type) {
                return {
                    .error = RangeSensorInitError::DirectMultiplexerConflict,
                    .sensor_index = index,
                };
            }
            if (current.multiplexer_index == other.multiplexer_index) {
                return {
                    .error = RangeSensorInitError::DuplicateMultiplexerChannel,
                    .sensor_index = index,
                };
            }
        }
    }
    return {};
}

RangeSensorInitResult UltrasonicArray::begin_buses() {
    for (std::size_t index = 0; index < config_count_; ++index) {
        const SensorInterface &current = configs_[index].interface;
        bool already_started = false;
        for (std::size_t previous = 0; previous < index; ++previous) {
            if (same_bus(current, configs_[previous].interface)) {
                already_started = true;
                break;
            }
        }
        if (already_started) {
            continue;
        }

        SoftwareI2c bus = bus_for(current);
        if (!bus.begin()) {
            return {
                .error = RangeSensorInitError::BusUnavailable,
                .sensor_index = index,
            };
        }
        if (current.type == SensorInterfaceType::Tca9548a) {
            const std::uint8_t disable_all = 0;
            if (!bus.write(kTca9548aAddress, &disable_all, 1)) {
                return {
                    .error = RangeSensorInitError::BusUnavailable,
                    .sensor_index = index,
                };
            }
        }
    }
    return {};
}

UltrasonicArray::SoftwareI2c UltrasonicArray::bus_for(
    const SensorInterface &interface) const {
    return SoftwareI2c(interface.sda_pin, interface.scl_pin);
}

bool UltrasonicArray::select_interface(
    const SensorInterface &interface) {
    if (interface.type == SensorInterfaceType::Direct) {
        return true;
    }
    SoftwareI2c bus = bus_for(interface);
    const std::uint8_t selection = static_cast<std::uint8_t>(
        1U << interface.multiplexer_index);
    return bus.write(kTca9548aAddress, &selection, 1);
}

void UltrasonicArray::disable_multiplexer(
    const SensorInterface &interface) {
    if (interface.type != SensorInterfaceType::Tca9548a) {
        return;
    }
    SoftwareI2c bus = bus_for(interface);
    const std::uint8_t disable_all = 0;
    bus.write(kTca9548aAddress, &disable_all, 1);
}

bool UltrasonicArray::start_measurement(
    const RangeSensorConfig &config) {
    switch (config.type) {
        case RangeSensorType::Tk50: {
            if (!select_interface(config.interface)) {
                return false;
            }
            SoftwareI2c bus = bus_for(config.interface);
            const std::uint8_t command = 0x01;
            const bool started = bus.write(kTk50Address, &command, 1);
            if (!started) {
                disable_multiplexer(config.interface);
            }
            return started;
        }
    }
    return false;
}

bool UltrasonicArray::finish_measurement(
    const RangeSensorConfig &config,
    float &metres) {
    switch (config.type) {
        case RangeSensorType::Tk50: {
            if (!select_interface(config.interface)) {
                disable_multiplexer(config.interface);
                return false;
            }
            SoftwareI2c bus = bus_for(config.interface);
            std::array<std::uint8_t, 3> response{};
            const bool read = bus.read(
                kTk50Address,
                response.data(),
                response.size());
            disable_multiplexer(config.interface);
            if (!read) {
                return false;
            }

            const std::uint32_t micrometres =
                (static_cast<std::uint32_t>(response[0]) << 16U)
                | (static_cast<std::uint32_t>(response[1]) << 8U)
                | static_cast<std::uint32_t>(response[2]);
            metres = static_cast<float>(micrometres) / 1'000'000.0F;
            return true;
        }
    }
    return false;
}

void UltrasonicArray::store_reading(
    std::size_t sensor_index,
    RangeReadingStatus status,
    float metres,
    std::uint32_t now_ms) {
    RangeReading &current = sensor_readings_[sensor_index];
    current.updated_ms = now_ms;
    current.status = status;
    current.metres = status == RangeReadingStatus::Valid
        ? filters_[sensor_index].push(metres)
        : metres;
}

void UltrasonicArray::rebuild_direction_readings(std::uint32_t now_ms) {
    for (RangeReading &direction : direction_readings_) {
        direction = {};
    }
    const std::uint32_t maximum_age_ms = freshness_ms();
    for (std::size_t index = 0; index < config_count_; ++index) {
        const RangeReading &sensor = sensor_readings_[index];
        RangeReading &direction = direction_readings_[
            direction_index(configs_[index].direction)];
        if (sensor.status == RangeReadingStatus::Valid
            && now_ms - sensor.updated_ms <= maximum_age_ms
            && (direction.status != RangeReadingStatus::Valid
                || sensor.metres < direction.metres)) {
            direction = sensor;
        } else if (
            direction.status == RangeReadingStatus::AwaitingMeasurement
            && sensor.status != RangeReadingStatus::Valid) {
            direction = sensor;
        }
    }
}

std::uint32_t UltrasonicArray::freshness_ms() const {
    return std::max(
        kMinimumFreshnessMs,
        static_cast<std::uint32_t>(config_count_)
            * kFreshnessPerSensorMs);
}

void UltrasonicArray::advance_sensor() {
    active_sensor_ = (active_sensor_ + 1) % config_count_;
    measurement_state_ = MeasurementState::Starting;
}

}  // namespace tank
