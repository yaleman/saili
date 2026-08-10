#include "tank_core.h"

#include <algorithm>
#include <cmath>

namespace tank {
namespace {

float clamp_unit(float value) {
    return std::clamp(value, -1.0F, 1.0F);
}

float approach(float current, float target, float maximum_change) {
    if (current < target) {
        return std::min(current + maximum_change, target);
    }
    return std::max(current - maximum_change, target);
}

bool deadline_pending(std::uint32_t now, std::uint32_t deadline) {
    return static_cast<std::int32_t>(deadline - now) > 0;
}

}  // namespace

TrackCommand mix_arcade(float drive, float turn) {
    drive = clamp_unit(drive);
    turn = clamp_unit(turn);

    float left = drive + turn;
    float right = drive - turn;
    const float scale = std::max({1.0F, std::fabs(left), std::fabs(right)});

    return {
        .left = left / scale,
        .right = right / scale,
    };
}

TrackLimiter::TrackLimiter(const DriveConfig &config) : config_(config) {}

float TrackLimiter::update(float target, std::uint32_t now_ms) {
    target = clamp_unit(target);
    if (std::fabs(target) < config_.input_deadband) {
        target = 0.0F;
    }

    if (!initialized_) {
        reset(now_ms);
    }

    const std::uint32_t elapsed_ms = now_ms - last_update_ms_;
    last_update_ms_ = now_ms;
    const float elapsed_seconds =
        std::min(static_cast<float>(elapsed_ms) / 1000.0F, 0.1F);

    if (deadline_pending(now_ms, neutral_until_ms_)) {
        command_ = 0.0F;
        return command_;
    }

    if (command_ * target < 0.0F) {
        command_ = approach(
            command_,
            0.0F,
            config_.deceleration_per_second * elapsed_seconds);
        if (std::fabs(command_) <= config_.input_deadband) {
            command_ = 0.0F;
            neutral_until_ms_ = now_ms + config_.reverse_neutral_ms;
        }
        return command_;
    }

    const bool slowing = std::fabs(target) < std::fabs(command_);
    const float rate = slowing ? config_.deceleration_per_second
                               : config_.acceleration_per_second;
    command_ = approach(command_, target, rate * elapsed_seconds);
    return command_;
}

void TrackLimiter::reset(std::uint32_t now_ms) {
    command_ = 0.0F;
    last_update_ms_ = now_ms;
    neutral_until_ms_ = now_ms;
    initialized_ = true;
}

DriveController::DriveController(DriveConfig config)
    : config_(config),
      left_limiter_(config_),
      right_limiter_(config_) {}

DriveOutput DriveController::update(
    const DriveInput &input,
    std::uint32_t now_ms) {
    if (!input.receiver_connected) {
        previous_arm_signal_ = input.arm_signal;
        stop(DriveState::Failsafe, now_ms);
        return {.state = state_, .tracks = {}};
    }

    if (!input.arm_signal) {
        previous_arm_signal_ = false;
        stop(DriveState::Disarmed, now_ms);
        return {.state = state_, .tracks = {}};
    }

    const bool arm_rising_edge = !previous_arm_signal_;
    previous_arm_signal_ = true;

    if (arm_rising_edge) {
        if (is_neutral(input, config_.input_deadband)) {
            state_ = DriveState::Armed;
        } else {
            stop(DriveState::WaitingForNeutral, now_ms);
        }
    }

    if (state_ != DriveState::Armed) {
        return {.state = state_, .tracks = {}};
    }

    float drive = input.drive;
    float turn = input.turn;
    if (std::fabs(drive) < config_.input_deadband) {
        drive = 0.0F;
    }
    if (std::fabs(turn) < config_.input_deadband) {
        turn = 0.0F;
    }

    const TrackCommand mixed = mix_arcade(drive, turn);
    const float maximum = std::clamp(config_.maximum_command, 0.0F, 1.0F);
    const TrackCommand limited_target = {
        .left = mixed.left * maximum,
        .right = mixed.right * maximum,
    };

    return {
        .state = state_,
        .tracks = {
            .left = left_limiter_.update(limited_target.left, now_ms),
            .right = right_limiter_.update(limited_target.right, now_ms),
        },
    };
}

DriveState DriveController::state() const {
    return state_;
}

bool DriveController::is_neutral(
    const DriveInput &input,
    float deadband) {
    return std::fabs(input.drive) <= deadband
        && std::fabs(input.turn) <= deadband;
}

void DriveController::stop(DriveState state, std::uint32_t now_ms) {
    state_ = state;
    left_limiter_.reset(now_ms);
    right_limiter_.reset(now_ms);
}

float MedianFilter3::push(float value) {
    samples_[next_] = value;
    next_ = (next_ + 1) % samples_.size();
    count_ = std::min(count_ + 1, samples_.size());

    std::array<float, 3> ordered = samples_;
    std::sort(ordered.begin(), ordered.begin() + count_);
    filtered_ = ordered[count_ / 2];
    return filtered_;
}

bool MedianFilter3::has_value() const {
    return count_ != 0;
}

float MedianFilter3::value() const {
    return filtered_;
}

std::uint8_t crc8_dvb_s2(
    const std::uint8_t *data,
    std::size_t length) {
    std::uint8_t crc = 0;
    for (std::size_t index = 0; index < length; ++index) {
        crc ^= data[index];
        for (std::uint8_t bit = 0; bit < 8; ++bit) {
            crc = (crc & 0x80U) != 0U
                ? static_cast<std::uint8_t>((crc << 1U) ^ 0xD5U)
                : static_cast<std::uint8_t>(crc << 1U);
        }
    }
    return crc;
}

std::array<std::uint8_t, kRangeFrameSize> encode_range_frame(
    const std::array<std::uint16_t, kRangeDirectionCount> &millimetres,
    std::uint8_t valid_mask) {
    std::array<std::uint8_t, kRangeFrameSize> frame{};
    frame[0] = 0xC8;
    frame[1] = 18;
    frame[2] = kRangeFrameType;
    frame[3] = 0x12;
    frame[4] = 0xC8;
    frame[5] = kRangeFrameVersion;
    frame[6] = valid_mask & 0x3FU;

    for (std::size_t index = 0; index < millimetres.size(); ++index) {
        const std::size_t offset = 7 + index * 2;
        frame[offset] =
            static_cast<std::uint8_t>(millimetres[index] >> 8U);
        frame[offset + 1] =
            static_cast<std::uint8_t>(millimetres[index] & 0xFFU);
    }

    frame.back() = crc8_dvb_s2(frame.data() + 2, frame.size() - 3);
    return frame;
}

const char *drive_state_name(DriveState state) {
    switch (state) {
        case DriveState::Disarmed:
            return "DISARMED";
        case DriveState::WaitingForNeutral:
            return "NEUTRAL REQUIRED";
        case DriveState::Armed:
            return "MANUAL";
        case DriveState::Failsafe:
            return "FAILSAFE";
    }
    return "UNKNOWN";
}

}  // namespace tank
