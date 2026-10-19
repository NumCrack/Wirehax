"""
This script will patch a binary file to NOP the bytes where
usually the option bytes for RDP 1 and WRP are modified.

The signature detection works on Epsilon 26.3.0, not sure about the others.

Usage:
  Bootloader:
    python3 nop_option_bytes.py target_0x8000000.bin --base 0x8000000 --output bootloader_patched.bin
  Slot A:
    python3 nop_option_bytes.py target_0x90000000.bin --base 0x90000000 --output slot_a_patched.bin
  Slot B:
    python3 nop_option_bytes.py target_0x90400000.bin --base 0x90400000 --output slot_b_patched.bin
"""

import argparse
import os
import re


def pattern_to_regex(pattern_str):
    tokens = pattern_str.strip().split()
    parts = []
    for token in tokens:
        if token in ("??", "?"):
            parts.append(b".")
        else:
            parts.append(re.escape(bytes.fromhex(token)))
    return b"".join(parts)


def get_default_output_path(input_path):
    directory, filename = os.path.split(input_path)
    prefix, suffix = os.path.splitext(filename)
    new_filename = f"{prefix}_patched{suffix}"
    return os.path.join(directory, new_filename)


def patch_file(input_path, output_path, pattern_str, total_bytes, base_address=0):
    with open(input_path, "rb") as f:
        data = bytearray(f.read())

    regex = pattern_to_regex(pattern_str)
    matches = [m.start() for m in re.finditer(regex, data, flags=re.DOTALL)]

    if len(matches) == 1:
        offset = matches[0]
        nop_sled = b"\x00\xbf" * (total_bytes // 2)
        data[offset : offset + total_bytes] = nop_sled

        target_output = (
            output_path if output_path else get_default_output_path(input_path)
        )

        with open(target_output, "wb") as f:
            f.write(data)

        target_ea = base_address + offset
        print(f"Patched 0x{target_ea:X} ({total_bytes} bytes)")
        print(f"Saved to: {target_output}")
    elif len(matches) > 1:
        print(f"Skipping: Ambiguous match ({len(matches)} occurrences found)")
    else:
        print(f"Pattern not found: {pattern_str}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Patch binary pattern with NOPs")
    parser.add_argument("binary", help="Target binary file to patch")
    parser.add_argument(
        "-o",
        "--output",
        default=None,
        help="Path for output file",
    )
    parser.add_argument(
        "-b",
        "--base",
        type=lambda x: int(x, 0),
        default=0x90000000,
        help="Base memory address (usage in script source)",
    )
    args = parser.parse_args()

    full_pattern = "2B 46 08 21 32 6A 0F 20 ?? ?? ?? ?? 23 46 30 62 00 21 F2 6B 07 20 ?? ?? ?? ?? F0 63"
    patch_file(args.binary, args.output, full_pattern, 28, base_address=args.base)
