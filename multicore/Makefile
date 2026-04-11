KERNEL_ELF := target/riscv64gc-unknown-none-elf/release/kernel
KERNEL_BIN := $(KERNEL_ELF).bin
USER_DIR := user
APPS_DIR := user/src/bin
TARGET_DIR := user/target/riscv64gc-unknown-none-elf/release

QEMU := qemu-system-riscv64
SMP ?= 4
MEM ?= 128M

QEMU_ARGS := -machine virt \
	-nographic \
	-smp $(SMP) \
	-m $(MEM) \
	-bios default \
	-kernel $(KERNEL_ELF)

OBJCOPY := rust-objcopy --binary-architecture=riscv64

# Collect user app names
APP_NAMES := $(patsubst $(APPS_DIR)/%.rs,%,$(wildcard $(APPS_DIR)/*.rs))

.PHONY: all kernel user clean run debug gdb experiments

all: kernel

user:
	@echo "Building user programs..."
	@cd $(USER_DIR) && cargo build --release

kernel: user
	@echo "Building kernel..."
	@cd kernel && cargo build --release

$(KERNEL_BIN): kernel
	$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $@

run: kernel
	$(QEMU) $(QEMU_ARGS)

debug: kernel
	$(QEMU) $(QEMU_ARGS) -s -S

gdb:
	riscv64-unknown-elf-gdb -ex 'file $(KERNEL_ELF)' -ex 'target remote :1234'

clean:
	cd kernel && cargo clean
	cd $(USER_DIR) && cargo clean
	rm -rf user/build

# Run with different core counts for testing
run1: SMP=1
run1: run

run2: SMP=2
run2: run

run4: SMP=4
run4: run

experiments:
	python3 run_experiments.py
