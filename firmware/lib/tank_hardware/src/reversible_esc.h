#pragma once

#include <hal/hal.h>

#include "tank_core.h"

namespace tank {

class ReversibleEsc {
  public:
    bool begin(
        int left_pin,
        int right_pin,
        bool left_reversed = false,
        bool right_reversed = false);

    void write(const TrackCommand &command);
    void neutral();

  private:
    static float to_pulse_us(float command, bool reversed);

    PWM left_pwm_;
    PWM right_pwm_;
    bool initialized_ = false;
    bool left_reversed_ = false;
    bool right_reversed_ = false;
};

}  // namespace tank

