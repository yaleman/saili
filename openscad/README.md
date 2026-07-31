# SAILI baseboard

`saili_baseboard.scad` is a parametric flat baseboard for the Madflight FC3
Rev-B, an ESP32-C3 module, a NEO GPS board, and its 25 mm GPS antenna.

## Orientation and layout

The FC3 is centered. In the model coordinate system, `-X` is the USB side,
`+X` is the microSD-card side, `-Y` is the pin-array side, and `+Y` is the
power-pad side. The ESP32, GPS board, and antenna are arranged below the FC3
on the `-Y` side, from left to right.

The ESP32 USB connector faces `-X`, matching the FC3 USB direction. Its two
tie-down slots are outside the long edges and do not occupy the USB edge. The
antenna has one slot on every side for independent X/Y retention. The default
edge clearance is 10 mm between the ESP32 and GPS boards, and 15 mm between
the GPS board and antenna.

## Printing

- Print flat with the underside on the build plate.
- No supports should be needed.
- Use the default loose-fit settings for an ordinary 0.4 mm nozzle.
- Test-fit the actual boards before installing electronics; printed hole size
  depends on printer calibration and material shrinkage.

## Parameters to check before printing

The most important parameters are at the top of the SCAD file:

- `gps_mount_spacing`: assumed to be 30 x 20 mm from the supplied reference.
- `esp32_size` and `esp32_usb_clearance`: adjust to the actual ESP32-C3 board.
- `esp32_gps_gap` and `gps_antenna_gap`: adjust horizontal module spacing.
- `gps_center`: adjust the module row position; the other module positions are
  derived from the configured gaps.
- `hole_clearance`, `tie_slot_width`, and `tie_slot_length`: adjust for the
  printer, hardware, or chosen zip ties.

Set `show_reference_geometry = true` for a translucent layout preview. Keep it
false when exporting the printable baseboard.

Example export:

```sh
openscad -o saili_baseboard.stl saili_baseboard.scad
```
