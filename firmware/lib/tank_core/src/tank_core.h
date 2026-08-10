#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

namespace tank {

constexpr std::size_t kRangeDirectionCount = 6;
constexpr std::size_t kRangeFrameSize = 20;
constexpr std::uint8_t kRangeFrameType = 0x7C;
constexpr std::uint8_t kRangeFrameVersion = 2;

enum class DriveState : std::uint8_t {
    Disarmed,
    WaitingForNeutral,
    Armed,
    Failsafe,
};

struct TrackCommand {
    float left = 0.0F;
    float right = 0.0F;
};

struct DriveInput {
    bool receiver_connected = false;
    bool arm_signal = false;
    float drive = 0.0F;
    float turn = 0.0F;
};

struct DriveOutput {
    DriveState state = DriveState::Disarmed;
    TrackCommand tracks;
};

struct DriveConfig {
    float input_deadband = 0.05F;
    float maximum_command = 0.60F;
    float acceleration_per_second = 2.0F;
    float deceleration_per_second = 4.0F;
    std::uint32_t reverse_neutral_ms = 150;
};

TrackCommand mix_arcade(float drive, float turn);

class TrackLimiter {
  public:
    explicit TrackLimiter(const DriveConfig &config);

    float update(float target, std::uint32_t now_ms);
    void reset(std::uint32_t now_ms);

  private:
    const DriveConfig &config_;
    float command_ = 0.0F;
    std::uint32_t last_update_ms_ = 0;
    std::uint32_t neutral_until_ms_ = 0;
    bool initialized_ = false;
};

class DriveController {
  public:
    explicit DriveController(DriveConfig config = {});

    DriveOutput update(const DriveInput &input, std::uint32_t now_ms);
    DriveState state() const;

  private:
    static bool is_neutral(const DriveInput &input, float deadband);
    void stop(DriveState state, std::uint32_t now_ms);

    DriveConfig config_;
    TrackLimiter left_limiter_;
    TrackLimiter right_limiter_;
    DriveState state_ = DriveState::Disarmed;
    bool previous_arm_signal_ = false;
};

class MedianFilter3 {
  public:
    float push(float value);
    bool has_value() const;
    float value() const;

  private:
    std::array<float, 3> samples_{};
    std::size_t count_ = 0;
    std::size_t next_ = 0;
    float filtered_ = 0.0F;
};

std::uint8_t crc8_dvb_s2(const std::uint8_t *data, std::size_t length);

std::array<std::uint8_t, kRangeFrameSize> encode_range_frame(
    const std::array<std::uint16_t, kRangeDirectionCount> &millimetres,
    std::uint8_t valid_mask);

const char *drive_state_name(DriveState state);

}  // namespace tank
