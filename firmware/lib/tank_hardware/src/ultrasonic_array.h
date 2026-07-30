#pragma once

#include <Arduino.h>

#include <array>
#include <cstddef>
#include <cstdint>

#include "tank_core.h"

namespace tank {

enum class RangeDirection : std::uint8_t {
    Front = 0,
    Back = 1,
    Left = 2,
    Right = 3,
};

struct UltrasonicPins {
    int trigger;
    int echo;
};

struct RangeReading {
    float metres = 0.0F;
    std::uint32_t updated_ms = 0;
    bool valid = false;
};

class UltrasonicArray {
  public:
    explicit UltrasonicArray(
        std::array<UltrasonicPins, kRangeSensorCount> pins);

    bool begin();
    void update();

    const std::array<RangeReading, kRangeSensorCount> &readings() const;
    std::array<std::uint16_t, kRangeSensorCount> millimetres() const;
    std::uint8_t valid_mask() const;

    void handle_echo_interrupt(std::size_t sensor_index);

  private:
    static void front_echo_interrupt();
    static void back_echo_interrupt();
    static void left_echo_interrupt();
    static void right_echo_interrupt();

    static bool deadline_reached(std::uint32_t now, std::uint32_t deadline);
    void start_measurement(std::uint32_t now_us);
    void finish_measurement(std::uint32_t now_us);

    static UltrasonicArray *instance_;

    std::array<UltrasonicPins, kRangeSensorCount> pins_;
    std::array<RangeReading, kRangeSensorCount> readings_{};
    std::array<MedianFilter3, kRangeSensorCount> filters_{};
    volatile std::size_t active_sensor_ = kRangeSensorCount - 1;
    volatile std::uint32_t echo_rise_us_ = 0;
    volatile std::uint32_t echo_fall_us_ = 0;
    std::uint32_t measurement_started_us_ = 0;
    std::uint32_t next_measurement_us_ = 0;
    volatile bool waiting_for_echo_ = false;
};

}  // namespace tank
