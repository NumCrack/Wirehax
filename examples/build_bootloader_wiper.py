#!/usr/bin/env python3
"""
Builds a bootloader wiper binary that:
1. Boots as bootloader at 0x08000000.
2. Uses the stock bootloader's hardware initialization (clocks, PLL, MPU, OCTOSPI).
3. Turns ON the Red LED (PB4).
4. Invokes the stock bootloader's hardware Chip Erase routine (0x08007274)
   which erases all 8 MB of external flash (Slot A + Slot B) via SPI command 0xC7.
5. Turns OFF Red LED and turns ON Green LED (PB5).
6. Halts safely in low-power sleep (WFI).

There is no reason at all to use this unless you are doing a full factory reset of the device and want to wipe all firmware slots.
It will hard brick your device and make it recoverable only with an SWD debugger.
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_BIN = ROOT / "firmware" / "bootloader.bin"
OUT_BIN = ROOT / "firmware" / "examples" / "wipe_slots" / "bootloader.bin"

# Exact 74-byte machine code aligned for offset 0x8DD2
# (0x00, 0xBF is NOP to align code start to 4-byte boundary 0x08008DD4)
WIPER_CODE = bytes(
    [
        0x00,
        0xBF,  # 0x08008DD2: nop (aligns to 0x08008DD4)
        # 0x08008DD4:
        0x0C,
        0x48,  # ldr r0, [pc, #0x30] -> 0x58024438 (RCC_AHB4ENR)
        0x01,
        0x68,  # ldr r1, [r0]
        0x41,
        0xF0,
        0x02,
        0x01,  # orr r1, r1, #2 (enable GPIOBEN)
        0x01,
        0x60,  # str r1, [r0]
        0x0B,
        0x48,  # ldr r0, [pc, #0x2c] -> 0x58020400 (GPIOB_MODER)
        0x01,
        0x68,  # ldr r1, [r0]
        0x21,
        0xF4,
        0x40,
        0x71,  # bic r1, r1, #0x300 (PB4)
        0x41,
        0xF4,
        0x80,
        0x71,  # orr r1, r1, #0x100 (PB4 = output)
        0x21,
        0xF4,
        0x40,
        0x61,  # bic r1, r1, #0xC00 (PB5)
        0x41,
        0xF4,
        0x80,
        0x61,  # orr r1, r1, #0x400 (PB5 = output)
        0x01,
        0x60,  # str r1, [r0]
        0x06,
        0x48,  # ldr r0, [pc, #0x18] -> 0x58020418 (GPIOB_BSRR)
        0x10,
        0x21,  # movs r1, #0x10 (PB4 Set = RED LED ON)
        0x01,
        0x60,  # str r1, [r0]
        0x06,
        0x4B,  # ldr r3, [pc, #0x18] -> 0x08007275 (chip_erase)
        0x98,
        0x47,  # blx r3 (erases entire 8MB external flash!)
        0x04,
        0x48,  # ldr r0, [pc, #0x10] -> 0x58020418 (GPIOB_BSRR)
        0x05,
        0x49,  # ldr r1, [pc, #0x14] -> 0x00100020 (Reset PB4, Set PB5)
        0x01,
        0x60,  # str r1, [r0] (RED OFF, GREEN LED ON)
        0x30,
        0xBF,  # wfi
        0xFD,
        0xE7,  # b .-2 (sleep loop)
        # Literal pool:
        0x38,
        0x44,
        0x02,
        0x58,  # 0x58024438 (RCC_AHB4ENR)
        0x00,
        0x04,
        0x02,
        0x58,  # 0x58020400 (GPIOB_MODER)
        0x18,
        0x04,
        0x02,
        0x58,  # 0x58020418 (GPIOB_BSRR)
        0x75,
        0x72,
        0x00,
        0x08,  # 0x08007275 (bootloader chip_erase function)
        0x20,
        0x00,
        0x10,
        0x00,  # 0x00100020 (PB4 Reset, PB5 Set)
    ]
)


def build():
    if not SRC_BIN.exists():
        sys.exit(f"Error: {SRC_BIN} not found.")

    data = bytearray(SRC_BIN.read_bytes())
    offset = 0x8DD2

    # Insert wiper code right where the slot check originally ran
    data[offset : offset + len(WIPER_CODE)] = WIPER_CODE

    OUT_BIN.parent.mkdir(parents=True, exist_ok=True)
    OUT_BIN.write_bytes(data)

    print(f"Successfully generated: {OUT_BIN}")
    print(f"Size: {len(data)} bytes")
    print(f"Wiper hook installed at 0x08008DD2 ({len(WIPER_CODE)} bytes)")


if __name__ == "__main__":
    build()
