use saili::{
    CRSF_FRAME_TYPE_ATTITUDE, CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE, CRSF_FRAME_TYPE_BATTERY,
    CRSF_FRAME_TYPE_FLIGHT_MODE, CRSF_FRAME_TYPE_GPS, CRSF_FRAME_TYPE_RANGE, CrsfError, CrsfFrame,
    CrsfTelemetry, crc8_dvb_s2,
};

fn make_frame(frame_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0xC8, (payload.len() + 2) as u8, frame_type];
    frame.extend_from_slice(payload);
    let crc = crc8_dvb_s2(&frame[2..]);
    frame.push(crc);
    frame
}

#[test]
fn decodes_battery_telemetry() {
    let raw = make_frame(
        CRSF_FRAME_TYPE_BATTERY,
        &[0x00, 0xFC, 0x00, 0x22, 0x00, 0x12, 0x34, 75],
    );
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("battery frame should decode");

    assert_eq!(frame.address(), 0xC8);
    assert_eq!(frame.frame_type(), CRSF_FRAME_TYPE_BATTERY);
    assert_eq!(
        frame.telemetry().expect("battery payload should decode"),
        CrsfTelemetry::Battery {
            voltage_v: 25.2,
            current_a: 3.4,
            capacity_mah: 0x1234,
            remaining_percent: 75,
        }
    );
}

#[test]
fn decodes_attitude_and_flight_mode_telemetry() {
    let attitude_raw = make_frame(
        CRSF_FRAME_TYPE_ATTITUDE,
        &[0x27, 0x10, 0xEC, 0x78, 0x00, 0x00],
    );
    let attitude =
        CrsfFrame::try_from(attitude_raw.as_slice()).expect("attitude frame should decode");

    assert_eq!(
        attitude
            .telemetry()
            .expect("attitude payload should decode"),
        CrsfTelemetry::Attitude {
            pitch_radians: 1.0,
            roll_radians: -0.5,
            yaw_radians: 0.0,
        }
    );

    let mode_raw = make_frame(CRSF_FRAME_TYPE_FLIGHT_MODE, b"ACRO*\0");
    let mode = CrsfFrame::try_from(mode_raw.as_slice()).expect("flight mode should decode");
    assert_eq!(
        mode.telemetry().expect("flight mode payload should decode"),
        CrsfTelemetry::FlightMode("ACRO*".to_owned())
    );
}

#[test]
fn decodes_gps_telemetry() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(-274_700_000_i32).to_be_bytes());
    payload.extend_from_slice(&(1_531_000_000_i32).to_be_bytes());
    payload.extend_from_slice(&(425_u16).to_be_bytes());
    payload.extend_from_slice(&(12_345_u16).to_be_bytes());
    payload.extend_from_slice(&(1_123_u16).to_be_bytes());
    payload.push(14);
    let raw = make_frame(CRSF_FRAME_TYPE_GPS, &payload);
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("GPS frame should decode");

    let CrsfTelemetry::Gps {
        latitude_degrees,
        longitude_degrees,
        ground_speed_kmh,
        heading_degrees,
        altitude_metres,
        satellites,
    } = frame.telemetry().expect("GPS payload should decode")
    else {
        panic!("expected GPS telemetry");
    };
    assert!((latitude_degrees - -27.47).abs() < 0.000_001);
    assert!((longitude_degrees - 153.1).abs() < 0.000_001);
    assert_eq!(ground_speed_kmh, 42.5);
    assert_eq!(heading_degrees, 123.45);
    assert_eq!(altitude_metres, 123);
    assert_eq!(satellites, 14);
}

#[test]
fn parses_hex_and_rejects_bad_crc() {
    let raw = make_frame(CRSF_FRAME_TYPE_FLIGHT_MODE, b"ACRO\0");
    let encoded = raw
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let frame = CrsfFrame::from_hex(&encoded).expect("hex frame should decode");
    assert_eq!(frame.raw(), raw);

    let mut damaged = raw;
    let last = damaged.len() - 1;
    damaged[last] ^= 0xFF;
    assert!(matches!(
        CrsfFrame::try_from(damaged.as_slice()),
        Err(CrsfError::CrcMismatch { .. })
    ));
}

#[test]
fn decodes_packed_barometric_altitude() {
    let raw = make_frame(CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE, &[0x27, 0xD8, 27]);
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("barometric altitude should decode");

    let CrsfTelemetry::BarometricAltitude {
        altitude_metres,
        vertical_speed_ms,
    } = frame
        .telemetry()
        .expect("barometric altitude payload should decode")
    else {
        panic!("expected barometric altitude telemetry");
    };
    assert_eq!(altitude_metres, 20.0);
    assert!((vertical_speed_ms - 1.017).abs() < 0.001);
}

#[test]
fn decodes_four_byte_barometric_altitude() {
    let raw = make_frame(
        CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE,
        &[0x27, 0xD8, 0x00, 0x96],
    );
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("barometric altitude should decode");

    assert_eq!(
        frame.telemetry(),
        Ok(CrsfTelemetry::BarometricAltitude {
            altitude_metres: 20.0,
            vertical_speed_ms: 1.5,
        })
    );
}

#[test]
fn decodes_directional_range_telemetry() {
    let payload = [
        0x12, 0xC8, 1, 0b1101, 0x01, 0xF4, 0xFF, 0xFF, 0x04, 0xD2, 0x07, 0xD0,
    ];
    let raw = make_frame(CRSF_FRAME_TYPE_RANGE, &payload);
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("range frame should decode");

    assert_eq!(
        frame.telemetry().expect("range payload should decode"),
        CrsfTelemetry::Range {
            front_metres: Some(0.5),
            back_metres: None,
            left_metres: Some(1.234),
            right_metres: Some(2.0),
        }
    );
}

#[test]
fn rejects_unknown_range_telemetry_version() {
    let payload = [
        0x12, 0xC8, 2, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let raw = make_frame(CRSF_FRAME_TYPE_RANGE, &payload);
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("range frame should decode");

    assert_eq!(
        frame.telemetry(),
        Err(CrsfError::UnsupportedRangeVersion { version: 2 })
    );
}

#[test]
fn ignores_standard_msp_write_frames_using_the_private_range_type() {
    let raw = make_frame(0x7C, &[0x01, 0x02, 0x03, 0x04]);
    let frame = CrsfFrame::try_from(raw.as_slice()).expect("MSP write frame should decode");

    assert_eq!(
        frame.telemetry(),
        Ok(CrsfTelemetry::Unknown { frame_type: 0x7C })
    );
}
