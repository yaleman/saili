#!/usr/bin/env -S uv run

import sys

import hid  # ty: ignore[unresolved-import]

VID = 0x1781
PID = 0x0898
EXPECTED_PACKET_SIZE = 8
ANALOGUE_CHANGE_THRESHOLD = 2
ANALOGUE_BYTE_INDICES = (0, 2, 3, 4, 5, 6, 7)


class AdapterNotFoundError(Exception):
    pass


def find_device_path() -> bytes:
    devices = hid.enumerate(VID, PID)
    if not devices:
        raise AdapterNotFoundError

    path = devices[0].get("path")
    if not isinstance(path, bytes):
        raise hid.HIDException("adapter has no usable HID path")

    return path


def print_packet(packet: bytes) -> None:
    channels = (
        packet[0],
        packet[2],
        packet[3],
        packet[4],
        packet[5],
        packet[6],
        packet[7],
    )
    switch = packet[1] != 0

    print(
        "channels="
        + " ".join(f"{value:3d}" for value in channels)
        + f"  switch={switch}"
        + f"  raw={packet.hex(' ')}"
    )


def has_meaningful_change(packet: bytes, previous: bytes) -> bool:
    if packet[1] != previous[1]:
        return True

    return any(
        abs(packet[index] - previous[index]) >= ANALOGUE_CHANGE_THRESHOLD
        for index in ANALOGUE_BYTE_INDICES
    )


def read_packets(device_path: bytes) -> None:
    device = hid.device()

    try:
        device.open_path(device_path)
        print(
            f"Found adapter: {device.get_manufacturer_string()} "
            f"{device.get_product_string()}"
        )

        last_packet: bytes | None = None

        while True:
            data = device.read(EXPECTED_PACKET_SIZE, 1000)
            if not data:
                continue

            packet = bytes(data)
            if len(packet) != EXPECTED_PACKET_SIZE:
                print(f"Unexpected {len(packet)}-byte packet: {packet.hex(' ')}")
                continue

            if last_packet is not None and not has_meaningful_change(
                packet, last_packet
            ):
                continue

            last_packet = packet
            print_packet(packet)
    finally:
        device.close()


def main() -> int:
    try:
        read_packets(find_device_path())
    except KeyboardInterrupt:
        print("\nStopped")
    except AdapterNotFoundError:
        print("SAILI/PhoenixRC adapter not found", file=sys.stderr)
        return 1
    except hid.HIDException as error:
        print(f"Could not read SAILI adapter: {error}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
