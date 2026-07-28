use saili::{DeviceState, PacketError};

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
