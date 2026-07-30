#include "range_telemetry.h"

#include "tank_core.h"

namespace tank {

bool RangeTelemetry::send(
    MF_Serial &serial,
    const UltrasonicArray &sensors) {
    auto frame =
        encode_range_frame(sensors.millimetres(), sensors.valid_mask());
    if (serial.availableForWrite() < static_cast<int>(frame.size())) {
        return false;
    }
    return serial.write(frame.data(), static_cast<int>(frame.size()))
        == static_cast<int>(frame.size());
}

}  // namespace tank
