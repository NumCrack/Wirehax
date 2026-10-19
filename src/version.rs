use anyhow::{bail, Context, Result};
use rusb::{DeviceHandle, GlobalContext};
use std::fs;
use std::path::Path;

pub const MAGIC_HEADER: u32 = 0xDEC00DF0;
pub const SLOT_A_ADDR: u32 = 0x90000000;
pub const SLOT_B_ADDR: u32 = 0x90400000;
pub const DEFAULT_SUPPORTED: &[&str] = &[];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpsilonHeader {
    pub offset: usize,
    pub slot_type: u32,
    pub length: u32,
    pub version: String,
    pub commit: String,
}

pub fn parse_header_at(data: &[u8], offset: usize) -> Option<EpsilonHeader> {
    if data.len() < offset + 32 {
        return None;
    }

    let magic1 = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().ok()?);
    let magic2 = u32::from_le_bytes(data[offset + 28..offset + 32].try_into().ok()?);
    if magic1 != MAGIC_HEADER || magic2 != MAGIC_HEADER {
        return None;
    }

    let slot_type = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?);
    let length = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);

    let raw_version = &data[offset + 12..offset + 20];
    let ver_len = raw_version.iter().position(|&b| b == 0).unwrap_or(raw_version.len());
    let version = String::from_utf8_lossy(&raw_version[..ver_len]).trim().to_string();

    let raw_commit = &data[offset + 20..offset + 28];
    let com_len = raw_commit.iter().position(|&b| b == 0).unwrap_or(raw_commit.len());
    let commit = String::from_utf8_lossy(&raw_commit[..com_len]).trim().to_string();

    Some(EpsilonHeader {
        offset,
        slot_type,
        length,
        version,
        commit,
    })
}

pub fn find_headers_in_data(data: &[u8]) -> Vec<EpsilonHeader> {
    let mut results = Vec::new();
    let magic_bytes = MAGIC_HEADER.to_le_bytes();

    let mut idx = 0;
    while idx + 4 <= data.len() {
        if let Some(pos) = data[idx..].windows(4).position(|w| w == magic_bytes) {
            let found_idx = idx + pos;
            if found_idx >= 8 && found_idx - 8 + 32 <= data.len() {
                let header_start = found_idx - 8;
                if let Some(hdr) = parse_header_at(data, header_start)
                    && !results.iter().any(|r: &EpsilonHeader| r.version == hdr.version && r.commit == hdr.commit) {
                        results.push(hdr);
                    }
            }
            idx = found_idx + 4;
        } else {
            break;
        }
    }

    results
}

pub fn check_file(file_path: &Path, supported_versions: &[&str]) -> Result<bool> {
    if !file_path.exists() {
        bail!("File does not exist: '{}'", file_path.display());
    }

    println!("Scanning file: '{}'...", file_path.display());
    let data = fs::read(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
    let headers = find_headers_in_data(&data);

    if headers.is_empty() {
        match crate::bootloader::check_flash(&data) {
            crate::bootloader::HashStatus::Unsupported => {
                let hash = crate::bootloader::compute_sha256(&data);
                println!(
                    "  Detected Bootloader: Patched bootloader (SHA-256: {hash}) -> [UNSUPPORTED]"
                );
                println!("\nStatus: FAIL - Target bootloader is unsupported (already patched).");
                return Ok(false);
            }
            crate::bootloader::HashStatus::Supported(_kind, name) => {
                let hash = crate::bootloader::compute_sha256(&data);
                println!(
                    "  Detected Bootloader: {name} (SHA-256: {hash}) -> [SUPPORTED]"
                );
                println!("\nStatus: OK - Target bootloader version is supported.");
                return Ok(true);
            }
            crate::bootloader::HashStatus::Unknown => {}
        }
        println!("No valid Epsilon headers found in this file.");
        return Ok(false);
    }

    let mut all_supported = true;
    for hdr in &headers {
        if !supported_versions.is_empty() {
            let is_supported = supported_versions.contains(&hdr.version.as_str());
            let status_str = if is_supported {
                "SUPPORTED".to_string()
            } else {
                "UNSUPPORTED".to_string()
            };
            println!(
                "  Found at offset 0x{:x}: Epsilon {} (commit {}) -> [{}]",
                hdr.offset, hdr.version, hdr.commit, status_str
            );
            if !is_supported {
                all_supported = false;
            }
        } else {
            println!(
                "  Found at offset 0x{:x}: Epsilon {} (commit {})",
                hdr.offset, hdr.version, hdr.commit
            );
        }
    }

    if !supported_versions.is_empty() {
        println!("\nSupported versions: {}", supported_versions.join(", "));
        if all_supported {
            println!("Status: OK - Target OS version is supported.");
            Ok(true)
        } else {
            println!("Status: FAIL - Detected OS version is not in the supported list.");
            Ok(false)
        }
    } else {
        println!("\nStatus: OK - Valid Epsilon header detected.");
        Ok(true)
    }
}

pub fn check_live_device(
    handle: &DeviceHandle<GlobalContext>,
    supported_versions: &[&str],
    slot: &str,
) -> Result<bool> {
    let mut slots_to_check = Vec::new();
    if slot == "a" || slot == "all" {
        slots_to_check.push(("Slot A", SLOT_A_ADDR));
    }
    if slot == "b" || slot == "all" {
        slots_to_check.push(("Slot B", SLOT_B_ADDR));
    }

    let mut found_any = false;
    let mut all_supported = true;

    for (slot_name, addr) in slots_to_check {
        println!("Reading {slot_name} header at 0x{addr:08x}...");
        let header_bytes = crate::dfu::read_memory(handle, addr, 64)?;
        if let Some(hdr) = parse_header_at(&header_bytes, 0)
            && !hdr.version.is_empty() {
                found_any = true;
                if !supported_versions.is_empty() {
                    let is_supported = supported_versions.contains(&hdr.version.as_str());
                    let status_str = if is_supported {
                        "SUPPORTED".to_string()
                    } else {
                        "UNSUPPORTED".to_string()
                    };
                    println!(
                        "  {}: Epsilon {} (commit {}) -> [{}]",
                        slot_name, hdr.version, hdr.commit, status_str
                    );
                    if !is_supported {
                        all_supported = false;
                    }
                } else {
                    println!(
                        "  {}: Epsilon {} (commit {})",
                        slot_name, hdr.version, hdr.commit
                    );
                }
                continue;
            }
        println!("  {slot_name}: No valid Epsilon header detected.");
    }

    if !found_any {
        println!("\nWarning: No valid Epsilon header was found in external flash.");
        return Ok(false);
    }

    if !supported_versions.is_empty() {
        println!("\nSupported versions: {}", supported_versions.join(", "));
        if all_supported {
            println!("Status: OK - Target OS version is supported.");
            Ok(true)
        } else {
            println!("Status: FAIL - Detected OS version is not in the supported list.");
            Ok(false)
        }
    } else {
        println!("\nStatus: OK - Detected Epsilon OS header(s).");
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_dummy_header(slot: u32, length: u32, version: &str, commit: &str) -> Vec<u8> {
        let mut data = vec![0u8; 32];
        data[0..4].copy_from_slice(&slot.to_le_bytes());
        data[4..8].copy_from_slice(&length.to_le_bytes());
        data[8..12].copy_from_slice(&MAGIC_HEADER.to_le_bytes());

        let ver_bytes = version.as_bytes();
        let copy_ver = ver_bytes.len().min(8);
        data[12..12 + copy_ver].copy_from_slice(&ver_bytes[..copy_ver]);

        let com_bytes = commit.as_bytes();
        let copy_com = com_bytes.len().min(8);
        data[20..20 + copy_com].copy_from_slice(&com_bytes[..copy_com]);

        data[28..32].copy_from_slice(&MAGIC_HEADER.to_le_bytes());
        data
    }

    #[test]
    fn test_parse_header_at_valid() {
        let header_data = create_dummy_header(1, 0x50000, "26.3.0", "c0ffee");
        let parsed = parse_header_at(&header_data, 0).expect("failed to parse valid header");

        assert_eq!(parsed.slot_type, 1);
        assert_eq!(parsed.length, 0x50000);
        assert_eq!(parsed.version, "26.3.0");
        assert_eq!(parsed.commit, "c0ffee");
        assert_eq!(parsed.offset, 0);
    }

    #[test]
    fn test_parse_header_at_corrupt() {
        // Too short
        assert!(parse_header_at(&[0u8; 10], 0).is_none());

        // Incorrect magic
        let mut bad_magic = create_dummy_header(1, 0x1000, "26.0.0", "1234");
        bad_magic[8..12].copy_from_slice(&0x12345678u32.to_le_bytes());
        assert!(parse_header_at(&bad_magic, 0).is_none());
    }

    #[test]
    fn test_find_headers_in_data() {
        let mut buffer = vec![0xFFu8; 512];
        let h1 = create_dummy_header(0, 0x10000, "26.0.3", "aaa111");
        let h2 = create_dummy_header(1, 0x20000, "26.3.0", "bbb222");

        buffer[64..64 + 32].copy_from_slice(&h1);
        buffer[256..256 + 32].copy_from_slice(&h2);

        let results = find_headers_in_data(&buffer);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].offset, 64);
        assert_eq!(results[0].version, "26.0.3");
        assert_eq!(results[1].offset, 256);
        assert_eq!(results[1].version, "26.3.0");
    }

    #[test]
    fn test_check_file_with_and_without_filter() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_epsilon_slot.bin");
        let header = create_dummy_header(0, 0x10000, "99.0.0", "custom");
        fs::write(&test_file, &header).unwrap();

        // Without filter (empty slice): should accept any valid header
        let ok_no_filter = check_file(&test_file, DEFAULT_SUPPORTED).unwrap();
        assert!(ok_no_filter);

        // With filter matching:
        let ok_matching = check_file(&test_file, &["99.0.0"]).unwrap();
        assert!(ok_matching);

        // With filter not matching:
        let fail_mismatch = check_file(&test_file, &["26.3.0"]).unwrap();
        assert!(!fail_mismatch);

        let _ = fs::remove_file(&test_file);
    }
}
