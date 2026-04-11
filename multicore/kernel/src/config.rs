pub const MAX_HARTS: usize = 4;
pub const MEMORY_END: usize = 0x8800_0000; // 128MB
pub const KERNEL_HEAP_SIZE: usize = 0xC0_0000; // 12MB
pub const KERNEL_STACK_SIZE: usize = 0x1_0000; // 64KB per hart
pub const USER_STACK_SIZE: usize = 0x4000; // 16KB
pub const PAGE_SIZE: usize = 0x1000; // 4KB
pub const PAGE_SIZE_BITS: usize = 12;
#[allow(dead_code)]
pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
#[allow(dead_code)]
pub const TRAP_CONTEXT_BASE: usize = TRAMPOLINE - PAGE_SIZE;
pub const CLOCK_FREQ: usize = 10_000_000; // QEMU virt: 10MHz
pub const TICK_MS: usize = 10; // timer interrupt every 10ms
pub const TICKS_PER_SEC: usize = 1000 / TICK_MS;

// QEMU virt MMIO addresses
/// Thread exit trampoline: mapped at a fixed low VA in every process.
/// Contains: li a7,93; li a0,0; ecall  (i.e. exit(0))
pub const THREAD_EXIT_TRAMPOLINE: usize = 0x1000;
pub const UART_BASE: usize = 0x1000_0000;
pub const PLIC_BASE: usize = 0x0C00_0000;
pub const VIRTIO0_BASE: usize = 0x1000_1000;

// PLIC constants
pub const PLIC_PRIORITY: usize = PLIC_BASE;
#[allow(dead_code)]
pub const PLIC_PENDING: usize = PLIC_BASE + 0x1000;

pub const fn plic_senable(hart: usize) -> usize {
    PLIC_BASE + 0x2080 + hart * 0x100
}

pub const fn plic_spriority(hart: usize) -> usize {
    PLIC_BASE + 0x201000 + hart * 0x2000
}

pub const fn plic_sclaim(hart: usize) -> usize {
    PLIC_BASE + 0x201004 + hart * 0x2000
}

// UART IRQ number on QEMU virt
pub const UART_IRQ: u32 = 10;
pub const VIRTIO0_IRQ: u32 = 1;
