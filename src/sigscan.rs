use anyhow::{Context, Result, bail};

pub const FLASH_BASE: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbols {
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

impl Symbols {
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

pub fn parse_pattern(ida_sig: &str) -> Result<Vec<Option<u8>>> {
    let mut pattern = Vec::new();
    for token in ida_sig.split_whitespace() {
        if token == "?" || token == "??" {
            pattern.push(None);
        } else {
            let byte = u8::from_str_radix(token, 16).map_err(|e| {
                anyhow::anyhow!("Invalid hex token '{token}': {e}")
            })?;
            pattern.push(Some(byte));
        }
    }
    Ok(pattern)
}

pub fn find_pattern(data: &[u8], pattern: &[Option<u8>], align: usize) -> Option<usize> {
    if pattern.is_empty() || data.len() < pattern.len() {
        return None;
    }
    let max_start = data.len() - pattern.len();
    'outer: for i in (0..=max_start).step_by(align) {
        for (j, &pat) in pattern.iter().enumerate() {
            if let Some(expected) = pat
                && data[i + j] != expected {
                    continue 'outer;
                }
        }
        return Some(i);
    }
    None
}

pub fn find_signature(data: &[u8], ida_sig: &str, thumb: bool, base_address: u32) -> Result<u32> {
    let pattern = parse_pattern(ida_sig)?;
    let align = if thumb { 2 } else { 1 };
    let offset = find_pattern(data, &pattern, align).ok_or_else(|| {
        anyhow::anyhow!("Signature match failed for pattern: {ida_sig}")
    })?;
    let addr = base_address
        .checked_add(offset as u32)
        .with_context(|| "Base address + offset overflowed u32".to_string())?;
    Ok(if thumb { addr | 1 } else { addr })
}

pub fn find_vtable_offset(data: &[u8], stock_callback: u32, stock_setter: u32) -> Result<usize> {
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
            return Ok(pos - 8);
        }
    }

    bail!("Could not find stock vtable reference pointers in firmware");
}

pub fn find_vtable(
    data: &[u8],
    stock_callback: u32,
    stock_setter: u32,
    vtable_size: usize,
) -> Result<Vec<u8>> {
    let start = find_vtable_offset(data, stock_callback, stock_setter)?;
    if start + vtable_size > data.len() {
        bail!("vtable exceeds firmware size: start=0x{start:x}, size=0x{vtable_size:x}, data_len=0x{:x}", data.len());
    }
    Ok(data[start..start + vtable_size].to_vec())
}

pub fn resolve_symbols(flash_bytes: &[u8], base_address: u32) -> Result<Symbols> {
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
        "80 F8 24 18 70 47",
        true,
        base_address,
    )?;
    let word_store = find_signature(flash_bytes, "02 60 70 47", true, base_address)?;
    let initial_callback = find_signature(
        flash_bytes,
        "F8 B5 1C 46 0B 78 15 46 BD F8 18 60 13 F0 60 0F",
        true,
        base_address,
    )?;
    let stock_callback = find_signature(
        flash_bytes,
        "F0 B5 04 46 08 78 87 B0 1D 46 10 F0 60 0F",
        true,
        base_address,
    )?;

    Ok(Symbols {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_matching() {
        let dummy = vec![0x00, 0x11, 0x08, 0xBD, 0x22];
        let addr = find_signature(&dummy, "08 BD", true, 0x08000000).unwrap();
        assert_eq!(addr, 0x08000002 | 1);

        // Without thumb bit
        let addr_no_thumb = find_signature(&dummy, "08 BD", false, 0x08000000).unwrap();
        assert_eq!(addr_no_thumb, 0x08000002);

        // Pattern not found
        assert!(find_signature(&dummy, "FF FF", true, 0x08000000).is_err());
    }

    #[test]
    fn test_signature_matching_wildcards() {
        let dummy = vec![0xF8, 0xB5, 0xAA, 0x46, 0x0B, 0x78];
        let addr = find_signature(&dummy, "F8 B5 ? 46 0B ??", false, 0x20000000).unwrap();
        assert_eq!(addr, 0x20000000);

        let non_match = vec![0xF8, 0xB5, 0xAA, 0x47, 0x0B, 0x78];
        assert!(find_signature(&non_match, "F8 B5 ? 46 0B ??", false, 0x20000000).is_err());
    }

    #[test]
    fn test_parse_pattern() {
        let pat = parse_pattern("08 BD ? ?? 46").unwrap();
        assert_eq!(
            pat,
            vec![Some(0x08), Some(0xBD), None, None, Some(0x46)]
        );

        // Invalid hex token
        assert!(parse_pattern("08 ZZ").is_err());
    }

    #[test]
    fn test_firmware_symbols() {
        let fw_path = std::path::Path::new("firmware/internal-flash-live.bin");
        if fw_path.exists() {
            let data = std::fs::read(fw_path).unwrap();
            let sym = resolve_symbols(&data, FLASH_BASE).unwrap();
            assert_eq!(sym.pop_r3_pc, 0x08000f73);
            assert_eq!(sym.pop_r0_r1_r2_r4_r6_r7_pc, 0x0800a3af);
            assert_eq!(sym.bx_r3, 0x08008fcf);
            assert_eq!(sym.memmove, 0x080045cf);
            assert_eq!(sym.pivot, 0x08007f59);
            assert_eq!(sym.nested_pivot, 0x08007779);
            assert_eq!(sym.stock_setter, 0x08002651);
            assert_eq!(sym.word_store, 0x080026bd);
            assert_eq!(sym.initial_callback, 0x08005125);
            assert_eq!(sym.stock_callback, 0x08004c19);
        }
    }
}
