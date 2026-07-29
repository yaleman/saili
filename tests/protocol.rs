use saili::{
    DeviceState, MappingError, PacketError, RC_MAX_US, RC_MID_US, RC_MIN_US, RcChannels, RcMapping,
};

#[test]
fn decodes_channels_and_switch_from_report() {
    let report = [10, 1, 20, 30, 40, 50, 60, 70];

    let state = DeviceState::try_from(report.as_slice()).expect("valid report should decode");

    assert_eq!(state.channels(), &[10, 20, 30, 40, 50, 60, 70]);
    assert!(state.digital_switch());
    assert_eq!(state.raw(), &report);
}

#[test]
fn rejects_reports_with_the_wrong_length() {
    let error =
        DeviceState::try_from([1, 2, 3].as_slice()).expect_err("short report should be rejected");

    assert_eq!(
        error,
        PacketError::UnexpectedLength {
            expected: 8,
            actual: 3,
        }
    );
}

#[test]
fn maps_aetr_and_switch_to_safe_rc_ranges() {
    let state = DeviceState::try_from([0, 1, 127, 255, 128, 64, 192, 32].as_slice())
        .expect("valid report should decode");

    let channels = RcMapping::default().map(state);

    assert_eq!(channels.roll(), RC_MIN_US);
    assert_eq!(channels.pitch(), RC_MID_US);
    assert_eq!(channels.throttle(), RC_MAX_US);
    assert_eq!(channels.yaw(), RC_MID_US);
    assert!(channels.armed());
    assert_eq!(channels.values()[5], 1245);
    assert_eq!(channels.values()[6], 1759);
    assert_eq!(channels.values()[7], 1117);
}

#[test]
fn applies_primary_channel_selection_and_inversion() {
    let state = DeviceState::try_from([0, 0, 64, 128, 192, 255, 32, 96].as_slice())
        .expect("valid report should decode");
    let mapping =
        RcMapping::new([4, 3, 2, 1], [true, false, true, false]).expect("mapping should be valid");

    let channels = mapping.map(state);

    assert_eq!(channels.roll(), 1241);
    assert_eq!(channels.pitch(), RC_MID_US);
    assert_eq!(channels.throttle(), 1755);
    assert_eq!(channels.yaw(), RC_MIN_US);
    assert!(!channels.armed());
}

#[test]
fn rejects_duplicate_primary_channel_assignments() {
    let error =
        RcMapping::new([1, 1, 3, 4], [false; 4]).expect_err("duplicate mapping should fail");

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
