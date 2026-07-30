#pragma once

#include <hal/MF_Serial.h>

#include "ultrasonic_array.h"

namespace tank {

class RangeTelemetry {
  public:
    bool send(MF_Serial &serial, const UltrasonicArray &sensors);
};

}  // namespace tank

