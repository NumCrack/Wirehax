.syntax unified
.thumb
.global _start

_start:
  cpsid i
  ldr r3, marker_addr
  ldr r2, marker_value
  str r2, [r3]

  /* Take over MPU configuration while this code is running from SRAM. */
  ldr r3, mpu_ctrl_addr
  movs r2, #0
  str r2, [r3]
  dsb sy

  ldr r3, mpu_rnr_addr
  movs r2, #6
  str r2, [r3]
  ldr r3, mpu_rbar_addr
  ldr r2, mpu_rbar_value
  str r2, [r3]
  ldr r3, mpu_rasr_addr
  ldr r2, mpu_rasr_value
  str r2, [r3]
  ldr r3, mpu_ctrl_addr
  movs r2, #1
  str r2, [r3]
  dsb sy
  isb

  dsb sy
  isb

  cpsie i
  isb

  ldr r0, vector_addr
  ldr r1, [r0]
  ldr r2, [r0, #4]
  msr MSP, r1
  bx r2

1:
  b 1b

.align 2
marker_addr:
  .word 0x240101f0
marker_value:
  .word 0x58495021
mpu_ctrl_addr:
  .word 0xE000ED94
mpu_rnr_addr:
  .word 0xE000ED98
mpu_rbar_addr:
  .word 0xE000ED9C
mpu_rbar_value:
  .word 0x90000000
mpu_rasr_addr:
  .word 0xE000EDA0
mpu_rasr_value:
  .word 0x0102002D
vector_addr:
  .word 0x90400020
