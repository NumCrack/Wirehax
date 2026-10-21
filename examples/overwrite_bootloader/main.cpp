#include "display.hpp"

// Embedded stock bootloader binary
extern "C" {
    extern const uint8_t boot_bin_start[];
    extern const uint8_t boot_bin_end[];
}

// STM32H7 Flash register interface (Bank 1)
constexpr uintptr_t FLASH_BASE = 0x52002000;

volatile uint32_t* const FLASH_KEYR      = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x04);
volatile uint32_t* const FLASH_OPTKEYR   = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x08);
volatile uint32_t* const FLASH_CR        = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x0C);
volatile uint32_t* const FLASH_SR        = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x10);
volatile uint32_t* const FLASH_CCR       = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x14);
volatile uint32_t* const FLASH_OPTCR     = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x18);
volatile uint32_t* const FLASH_OPTSR_CUR = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x1C);
volatile uint32_t* const FLASH_OPTCCR    = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x24);
volatile uint32_t* const FLASH_WPSN_CUR  = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x38);
volatile uint32_t* const FLASH_WPSN_PRG  = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x3C);
volatile uint32_t* const FLASH_BOOT_CUR  = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x40);
volatile uint32_t* const FLASH_BOOT_PRG  = reinterpret_cast<volatile uint32_t*>(FLASH_BASE + 0x44);

constexpr uint32_t ROM_BOOT_ADDRESS = 0x1FF0;
constexpr uint32_t ROM_BOOT_ADDRESSES = ROM_BOOT_ADDRESS | (ROM_BOOT_ADDRESS << 16);

// Flash unlock keys
constexpr uint32_t FLASH_KEY1 = 0x45670123;
constexpr uint32_t FLASH_KEY2 = 0xCDEF89AB;

// Option bytes unlock keys
constexpr uint32_t OPTKEY1 = 0x08192A3B;
constexpr uint32_t OPTKEY2 = 0x4C5D6E7F;

// FLASH_CR bits
constexpr uint32_t FLASH_CR_LOCK  = (1 << 0);
constexpr uint32_t FLASH_CR_PG    = (1 << 1);
constexpr uint32_t FLASH_CR_SER   = (1 << 2);
constexpr uint32_t FLASH_CR_START = (1 << 7);

// FLASH_OPTCR bits
constexpr uint32_t OPT_LOCK             = (1 << 0);
constexpr uint32_t OPT_START            = (1 << 1);  // OPTSTART: writes PRG → NVM
constexpr uint32_t OBL_LAUNCH           = (1 << 4);  // OBL_LAUNCH: reloads option bytes from NVM
constexpr uint32_t OPT_OPTCHANGEERR     = (1 << 30);
constexpr uint32_t OPT_CLR_OPTCHANGEERR = (1 << 30);

// FLASH_SR bits
constexpr uint32_t FLASH_SR_BSY                  = (1 << 0);
[[maybe_unused]] constexpr uint32_t FLASH_SR_WBNE = (1 << 1);
constexpr uint32_t FLASH_SR_QW                   = (1 << 2);
constexpr uint32_t FLASH_SR_WRPERR  = (1 << 17);
constexpr uint32_t FLASH_SR_PGSERR  = (1 << 18);
constexpr uint32_t FLASH_SR_STRBERR = (1 << 19);
constexpr uint32_t FLASH_SR_INCERR  = (1 << 21);
constexpr uint32_t FLASH_SR_OPERR   = (1 << 22);

constexpr uint32_t FLASH_ERRORS = FLASH_SR_WRPERR | FLASH_SR_PGSERR | FLASH_SR_STRBERR |
                                  FLASH_SR_INCERR | FLASH_SR_OPERR | FLASH_SR_BSY | FLASH_SR_QW;



static void hex32(uint32_t val, char* out) {
    const char hexDigits[] = "0123456789ABCDEF";
    out[0] = '0';
    out[1] = 'x';
    for (int i = 7; i >= 0; --i) {
        out[2 + (7 - i)] = hexDigits[(val >> (i * 4)) & 0xF];
    }
    out[10] = '\0';
}

static void delay_ms(uint32_t ms) {
    volatile uint32_t* const SYST_CSR = reinterpret_cast<volatile uint32_t*>(0xE000E010);
    volatile uint32_t* const SYST_RVR = reinterpret_cast<volatile uint32_t*>(0xE000E014);
    volatile uint32_t* const SYST_CVR = reinterpret_cast<volatile uint32_t*>(0xE000E018);

    // 480,000 ticks = 1ms at 480 MHz (STM32H7 core clock)
    *SYST_RVR = 480000 - 1;
    *SYST_CVR = 0;
    *SYST_CSR = 0x05; // ENABLE | CLKSOURCE

    for (uint32_t i = 0; i < ms; ++i) {
        while ((*SYST_CSR & (1 << 16)) == 0) {
            __asm__ volatile ("nop");
        }
    }
    *SYST_CSR = 0;
}


struct WrpPrgResult {
    bool cr_unlocked;
    bool opt_unlocked;
    bool timeout;
    bool change_err;
    uint32_t optcr_val;
    uint32_t optsr_val;
    uint32_t flash_sr_val;
    uint32_t prg_after;
    uint32_t cur_after;      // WPSN_CUR after OBL_LAUNCH completes
    uint32_t boot_after;
    uint32_t boot_prg_after;
};

static WrpPrgResult program_recovery_options(uint32_t bootAddresses, uint32_t wpsnProgram) {
    WrpPrgResult res = {};

    // Clear pending errors before unlocking the flash and option-byte registers.
    *FLASH_CCR  = 0x0FFE0000;  // clear all Bank 1 error/status bits (EOP, CRCEND, errors)
    *FLASH_OPTCCR = OPT_CLR_OPTCHANGEERR;
    __asm__ volatile ("dsb sy\nisb");

    // 2. Unlock FLASH_CR (required by RM0468 before OPTKEYR can be written)
    if (*FLASH_CR & FLASH_CR_LOCK) {
        *FLASH_KEYR = FLASH_KEY1;
        __asm__ volatile ("dsb sy\nisb");
        *FLASH_KEYR = FLASH_KEY2;
        __asm__ volatile ("dsb sy\nisb");
    }
    res.cr_unlocked = ((*FLASH_CR & FLASH_CR_LOCK) == 0);
    if (!res.cr_unlocked) {
        return res;
    }

    // 3. Unlock FLASH_OPTCR
    if (*FLASH_OPTCR & OPT_LOCK) {
        *FLASH_OPTKEYR = OPTKEY1;
        __asm__ volatile ("dsb sy\nisb");
        *FLASH_OPTKEYR = OPTKEY2;
        __asm__ volatile ("dsb sy\nisb");
    }
    res.optcr_val = *FLASH_OPTCR;
    res.opt_unlocked = ((res.optcr_val & OPT_LOCK) == 0);
    if (!res.opt_unlocked) {
        *FLASH_CR |= FLASH_CR_LOCK;
        return res;
    }

    // Keep Bank 1 writable and select the requested BOOT_ADD0/BOOT_ADD1 values.
    *FLASH_WPSN_PRG = wpsnProgram;
    *FLASH_BOOT_PRG = bootAddresses;
    __asm__ volatile ("dsb sy\nisb");

    // Commit PRG → NVM via OPTSTART, then reload NVM → CUR via OBL_LAUNCH.
    *FLASH_OPTCCR = OPT_CLR_OPTCHANGEERR;
    *FLASH_CCR    = 0x0FFE0000;
    __asm__ volatile ("dsb sy\nisb");

    *FLASH_OPTCR |= OPT_START;
    __asm__ volatile ("dsb sy\nisb");

    // Wait for OPTSTART to finish writing option bytes.
    uint32_t iters = 0;
    while ((*FLASH_OPTCR & OPT_START) && iters < 5000000u) {
        __asm__ volatile ("nop");
        ++iters;
    }
    if (iters >= 5000000u) {
        res.timeout = true;
        *FLASH_OPTCR |= OPT_LOCK;
        *FLASH_CR    |= FLASH_CR_LOCK;
        return res;
    }

    // Reload the committed option bytes into the current registers.
    *FLASH_OPTCR |= OBL_LAUNCH;
    __asm__ volatile ("dsb sy\nisb");

    uint32_t oblIters = 0;
    while ((*FLASH_OPTCR & OBL_LAUNCH) && oblIters < 5000000u) {
        __asm__ volatile ("nop");
        ++oblIters;
    }
    if (oblIters >= 5000000u) {
        res.timeout = true;
        *FLASH_OPTCR |= OPT_LOCK;
        *FLASH_CR    |= FLASH_CR_LOCK;
        return res;
    }

    // Verify the write protection and boot addresses before touching flash.
    res.cur_after    = *FLASH_WPSN_CUR;
    res.optsr_val    = *FLASH_OPTSR_CUR;
    res.flash_sr_val = *FLASH_SR;
    res.prg_after    = *FLASH_WPSN_PRG;
    res.boot_after   = *FLASH_BOOT_CUR;
    res.boot_prg_after = *FLASH_BOOT_PRG;

    if (res.optsr_val & OPT_OPTCHANGEERR) {
        res.change_err = true;
    }

    // Re-lock option bytes and flash.
    *FLASH_OPTCR |= OPT_LOCK;
    *FLASH_CR    |= FLASH_CR_LOCK;

    return res;
}

[[noreturn]] static void reboot_to_rom_dfu() {
    volatile uint32_t* const SCB_AIRCR = reinterpret_cast<volatile uint32_t*>(0xE000ED0C);
    __asm__ volatile ("cpsid i\ndsb sy");
    *SCB_AIRCR = 0x05FA0004; // VECTKEY | SYSRESETREQ
    __asm__ volatile ("dsb sy");
    while (true) { __asm__ volatile ("wfi"); }
}

static bool flash_unlock() {
    if ((*FLASH_CR & FLASH_CR_LOCK) == 0) {
        return true;
    }
    *FLASH_KEYR = FLASH_KEY1;
    __asm__ volatile ("dsb sy\nisb");
    *FLASH_KEYR = FLASH_KEY2;
    __asm__ volatile ("dsb sy\nisb");
    return ((*FLASH_CR & FLASH_CR_LOCK) == 0);
}

static void flash_lock() {
    *FLASH_CR |= FLASH_CR_LOCK;
}

static void flash_clear_errors() {
    *FLASH_CCR = 0x0FFE0000;  // clear all Bank 1 error/status bits
    __asm__ volatile ("dsb sy\nisb");
}

static uint32_t flash_wait_busy() {
    int timeout = 5000000;
    while ((*FLASH_SR & (FLASH_SR_BSY | FLASH_SR_QW)) && --timeout > 0) {
        __asm__ volatile ("nop");
    }
    return *FLASH_SR;
}

static uint32_t flash_wait_erase(uint32_t timeout_ms = 10000) {
    while ((*FLASH_SR & (FLASH_SR_BSY | FLASH_SR_QW)) && timeout_ms > 0) {
        delay_ms(1);
        --timeout_ms;
    }
    return *FLASH_SR;
}

static uint32_t flash_erase_sector(uint32_t sector) {
    flash_clear_errors();

    *FLASH_CR = FLASH_CR_SER | (sector << 8);
    __asm__ volatile ("dsb sy\nisb");

    *FLASH_CR |= FLASH_CR_START;
    __asm__ volatile ("dsb sy\nisb");

    uint32_t sr = flash_wait_erase(10000);

    *FLASH_CR &= ~FLASH_CR_SER;
    __asm__ volatile ("dsb sy\nisb");

    return sr;
}

static uint32_t flash_program_word256(uint32_t dest, const uint32_t* words8) {
    flash_clear_errors();

    *FLASH_CR |= FLASH_CR_PG;
    __asm__ volatile ("dsb sy\nisb");

    volatile uint32_t* pDest = reinterpret_cast<volatile uint32_t*>(dest);
    for (int i = 0; i < 8; ++i) {
        pDest[i] = words8[i];
    }
    __asm__ volatile ("dsb sy\nisb");

    flash_wait_busy();

    *FLASH_CR &= ~FLASH_CR_PG;
    __asm__ volatile ("dsb sy\nisb");

    return *FLASH_SR;
}

extern "C" int main() {
    Display display;
    display.init();
    display.clear(Color::Background);

    // Title Bar
    display.fillRect(0, 0, Display::WIDTH, 28, Color::Surface);
    display.fillRect(0, 28, Display::WIDTH, 2, Color::NumWorksYellow);
    display.drawString(10, 10, "NumWorks N0120 | Bootloader Overwrite", Color::NumWorksYellow, Color::Surface);

    const int boxX = 10;
    const int boxY = 38;
    const int boxW = Display::WIDTH - 20;
    const int boxH = 168;

    display.fillRect(boxX, boxY, boxW, boxH, Color::Surface);
    display.drawRect(boxX, boxY, boxW, boxH, Color::SurfaceBorder);

    char buf[16];

    // Read current option byte state
    uint32_t wpsnCur = *FLASH_WPSN_CUR;
    uint32_t wpsnPrg = *FLASH_WPSN_PRG;
    uint32_t originalBootAddresses = *FLASH_BOOT_CUR;

    display.drawString(boxX + 10, boxY + 10, "HARDWARE STATUS:", Color::Cyan, Color::Surface);
    display.drawString(boxX + 10, boxY + 28, "WRP0_CUR : ", Color::TextDim, Color::Surface);
    hex32(wpsnCur, buf);
    display.drawString(boxX + 100, boxY + 28, buf, (wpsnCur & 1) ? Color::Green : Color::Red, Color::Surface);

    display.drawString(boxX + 10, boxY + 44, "WRP0_PRG : ", Color::TextDim, Color::Surface);
    hex32(wpsnPrg, buf);
    display.drawString(boxX + 100, boxY + 44, buf, (wpsnPrg & 1) ? Color::Green : Color::Red, Color::Surface);

    // =========================================================================
    // PHASE 1: Arm ROM recovery before any internal flash erase.
    // =========================================================================
    display.drawString(boxX + 10, boxY + 62, "PHASE 1: Arming ROM DFU fallback...", Color::NumWorksYellow, Color::Surface);

    uint32_t wpsnProgram = (wpsnCur & 1) ? (wpsnCur & 0xFF) : 0xFF;
    WrpPrgResult prgRes = program_recovery_options(ROM_BOOT_ADDRESSES, wpsnProgram);
    bool fallbackArmed = prgRes.cr_unlocked && prgRes.opt_unlocked && !prgRes.timeout &&
                         !prgRes.change_err && (prgRes.prg_after & 0xFF) == wpsnProgram &&
                         (prgRes.cur_after & 0xFF) == wpsnProgram &&
                         prgRes.boot_after == ROM_BOOT_ADDRESSES &&
                         prgRes.boot_prg_after == ROM_BOOT_ADDRESSES;

    if (!fallbackArmed) {
        display.drawString(boxX + 10, boxY + 82, "FAILED: ROM fallback not confirmed!", Color::Red, Color::Surface);
        display.drawString(boxX + 10, boxY + 102, "OPTCR : ", Color::TextDim, Color::Surface);
        hex32(prgRes.optcr_val, buf);
        display.drawString(boxX + 80, boxY + 102, buf, Color::TextLight, Color::Surface);
        display.drawString(boxX + 10, boxY + 118, "OPTSR : ", Color::TextDim, Color::Surface);
        hex32(prgRes.optsr_val, buf);
        display.drawString(boxX + 80, boxY + 118, buf, Color::TextLight, Color::Surface);
        display.drawString(boxX + 10, boxY + 134, "BOOT  : ", Color::TextDim, Color::Surface);
        hex32(prgRes.boot_after, buf);
        display.drawString(boxX + 80, boxY + 134, buf, Color::TextLight, Color::Surface);
        display.drawString(boxX + 10, boxY + 150, "SR    : ", Color::TextDim, Color::Surface);
        hex32(prgRes.flash_sr_val, buf);
        display.drawString(boxX + 80, boxY + 150, buf, Color::TextLight, Color::Surface);
        while (true) { __asm__ volatile ("wfi"); }
    }

    display.drawString(boxX + 10, boxY + 76, "ROM fallback armed for both BOOT states.", Color::Green, Color::Surface);
    display.drawString(boxX + 10, boxY + 92, "WRP_CUR:", Color::TextDim, Color::Surface);
    hex32(prgRes.cur_after, buf);
    display.drawString(boxX + 80, boxY + 92, buf, Color::Green, Color::Surface);
    display.drawString(boxX + 10, boxY + 108, "BOOT_CUR:", Color::TextDim, Color::Surface);
    hex32(prgRes.boot_after, buf);
    display.drawString(boxX + 80, boxY + 108, buf, Color::Green, Color::Surface);
    display.drawString(boxX + 10, boxY + 124, "Power loss now still boots STM32 ROM.", Color::TextLight, Color::Surface);
    display.drawString(boxX + 10, boxY + 142, "Starting overwrite...", Color::Green, Color::Surface);
    delay_ms(500);

    // =========================================================================
    // PHASE 2: ROM fallback is armed; proceed to overwrite internal flash.
    // =========================================================================
    // Clear the info box so Phase 1 diagnostics don't overlay Phase 2 output.
    display.fillRect(boxX, boxY, boxW, boxH, Color::Surface);
    display.drawRect(boxX, boxY, boxW, boxH, Color::SurfaceBorder);

    display.drawString(boxX + 10, boxY + 10, "PHASE 2: ROM fallback armed...", Color::Green, Color::Surface);

    // Wait for the flash controller to be fully idle after OBL_LAUNCH.
    // BSY and QW can remain set for a short time after the option-byte reload;
    // attempting an erase or program while either is set causes a silent rejection.
    {
        uint32_t idleIters = 0;
        while ((*FLASH_SR & (FLASH_SR_BSY | FLASH_SR_QW)) && idleIters < 5000000u) {
            __asm__ volatile ("nop");
            ++idleIters;
        }
        // Clear any stale errors left over from the option-byte sequence.
        *FLASH_CCR  = 0x0FFE0000;
        *FLASH_OPTCCR = OPT_CLR_OPTCHANGEERR;
        __asm__ volatile ("dsb sy\nisb");
    }

    // 1. Unlock Flash Interface
    if (!flash_unlock()) {
        display.drawString(boxX + 10, boxY + 28, "FLASH_CR : UNLOCK FAILED!", Color::Red, Color::Surface);
        display.drawString(boxX + 10, boxY + 46, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
        delay_ms(500);
        reboot_to_rom_dfu();
    }

    // 2. Determine which 128 KB sectors contain non-0xFF data and erase them
    constexpr uint32_t SECTOR_SIZE = 128 * 1024;
    const uint32_t totalImageSize = static_cast<uint32_t>(boot_bin_end - boot_bin_start);

    bool sectorNeeded[4] = { false, false, false, false };
    for (uint32_t s = 0; s < 4; ++s) {
        uint32_t secStart = s * SECTOR_SIZE;
        uint32_t secEnd = (s + 1) * SECTOR_SIZE;
        if (secStart >= totalImageSize) break;
        if (secEnd > totalImageSize) secEnd = totalImageSize;
        for (uint32_t i = secStart; i < secEnd; ++i) {
            if (boot_bin_start[i] != 0xFF) {
                sectorNeeded[s] = true;
                break;
            }
        }
    }

    for (uint32_t s = 0; s < 4; ++s) {
        if (!sectorNeeded[s]) continue;

        char msg[48] = "ERASE Sector X ...            ";
        msg[13] = static_cast<char>('0' + s);
        display.drawString(boxX + 10, boxY + 28, msg, Color::NumWorksYellow, Color::Surface);

        uint32_t eraseSr = flash_erase_sector(s);
        hex32(eraseSr, buf);

        if (eraseSr & (FLASH_SR_BSY | FLASH_SR_QW)) {
            char failMsg[48] = "ERASE Sector X : TIMEOUT!";
            failMsg[13] = static_cast<char>('0' + s);
            display.drawString(boxX + 10, boxY + 28, failMsg, Color::Red, Color::Surface);
            display.drawString(boxX + 10, boxY + 46, "FLASH_SR : ", Color::TextDim, Color::Surface);
            display.drawString(boxX + 100, boxY + 46, buf, Color::Red, Color::Surface);
            flash_lock();
            display.drawString(boxX + 10, boxY + 64, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
            delay_ms(500);
            reboot_to_rom_dfu();
        } else if (eraseSr & FLASH_SR_WRPERR) {
            char failMsg[48] = "ERASE Sector X : FAILED (WRPERR)";
            failMsg[13] = static_cast<char>('0' + s);
            display.drawString(boxX + 10, boxY + 28, failMsg, Color::Red, Color::Surface);
            display.drawString(boxX + 10, boxY + 46, "SR Error : ", Color::TextDim, Color::Surface);
            display.drawString(boxX + 100, boxY + 46, buf, Color::Red, Color::Surface);
            flash_lock();
            display.drawString(boxX + 10, boxY + 64, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
            delay_ms(500);
            reboot_to_rom_dfu();
        } else if (eraseSr & FLASH_ERRORS) {
            char failMsg[48] = "ERASE Sector X : FAILED (ERROR)";
            failMsg[13] = static_cast<char>('0' + s);
            display.drawString(boxX + 10, boxY + 28, failMsg, Color::Red, Color::Surface);
            display.drawString(boxX + 10, boxY + 46, "FLASH_SR : ", Color::TextDim, Color::Surface);
            display.drawString(boxX + 100, boxY + 46, buf, Color::Red, Color::Surface);
            flash_lock();
            display.drawString(boxX + 10, boxY + 64, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
            delay_ms(500);
            reboot_to_rom_dfu();
        }
    }

    display.drawString(boxX + 10, boxY + 28, "ERASE Sectors : OK                ", Color::Green, Color::Surface);

    // 3. Program Flash with boot.bin
    display.drawString(boxX + 10, boxY + 46, "PROGRAM boot.bin ...              ", Color::NumWorksYellow, Color::Surface);

    // Progress Bar
    const int barX = boxX + 10;
    const int barY = boxY + 64;
    const int barW = boxW - 20;
    const int barH = 12;
    display.drawRect(barX, barY, barW, barH, Color::SurfaceBorder);

    // Count non-blank 32-byte blocks across the binary
    uint32_t nonBlankBlocks = 0;
    for (uint32_t offset = 0; offset < totalImageSize; offset += 32) {
        bool blockHasData = false;
        for (uint32_t b = 0; b < 32 && (offset + b) < totalImageSize; ++b) {
            if (boot_bin_start[offset + b] != 0xFF) {
                blockHasData = true;
                break;
            }
        }
        if (blockHasData) {
            ++nonBlankBlocks;
        }
    }

    uint32_t writeSr = 0;
    bool writeFailed = false;
    uint32_t failedAddr = 0;
    int lastFillW = -1;
    uint32_t blocksWritten = 0;

    for (uint32_t offset = 0; offset < totalImageSize; offset += 32) {
        uint32_t blockWords[8];
        bool blockHasData = false;
        for (int w = 0; w < 8; ++w) {
            uint32_t wordOffset = offset + w * 4;
            if (wordOffset + 4 <= totalImageSize) {
                blockWords[w] = *reinterpret_cast<const uint32_t*>(boot_bin_start + wordOffset);
            } else if (wordOffset < totalImageSize) {
                uint32_t val = 0xFFFFFFFF;
                uint8_t* pVal = reinterpret_cast<uint8_t*>(&val);
                for (uint32_t b = 0; b < totalImageSize - wordOffset; ++b) {
                    pVal[b] = boot_bin_start[wordOffset + b];
                }
                blockWords[w] = val;
            } else {
                blockWords[w] = 0xFFFFFFFF;
            }
            if (blockWords[w] != 0xFFFFFFFF) {
                blockHasData = true;
            }
        }

        if (!blockHasData) {
            continue; // Skip blank blocks (already erased to 0xFF)
        }

        writeSr = flash_program_word256(0x08000000 + offset, blockWords);
        if (writeSr & FLASH_ERRORS) {
            writeFailed = true;
            failedAddr = 0x08000000 + offset;
            break;
        }

        ++blocksWritten;
        if (nonBlankBlocks > 0) {
            int fillW = (blocksWritten * (barW - 2)) / nonBlankBlocks;
            if (fillW < 1) fillW = 1;
            if (fillW != lastFillW) {
                display.fillRect(barX + 1, barY + 1, fillW, barH - 2, Color::NumWorksYellow);
                lastFillW = fillW;
            }
        }
    }

    if (writeFailed) {
        display.fillRect(barX + 1, barY + 1, barW - 2, barH - 2, Color::Red);
        display.drawString(boxX + 10, boxY + 46, "PROGRAM boot.bin : FAILED!        ", Color::Red, Color::Surface);
        display.drawString(boxX + 10, boxY + 82, "Failed at: ", Color::TextDim, Color::Surface);
        hex32(failedAddr, buf);
        display.drawString(boxX + 100, boxY + 82, buf, Color::Red, Color::Surface);
        flash_lock();
        display.drawString(boxX + 10, boxY + 100, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
        delay_ms(500);
        reboot_to_rom_dfu();
    }

    // Complete Progress Bar
    display.fillRect(barX + 1, barY + 1, barW - 2, barH - 2, Color::Green);
    display.drawString(boxX + 10, boxY + 46, "PROGRAM boot.bin : SUCCESS        ", Color::Green, Color::Surface);

    // 4. Readback Verification
    display.drawString(boxX + 10, boxY + 82, "VERIFY   : Checking Flash against image...", Color::Cyan, Color::Surface);

    bool verifyOk = true;
    uint32_t mismatchAddr = 0;
    const uint8_t* pFlash = reinterpret_cast<const uint8_t*>(0x08000000);

    for (uint32_t i = 0; i < totalImageSize; ++i) {
        uint32_t s = i / SECTOR_SIZE;
        if (s < 4 && !sectorNeeded[s]) {
            continue;
        }
        if (pFlash[i] != boot_bin_start[i]) {
            verifyOk = false;
            mismatchAddr = 0x08000000 + i;
            break;
        }
    }

    flash_lock();

    if (!verifyOk) {
        display.drawString(boxX + 10, boxY + 82, "VERIFY   : MISMATCH!                   ", Color::Red, Color::Surface);
        hex32(mismatchAddr, buf);
        display.drawString(boxX + 10, boxY + 100, "Address  : ", Color::TextDim, Color::Surface);
        display.drawString(boxX + 100, boxY + 100, buf, Color::Red, Color::Surface);
        display.drawString(boxX + 10, boxY + 118, "Rebooting into ROM DFU...", Color::NumWorksYellow, Color::Surface);
        delay_ms(500);
        reboot_to_rom_dfu();
    }

    display.drawString(boxX + 10, boxY + 82, "VERIFY   : 100% MATCH! Zero Errors.     ", Color::Green, Color::Surface);
    display.drawString(boxX + 10, boxY + 100, "Restoring saved boot configuration...", Color::NumWorksYellow, Color::Surface);
    WrpPrgResult restoreRes = program_recovery_options(originalBootAddresses, wpsnProgram);
    bool restoreOk = restoreRes.cr_unlocked && restoreRes.opt_unlocked && !restoreRes.timeout &&
                     !restoreRes.change_err && (restoreRes.prg_after & 0xFF) == wpsnProgram &&
                     (restoreRes.cur_after & 0xFF) == wpsnProgram &&
                     restoreRes.boot_after == originalBootAddresses &&
                     restoreRes.boot_prg_after == originalBootAddresses;
    if (!restoreOk) {
        display.drawString(boxX + 10, boxY + 118, "Restore failed; forcing ROM DFU...", Color::Red, Color::Surface);
        (void)program_recovery_options(ROM_BOOT_ADDRESSES, wpsnProgram);
        delay_ms(500);
        reboot_to_rom_dfu();
    }

    display.drawString(boxX + 10, boxY + 118, "Boot configuration restored.", Color::Green, Color::Surface);
    display.drawString(10, 218, "Press rear RESET to reboot into the rewritten bootloader.", Color::TextDim, Color::Background);

    while (true) {
        __asm__ volatile ("wfi");
    }

    return 0;
}
