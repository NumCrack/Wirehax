#include <stdint.h>
#include <stddef.h>

extern "C" {

// Linker script symbols
extern uint32_t _estack;
extern uint32_t _sidata;
extern uint32_t _sdata;
extern uint32_t _edata;
extern uint32_t _sbss;
extern uint32_t _ebss;

typedef void (*init_fn_t)(void);
extern init_fn_t __init_array_start[];
extern init_fn_t __init_array_end[];

int main(void);
void Reset_Handler(void);
void HardFault_Handler(void);
void Default_Handler(void);
void init_c_runtime(void);

// Weak aliases for Cortex-M7 system exceptions
void NMI_Handler(void)        __attribute__((weak, alias("Default_Handler")));
void MemManage_Handler(void)  __attribute__((weak, alias("HardFault_Handler")));
void BusFault_Handler(void)   __attribute__((weak, alias("HardFault_Handler")));
void UsageFault_Handler(void) __attribute__((weak, alias("HardFault_Handler")));
void SVC_Handler(void)        __attribute__((weak, alias("Default_Handler")));
void PendSV_Handler(void)     __attribute__((weak, alias("Default_Handler")));
void SysTick_Handler(void)    __attribute__((weak, alias("Default_Handler")));

// Entry vector table placed at 0x90400020 for xip-chainloader
__attribute__((section(".entry_vector"), used))
void* const g_entry_vector[] = {
    &_estack,                // 0x90400020: Initial Main Stack Pointer (MSP)
    (void*)Reset_Handler     // 0x90400024: Reset Handler
};

// Aligned vector table for Cortex-M7 VTOR
__attribute__((section(".isr_vector"), used))
void* const g_vector_table[] = {
    &_estack,                // Initial Main Stack Pointer (MSP)
    (void*)Reset_Handler,    // Reset Handler
    (void*)NMI_Handler,      // NMI
    (void*)HardFault_Handler,// Hard Fault
    (void*)MemManage_Handler,// MemManage Fault
    (void*)BusFault_Handler, // Bus Fault
    (void*)UsageFault_Handler,// Usage Fault
    nullptr,                 // Reserved
    nullptr,                 // Reserved
    nullptr,                 // Reserved
    nullptr,                 // Reserved
    (void*)SVC_Handler,      // SVCall
    nullptr,                 // Debug Monitor
    nullptr,                 // Reserved
    (void*)PendSV_Handler,   // PendSV
    (void*)SysTick_Handler   // SysTick
};

__attribute__((naked)) void Reset_Handler(void) {
    __asm__ volatile (
        // 1. Immediately disable interrupts so DFU bootloader interrupts (USB/SysTick) don't fire
        "cpsid i\n"

        // 2. Repoint VTOR to our aligned vector table
        "ldr r0, =0xE000ED08\n"
        "ldr r1, =g_vector_table\n"
        "str r1, [r0]\n"

        // 3. Disable SysTick timer left running by DFU bootloader
        "ldr r0, =0xE000E010\n"
        "movs r1, #0\n"
        "str r1, [r0]\n"

        // 4. Disable all NVIC external interrupt lines
        "ldr r0, =0xE000E180\n" // NVIC_ICER0
        "mov.w r1, #0xFFFFFFFF\n"
        "str r1, [r0]\n"
        "str r1, [r0, #4]\n"
        "str r1, [r0, #8]\n"
        "str r1, [r0, #12]\n"

        // Clear all pending NVIC interrupt flags
        "ldr r0, =0xE000E280\n" // NVIC_ICPR0
        "str r1, [r0]\n"
        "str r1, [r0, #4]\n"
        "str r1, [r0, #8]\n"
        "str r1, [r0, #12]\n"

        // 5. Configure MPU Region 7 for Internal Flash (0x08000000 - 0x08100000)
        // Ensure Sector 0 has RW access and is Non-Cacheable (bypasses D-Cache)
        "ldr r0, =0xE000ED98\n" // MPU_RNR
        "movs r1, #7\n"
        "str r1, [r0]\n"
        "ldr r0, =0xE000ED9C\n" // MPU_RBAR
        "ldr r1, =0x08000000\n"
        "str r1, [r0]\n"
        "ldr r0, =0xE000EDA0\n" // MPU_RASR
        "ldr r1, =0x03080027\n" // Full Access (RW), Normal Non-cacheable, 1MB, Enable
        "str r1, [r0]\n"

        // 6. Update MPU_CTRL: Keep MPU enabled (Region 6 XIP) and enable PRIVDEFENA (bit 2)
        // CRITICAL: DO NOT set MPU_CTRL to 0; doing so disables Region 6 and crashes external flash XIP!
        "ldr r0, =0xE000ED94\n" // MPU_CTRL
        "ldr r1, [r0]\n"
        "orr r1, r1, #0x05\n"   // ENABLE (bit 0) | PRIVDEFENA (bit 2)
        "str r1, [r0]\n"
        "dsb sy\n"
        "isb sy\n"

        // 6. Ensure stack pointer is initialized to _estack
        "ldr r0, =_estack\n"
        "msr msp, r0\n"

        // 7. Proceed to C runtime initialization
        "b init_c_runtime\n"
    );
}

void init_c_runtime(void) {
    // Copy .data from flash to SRAM
    uint32_t* pSrc = &_sidata;
    uint32_t* pDst = &_sdata;
    while (pDst < &_edata) {
        *pDst++ = *pSrc++;
    }

    // Zero out .bss in SRAM
    pDst = &_sbss;
    while (pDst < &_ebss) {
        *pDst++ = 0;
    }

    // Call static C++ constructors (.init_array)
    for (init_fn_t* pInit = __init_array_start; pInit < __init_array_end; ++pInit) {
        (*pInit)();
    }

    // Run user application
    main();

    // Halt if main returns
    while (1) {
        __asm__ volatile ("wfi");
    }
}

// If a fault occurs, paint the screen RED so we can immediately diagnose faults
__attribute__((naked)) void HardFault_Handler(void) {
    __asm__ volatile (
        "cpsid i\n"
        "ldr r0, =0xE000ED94\n"
        "ldr r1, [r0]\n"
        "orr r1, r1, #0x05\n"
        "str r1, [r0]\n"
        "dsb sy\n"
        "isb sy\n"

        "ldr r1, =0x60000000\n" // Command port
        "ldr r2, =0x60020000\n" // Data port

        // CASET (0x2A) -> 0..319
        "movs r0, #0x2A\n"
        "strh r0, [r1]\n"
        "movs r0, #0\n"
        "strh r0, [r2]\n"
        "strh r0, [r2]\n"
        "movs r0, #1\n"
        "strh r0, [r2]\n"
        "movs r0, #0x3F\n"
        "strh r0, [r2]\n"

        // RASET (0x2B) -> 0..239
        "movs r0, #0x2B\n"
        "strh r0, [r1]\n"
        "movs r0, #0\n"
        "strh r0, [r2]\n"
        "strh r0, [r2]\n"
        "movs r0, #0\n"
        "strh r0, [r2]\n"
        "movs r0, #0xEF\n"
        "strh r0, [r2]\n"

        // RAMWR (0x2C)
        "movs r0, #0x2C\n"
        "strh r0, [r1]\n"

        // Fill with RED (0xF800)
        "mov.w r0, #0xF800\n"
        "mov.w r3, #76800\n"
        "1:\n"
        "strh r0, [r2]\n"
        "subs r3, r3, #1\n"
        "bne 1b\n"

        "2:\n"
        "wfi\n"
        "b 2b\n"
    );
}

void Default_Handler(void) {
    while (1) {
        __asm__ volatile ("wfi");
    }
}

// Minimal standard library stubs for freestanding C++
void* memset(void* dest, int c, size_t n) {
    uint8_t* p = reinterpret_cast<uint8_t*>(dest);
    while (n--) {
        *p++ = static_cast<uint8_t>(c);
    }
    return dest;
}

void* memcpy(void* dest, const void* src, size_t n) {
    uint8_t* d = reinterpret_cast<uint8_t*>(dest);
    const uint8_t* s = reinterpret_cast<const uint8_t*>(src);
    while (n--) {
        *d++ = *s++;
    }
    return dest;
}

void __cxa_pure_virtual(void) {
    while (1) {}
}

} // extern "C"
