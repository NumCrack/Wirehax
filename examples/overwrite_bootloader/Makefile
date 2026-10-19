CXX = clang++
CC = clang
OBJCOPY = llvm-objcopy
DFU_UTIL = dfu-util

BUILD_DIR ?= build
BOOT_BIN ?= ../../firmware/bootloader.bin

ifeq ($(filter clean,$(MAKECMDGOALS)),)
  ifeq ($(wildcard $(BOOT_BIN)),)
    $(error Target bootloader binary not found at '$(BOOT_BIN)'. Please dump your bootloader or specify: make BOOT_BIN=/path/to/bootloader.bin)
  endif
endif

TARGET = slot_b
ELF = $(BUILD_DIR)/$(TARGET).elf
BIN = $(BUILD_DIR)/$(TARGET).bin

SRCS_CPP = startup.cpp display.cpp main.cpp
OBJS_CPP = $(patsubst %.cpp,$(BUILD_DIR)/%.o,$(SRCS_CPP))
ASM_CPP  = $(patsubst %.cpp,$(BUILD_DIR)/%.s,$(SRCS_CPP))

BOOT_ASM = $(BUILD_DIR)/boot_image.s
BOOT_OBJ = $(BUILD_DIR)/boot_image.o

OBJS = $(OBJS_CPP) $(BOOT_OBJ)
ASMS = $(ASM_CPP) $(BOOT_ASM)

CXXFLAGS = -target arm-none-eabi \
           -mcpu=cortex-m7 \
           -mthumb \
           -mfloat-abi=soft \
           -O2 \
           -std=c++17 \
           -ffreestanding \
           -fno-exceptions \
           -fno-rtti \
           -fno-unwind-tables \
           -Wall \
           -Wextra

ASFLAGS = -target arm-none-eabi \
          -mcpu=cortex-m7 \
          -mthumb \
          -I $(dir $(BOOT_BIN))

LDFLAGS = -fuse-ld=lld \
          -nostdlib \
          -T linker.ld \
          -Wl,--gc-sections

.PHONY: all clean flash boot check

.SECONDARY:

all: $(BIN)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Generate intermediate assembly (.s) from C++ sources into BUILD_DIR
$(BUILD_DIR)/%.s: %.cpp | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) -S $< -o $@

# Assemble intermediate .s to .o in BUILD_DIR
$(BUILD_DIR)/%.o: $(BUILD_DIR)/%.s | $(BUILD_DIR)
	$(CC) $(ASFLAGS) -c $< -o $@

# Generate intermediate boot_image.s embedding the bootloader into BUILD_DIR
$(BOOT_ASM): $(BOOT_BIN) | $(BUILD_DIR)
	@echo '.syntax unified' > $@
	@echo '.section .rodata' >> $@
	@echo '.global boot_bin_start, boot_bin_end' >> $@
	@echo '.align 4' >> $@
	@echo 'boot_bin_start:' >> $@
	@echo '.incbin "$(notdir $(BOOT_BIN))"' >> $@
	@echo 'boot_bin_end:' >> $@

$(BOOT_OBJ): $(BOOT_ASM) | $(BUILD_DIR)
	$(CC) $(ASFLAGS) -c $< -o $@

$(ELF): $(OBJS) linker.ld | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) $(LDFLAGS) $(OBJS) -o $@

$(BIN): $(ELF)
	$(OBJCOPY) -O binary $< $@
	@echo "Built $(BIN) (size: $$(stat -c %s $(BIN)) bytes)"

clean:
	rm -rf $(BUILD_DIR)

# Check the generated slot header
check: $(BIN)
	../../target/release/wirehax version -f $(BIN)

# Flash Slot B to external flash (calculator must be in rescue DFU: hold 6 + press RESET)
flash: $(BIN)
	$(DFU_UTIL) -a 0 -s 0x90400000:force -D $(BIN)

# Boot the flashed Slot B using WireHax RCE
boot:
	../../target/release/wirehax
