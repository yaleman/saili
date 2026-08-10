#pragma once

#include <Arduino.h>

#include <array>
#include <cstddef>
#include <cstdint>

#include "tank_core.h"

namespace tank {

constexpr std::size_t kMaximumRangeSensors = 16;

enum class RangeDirection : std::uint8_t {
    Front = 0,
    Rear = 1,
    Left = 2,
    Right = 3,
    Up = 4,
    Down = 5,
};

enum class RangeSensorType : std::uint8_t {
    Tk50,
};

enum class SensorInterfaceType : std::uint8_t {
    Direct,
    Tca9548a,
};

struct SensorInterface {
    SensorInterfaceType type;
    std::uint8_t sda_pin;
    std::uint8_t scl_pin;
    std::uint8_t multiplexer_index;

    static constexpr SensorInterface direct(
        std::uint8_t sda_pin,
        std::uint8_t scl_pin) {
        return {
            .type = SensorInterfaceType::Direct,
            .sda_pin = sda_pin,
            .scl_pin = scl_pin,
            .multiplexer_index = 0,
        };
    }

    static constexpr SensorInterface tca9548a(
        std::uint8_t sda_pin,
        std::uint8_t scl_pin,
        std::uint8_t multiplexer_index) {
        return {
            .type = SensorInterfaceType::Tca9548a,
            .sda_pin = sda_pin,
            .scl_pin = scl_pin,
            .multiplexer_index = multiplexer_index,
        };
    }
};

struct RangeSensorConfig {
    RangeDirection direction;
    RangeSensorType type;
    SensorInterface interface;
};

enum class RangeReadingStatus : std::uint8_t {
    AwaitingMeasurement,
    Valid,
    BusError,
    OutOfRange,
};

struct RangeReading {
    float metres = 0.0F;
    std::uint32_t updated_ms = 0;
    RangeReadingStatus status = RangeReadingStatus::AwaitingMeasurement;
};

enum class RangeSensorInitError : std::uint8_t {
    None,
    NoSensors,
    TooManySensors,
    InvalidPins,
    PinConflict,
    InvalidMultiplexerIndex,
    DirectAddressConflict,
    DirectMultiplexerConflict,
    DuplicateMultiplexerChannel,
    BusUnavailable,
};

struct RangeSensorInitResult {
    RangeSensorInitError error = RangeSensorInitError::None;
    std::size_t sensor_index = 0;

    bool ok() const {
        return error == RangeSensorInitError::None;
    }
};

const char *range_sensor_init_error_name(RangeSensorInitError error);

class UltrasonicArray {
  public:
    UltrasonicArray(
        const RangeSensorConfig *configs,
        std::size_t config_count);

    template <std::size_t N>
    explicit UltrasonicArray(
        const std::array<RangeSensorConfig, N> &configs)
        : UltrasonicArray(configs.data(), configs.size()) {}

    RangeSensorInitResult begin();
    void update();

    const RangeReading &reading(RangeDirection direction) const;
    std::array<std::uint16_t, kRangeDirectionCount> millimetres() const;
    std::uint8_t valid_mask() const;

  private:
    enum class MeasurementState : std::uint8_t {
        Starting,
        Waiting,
    };

    class SoftwareI2c {
      public:
        SoftwareI2c(std::uint8_t sda_pin, std::uint8_t scl_pin);

        bool begin();
        bool write(std::uint8_t address, const std::uint8_t *data, std::size_t size);
        bool read(std::uint8_t address, std::uint8_t *data, std::size_t size);

      private:
        bool start();
        void stop();
        bool write_byte(std::uint8_t value);
        bool read_byte(std::uint8_t &value, bool acknowledge);
        bool raise_scl();
        void release_sda();
        void pull_sda_low();
        void pull_scl_low();

        std::uint8_t sda_pin_;
        std::uint8_t scl_pin_;
    };

    static constexpr std::uint8_t kTk50Address = 0x57;
    static constexpr std::uint8_t kTca9548aAddress = 0x70;
    static constexpr std::uint32_t kTk50MeasurementMs = 150;

    RangeSensorInitResult validate_config() const;
    RangeSensorInitResult begin_buses();
    SoftwareI2c bus_for(const SensorInterface &interface) const;
    bool select_interface(const SensorInterface &interface);
    void disable_multiplexer(const SensorInterface &interface);
    bool start_measurement(const RangeSensorConfig &config);
    bool finish_measurement(
        const RangeSensorConfig &config,
        float &metres);
    void store_reading(
        std::size_t sensor_index,
        RangeReadingStatus status,
        float metres,
        std::uint32_t now_ms);
    void rebuild_direction_readings(std::uint32_t now_ms);
    std::uint32_t freshness_ms() const;
    void advance_sensor();

    const RangeSensorConfig *configs_;
    std::size_t config_count_;
    std::array<RangeReading, kMaximumRangeSensors> sensor_readings_{};
    std::array<MedianFilter3, kMaximumRangeSensors> filters_{};
    std::array<RangeReading, kRangeDirectionCount> direction_readings_{};
    std::size_t active_sensor_ = 0;
    std::uint32_t measurement_started_ms_ = 0;
    MeasurementState measurement_state_ = MeasurementState::Starting;
};

}  // namespace tank
