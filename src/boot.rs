pub mod base;
pub mod v20_24;
pub mod v24_26;

use anyhow::{Context, Result, bail};
use rusb::{DeviceHandle, GlobalContext};
use std::fs;
use std::path::Path;

pub use crate::bootloader::*;
pub use base::*;
pub use v20_24::{build_stage, stock_vtable};

pub fn self_check(firmware_path: Option<&Path>) -> Result<()> {
    let default_flash = Path::new("firmware/internal-flash-live.bin");
    let flash_data = if let Some(p) = firmware_path {
        fs::read(p).with_context(|| {
            format!("Failed to read firmware binary from: {}", p.display())
        })?
    } else if default_flash.exists() {
        fs::read(default_flash)?
    } else {
        bail!(
            "Self-check requires a firmware binary. Dump one with wirehax dump \
            or supply one with --firmware <path>."
        );
    };

    let hash = compute_sha256(&flash_data);
    let status = crate::bootloader::check_flash(&flash_data);

    println!(
        "Self-check: inspecting bootloader hash and selecting codebase..."
    );
    match status {
        HashStatus::Unsupported => {
            println!(
                "\n[!] DECLINING SELF-CHECK: Target bootloader has an UNSUPPORTED hash: Declined (SHA-256: {hash})\nReason: This bootloader is already patched and unsupported.\n"
            );
            bail!("Self-check declined: {hash} (unsupported bootloader hash)");
        }
        HashStatus::Supported(kind, name) => {
            println!(
                "  Detected bootloader: {name} (SHA-256: {hash})"
            );
            match kind {
                BootloaderKind::V20_24 => v20_24::self_check(&flash_data),
                BootloaderKind::V24_26 => v24_26::self_check(&flash_data),
            }
        }
        HashStatus::Unknown => {
            println!(
                "Bootloader hash not recognized in database; probing fallback methods..."
            );
            let v20_24_res = v20_24::probe(&flash_data);
            let v24_26_res = v24_26::probe(&flash_data);

            if v20_24_res.is_ok() && v24_26_res.is_err() {
                println!("Fallback probe: v20_24 method matched!");
                v20_24::self_check(&flash_data)
            } else if v20_24_res.is_err() && v24_26_res.is_ok() {
                println!("Fallback probe: v24_26 method matched!");
                v24_26::self_check(&flash_data)
            } else if v20_24_res.is_ok() && v24_26_res.is_ok() {
                println!(
                    "Fallback probe: Both methods matched! Testing v24_26..."
                );
                v24_26::self_check(&flash_data)
            } else {
                bail!(
                    "Self-check failed: unknown bootloader and fallback methods failed signature matching: v20_24 error: {}; v24_26 error: {}",
                    v20_24_res.unwrap_err(),
                    v24_26_res.unwrap_err()
                );
            }
        }
    }
}

pub fn boot(
    handle: &DeviceHandle<GlobalContext>,
    firmware_path: Option<&Path>,
    save_dump: bool,
) -> Result<()> {
    let flash_data = if let Some(p) = firmware_path {
        fs::read(p).with_context(|| {
            format!("Failed to read firmware from: {}", p.display())
        })?
    } else {
        println!(
            "Dumping internal flash to detect bootloader version..."
        );
        crate::dfu::dump_flash_data(handle, 0x08000000, 0x80000, 0, false)?
    };

    let hash = compute_sha256(&flash_data);
    let status = crate::bootloader::check_flash(&flash_data);

    // Check hash status against hardcoded database
    match status {
        HashStatus::Unsupported => {
            println!(
                "\n[!] DECLINING BOOT: Detected bootloader has an UNSUPPORTED hash: Declined (SHA-256: {hash})\nReason: This bootloader is already patched and unsupported.\n"
            );
            bail!("Declined to boot: {hash} (unsupported bootloader hash)");
        }
        HashStatus::Supported(kind, name) => {
            println!(
                "Selected bootloader codebase for: {name} (SHA-256: {hash})"
            );

            match kind {
                BootloaderKind::V20_24 => v20_24::boot(handle, &flash_data, save_dump),
                BootloaderKind::V24_26 => v24_26::boot(handle, &flash_data, save_dump),
            }
        }
        HashStatus::Unknown => {
            // Unrecognized hash: probe fallback
            println!(
                "Bootloader hash not recognized in database: {hash} (testing fallback Wirehax methods...)"
            );

            let v20_24_res = v20_24::probe(&flash_data);
            let v24_26_res = v24_26::probe(&flash_data);

            if v20_24_res.is_ok() && v24_26_res.is_err() {
                println!(
                    "Fallback probe: v20_24 method matched successfully!"
                );
                println!(
                    "Automatically performing Wirehax with v20_24 method..."
                );
                v20_24::boot(handle, &flash_data, save_dump)
            } else if v20_24_res.is_err() && v24_26_res.is_ok() {
                println!(
                    "Fallback probe: v24_26 method matched successfully!"
                );
                println!(
                    "Automatically performing Wirehax with v24_26 method..."
                );
                v24_26::boot(handle, &flash_data, save_dump)
            } else if v20_24_res.is_ok() && v24_26_res.is_ok() {
                println!(
                    "Fallback probe: Both methods matched! Selecting modern v24_26 method..."
                );
                println!(
                    "Automatically performing Wirehax with v24_26 method..."
                );
                v24_26::boot(handle, &flash_data, save_dump)
            } else {
                let combined_err = format!(
                    "v20_24 method failed: {:#}; v24_26 method failed: {:#}",
                    v20_24_res.unwrap_err(),
                    v24_26_res.unwrap_err()
                );
                println!(
                    "Fallback Wirehax methods failed on unknown bootloader: {combined_err}"
                );
                bail!("Unrecognized bootloader: fallback Wirehax methods failed: {combined_err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootloaders_grouping_and_resolution() {
        // Verify unsupported hash on stock_patched/bootloader.bin
        let patched_path = Path::new("firmware/stock_patched/bootloader.bin");
        if patched_path.exists() {
            let data = fs::read(patched_path).unwrap();
            let status = check_flash(&data);
            assert_eq!(status, HashStatus::Unsupported);
        }

        // Test supported old bootloaders
        for path in &[
            Path::new("firmware/bootloaders/20-4-0.bin"),
            Path::new("firmware/bootloaders/boot_old.bin"),
            Path::new("firmware/bootloaders/boot_old_2.bin"),
        ] {
            if path.exists() {
                let data = fs::read(path).unwrap();
                let status = check_flash(&data);
                assert!(matches!(status, HashStatus::Supported(BootloaderKind::V20_24, _)));
                assert!(v20_24::self_check(&data).is_ok());

                let syms =
                    crate::sigscan::resolve_symbols(&data, crate::sigscan::FLASH_BASE).unwrap();
                let stage = build_stage(FAKE_VTABLE, &syms);
                assert_eq!(word(&stage, 0x84), syms.stock_setter);
                assert_eq!(word(&stage, 0xB8), syms.word_store);
                assert_eq!(word(&stage, 0x1E4), SRAM_CODE | 1);
                break;
            }
        }

        // Test supported new bootloader
        for path in &[
            Path::new("firmware/bootloaders/24-2-0.bin"),
            Path::new("firmware/bootloaders/boot_new.bin"),
        ] {
            if path.exists() {
                let data = fs::read(path).unwrap();
                let status = check_flash(&data);
                assert!(matches!(status, HashStatus::Supported(BootloaderKind::V24_26, _)));
                assert!(v24_26::self_check(&data).is_ok());

                let syms = v24_26::resolve_symbols(&data, crate::sigscan::FLASH_BASE).unwrap();
                let stage = v24_26::build_stage(v24_26::FAKE_VTABLE, &syms);
                assert_eq!(v24_26::word(&stage, 0x84), syms.stock_setter);
                assert_eq!(v24_26::word(&stage, 0xB4), syms.word_store);
                assert_eq!(v24_26::word(&stage, 0x1E4), SRAM_CODE | 1);
                break;
            }
        }

        // Test unknown flash hash
        let dummy = vec![0x33; 0x1000];
        let status = check_flash(&dummy);
        assert_eq!(status, HashStatus::Unknown);
    }

    #[test]
    fn test_chainloader_payload() {
        let chainloader = get_chainloader();
        assert_eq!(chainloader.len(), 116);
        assert!(validate_chainloader(&chainloader).is_ok());
        assert_eq!(chainloader, v20_24::get_chainloader());
        assert_eq!(chainloader, v24_26::get_chainloader());
        assert!(chainloader.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_fallback_probing() {
        // v20_24 binary should succeed with v20_24::probe and fail with others
        for old_path in &[
            Path::new("firmware/internal-flash-live.bin"),
            Path::new("firmware/bootloaders/boot_old.bin"),
        ] {
            if old_path.exists() {
                let data = fs::read(old_path).unwrap();
                assert!(v20_24::probe(&data).is_ok());
                assert!(v24_26::probe(&data).is_err());
                break;
            }
        }

        // 24.2.0 binary should succeed with v24_26::probe and fail with others
        for v24_path in &[
            Path::new("firmware/bootloaders/24-2-0.bin"),
            Path::new("firmware/bootloaders/boot_new.bin"),
        ] {
            if v24_path.exists() {
                let data = fs::read(v24_path).unwrap();
                assert!(v24_26::probe(&data).is_ok());
                assert!(v20_24::probe(&data).is_err());
                break;
            }
        }

        // Invalid dummy data should fail all probes
        let dummy = vec![0xAA; 0x1000];
        assert!(v20_24::probe(&dummy).is_err());
        assert!(v24_26::probe(&dummy).is_err());
    }
}
