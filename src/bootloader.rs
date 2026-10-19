use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderKind {
    /// Legacy bootloader family (boot_old and boot_old_2, versions 20-24).
    /// Both binaries share the exact same ROP execution logic and 0x824 offset,
    /// with addresses dynamically resolved via Thumb-2 signature scanning.
    V20_24,
    /// Modern bootloader family (boot_new, versions 24-26).
    /// Re-architected codebase required due to missing WORD_STORE gadget,
    /// 0x820 setter offset, altered stack frame, and reorganized vtable.
    V24_26,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootloaderInfo {
    pub kind: BootloaderKind,
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashStatus {
    Supported(BootloaderKind, String),
    Unsupported,
    Unknown,
}

pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in result {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

pub struct BootloaderEntry {
    pub hash: &'static str,
    pub kind: BootloaderKind,
    pub name: &'static str,
}

pub const KNOWN_BOOTLOADERS: &[BootloaderEntry] = &[
    // v20_24 family
    BootloaderEntry { hash: "049dc91219bd1e514c1ddc0444e33a81717d3f964f2e049417a13ff641f5393a", kind: BootloaderKind::V20_24, name: "20.4.0" },
    BootloaderEntry { hash: "b5226d678a5c9bfeff70f4f0583b61f7726b14fd98d97477ac95cd504bb9b905", kind: BootloaderKind::V20_24, name: "21.0.0" },
    BootloaderEntry { hash: "2d2240c1ea2b442ef4f9a25a5058e70c9374703180916449726abd7ae572719c", kind: BootloaderKind::V20_24, name: "21.1.0 - 21.3.0" },
    BootloaderEntry { hash: "c105723aea7857ad047669f97fcf3b04890a29e60acda32b9e3ad37ca8fc694e", kind: BootloaderKind::V20_24, name: "22.0.0" },
    BootloaderEntry { hash: "0528d82283775e39ac7bb2b7cdd441b86cbecfcd3100852d5ba94bde31afe369", kind: BootloaderKind::V20_24, name: "22.1.0 & 22.2.0" },
    BootloaderEntry { hash: "29078bc4b92f417ac272bcbbca86774b297e3ebd529f4895a24089e0ef957585", kind: BootloaderKind::V20_24, name: "22.2.1 - 23.1.0" },
    BootloaderEntry { hash: "adde23139cd75abd37712dbdcfc35be19dd88b32359903997d08326d2932a650", kind: BootloaderKind::V20_24, name: "23.2.0" },
    BootloaderEntry { hash: "1e6a5e151890a9d3ba026faf4a0f353e234fffa6d64bc761bfa95eef73b61730", kind: BootloaderKind::V20_24, name: "23.2.1 - 23.2.3" },
    BootloaderEntry { hash: "9f05a826da9bdf710281059762d9c4b420577666658b94f6546aba6573dcda74", kind: BootloaderKind::V20_24, name: "23.2.4 - 23.2.6" },
    BootloaderEntry { hash: "e16bbf2439ca3d3e0e1b7eb3516a106409bd877a730fc318b8d3f17f1164703c", kind: BootloaderKind::V20_24, name: "24.0.1" },
    BootloaderEntry { hash: "2941afa1fe02cd333aeff271ba2f8d34c30d663116a72f4b67937745c268ccc2", kind: BootloaderKind::V20_24, name: "24.1.0" },

    // v24_26 family
    BootloaderEntry { hash: "6e709dc4c2244ed22f903aec016d4cd5a06d389051d724135ff04b779f0bdd15", kind: BootloaderKind::V24_26, name: "24.2.0" },
    BootloaderEntry { hash: "136c56ff5dbd6a1174f41a2364e5c4b1aea362befd761f68d68e15996aecbf79", kind: BootloaderKind::V24_26, name: "24.2.0 (alt)" },
    BootloaderEntry { hash: "5632830ef638b01c8c7f0981f4d54953d7acc39698a67e67a26aa1c377e5ef32", kind: BootloaderKind::V24_26, name: "24.2.2" },
    BootloaderEntry { hash: "03bc3f87693eeb43ce2f95929af5ffdc52d280137e0009f93059b6ef0635e438", kind: BootloaderKind::V24_26, name: "24.3.0 - 24.11.0" },
    BootloaderEntry { hash: "bf1af7e562bc627b2252b081034bc1061c0ace6b449e05903c46724ce36202f2", kind: BootloaderKind::V24_26, name: "25.1.0 & 25.2.0" },
    BootloaderEntry { hash: "29cfbf9c169d013b4df7d64c78d205aac4f7f6e93abbd31a5bc8fb127a6fa1ac", kind: BootloaderKind::V24_26, name: "26.1.0 & 26.3.0" },
];

pub const UNSUPPORTED_HASHES: &[&str] = &[
    "bce415e9f16c1678760f8734be7faa449719ef9add16b80f3cefade0d02beda8", // 26.3.0 with both patches in the patches/ folder applied
    "1f92196dd933b9807db954b06bc5b1a2d1150e4f98033bb36cfce0cbb2508ffb", // Patched stock bootloader
];

/// Checks a SHA-256 hash against the hardcoded bootloader signatures.
pub fn check_hash(hash: &str) -> HashStatus {
    for &unsupported in UNSUPPORTED_HASHES {
        if hash.eq_ignore_ascii_case(unsupported) {
            return HashStatus::Unsupported;
        }
    }
    for entry in KNOWN_BOOTLOADERS {
        if entry.hash.eq_ignore_ascii_case(hash) {
            return HashStatus::Supported(entry.kind, entry.name.to_string());
        }
    }
    HashStatus::Unknown
}

/// Checks the flash data hash (and 0xFF-padded hash if < 512 KiB) against hardcoded signatures.
pub fn check_flash(flash_data: &[u8]) -> HashStatus {
    let hash = compute_sha256(flash_data);
    let status = check_hash(&hash);
    if status != HashStatus::Unknown {
        return status;
    }

    if flash_data.len() < 0x80000 {
        let mut padded = flash_data.to_vec();
        padded.resize(0x80000, 0xFF);
        let padded_hash = compute_sha256(&padded);
        let padded_status = check_hash(&padded_hash);
        if padded_status != HashStatus::Unknown {
            return padded_status;
        }
    }

    HashStatus::Unknown
}

pub fn resolve_bootloader(flash_data: &[u8]) -> Result<BootloaderInfo> {
    let hash = compute_sha256(flash_data);
    match check_flash(flash_data) {
        HashStatus::Unsupported => {
            bail!(
                "{}: {} (SHA-256: {}). {}",
                "Target bootloader is UNSUPPORTED",
                "Patched",
                hash,
                "This bootloader version has already been patched."
            );
        }
        HashStatus::Supported(kind, name) => Ok(BootloaderInfo {
            kind,
            name,
            hash,
        }),
        HashStatus::Unknown => {
            bail!(
                "{}: {} (SHA-256: {})",
                "Unrecognized bootloader binary",
                "Unknown",
                hash
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256() {
        // SHA-256 of empty bytes
        assert_eq!(
            compute_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // SHA-256 of "wirehax"
        assert_eq!(
            compute_sha256(b"wirehax"),
            "c0f2dfd3a43fe403709607a56f276e2a651c398bfb2c736a1e70da56ac9847b7"
        );
    }

    #[test]
    fn test_bootloader_types() {
        let info = BootloaderInfo {
            kind: BootloaderKind::V20_24,
            name: "v20_24".to_string(),
            hash: "abc".to_string(),
        };
        assert_eq!(info.kind, BootloaderKind::V20_24);
        assert_eq!(info.name, "v20_24");
        assert_eq!(info.clone(), info);

        let status = HashStatus::Supported(BootloaderKind::V24_26, "v24_26".to_string());
        assert_eq!(status, status.clone());
    }

    #[test]
    fn test_check_hash_known_supported() {
        assert_eq!(
            check_hash("049dc91219bd1e514c1ddc0444e33a81717d3f964f2e049417a13ff641f5393a"),
            HashStatus::Supported(BootloaderKind::V20_24, "20.4.0".to_string())
        );
        assert_eq!(
            check_hash("6e709dc4c2244ed22f903aec016d4cd5a06d389051d724135ff04b779f0bdd15"),
            HashStatus::Supported(BootloaderKind::V24_26, "24.2.0".to_string())
        );
    }

    #[test]
    fn test_check_hash_unsupported() {
        assert_eq!(
            check_hash("bce415e9f16c1678760f8734be7faa449719ef9add16b80f3cefade0d02beda8"),
            HashStatus::Unsupported
        );
        assert_eq!(
            check_hash("1f92196dd933b9807db954b06bc5b1a2d1150e4f98033bb36cfce0cbb2508ffb"),
            HashStatus::Unsupported
        );
    }

    #[test]
    fn test_check_hash_unknown() {
        assert_eq!(
            check_hash("0000000000000000000000000000000000000000000000000000000000000000"),
            HashStatus::Unknown
        );
    }

    #[test]
    fn test_resolve_bootloader() {
        // Supported entry
        let dummy_20_4_0 = vec![0u8; 100];
        // Test with arbitrary unknown data
        assert!(resolve_bootloader(&dummy_20_4_0).is_err());
    }
}

