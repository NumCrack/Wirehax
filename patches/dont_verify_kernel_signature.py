"""
This script will patch a binary file to make the kernel
verification (before a slot is booted) always return success,
effectively always booting slot A no matter what is in it.

The signature matcher handles the known modern and legacy bootloader prologues.

Usage:
  Bootloader:
    python3 dont_verify_kernel_signature.py target_0x8000000.bin --base 0x8000000 --output bootloader_patched.bin
  Slot A:
    python3 dont_verify_kernel_signature.py target_0x90000000.bin --base 0x90000000 --output slot_a_patched.bin
  Slot B:
    python3 dont_verify_kernel_signature.py target_0x90400000.bin --base 0x90400000 --output slot_b_patched.bin
"""

import argparse
import os
import re

PATTERNS = (
    "2D E9 F0 41 D0 E9 00 54 8C B0",  # Modern bootloaders
    "2D E9 F0 41 05 68 8C B0",  # Legacy bootloaders
)
PATCH_BYTES = bytes([0x01, 0x20, 0x70, 0x47])


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


def patch_signature_file(
    input_path, output_path, patterns, patch_bytes, base_address=0
):
    with open(input_path, "rb") as f:
        data = bytearray(f.read())

    matches = sorted(
        {
            m.start()
            for pattern in patterns
            for m in re.finditer(pattern_to_regex(pattern), data, flags=re.DOTALL)
        }
    )
    match_count = len(matches)

    if match_count == 0:
        print("Error: Pattern not found.")
        return

    if match_count > 2:
        print(
            f"Error: Ambiguous result. Found {match_count} matches (maximum allowed is 2):"
        )
        for offset in matches:
            print(f"  - 0x{base_address + offset:08X}")
        return

    print(f"Found {match_count} match(es):")
    patch_len = len(patch_bytes)
    for offset in matches:
        ea = base_address + offset
        print(f"  Patching at: 0x{ea:08X}")
        data[offset : offset + patch_len] = patch_bytes

    target_output = output_path if output_path else get_default_output_path(input_path)
    with open(target_output, "wb") as f:
        f.write(data)

    print("Patching completed.")
    print(f"Saved to: {target_output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Patch binary signature check functions"
    )
    parser.add_argument("binary", help="Target binary file to patch")
    parser.add_argument(
        "-o",
        "--output",
        default=None,
        help="Path for output file (default: <prefix>_patched.<suffix>)",
    )
    parser.add_argument(
        "--base",
        type=lambda x: int(x, 0),
        default=0x08000000,
        help="Base memory address (e.g. 0x08000000 or 0x90000000)",
    )
    args = parser.parse_args()

    patch_signature_file(
        args.binary,
        args.output,
        PATTERNS,
        PATCH_BYTES,
        base_address=args.base,
    )
