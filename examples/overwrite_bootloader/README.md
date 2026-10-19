# Bootloader Overwrite & Flash Diagnostic Utility

A bare-metal C++ firmware application designed to run from **Slot B** (`0x90400000`) via [WireHax](../../README.md).

It provides an on-screen diagnostic dashboard on the calculator's LCD to test STM32H7 internal flash write-protection status (WRP) and optionally overwrites internal flash Sector 0 (`0x08000000`) with a replacement bootloader binary.

>[!CAUTION]
>**HIGH BRICK RISK:**
>Overwriting internal flash (`0x08000000`) is an advanced research and recovery procedure.
>If the sector erase succeeds but writing fails, or if the supplied bootloader binary is invalid or corrupt,
>**your calculator will be hard-bricked** and unable to boot into either regular mode or rescue DFU.
>
>Do **NOT** run this utility unless you have access to an external hardware ST-Link / SWD debugger and have verified your ability to recover the device.

---

## What It Does

When booted into Slot B by WireHax:

1. **Initializes the Display:** Takes control of the display controller (ST7789V) and renders an on-device diagnostic interface.
2. **Checks Flash Protection:** Unlocks the STM32H7 Flash control registers (`0x52002000`) using standard keys and queries `FLASH_WPSN_CUR` for write-protection status.
3. **Tests Writeability:**
   - If Sector 0 is protected, the display immediately reports a write-protection error (`WRPERR`) and halts safely without damaging the existing bootloader.
   - If Sector 0 is writable, it erases the necessary 128 KiB sectors in Bank 1.
4. **Programs the Bootloader:** Embeds the bootloader image provided at build time, flashing it in 256-bit (32-byte) blocks with a real-time progress bar on the LCD.
5. **Readback Verification:** Compares the written internal flash memory against the original image byte-for-byte and displays the verification status (`100% MATCH` or mismatch address).

---

## Prerequisites

* **LLVM/Clang Toolchain:** `clang`, `clang++`, `lld`, and `llvm-objcopy` with ARM cross-compilation support (`-target arm-none-eabi`).
* **`dfu-util`:** Installed on your host machine.
* **WireHax:** Compiled in release mode (`cargo build --release` in the project root).
* **Target Bootloader Binary:** You must supply your own bootloader binary (e.g. dumped from your calculator via `wirehax dump` or an extracted stock bootloader). WireHax does not distribute proprietary NumWorks binaries.

---

## Build, Flash, and Run

### 1. Provide the Target Bootloader
Place your replacement bootloader at `firmware/bootloader.bin` in the repository root:

```sh
# Example: copy a previously dumped flash or bootloader image
cp path/to/your/bootloader.bin ../../firmware/bootloader.bin
```

Alternatively, pass `BOOT_BIN` directly to `make`:

```sh
make BOOT_BIN=/path/to/bootloader.bin
```

### 2. Compile the Slot B Binary
From this directory (`examples/overwrite_bootloader`):

```sh
make
```

This compiles the bare-metal C++ code and links the bootloader binary into a raw image at `build/slot_b.bin`.

Verify the slot header format with:

```sh
make check
```

### 3. Flash to Slot B
Put the calculator into **Rescue DFU** mode:
1. Hold **6**.
2. Press and release the rear **RESET** button.
3. Release **6**.

Flash the generated utility to Slot B:

```sh
make flash
# Equivalent to:
# dfu-util -a 0 -s 0x90400000:force -D build/slot_b.bin
```

### 4. Boot with WireHax
With the calculator still in rescue DFU, execute WireHax:

```sh
make boot
# Equivalent to:
# ../../target/release/wirehax
```

---

## Interpreting the Screen Output

The calculator LCD will report step-by-step execution:

* **WRP Check:** Displays whether internal flash Bank 1 is unlocked or write-protected.
* **ERASE Sector 0:** Shows whether the bootloader sector erase completed or failed with `WRPERR`.
* **PROGRAM:** Renders a progress bar while writing 32-byte blocks. Turns green on `SUCCESS` or red on `FAILED`.
* **VERIFY:** Confirms `100% MATCH! Zero Errors.` upon successful readback, or flags the exact mismatch address if verification fails.
* **Exit:** Once complete, press the rear **RESET** button to reboot into the newly programmed bootloader.
