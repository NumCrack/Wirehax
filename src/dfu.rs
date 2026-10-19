use anyhow::{bail, Context, Result};
use rusb::{DeviceHandle, GlobalContext};
use std::io::Write;
use std::time::Duration;

pub const VID: u16 = 0x0483;
pub const PID: u16 = 0xA291;
pub const TIMEOUT: Duration = Duration::from_millis(4000);
pub const TRANSFER: usize = 2048;

pub fn open_device() -> Result<DeviceHandle<GlobalContext>> {
    let handle = rusb::open_device_with_vid_pid(VID, PID).ok_or_else(|| {
        anyhow::anyhow!(
            "Rescue DFU device (0483:a291) not found.\n\n\
            Troubleshooting:\n  \
            1. Ensure the calculator is connected with a data-capable USB-C cable.\n  \
            2. Put the calculator into Rescue DFU mode:\n     \
               - Hold key '6'\n     \
               - Press and release the rear RESET button\n     \
               - Release key '6'\n  \
            3. On Windows: If connected but not recognized, ensure the WinUSB driver is installed\n     \
               for 'STM32 BOOTLOADER' (0483:a291) using Zadig (https://zadig.akeo.ie/).\n  \
            4. On Linux: Check 'lsusb | grep 0483:a291' to verify the device is detected by the OS."
        )
    })?;

    // Try detach kernel driver if active (Linux)
    let _ = handle.set_auto_detach_kernel_driver(true);

    if let Ok(active_config) = handle.active_configuration() {
        if active_config == 0 {
            let _ = handle.set_active_configuration(1);
        }
    } else {
        let _ = handle.set_active_configuration(1);
    }

    handle.claim_interface(0).map_err(|e| {
        anyhow::anyhow!(
            "USB access to rescue DFU device (0483:a291) was denied: {e}\n\n\
            Troubleshooting:\n  \
            - On Linux: Non-root USB access requires udev rules. Run:\n      \
                sudo cp 50-numworks.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger\n    \
                (Alternatively, re-run with 'sudo').\n  \
            - On Windows: Ensure the WinUSB driver is assigned to device 0483:a291 using Zadig (https://zadig.akeo.ie/)."
        )
    })?;

    if handle.write_control(0x01, 11, 1, 0, &[], TIMEOUT).is_err() {
        let mut raw = [0u8; 6];
        handle.read_control(0xA1, 3, 0, 0, &mut raw, TIMEOUT).map_err(|e| {
            anyhow::anyhow!(
                "USB device is not responding as rescue DFU ({e})"
            )
        })?;
        if raw[0] != 0 {
            bail!(
                "Rescue DFU reported an error state ({})",
                raw[0]
            );
        }
    }

    idle(&handle).map_err(|e| {
        anyhow::anyhow!(
            "Rescue DFU is not ready for a transfer ({e}). Try pressing the rear RESET button and re-entering rescue DFU (Hold '6' + RESET)."
        )
    })?;

    Ok(handle)
}

pub fn status(handle: &DeviceHandle<GlobalContext>) -> Result<u8> {
    let mut raw = [0u8; 6];
    handle.read_control(0xA1, 3, 0, 0, &mut raw, TIMEOUT)
        .with_context(|| "DFU GETSTATUS transfer failed".to_string())?;
    if raw[0] != 0 {
        bail!("DFU status error: {}", raw[0]);
    }
    let delay_ms = raw[1] as u64 | ((raw[2] as u64) << 8) | ((raw[3] as u64) << 16);
    if delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
    Ok(raw[4])
}

pub fn ready(handle: &DeviceHandle<GlobalContext>) -> Result<u8> {
    for _ in 0..12 {
        let state = status(handle)?;
        if state != 3 && state != 4 {
            if state != 2 && state != 5 && state != 9 {
                bail!("unexpected DFU state: {state}");
            }
            return Ok(state);
        }
    }
    bail!("DFU stayed busy")
}

pub fn idle(handle: &DeviceHandle<GlobalContext>) -> Result<()> {
    let mut state = status(handle)?;
    if state == 3 || state == 4 {
        state = ready(handle)?;
    }
    if state == 5 || state == 9 {
        handle.write_control(0x21, 6, 0, 0, &[], TIMEOUT)
            .with_context(|| "DFU ABORT transfer failed".to_string())?;
        state = ready(handle)?;
    }
    if state != 2 {
        bail!("DFU is not idle (state {state})");
    }
    Ok(())
}

pub fn download(handle: &DeviceHandle<GlobalContext>, block: u16, data: &[u8]) -> Result<u8> {
    handle.write_control(0x21, 1, block, 0, data, TIMEOUT)
        .with_context(|| "DFU DNLOAD transfer failed".to_string())?;
    ready(handle)
}

pub fn set_address(handle: &DeviceHandle<GlobalContext>, address: u32) -> Result<()> {
    idle(handle)?;
    let mut cmd = [0u8; 5];
    cmd[0] = 0x21;
    cmd[1..5].copy_from_slice(&address.to_le_bytes());
    download(handle, 0, &cmd)?;
    Ok(())
}

pub fn abort(handle: &DeviceHandle<GlobalContext>) -> Result<()> {
    handle.write_control(0x21, 6, 0, 0, &[], TIMEOUT)
        .with_context(|| "DFU ABORT transfer failed".to_string())?;
    ready(handle)?;
    Ok(())
}

pub fn write_memory(handle: &DeviceHandle<GlobalContext>, address: u32, data: &[u8]) -> Result<()> {
    for (i, chunk) in data.chunks(TRANSFER).enumerate() {
        let offset = (i * TRANSFER) as u32;
        set_address(handle, address + offset)?;
        download(handle, 2, chunk)?;
    }
    Ok(())
}

pub fn read_memory(
    handle: &DeviceHandle<GlobalContext>,
    address: u32,
    size: usize,
) -> Result<Vec<u8>> {
    set_address(handle, address)?;
    abort(handle)?;
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let amount = std::cmp::min(TRANSFER, size - result.len());
        let block = (2 + result.len() / TRANSFER) as u16;
        let mut chunk = vec![0u8; amount];
        let bytes_read = handle.read_control(0xA1, 2, block, 0, &mut chunk, TIMEOUT)
            .with_context(|| "DFU UPLOAD transfer failed".to_string())?;
        result.extend_from_slice(&chunk[..bytes_read]);
    }
    abort(handle)?;
    Ok(result)
}

pub fn dump_flash_data(
    handle: &DeviceHandle<GlobalContext>,
    address: u32,
    size: usize,
    alt: u16,
    progress: bool,
) -> Result<Vec<u8>> {
    handle.write_control(0x01, 11, alt, 0, &[], TIMEOUT)
        .with_context(|| "Failed to set alternate setting for flash dump".to_string())?;
    idle(handle)?;

    set_address(handle, address)?;
    abort(handle)?;

    let mut data = Vec::with_capacity(size);
    let t0 = std::time::Instant::now();
    while data.len() < size {
        let chunk_size = std::cmp::min(TRANSFER, size - data.len());
        let block = (2 + data.len() / TRANSFER) as u16;
        let mut chunk = vec![0u8; chunk_size];
        let bytes_read = handle.read_control(0xA1, 2, block, 0, &mut chunk, TIMEOUT)
            .with_context(|| "DFU UPLOAD chunk failed".to_string())?;
        data.extend_from_slice(&chunk[..bytes_read]);

        if progress && (data.len() % (32 * 1024) == 0 || data.len() == size) {
            let pct = (data.len() as f64 / size as f64) * 100.0;
            let elapsed = t0.elapsed().as_secs_f64().max(0.001);
            let speed = (data.len() as f64 / 1024.0) / elapsed;
            print!(
                "\rDumping flash: {:6} / {:6} bytes ({:5.1}%) [{:6.1} KiB/s]",
                data.len(),
                size,
                pct,
                speed
            );
            let _ = std::io::stdout().flush();
        }
    }
    abort(handle)?;
    if progress {
        println!();
    }
    Ok(data)
}
