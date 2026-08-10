#pragma once

// The board header supplies the FC3v2 Rev-B IMU, magnetometer, barometer,
// battery monitor, SD card, RGB LED, UART, and output pin definitions.
#define MF_BOARD "brd/madflight_FC3v2.h"

const char madflight_config[] = R"(

// CRSF receiver/telemetry on SER0: GPIO1 RX, GPIO0 TX.
rcl_gizmo      CRSF
rcl_ser_bus    0
rcl_num_ch     8
rcl_deadband   25
rcl_thr_pull   988
rcl_thr_mid    1500
rcl_thr_push   2012
rcl_rol_left   988
rcl_rol_mid    1500
rcl_rol_right  2012
rcl_pit_pull   988
rcl_pit_mid    1500
rcl_pit_push   2012
rcl_yaw_left   988
rcl_yaw_mid    1500
rcl_yaw_right  2012
rcl_arm_ch     5
rcl_arm_min    1600
rcl_arm_max    2500

// u-blox NEO-6 on SER1: GPIO5 RX, GPIO4 TX. The madflight driver scans
// standard u-blox baud rates and configures binary telemetry automatically.
gps_gizmo      UBLOX
gps_baud       0
gps_ser_bus    1

// A dedicated configurable TK50 driver is used instead of madflight's single
// rangefinder instance.
rdr_gizmo      NONE

)";
