use anyhow::{Result, bail};
use std::path::PathBuf;

use wirehax::boot::{boot, self_check};
use wirehax::dfu::open_device;
use wirehax::version::{DEFAULT_SUPPORTED, check_file, check_live_device};

fn parse_hex_or_dec_u32(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(stripped, 16)
            .map_err(|e| anyhow::anyhow!("invalid hex number: {e}"))
    } else {
        s.parse::<u32>()
            .map_err(|e| anyhow::anyhow!("invalid number: {e}"))
    }
}

fn parse_hex_or_dec_usize(s: &str) -> Result<usize> {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(stripped, 16)
            .map_err(|e| anyhow::anyhow!("invalid hex number: {e}"))
    } else {
        s.parse::<usize>()
            .map_err(|e| anyhow::anyhow!("invalid number: {e}"))
    }
}

fn print_help() {
    println!(
        "Usage: wirehax [OPTIONS] [COMMAND]\n\n\
        Commands:\n  \
          boot        Boot modified Slot B image (default)\n  \
          self-check  Verify bundled assets and ROP layout\n  \
          dump        Dump internal flash memory over USB DFU\n  \
          version     Check NumWorks N0120 Epsilon OS version\n\n\
        Options:\n      \
          --self-check           Verify bundled assets and ROP layout\n      \
          --firmware <PATH>      Path to custom internal flash dump\n      \
          --save-dump            Save live flash dump\n      \
          -h, --help             Print help"
    );
}

fn main() -> Result<()> {
    print!(
        "\n     ________  __               __\n    \
        |  |  |  ||__|.----..-----.|  |--..---.-..--.--.\n    \
        |  |  |  ||  ||   _||  -__||     ||  _  ||_   _|\n    \
        |________||__||__|  |_____||__|__||___._||__.__|\n\n"
    );

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        let handle = open_device()?;
        return boot(&handle, None, false);
    }

    let first = &args[0];
    if first == "-h" || first == "--help" || first == "help" {
        print_help();
        return Ok(());
    }

    if first == "-V" || first == "--version" {
        println!("wirehax 0.1.0");
        return Ok(());
    }

    // Subcommand: self-check
    if first == "self-check" {
        let mut firmware = None;
        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            if (arg == "--firmware" || arg == "-f") && i + 1 < args.len() {
                firmware = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            } else if arg == "-h" || arg == "--help" {
                println!("Usage: wirehax self-check [-f <firmware>]");
                return Ok(());
            }
            i += 1;
        }
        return self_check(firmware.as_deref());
    }

    // Subcommand: dump
    if first == "dump" {
        let mut output = PathBuf::from("firmware/internal-flash-live.bin");
        let mut size = 0x80000usize;
        let mut address = 0x08000000u32;
        let mut alt = 0u16;

        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            if (arg == "-o" || arg == "--output") && i + 1 < args.len() {
                output = PathBuf::from(&args[i + 1]);
                i += 1;
            } else if (arg == "-s" || arg == "--size") && i + 1 < args.len() {
                size = parse_hex_or_dec_usize(&args[i + 1])?;
                i += 1;
            } else if (arg == "-a" || arg == "--address") && i + 1 < args.len() {
                address = parse_hex_or_dec_u32(&args[i + 1])?;
                i += 1;
            } else if (arg == "-A" || arg == "--alt") && i + 1 < args.len() {
                alt = args[i + 1].parse::<u16>()?;
                i += 1;
            } else if arg == "-h" || arg == "--help" {
                println!(
                    "Usage: wirehax dump [-o <output>] [-s <size>] [-a <address>] [-A <alt>]"
                );
                return Ok(());
            }
            i += 1;
        }

        println!(
            "Connecting to rescue DFU device (0483:a291)..."
        );
        let handle = open_device()?;
        let data = wirehax::dfu::dump_flash_data(&handle, address, size, alt, true)?;
        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&output, &data)?;
        println!(
            "Saved: {} bytes to {}",
            data.len(),
            output.display()
        );
        let hash = wirehax::boot::compute_sha256(&data);
        println!("Flash dump SHA-256: {hash}");
        match wirehax::boot::resolve_bootloader(&data) {
            Ok(bl) => println!(
                "  Detected bootloader: {} -> [SUPPORTED]",
                bl.name
            ),
            Err(e) => println!("  Bootloader status: {e}"),
        }
        return Ok(());
    }

    // Subcommand: version
    if first == "version" {
        let mut file = None;
        let mut slot = "all".to_string();
        let mut supported: Option<String> = None;

        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            if (arg == "-f" || arg == "--file") && i + 1 < args.len() {
                file = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            } else if arg == "--slot" && i + 1 < args.len() {
                slot = args[i + 1].clone();
                i += 1;
            } else if arg == "--supported" && i + 1 < args.len() {
                supported = Some(args[i + 1].clone());
                i += 1;
            } else if arg == "-h" || arg == "--help" {
                println!(
                    "Usage: wirehax version [-f <file>] [--slot <slot>] [--supported <versions>]"
                );
                return Ok(());
            }
            i += 1;
        }

        let supported_list: Vec<&str> = supported
            .as_deref()
            .map(|s| {
                s.split(',')
                    .map(|item| item.trim())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let supported_slice = if supported_list.is_empty() {
            DEFAULT_SUPPORTED
        } else {
            &supported_list
        };

        if let Some(file_path) = file {
            let ok = check_file(&file_path, supported_slice)?;
            if !ok {
                std::process::exit(1);
            }
        } else {
            println!(
                "Connecting to rescue DFU device (0483:a291)..."
            );
            let handle = open_device()?;
            let ok = check_live_device(&handle, supported_slice, &slot)?;
            if !ok {
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // Subcommand: boot or top-level flags
    let mut is_self_check = false;
    let mut firmware = None;
    let mut save_dump = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "boot" {
            // default command
        } else if arg == "--self-check" {
            is_self_check = true;
        } else if (arg == "--firmware" || arg == "-f") && i + 1 < args.len() {
            firmware = Some(PathBuf::from(&args[i + 1]));
            i += 1;
        } else if arg == "--save-dump" {
            save_dump = true;
        } else if arg == "-h" || arg == "--help" {
            print_help();
            return Ok(());
        } else {
            bail!("Unknown argument: {arg}");
        }
        i += 1;
    }

    if is_self_check {
        self_check(firmware.as_deref())
    } else {
        let handle = open_device()?;
        boot(&handle, firmware.as_deref(), save_dump)
    }
}
