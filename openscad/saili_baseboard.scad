// SAILI electronics baseboard
//
// Coordinate convention:
//   X increases from the USB side toward the SD-card side.
//   Y increases from the pin-array side toward the FC3 power-pad side.
//   The FC3 center is (0, 0); Z=0 is the underside of the printed plate.
//
// The GPS and ESP32 dimensions come from the supplied reference images.
// Confirm the actual boards and adjust the parameters below before printing.

// ---------- Print and fit parameters ----------
plate_thickness = 3.0;
corner_radius = 4.0;
edge_margin = 4.0;
hole_clearance = 0.35;       // radial clearance for loose/field FDM fit
tie_slot_width = 3.2;        // accommodates common 2.5-3 mm zip ties
tie_slot_length = 7.0;
tie_slot_offset = 3.5;       // distance from an item edge to slot centre

// ---------- FC3 Rev-B ----------
fc3_size = [50.7, 41.6];
fc3_mount_spacing = 30.5;
fc3_board_h = 4.4;
fc3_hole_nominal = 4.0;
fc3_hole_d = fc3_hole_nominal + 2 * hole_clearance;

// ---------- NEO GPS board ----------
gps_size = [35.0, 25.0];
gps_mount_spacing = [30.0, 20.0]; // assumed from the supplied reference image
gps_hole_nominal = 3.5;
gps_hole_d = gps_hole_nominal + 2 * hole_clearance;

// ---------- ESP32-C3 module ----------
esp32_size = [25.0, 20.0];
esp32_usb_clearance = [9.0, 5.0]; // editable envelope for the side USB connector

// ---------- GPS antenna ----------
antenna_size = [25.0, 25.0];

// Horizontal edge-to-edge clearances between the modules. The GPS-to-antenna
// gap is intentionally large for antenna separation and cable routing.
esp32_gps_gap = 10.0;
gps_antenna_gap = 15.0;

// Modules sit on the pin-array side of the FC3. The USB edges of the FC3 and
// ESP32 both face -X. The GPS antenna is in line with the GPS board.
gps_center = [0.0, -52.0];
esp32_center = [gps_center[0] - gps_size[0] / 2 - esp32_size[0] / 2 - esp32_gps_gap,
                gps_center[1]];
antenna_center = [gps_center[0] + gps_size[0] / 2 + antenna_size[0] / 2 + gps_antenna_gap,
                  gps_center[1]];

// Set true for a translucent layout preview in OpenSCAD; reference geometry
// is never included in the exported plate when this is false.
show_reference_geometry = false;

// ---------- Derived plate bounds ----------
function min2(a, b) = a < b ? a : b;
function max2(a, b) = a > b ? a : b;
function item_min_x(center, size) = center[0] - size[0] / 2;
function item_max_x(center, size) = center[0] + size[0] / 2;
function item_min_y(center, size) = center[1] - size[1] / 2;
function item_max_y(center, size) = center[1] + size[1] / 2;

retention_extent = tie_slot_offset + tie_slot_length / 2;
plate_min_x = min2(
    min2(item_min_x([0, 0], fc3_size), item_min_x(esp32_center, esp32_size)),
    min2(item_min_x(gps_center, gps_size), item_min_x(antenna_center, antenna_size) - retention_extent)
);
plate_max_x = max2(
    max2(item_max_x([0, 0], fc3_size), item_max_x(esp32_center, esp32_size)),
    max2(item_max_x(gps_center, gps_size), item_max_x(antenna_center, antenna_size) + retention_extent)
);
plate_min_y = min2(
    min2(item_min_y([0, 0], fc3_size), item_min_y(esp32_center, esp32_size) - retention_extent),
    min2(item_min_y(gps_center, gps_size), item_min_y(antenna_center, antenna_size) - retention_extent)
);
plate_max_y = max2(
    max2(item_max_y([0, 0], fc3_size), item_max_y(esp32_center, esp32_size) + retention_extent),
    max2(item_max_y(gps_center, gps_size), item_max_y(antenna_center, antenna_size) + retention_extent)
);

plate_size = [plate_max_x - plate_min_x + 2 * edge_margin,
              plate_max_y - plate_min_y + 2 * edge_margin];
plate_center = [(plate_min_x + plate_max_x) / 2,
                (plate_min_y + plate_max_y) / 2];

module rounded_plate(size, radius, height) {
    hull() {
        for (x = [-size[0] / 2 + radius, size[0] / 2 - radius])
            for (y = [-size[1] / 2 + radius, size[1] / 2 - radius])
                translate([x, y]) cylinder(r=radius, h=height, $fn=48);
    }
}

module rounded_slot(length, width, height) {
    hull() {
        translate([-(length - width) / 2, 0]) cylinder(d=width, h=height, $fn=32);
        translate([(length - width) / 2, 0]) cylinder(d=width, h=height, $fn=32);
    }
}

module through_hole(position, diameter) {
    translate([position[0], position[1], -0.5])
        cylinder(d=diameter, h=plate_thickness + 1, $fn=48);
}

module fc3_mount_holes() {
    for (x = [-fc3_mount_spacing / 2, fc3_mount_spacing / 2])
        for (y = [-fc3_mount_spacing / 2, fc3_mount_spacing / 2])
            through_hole([x, y], fc3_hole_d);
}

module gps_mount_holes() {
    for (x = [-gps_mount_spacing[0] / 2, gps_mount_spacing[0] / 2])
        for (y = [-gps_mount_spacing[1] / 2, gps_mount_spacing[1] / 2])
            through_hole([gps_center[0] + x, gps_center[1] + y], gps_hole_d);
}

module horizontal_slot(position) {
    translate([position[0], position[1], -0.5])
        rounded_slot(tie_slot_length, tie_slot_width, plate_thickness + 1);
}

module vertical_slot(position) {
    translate([position[0], position[1], -0.5])
        rotate(90) rounded_slot(tie_slot_length, tie_slot_width, plate_thickness + 1);
}

module esp32_tie_slots() {
    // Slots are on the long sides of the module, clear of the -X USB edge.
    x = esp32_center[0] + 3.5;
    horizontal_slot([x, esp32_center[1] - esp32_size[1] / 2 - tie_slot_offset]);
    horizontal_slot([x, esp32_center[1] + esp32_size[1] / 2 + tie_slot_offset]);
}

module antenna_tie_slots() {
    horizontal_slot([antenna_center[0], antenna_center[1] - antenna_size[1] / 2 - tie_slot_offset]);
    horizontal_slot([antenna_center[0], antenna_center[1] + antenna_size[1] / 2 + tie_slot_offset]);
    vertical_slot([antenna_center[0] - antenna_size[0] / 2 - tie_slot_offset, antenna_center[1]]);
    vertical_slot([antenna_center[0] + antenna_size[0] / 2 + tie_slot_offset, antenna_center[1]]);
}

module baseboard() {
    difference() {
        translate([plate_center[0], plate_center[1], 0])
            rounded_plate(plate_size, corner_radius, plate_thickness);
        fc3_mount_holes();
        gps_mount_holes();
        esp32_tie_slots();
        antenna_tie_slots();
    }
}

module reference_item(center, size, color_name) {
    color(color_name, 0.35)
        translate([center[0], center[1], plate_thickness])
            cube([size[0], size[1], 2.0], center=true);
}

baseboard();

if (show_reference_geometry) {
    reference_item([0, 0], fc3_size, "purple");
    reference_item(esp32_center, esp32_size, "blue");
    reference_item(gps_center, gps_size, "green");
    reference_item(antenna_center, antenna_size, "gold");
}
