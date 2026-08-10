#include <unity.h>

#include <array>
#include <cstdint>

#include "tank_core.h"

namespace {

void assert_near(float expected, float actual) {
    TEST_ASSERT_FLOAT_WITHIN(0.001F, expected, actual);
}

void test_arcade_mixer_normalizes_without_losing_ratio() {
    const tank::TrackCommand forward = tank::mix_arcade(1.0F, 0.0F);
    assert_near(1.0F, forward.left);
    assert_near(1.0F, forward.right);

    const tank::TrackCommand pivot = tank::mix_arcade(0.0F, 1.0F);
    assert_near(1.0F, pivot.left);
    assert_near(-1.0F, pivot.right);

    const tank::TrackCommand curved = tank::mix_arcade(1.0F, 0.5F);
    assert_near(1.0F, curved.left);
    assert_near(1.0F / 3.0F, curved.right);
}

void test_arming_requires_neutral_on_arm_edge() {
    tank::DriveConfig config;
    config.maximum_command = 1.0F;
    config.acceleration_per_second = 100.0F;
    tank::DriveController controller(config);

    auto output = controller.update(
        {.receiver_connected = true,
         .arm_signal = true,
         .drive = 0.5F,
         .turn = 0.0F},
        10);
    TEST_ASSERT_EQUAL(
        static_cast<int>(tank::DriveState::WaitingForNeutral),
        static_cast<int>(output.state));

    output = controller.update(
        {.receiver_connected = true,
         .arm_signal = true,
         .drive = 0.0F,
         .turn = 0.0F},
        20);
    TEST_ASSERT_EQUAL(
        static_cast<int>(tank::DriveState::WaitingForNeutral),
        static_cast<int>(output.state));

    controller.update(
        {.receiver_connected = true, .arm_signal = false},
        30);
    output = controller.update(
        {.receiver_connected = true, .arm_signal = true},
        40);
    TEST_ASSERT_EQUAL(
        static_cast<int>(tank::DriveState::Armed),
        static_cast<int>(output.state));
}

void test_failsafe_stops_immediately() {
    tank::DriveConfig config;
    config.maximum_command = 1.0F;
    config.acceleration_per_second = 100.0F;
    tank::DriveController controller(config);

    controller.update(
        {.receiver_connected = true, .arm_signal = true},
        0);
    auto output = controller.update(
        {.receiver_connected = true,
         .arm_signal = true,
         .drive = 1.0F},
        20);
    TEST_ASSERT_GREATER_THAN_FLOAT(0.0F, output.tracks.left);

    output = controller.update(
        {.receiver_connected = false,
         .arm_signal = true,
         .drive = 1.0F},
        21);
    TEST_ASSERT_EQUAL(
        static_cast<int>(tank::DriveState::Failsafe),
        static_cast<int>(output.state));
    assert_near(0.0F, output.tracks.left);
    assert_near(0.0F, output.tracks.right);
}

void test_reverse_command_passes_through_neutral_hold() {
    tank::DriveConfig config;
    config.input_deadband = 0.01F;
    config.acceleration_per_second = 100.0F;
    config.deceleration_per_second = 100.0F;
    config.reverse_neutral_ms = 150;
    tank::TrackLimiter limiter(config);

    limiter.reset(0);
    assert_near(1.0F, limiter.update(1.0F, 20));
    assert_near(0.0F, limiter.update(-1.0F, 40));
    assert_near(0.0F, limiter.update(-1.0F, 100));
    assert_near(-1.0F, limiter.update(-1.0F, 200));
}

void test_median_filter_rejects_single_outlier() {
    tank::MedianFilter3 filter;
    assert_near(1.0F, filter.push(1.0F));
    assert_near(1.1F, filter.push(1.1F));
    assert_near(1.1F, filter.push(9.0F));
}

void test_range_frame_contains_six_ranges_and_validity() {
    const auto frame = tank::encode_range_frame(
        std::array<std::uint16_t, 6>{123, 456, 789, 1000, 1200, 1500},
        0b101101);

    TEST_ASSERT_EQUAL_HEX8(0xC8, frame[0]);
    TEST_ASSERT_EQUAL_UINT8(18, frame[1]);
    TEST_ASSERT_EQUAL_HEX8(tank::kRangeFrameType, frame[2]);
    TEST_ASSERT_EQUAL_UINT8(tank::kRangeFrameVersion, frame[5]);
    TEST_ASSERT_EQUAL_HEX8(0x2D, frame[6]);
    TEST_ASSERT_EQUAL_HEX8(0x00, frame[7]);
    TEST_ASSERT_EQUAL_HEX8(0x7B, frame[8]);
    TEST_ASSERT_EQUAL_HEX8(
        tank::crc8_dvb_s2(frame.data() + 2, frame.size() - 3),
        frame.back());
}

}  // namespace

int main(int, char **) {
    UNITY_BEGIN();
    RUN_TEST(test_arcade_mixer_normalizes_without_losing_ratio);
    RUN_TEST(test_arming_requires_neutral_on_arm_edge);
    RUN_TEST(test_failsafe_stops_immediately);
    RUN_TEST(test_reverse_command_passes_through_neutral_hold);
    RUN_TEST(test_median_filter_rejects_single_outlier);
    RUN_TEST(test_range_frame_contains_six_ranges_and_validity);
    return UNITY_END();
}
