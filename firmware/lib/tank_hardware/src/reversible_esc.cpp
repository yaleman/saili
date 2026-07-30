#include "reversible_esc.h"

#include <algorithm>

namespace tank {
namespace {

constexpr float kEscFrequencyHz = 50.0F;
constexpr float kMinimumPulseUs = 1000.0F;
constexpr float kNeutralPulseUs = 1500.0F;
constexpr float kMaximumPulseUs = 2000.0F;

}  // namespace

bool ReversibleEsc::begin(
    int left_pin,
    int right_pin,
    bool left_reversed,
    bool right_reversed) {
    left_reversed_ = left_reversed;
    right_reversed_ = right_reversed;

    const bool left_ready = left_pwm_.begin(
        left_pin,
        kEscFrequencyHz,
        kMinimumPulseUs,
        kMaximumPulseUs);
    const bool right_ready = right_pwm_.begin(
        right_pin,
        kEscFrequencyHz,
        kMinimumPulseUs,
        kMaximumPulseUs);
    initialized_ = left_ready && right_ready;
    neutral();
    return initialized_;
}

void ReversibleEsc::write(const TrackCommand &command) {
    if (!initialized_) {
        return;
    }
    left_pwm_.writeMicroseconds(to_pulse_us(command.left, left_reversed_));
    right_pwm_.writeMicroseconds(to_pulse_us(command.right, right_reversed_));
}

void ReversibleEsc::neutral() {
    if (!initialized_) {
        return;
    }
    left_pwm_.writeMicroseconds(kNeutralPulseUs);
    right_pwm_.writeMicroseconds(kNeutralPulseUs);
}

float ReversibleEsc::to_pulse_us(float command, bool reversed) {
    command = std::clamp(command, -1.0F, 1.0F);
    if (reversed) {
        command = -command;
    }
    return kNeutralPulseUs + command * 500.0F;
}

}  // namespace tank

