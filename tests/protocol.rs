use std::time::{Duration, Instant};

use saili::{
    ArmConfig, ArmController, ChannelCalibration, DecodedState, Decoder, MappingError, PacketError,
    RC_MAX_US, RC_MID_US, RC_MIN_US, RawReport, RcChannels, RcMapping, ReportFormat,
};

fn raw(report: [u8; 8], sequence: u64) -> RawReport {
    raw_at(report, sequence, Instant::now(), None)
}

fn raw_at(
    report: [u8; 8],
    sequence: u64,
    received_at: Instant,
    previous: Option<&RawReport>,
) -> RawReport {
    RawReport::try_new(&report, sequence, received_at, previous)
        .expect("test report should have the expected length")
}

fn calibrate(decoder: &mut Decoder, input_seven_phase: usize) -> RawReport {
    decoder
        .start_mux_calibration()
        .expect("raw mux calibration should start");
    let started = Instant::now();
    let mut previous = None;
    for index in 0..16 {
        let phase = index % 2;
        let value = if phase == input_seven_phase {
            if (index / 2) % 2 == 0 { 20 } else { 220 }
        } else {
            100
        };
        let current = raw_at(
            [10, 0, 20, 30, 40, 50, 60, value],
            1_000 + index as u64 * 17,
            started + Duration::from_millis(index as u64 * 5),
            previous.as_ref(),
        );
        assert!(
            decoder
                .decode(current)
                .expect("valid calibration report")
                .is_none()
        );
        previous = Some(current);
    }
    decoder
        .confirm_mux_calibration()
        .expect("first mux control should calibrate");

    for index in 16..32 {
        let phase = index % 2;
        let value = if phase == (input_seven_phase ^ 1) {
            if (index / 2) % 2 == 0 { 30 } else { 210 }
        } else {
            110
        };
        let current = raw_at(
            [10, 0, 20, 30, 40, 50, 60, value],
            1_000 + index as u64 * 17,
            started + Duration::from_millis(index as u64 * 5),
            previous.as_ref(),
        );
        assert!(
            decoder
                .decode(current)
                .expect("valid calibration report")
                .is_none()
        );
        previous = Some(current);
    }
    decoder
        .confirm_mux_calibration()
        .expect("second mux control should calibrate");
    assert_eq!(
        decoder.status().mux_state,
        Some(saili::MuxState::Calibrated)
    );
    previous.expect("calibration should produce a final report")
}

fn decoded(report: [u8; 8], format: ReportFormat) -> DecodedState {
    Decoder::new(format, false)
        .decode(raw(report, 1))
        .expect("decoder should not fail")
        .expect("non-muxed format should decode immediately")
}

#[test]
fn raw_muxed_reports_wait_for_both_phases() {
    let mut decoder = Decoder::new(ReportFormat::RawMuxed8, false);
    let previous = calibrate(&mut decoder, 1);
    let first = raw_at(
        [11, 98, 21, 31, 41, 51, 61, 70],
        42,
        previous.received_at() + Duration::from_millis(5),
        Some(&previous),
    );
    assert!(decoder.decode(first).expect("valid report").is_none());

    let second = raw_at(
        [11, 98, 21, 31, 41, 51, 61, 80],
        9_999,
        first.received_at() + Duration::from_millis(5),
        Some(&first),
    );
    let state = decoder
        .decode(second)
        .expect("valid report")
        .expect("both mux phases should now be available");
    assert_eq!(
        state.channels(),
        &[
            Some(11),
            Some(21),
            Some(31),
            Some(41),
            Some(51),
            Some(61),
            Some(80),
            Some(70)
        ]
    );
    assert_eq!(state.format(), ReportFormat::RawMuxed8);
}

#[test]
fn raw_muxed_channel_swap_is_deterministic() {
    let mut decoder = Decoder::new(ReportFormat::RawMuxed8, true);
    let previous = calibrate(&mut decoder, 1);
    let first = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 10],
        400,
        previous.received_at() + Duration::from_millis(5),
        Some(&previous),
    );
    let _ = decoder.decode(first);
    let state = decoder
        .decode(raw_at(
            [0, 0, 0, 0, 0, 0, 0, 20],
            401,
            first.received_at() + Duration::from_millis(5),
            Some(&first),
        ))
        .expect("valid report")
        .expect("both phases should be available");
    assert_eq!(state.channel(6), Some(10));
    assert_eq!(state.channel(7), Some(20));
}

#[test]
fn linux_demuxed_layout_uses_byte_one_as_an_axis() {
    let state = decoded(
        [10, 200, 20, 30, 40, 50, 60, 70],
        ReportFormat::LinuxDemuxed8,
    );
    assert_eq!(
        state.channels(),
        &[
            Some(10),
            Some(20),
            Some(30),
            Some(40),
            Some(50),
            Some(60),
            Some(200),
            Some(70)
        ]
    );
    assert_eq!(state.legacy_button(), None);
}

#[test]
fn legacy_button_is_only_available_in_explicit_legacy_format() {
    let legacy = decoded([10, 1, 20, 30, 40, 50, 60, 70], ReportFormat::Legacy7Button);
    assert_eq!(legacy.channels()[7], None);
    assert_eq!(legacy.legacy_button(), Some(true));

    let clone = decoded([10, 1, 20, 30, 40, 50, 60, 70], ReportFormat::LinuxDemuxed8);
    assert_eq!(clone.legacy_button(), None);
}

#[test]
fn reconnect_reset_requires_both_mux_phases_again() {
    let mut decoder = Decoder::new(ReportFormat::RawMuxed8, false);
    let previous = calibrate(&mut decoder, 0);
    let _ = decoder.decode(raw_at(
        [0, 0, 0, 0, 0, 0, 0, 10],
        1,
        previous.received_at() + Duration::from_millis(5),
        Some(&previous),
    ));
    decoder.reset();
    assert!(
        decoder
            .decode(raw([0, 0, 0, 0, 0, 0, 0, 20], 2))
            .expect("valid report")
            .is_none()
    );
}

#[test]
fn raw_mux_calibration_handles_either_initial_phase_and_skipped_sequences() {
    for input_seven_phase in [0, 1] {
        let mut decoder = Decoder::new(ReportFormat::RawMuxed8, false);
        let previous = calibrate(&mut decoder, input_seven_phase);
        let first = raw_at(
            [0, 0, 0, 0, 0, 0, 0, 40],
            77,
            previous.received_at() + Duration::from_millis(5),
            Some(&previous),
        );
        let second = raw_at(
            [0, 0, 0, 0, 0, 0, 0, 200],
            4_000,
            first.received_at() + Duration::from_millis(5),
            Some(&first),
        );
        let _ = decoder.decode(first).expect("valid report");
        let state = decoder
            .decode(second)
            .expect("valid report")
            .expect("calibrated mux should publish complete state");
        let expected = if input_seven_phase == 0 {
            [Some(40), Some(200)]
        } else {
            [Some(200), Some(40)]
        };
        assert_eq!([state.channel(6), state.channel(7)], expected);
    }
}

#[test]
fn raw_mux_cadence_jitter_is_allowed_but_gap_loses_phase() {
    let mut decoder = Decoder::new(ReportFormat::RawMuxed8, false);
    let previous = calibrate(&mut decoder, 0);
    let first = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 40],
        1,
        previous.received_at() + Duration::from_millis(6),
        Some(&previous),
    );
    assert!(decoder.decode(first).expect("valid report").is_none());
    let second = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 200],
        2,
        first.received_at() + Duration::from_millis(7),
        Some(&first),
    );
    assert!(decoder.decode(second).expect("valid report").is_some());

    let gap = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 50],
        3,
        second.received_at() + Duration::from_millis(20),
        Some(&second),
    );
    assert!(decoder.decode(gap).expect("valid report").is_none());
    assert_eq!(decoder.status().mux_state, Some(saili::MuxState::Lost));
    assert_eq!(
        decoder.status().loss_reason,
        Some(saili::MuxLossReason::CadenceGap)
    );
}

#[test]
fn raw_mux_can_recalibrate_after_loss() {
    let mut decoder = Decoder::new(ReportFormat::RawMuxed8, false);
    let previous = calibrate(&mut decoder, 0);
    let first = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 40],
        1,
        previous.received_at() + Duration::from_millis(5),
        Some(&previous),
    );
    let _ = decoder.decode(first);
    let gap = raw_at(
        [0, 0, 0, 0, 0, 0, 0, 50],
        2,
        first.received_at() + Duration::from_millis(20),
        Some(&first),
    );
    let _ = decoder.decode(gap);
    let _ = calibrate(&mut decoder, 1);
    assert_eq!(
        decoder.status().mux_state,
        Some(saili::MuxState::Calibrated)
    );
}

#[test]
fn rejects_reports_with_the_wrong_length() {
    let error = RawReport::try_new(&[1, 2, 3], 1, Instant::now(), None)
        .expect_err("short report should be rejected");
    assert_eq!(
        error,
        PacketError::UnexpectedLength {
            expected: 8,
            actual: 3
        }
    );
}

#[test]
fn default_mapping_exposes_eight_inputs_and_forces_ch5_low() {
    let state = decoded(
        [0, 1, 127, 255, 128, 64, 192, 32],
        ReportFormat::LinuxDemuxed8,
    );
    let channels = RcMapping::default().map(state);
    assert_eq!(channels.roll(), RC_MIN_US);
    assert_eq!(channels.pitch(), RC_MID_US);
    assert_eq!(channels.throttle(), RC_MAX_US);
    assert_eq!(channels.yaw(), RC_MID_US);
    assert!(!channels.armed());
    assert_ne!(channels.values()[5], RC_MIN_US);
    assert_ne!(channels.values()[8], RC_MIN_US);
}

#[test]
fn configured_arm_source_has_hysteresis_and_inversion() {
    let mut controller = ArmController::new(ArmConfig {
        channel: Some(0),
        threshold: 127,
        hysteresis: 4,
        inverted: false,
    });
    assert!(!controller.update(decoded(
        [130, 0, 0, 0, 0, 0, 0, 0],
        ReportFormat::LinuxDemuxed8
    )));
    assert!(controller.update(decoded(
        [132, 0, 0, 0, 0, 0, 0, 0],
        ReportFormat::LinuxDemuxed8
    )));
    assert!(controller.update(decoded(
        [125, 0, 0, 0, 0, 0, 0, 0],
        ReportFormat::LinuxDemuxed8
    )));
    assert!(!controller.update(decoded(
        [123, 0, 0, 0, 0, 0, 0, 0],
        ReportFormat::LinuxDemuxed8
    )));
}

#[test]
fn byte_one_never_arms_without_an_explicit_arm_channel() {
    let mut controller = ArmController::new(ArmConfig::default());
    assert!(!controller.update(decoded(
        [0, 255, 0, 0, 0, 0, 0, 0],
        ReportFormat::LinuxDemuxed8
    )));
    assert!(!controller.update(decoded(
        [0, 1, 0, 0, 0, 0, 0, 0],
        ReportFormat::Legacy7Button
    )));
    assert!(!controller.is_armed());
}

#[test]
fn mapping_calibration_and_full_mapping_are_validated() {
    let mapping = RcMapping::new_full([8, 7, 6, 5, 4, 3, 2, 1], [false; 8])
        .expect("all eight inputs should be accepted")
        .with_calibration(
            [ChannelCalibration {
                minimum: 10,
                centre: 100,
                maximum: 240,
                deadband: 0,
            }; 8],
        );
    let channels = mapping.map(decoded(
        [100, 0, 0, 0, 0, 0, 0, 10],
        ReportFormat::LinuxDemuxed8,
    ));
    assert_eq!(channels.roll(), RC_MIN_US);

    let error = RcMapping::new_full([1, 1, 3, 4, 5, 6, 7, 8], [false; 8])
        .expect_err("duplicate inputs should be rejected");
    assert_eq!(error, MappingError::DuplicateChannel { channel: 1 });
}

#[test]
fn safe_channels_are_centered_disarmed_and_throttle_low() {
    let channels = RcChannels::safe();
    assert_eq!(channels.roll(), RC_MID_US);
    assert_eq!(channels.pitch(), RC_MID_US);
    assert_eq!(channels.throttle(), RC_MIN_US);
    assert_eq!(channels.yaw(), RC_MID_US);
    assert!(!channels.armed());
}
