AS = arm-none-eabi-as
OBJCOPY = arm-none-eabi-objcopy
ASFLAGS = -mcpu=cortex-m7 -mthumb

TARGET = xip-chainloader
ELF = $(TARGET).elf
BIN = $(TARGET).bin
SRC = $(TARGET).s

.PHONY: all clean

all: $(BIN)

$(ELF): $(SRC)
	$(AS) $(ASFLAGS) $< -o $@

$(BIN): $(ELF)
	$(OBJCOPY) -O binary $< $@

clean:
	rm -f $(ELF) $(BIN)
