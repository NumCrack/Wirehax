>[!NOTE]
>This is the code as I (Bjarnos) received it from Andrei,
>it will not work on a live device. I only published it
>so that you can check out the Wirehax control flow in Python,
>instead of in Rust. Information below may be outdated.

# WireHax

WireHax boots a modified NumWorks N0120 Slot B image through the rescue-DFU
ROP/RCE path. It targets Epsilon 26.3.0 and uses one executable entrypoint.

The boot is temporary: the script stages its ROP chain and chainloader in RAM,
then branches to the external Slot B image. It does not replace the bootloader
or write internal flash.

## Requirements

- NumWorks N0120 running Epsilon 26.3.0.
- Python 3 and PyUSB.
- USB access for device `0483:a291`.
- A modified Slot B test sector at external address `0x904f0000`.

The checked-in binary payloads are already built. No compiler or platform shell
is needed to run the boot path.

## Setup

Run these commands from the repository root:

```sh
python3 -m pip install -r requirements.txt
./wirehax.py --self-check
```

The self-check validates the bundled chainloader and the ROP layout without
connecting to USB. It should print:

```text
SELF_CHECK_OK
```

The operating system must grant the current user access to USB device
`0483:a291`. The exact permission mechanism is platform-specific; verify USB
access before running the exploit rather than adding privileges to the script.

## Prepare the calculator

Enter rescue DFU before every boot attempt:

1. Hold **6**.
2. Press and release the rear **RESET** button.
3. Release **6**.

The calculator should remain on the rescue DFU screen and appear as USB device
`0483:a291`.

The boot script expects the modified external Slot B sector to already exist.
If this is a stock test device, install only the included 64 KiB candidate:

```sh
dfu-util -a 0 -s 0x904f0000:force -D firmware/slot-b-sector-modified.bin
```

That command changes external Slot B only. It is optional and is not part of
the boot script. Do not use it as a replacement for the official recovery
procedure.

## Boot

With the calculator in rescue DFU, run the only entrypoint:

```sh
./wirehax.py
```

The script prints `BOOT_ATTEMPT_SENT` after handing control to the SRAM
chainloader. A successful test image displays:

```text
UNSIGNED FIRMWARE RUNNING
```

The command must be run again after returning to rescue DFU for another test.

The final DFU request can report `DFU_DISCONNECT_EXPECTED`. This is normal: the
calculator has already left DFU and started the chainloader before the host
receives the USB acknowledgement. Continue if the calculator displays the
modified firmware.

## How it works

This section describes the exact control-flow handoff implemented by
`wirehax.py`. Addresses are for the checked-in N0120/Epsilon 26.3.0 image;
the script does not search for equivalent gadgets in another firmware build.

### Address map

The script treats the rescue DFU RAM window as a 320 KiB region beginning at
`0x24000000`:

| Symbol | Address | Purpose |
| --- | ---: | --- |
| `BASE` | `0x24000000` | Start of the readable/writable DFU RAM window |
| `FAKE_VTABLE` | `0x24000080` | Temporary vtable used by the probe |
| `STAGE` | `0x24010000` | 512-byte ROP stage |
| `MARKER` | `0x240100c0` | ROP marker written by the first `memmove` |
| `SRAM_MARKER` | `0x240101f0` | Chainloader marker (`XIP!`) |
| `SRAM_CODE` | `0x24010200` | Thumb entry of `xip-chainloader.bin` |
| `SRAM_STACK` | `0x24010300` | Stack selected when the ROP chain enters SRAM |
| `WARM` | `0x2401f000` | DFU download address used for the overflow payload |
| `MPU_CTRL` | `0xe000ed94` | Cortex-M MPU control register |
| `MPU_RNR` | `0xe000ed98` | Cortex-M MPU region selector |
| `MPU_RBAR` | `0xe000ed9c` | Cortex-M MPU region base register |
| `MPU_RASR` | `0xe000eda0` | Cortex-M MPU region attributes/size register |
| Slot B vector table | `0x90400020` | External XIP address containing MSP and reset handler |

The firmware snapshot at `firmware/internal-flash-live.bin`, offset
`0xa2e8`, supplies a 256-byte stock vtable. The entries used by this chain
are:

```text
vtable + 0x08 = 0x08007ced   stock callback
vtable + 0x0c = 0x08001765   stock byte setter
```

The script copies that table into RAM and changes only the callback entry when
it needs to redirect execution. `0x08001765` is important because it is the
known-good primitive used to write a selected byte. `0x080017d1` is the
known-good word-store primitive used for the MPU RASR write.

### USB DFU primitives

The Python code uses the standard DFU control-transfer shapes directly:

| Operation | `bmRequestType` | `bRequest` | Important arguments |
| --- | ---: | ---: | --- |
| Select DFU alternate setting | `0x01` | `11` | `wValue = 1` |
| Read status | `0xa1` | `3` | six-byte status response |
| Set address | `0x21` | `1` | block `0`, payload `0x21 || uint32(address)` |
| Download RAM data | `0x21` | `1` | block `2`, up to 2048 bytes |
| Upload RAM data | `0xa1` | `2` | block `2+N`, requested byte count |
| Abort current transfer | `0x21` | `6` | resets the DFU transfer state |

`write_memory()` splits writes into 2048-byte chunks. `read_memory()` sets the
address, aborts into the upload path, reads sequential upload blocks, and
aborts again when finished. Every transfer waits for the DFU status delay and
checks that the device reports an acceptable state.

### Phase 1: discover the live callback root

The first overflow is a calibration pass. The exact stack/object address is
not assumed, so the script finds it from the changed RAM state.

1. It reads the complete RAM window at `0x24000000` and saves it as `before`.
2. It writes the stock vtable to `FAKE_VTABLE = 0x24000080`.
3. It verifies the vtable readback before triggering anything.
4. It creates a `0x93c`-byte zeroed payload and stores the fake-vtable pointer
   at payload offset `0x938`.
5. It sets the DFU address to `WARM = 0x2401f000` and downloads that payload.
6. It sends the callback-setter trigger with value `0x5a`.
7. It reads the RAM window again as `after`.

`fake_hits()` scans every aligned word in `after` for the value
`FAKE_VTABLE`. For each match it examines the word at `match + 0x824` and
records the old and new byte values. The expected fresh root is the single
match for which:

```text
before[root_offset + 0x824] != 0x5a
after[root_offset + 0x824]  == 0x5a
```

The script rejects zero or multiple fresh roots. The resulting `root` is the
run-specific callback/object address. The final vtable pointer is derived as:

```text
dynamic_vtable = root - 0x138
callback_sp     = root - 0x9bc
```

This is why the final ROP payload can use the live object discovered on that
run instead of hard-coding a stack address.

### Phase 2: construct the ROP stage

`build_stage()` creates exactly `0x200` bytes at `STAGE`. The stage is a
sequence of return-oriented gadget addresses and their following stack words.
The important gadgets are:

```text
0x0800a8ef  pop {r0, r1, r2, r4, r6, r7, pc}
0x08003687  pop {r3, pc}
0x08007b07  bx r3
0x080035d3  memmove-like copy primitive
0x08008331  stack pivot primitive
0x08001765  stock byte setter
0x080017d1  stock word store
```

The stage executes in this order:

1. **Write the first marker.** The initial register state and the first
   `pop {r3,pc}; bx r3` pair call the copy primitive with an effective
   `memmove(MARKER, STAGE + 0x1c0, 4)`. The source contains
   `0xd00dfeed`, so `0x240100c0` becomes the ROP execution marker.
2. **Restore the callback entry.** The next register-pop gadget loads
   `r0 = dynamic_vtable + 8`, `r1 = STAGE + 0x1c4`, and `r2 = 4`. The same
   copy primitive restores `0x08007ced` over the temporary callback entry.
3. **Select MPU region 2.** The next register-pop gadget prepares the stock
   byte setter. Its target word is encoded as `MPU_RNR - 0x824` because the
   setter applies its callback-object offset internally. The value `2` is
   written to `0xe000ed98`, selecting MPU region 2.
4. **Make SRAM executable.** The following register-pop gadget loads
   `r0 = 0xe000eda0` and `r2 = 0x010b0039`. The word-store primitive writes
   that value to MPU RASR for region 2. This is the non-XN SRAM configuration
   needed before executing code at `SRAM_CODE`.
5. **Pivot into the SRAM payload.** The return path reaches the pivot gadget
   with a table at `STAGE + 0x1e0` containing:

   ```text
   [STAGE + 0x1e0] = SRAM_STACK       = 0x24010300
   [STAGE + 0x1e4] = SRAM_CODE | 1     = 0x24010201
   ```

   The low bit selects Thumb state. The pivot therefore supplies the SRAM
   stack and transfers control to the chainloader at `0x24010200`.

The `root` argument is used to derive `dynamic_vtable`; the fixed gadget
addresses and fixed SRAM layout are specific to the checked-in firmware.

### Phase 3: enter the ROP stage

Before the final trigger, the script writes and verifies:

```text
MARKER       = zeroed 4-byte ROP marker
SRAM_MARKER  = zeroed 4-byte chainloader marker
SRAM_CODE    = payloads/xip-chainloader.bin
STAGE        = the 0x200-byte ROP stage
```

It then builds the final `0x93c`-byte overflow payload:

```text
payload + 0x08 = STAGE
payload + 0x0c = 0x0800a8ef
payload + 0x938 = dynamic_vtable
```

At payload offset `0x800` it embeds a second stock vtable copy. That copy
uses `0x08006015` as its callback and `0x0800795d` in its nested pivot slot.
The final download goes to `WARM`, followed by the second trigger request.
The callback machinery consumes the overflow, reaches the ROP stage, and the
ROP stage pivots into SRAM.

### Phase 4: the SRAM chainloader

`payloads/xip-chainloader.bin` is the assembled form of
`payloads/xip-chainloader.s`. Its literal sequence is:

```asm
cpsid i                         ; block interrupts during MPU takeover
ldr  r3, [marker_addr]
ldr  r2, [marker_value]
str  r2, [r3]                   ; *(0x240101f0) = 0x58495021 ("XIP!")

ldr  r3, [mpu_ctrl_addr]
movs r2, #0
str  r2, [r3]                   ; MPU_CTRL = 0
dsb  sy                         ; complete the control-register write

MPU_RNR  = 6
MPU_RBAR = 0x90000000
MPU_RASR = 0x0102002d
MPU_CTRL = 1
dsb  sy
isb

cpsie i                         ; resume interrupts after the handoff setup

r0 = 0x90400020
r1 = [r0]                       ; Slot B initial MSP
r2 = [r0 + 4]                   ; Slot B reset handler
MSP = r1
bx   r2                         ; enter Slot B in Thumb state
```

The chainloader first leaves the `XIP!` marker at `SRAM_MARKER`, then disables
the MPU while changing region 6. Region 6 is configured with an XIP base of
`0x90000000` and attributes `0x0102002d`, covering the external-mapped Slot B
execution region. The MPU is re-enabled, instruction/data barriers complete
the configuration, and interrupts are restored.

The vector table address is `0x90400020`, not the XIP region base. Its first
word becomes the new main stack pointer; its second word becomes the reset
handler address. Writing MSP and branching to that handler abandons the DFU
callback stack and transfers execution to Slot B. There is no return path to
the Python process or to the stock DFU cleanup routine.

### What is and is not persistent

The ROP stage, fake vtables, markers, and chainloader are written to volatile
SRAM. The boot script does not write internal flash, the bootloader partition,
or the external Slot B sector. The optional `dfu-util` command in the setup
section is the only documented command that writes the external test sector.
After reset or recovery, the SRAM chain is gone and the process must be run
again from rescue DFU.


## Recovery

If the screen goes blank or the USB device disappears, return to rescue DFU:

1. Hold **6**.
2. Press and release the rear **RESET** button.
3. Release **6**.

`firmware/stock.dfu` is the stock recovery artifact. `wirehax.py` never
flashes it.

## Troubleshooting

`PyUSB is required` means the dependency setup was skipped:

```sh
python3 -m pip install -r requirements.txt
```

`rescue DFU device 0483:a291 not found` means the calculator is not in rescue
DFU, the USB cable is unsuitable, or the current user lacks USB access.

`USB device is not responding as rescue DFU` means a matching USB device was
found, but it did not answer DFU requests. Put the calculator back into rescue
DFU and retry.

The script performs this check before writing any RAM. It will stop if the
device is present but its DFU state is not ready for a transfer.

If `--self-check` fails, do not connect the calculator. Restore the checked-in
payload files before attempting a boot.

## Layout

- `wirehax.py` — the only executable entrypoint; performs the complete ROP/RCE
  boot flow.
- `payloads/` — SRAM chainloader source, ELF, and checked-in binary.
- `firmware/` — flash captures, Slot images, modified sector, and stock DFU.
- `analysis/` — supporting bootloader disassembly.
