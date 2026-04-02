//! # 第五章：进程
//!
//! 本章在第四章"地址空间"的基础上，引入了完整的 **进程管理** 机制。
//! 进程是操作系统中最核心的抽象之一，它将"运行中的程序"封装为一个可管理的实体。
//!
//! ## 核心概念
//!
//! - **进程控制块（PCB）**：管理进程的所有资源（PID、地址空间、上下文、堆空间）
//! - **进程创建**：`fork` 系统调用复制父进程的地址空间，创建子进程
//! - **程序替换**：`exec` 系统调用加载新的 ELF 程序替换当前进程的地址空间
//! - **进程等待**：`wait` 系统调用等待子进程退出并回收资源
//! - **进程退出**：`exit` 系统调用终止当前进程
//! - **进程标识**：每个进程有唯一的 PID
//! - **进程树**：通过父子关系形成树状结构
//! - **初始进程（initproc）**：内核创建的第一个用户进程，是所有用户进程的祖先
//!
//! ## 与第四章的区别
//!
//! | 特性 | 第四章 | 第五章 |
//! |------|--------|--------|
//! | 进程创建 | 内核直接从 ELF 加载 | fork/exec 组合创建 |
//! | 进程关系 | 无父子关系 | 完整的进程树 |
//! | 资源回收 | 内核直接回收 | wait 系统调用回收 |
//! | 用户交互 | 无 | Shell 命令行界面 |
//! | 进程管理 | 简单列表 | ProcManager + 调度器 |
//!
//! 教程阅读建议：
//!
//! - 先看 `rust_main`：建立“initproc -> 调度 -> trap -> 状态迁移”的全局主线；
//! - 再看 `map_portal` 与 `kernel_space`：理解进程切换时地址空间一致性保障；
//! - 最后看 `impls::Process`：重点理解 fork/exec/wait 的语义边界。

// 不使用标准库，裸机环境没有操作系统提供系统调用支持
#![no_std]
// 不使用默认的 main 函数入口，裸机环境需要自定义入口点
#![no_main]
// 在 RISC-V 架构上启用严格的编译警告和文档要求
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 在非 RISC-V 架构上允许未使用的代码（用于 IDE 开发体验）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code, unused_imports))]

/// 进程模块：定义 Process 结构体及其方法（from_elf、fork、exec 等）
mod process;
/// 处理器模块：定义 PROCESSOR 全局变量和进程管理器 ProcManager
mod processor;

#[macro_use]
extern crate tg_console;

extern crate alloc;

use crate::{
    impls::{Console, Sv39Manager, SyscallContext},
    process::Process,
    processor::{PROCESSOR, ProcManager},
};
use alloc::{alloc::alloc, collections::BTreeMap};
use core::{alloc::Layout, cell::UnsafeCell, ffi::CStr, mem::MaybeUninit, ptr::NonNull};
use riscv::register::*;
use spin::Lazy;
#[cfg(not(target_arch = "riscv64"))]
use stub::Sv39;
use tg_console::log;
use tg_kernel_context::foreign::MultislotPortal;
#[cfg(target_arch = "riscv64")]
use tg_kernel_vm::page_table::Sv39;
use tg_kernel_vm::{
    AddressSpace,
    page_table::{MmuMeta, PPN, VAddr, VPN, VmFlags, VmMeta},
};
use tg_sbi;
use tg_syscall::Caller;
use tg_task_manage::{PManager, ProcId};
use virtio_drivers::{DeviceType, Hal, MmioTransport, Transport, VirtIOGpu, VirtIOHeader};
use xmas_elf::ElfFile;

/// 构建 VmFlags（虚拟内存标志位）。
///
/// 在 RISC-V 架构上使用编译期构建，参数格式如 `"U_WRV"` 表示：
/// - U: 用户态可访问
/// - W: 可写
/// - R: 可读
/// - V: 有效位
#[cfg(target_arch = "riscv64")]
const fn build_flags(s: &str) -> VmFlags<Sv39> {
    VmFlags::build_from_str(s)
}

/// 运行时解析 VmFlags 字符串。
#[cfg(target_arch = "riscv64")]
fn parse_flags(s: &str) -> Result<VmFlags<Sv39>, ()> {
    s.parse()
}

#[cfg(not(target_arch = "riscv64"))]
use stub::{build_flags, parse_flags};

// ─── 内联用户应用程序 ───
// build.rs 会编译用户程序并生成 APP_ASM 汇编文件，
// 通过 global_asm! 将所有用户程序的二进制数据嵌入内核镜像
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!(env!("APP_ASM")));

// 定义内核入口点，设置启动栈大小为 32 页 = 128 KiB。
//
// 这里不再调用 tg_linker::boot0! 宏，避免外部已发布版本与 Rust 2024
// 在属性语义上的兼容差异影响本 crate 的发布校验。
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 32 * 4096;
    #[unsafe(link_section = ".boot.stack")]
    static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}",
        "j  {main}",
        stack = sym STACK,
        stack_size = const STACK_SIZE,
        main = sym rust_main,
    )
}

/// 物理内存容量 = 48 MiB
const MEMORY: usize = 48 << 20;

/// 异界传送门所在虚页（虚拟地址空间最高页）
///
/// 传送门是一段特殊的代码页面，同时映射到内核和所有用户地址空间的相同虚拟地址，
/// 用于解决切换 satp（页表基地址）时代码地址失效的问题。
const PROTAL_TRANSIT: VPN<Sv39> = VPN::MAX;

/// 内核地址空间的全局存储
///
/// 使用 UnsafeCell + MaybeUninit 实现延迟初始化，
/// 因为内核地址空间需要在堆分配器初始化之后才能创建。
struct KernelSpace {
    inner: UnsafeCell<MaybeUninit<AddressSpace<Sv39, Sv39Manager>>>,
}

unsafe impl Sync for KernelSpace {}

impl KernelSpace {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// 写入内核地址空间（仅在初始化时调用一次）
    unsafe fn write(&self, space: AddressSpace<Sv39, Sv39Manager>) {
        unsafe { *self.inner.get() = MaybeUninit::new(space) };
    }

    /// 获取内核地址空间的不可变引用
    unsafe fn assume_init_ref(&self) -> &AddressSpace<Sv39, Sv39Manager> {
        unsafe { &*(*self.inner.get()).as_ptr() }
    }
}

/// 内核地址空间全局实例
static KERNEL_SPACE: KernelSpace = KernelSpace::new();

static mut GPU_DEV: Option<VirtIOGpu<'static, VirtioHal, MmioTransport>> = None;
static mut FB_PTR: *mut u8 = core::ptr::null_mut();
static mut FB_LEN: usize = 0;
static mut FB_WIDTH: u32 = 0;
static mut FB_HEIGHT: u32 = 0;
static mut FB_FLUSH_OK_LOGGED: bool = false;
static mut FB_FLUSH_ERR_COUNT: usize = 0;

const MMIO: &[(usize, usize)] = &[
    // UART (NS16550A) used by console_getchar_nonblocking
    (0x1000_0000, 0x1000),
    (0x1000_1000, 0x1000),
    (0x1000_2000, 0x1000),
    (0x1000_3000, 0x1000),
    (0x1000_4000, 0x1000),
    (0x1000_5000, 0x1000),
    (0x1000_6000, 0x1000),
    (0x1000_7000, 0x1000),
    (0x1000_8000, 0x1000),
];

struct VirtioHal;

impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize {
        unsafe {
            alloc::alloc::alloc_zeroed(Layout::from_size_align_unchecked(
                pages << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            )) as _
        }
    }

    fn dma_dealloc(paddr: usize, pages: usize) -> i32 {
        unsafe {
            alloc::alloc::dealloc(
                paddr as _,
                Layout::from_size_align_unchecked(pages << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS),
            )
        }
        0
    }

    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }

    fn virt_to_phys(vaddr: usize) -> usize {
        const VALID: VmFlags<Sv39> = build_flags("__V");
        let ptr = unsafe {
            KERNEL_SPACE
                .assume_init_ref()
                .translate::<u8>(VAddr::new(vaddr), VALID)
                .unwrap()
        };
        ptr.as_ptr() as usize
    }
}

/// 应用程序名称到 ELF 数据的映射表
///
/// 在首次访问时通过 `Lazy` 初始化：
/// 1. 读取 build.rs 嵌入的应用元数据（`AppMeta`）获取各应用的二进制数据
/// 2. 读取 `app_names` 符号获取各应用的名称字符串
/// 3. 构建 BTreeMap 以支持按名称查找应用（供 exec 系统调用使用）
static APPS: Lazy<BTreeMap<&'static str, &'static [u8]>> = Lazy::new(|| {
    unsafe extern "C" {
        static app_names: u8;
    }
    unsafe {
        tg_linker::AppMeta::locate()
            .iter()
            .scan(&app_names as *const _ as usize, |addr, data| {
                let name = CStr::from_ptr(*addr as _).to_str().unwrap();
                *addr += name.as_bytes().len() + 1;
                Some((name, data))
            })
    }
    .collect()
});

/// 内核主函数——系统初始化和启动入口
///
/// 执行流程：
/// 1. 清零 BSS 段
/// 2. 初始化控制台和日志系统
/// 3. 初始化内核堆分配器
/// 4. 分配并创建异界传送门
/// 5. 建立内核地址空间（恒等映射），激活 Sv39 分页
/// 6. 初始化异界传送门和系统调用处理器
/// 7. 加载初始进程 `initproc`，进入调度循环
extern "C" fn rust_main() -> ! {
    let layout = tg_linker::KernelLayout::locate();
    // 步骤 1：清零 BSS 段（未初始化全局变量区域）
    unsafe { layout.zero_bss() };
    // 步骤 2：初始化控制台输出和日志系统
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    // 步骤 3：初始化内核堆分配器
    // 堆起始地址为内核镜像起始处，可用区域为内核镜像结束处到物理内存末尾
    tg_kernel_alloc::init(layout.start() as _);
    unsafe {
        tg_kernel_alloc::transfer(core::slice::from_raw_parts_mut(
            layout.end() as _,
            MEMORY - layout.len(),
        ))
    };
    // 步骤 4：分配异界传送门所需的物理页面
    let portal_size = MultislotPortal::calculate_size(1);
    let portal_layout = Layout::from_size_align(portal_size, 1 << Sv39::PAGE_BITS).unwrap();
    let portal_ptr = unsafe { alloc(portal_layout) };
    assert!(portal_layout.size() < 1 << Sv39::PAGE_BITS);
    // 步骤 5：建立内核地址空间并激活 Sv39 分页
    kernel_space(layout, MEMORY, portal_ptr as _);
    init_gpu();
    // 步骤 6：初始化异界传送门（设置传送门页面的虚拟地址和 slot 数量）
    let portal = unsafe { MultislotPortal::init_transit(PROTAL_TRANSIT.base().val(), 1) };
    // 步骤 7：初始化系统调用处理器
    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
    tg_syscall::init_scheduling(&SyscallContext);
    tg_syscall::init_clock(&SyscallContext);
    tg_syscall::init_memory(&SyscallContext);
    // 步骤 8：加载初始进程 initproc
    // initproc 是所有用户进程的祖先，它会 fork 出 shell 进程
    let initproc_data = APPS
        .get("ch5_pingpong")
        .or_else(|| APPS.get("user_shell"))
        .or_else(|| APPS.get("initproc"))
        .unwrap();
    if let Some(process) = Process::from_elf(ElfFile::new(initproc_data).unwrap()) {
        // 初始化进程管理器并添加 initproc
        PROCESSOR.get_mut().set_manager(ProcManager::new());
        PROCESSOR
            .get_mut()
            .add(process.pid, process, ProcId::from_usize(usize::MAX));
    }

    // ─── 主调度循环 ───
    // 不断从进程管理器中取出就绪进程执行，直到所有进程结束
    loop {
        let processor: *mut PManager<Process, ProcManager> = PROCESSOR.get_mut() as *mut _;
        if let Some(task) = unsafe { (*processor).find_next() } {
            // 通过异界传送门切换到用户地址空间执行用户程序
            unsafe { task.context.execute(portal, ()) };

            // ─── Trap 返回后处理 ───
            match scause::read().cause() {
                // ─── 系统调用（ecall 指令触发） ───
                scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                    use tg_syscall::{SyscallId as Id, SyscallResult as Ret};
                    let ctx = &mut task.context.context;
                    // 将 sepc 向前移动 4 字节，使返回用户态时跳过 ecall 指令
                    ctx.move_next();
                    // 解析系统调用号和参数
                    let id: Id = ctx.a(7).into();
                    let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];
                    // 分发并处理系统调用
                    match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                        Ret::Done(ret) => match id {
                            // exit 系统调用：标记当前进程为已退出
                            Id::EXIT => unsafe { (*processor).make_current_exited(ret) },
                            _ => {
                                // 其他系统调用：将返回值写入 a0 寄存器，暂停当前进程
                                let ctx = &mut task.context.context;
                                *ctx.a_mut(0) = ret as _;
                                unsafe { (*processor).make_current_suspend() };
                            }
                        },
                        Ret::Unsupported(_) => {
                            // 不支持的系统调用：终止进程
                            log::info!("id = {id:?}");
                            unsafe { (*processor).make_current_exited(-2) };
                        }
                    }
                }
                // ─── 其他异常/中断：杀死进程 ───
                e => {
                    log::error!("unsupported trap: {e:?}");
                    unsafe { (*processor).make_current_exited(-3) };
                }
            }
        } else {
            // 没有更多进程可执行
            println!("no task");
            break;
        }
    }
    // 所有进程执行完毕，关机
    tg_sbi::shutdown(false)
}

/// Rust panic 处理函数，打印错误信息并以异常方式关机
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    tg_sbi::shutdown(true)
}

/// 建立内核地址空间
///
/// 内核使用**恒等映射**（Identity Mapping）：虚拟地址 == 物理地址。
///
/// 映射内容：
/// 1. 内核代码段（.text）：可执行、可读、有效
/// 2. 只读数据段（.rodata）：可读、有效
/// 3. 数据段（.data）和启动段（.boot）：可写、可读、有效
/// 4. 堆区域：可写、可读、有效
/// 5. 异界传送门页面：全权限（内核和用户共享）
fn kernel_space(layout: tg_linker::KernelLayout, memory: usize, portal: usize) {
    let mut space = AddressSpace::new();
    // 映射内核各段（恒等映射：VPN == PPN）
    for region in layout.iter() {
        log::info!("{region}");
        use tg_linker::KernelRegionTitle::*;
        let flags = match region.title {
            Text => "X_RV",        // 代码段：可执行、可读
            Rodata => "__RV",      // 只读数据：可读
            Data | Boot => "_WRV", // 数据段：可写、可读
        };
        let s = VAddr::<Sv39>::new(region.range.start);
        let e = VAddr::<Sv39>::new(region.range.end);
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags(flags),
        )
    }
    // 映射堆区域（内核镜像结束处到物理内存末尾）
    let s = VAddr::<Sv39>::new(layout.end());
    let e = VAddr::<Sv39>::new(layout.start() + memory);
    log::info!("(heap) ---> {:#10x}..{:#10x}", s.val(), e.val());
    space.map_extern(
        s.floor()..e.ceil(),
        PPN::new(s.floor().val()),
        build_flags("_WRV"),
    );
    // 映射异界传送门页面到虚拟地址空间最高页
    // 标志位 __G_XWRV：全局、可执行、可写、可读、有效
    space.map_extern(
        PROTAL_TRANSIT..PROTAL_TRANSIT + 1,
        PPN::new(portal >> Sv39::PAGE_BITS),
        build_flags("__G_XWRV"),
    );
    for (base, len) in MMIO {
        let s = VAddr::<Sv39>::new(*base);
        let e = VAddr::<Sv39>::new(*base + *len);
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags("_WRV"),
        );
    }
    println!();
    // 激活 Sv39 分页模式：写入 satp 寄存器
    unsafe { satp::set(satp::Mode::Sv39, 0, space.root_ppn().val()) };
    // 保存内核地址空间到全局变量
    unsafe { KERNEL_SPACE.write(space) };
}

fn init_gpu() {
    let mut found = false;
    for addr in (0x1000_1000..=0x1000_8000).step_by(0x1000) {
        if unsafe { core::ptr::read_volatile(addr as *const u32) } != 0x74726976 {
            continue;
        }
        let header = NonNull::new(addr as *mut VirtIOHeader).unwrap();
        if let Ok(transport) = unsafe { MmioTransport::new(header) } {
            if transport.device_type() == DeviceType::GPU {
                if let Ok(mut gpu) = VirtIOGpu::<VirtioHal, MmioTransport>::new(transport) {
                    if let Ok((width, height)) = gpu.resolution() {
                        unsafe {
                            let fb = gpu.setup_framebuffer().unwrap();
                            let fb_ptr = fb.as_mut_ptr();
                            let fb_len = fb.len();
                            core::slice::from_raw_parts_mut(fb_ptr, fb_len).fill(0);
                            let _ = gpu.flush();
                            FB_PTR = fb_ptr;
                            FB_LEN = fb_len;
                            FB_WIDTH = width;
                            FB_HEIGHT = height;
                            GPU_DEV = Some(gpu);
                        }
                        log::info!("GPU initialized: {}x{}", width, height);
                    }
                }
                found = true;
                break;
            }
        }
    }
    if !found {
        log::warn!("VirtIO GPU device not found; framebuffer syscalls will be unavailable");
    }
}

/// 将内核地址空间中的异界传送门页表项复制到用户地址空间
///
/// 这确保了内核和用户地址空间在传送门虚拟地址处映射到同一物理页面，
/// 使得切换 satp 时代码仍然可以正常执行。
fn map_portal(space: &AddressSpace<Sv39, Sv39Manager>) {
    let portal_idx = PROTAL_TRANSIT.index_in(Sv39::MAX_LEVEL);
    space.root()[portal_idx] = unsafe { KERNEL_SPACE.assume_init_ref() }.root()[portal_idx];
}

/// 各种接口库的实现
///
/// 本模块为 tg-syscall 提供的各个 trait 提供具体实现，
/// 包括 IO、Process、Scheduling、Clock、Memory 等系统调用接口。
mod impls {
    use crate::{
        APPS, PROCESSOR, Sv39, build_flags, process::Process as ProcStruct, processor::ProcManager,
    };
    use alloc::{alloc::alloc_zeroed, collections::VecDeque, vec::Vec};
    use core::{alloc::Layout, ptr::NonNull};
    use spin::{Lazy, Mutex};
    use tg_console::log;
    use tg_kernel_vm::{
        PageManager,
        page_table::{MmuMeta, PPN, Pte, VAddr, VPN, VmFlags},
    };
    use tg_syscall::*;
    use tg_task_manage::{PManager, ProcId};
    use xmas_elf::ElfFile;

    // ─── Sv39 页表管理器 ───

    /// Sv39 页表管理器
    ///
    /// 实现 `PageManager<Sv39>` trait，负责：
    /// - 物理页面的分配和释放
    /// - 物理地址与虚拟地址的转换（恒等映射下两者相等）
    /// - 页面所有权标记（OWNED 标志位）
    #[repr(transparent)]
    pub struct Sv39Manager(NonNull<Pte<Sv39>>);

    impl Sv39Manager {
        /// 自定义标志位：标记此页面由内核分配（用于区分恒等映射的外部页面）
        const OWNED: VmFlags<Sv39> = unsafe { VmFlags::from_raw(1 << 8) };

        /// 分配对齐的物理页面（已清零）
        #[inline]
        fn page_alloc<T>(count: usize) -> *mut T {
            unsafe {
                alloc_zeroed(Layout::from_size_align_unchecked(
                    count << Sv39::PAGE_BITS,
                    1 << Sv39::PAGE_BITS,
                ))
            }
            .cast()
        }
    }

    impl PageManager<Sv39> for Sv39Manager {
        /// 创建新的根页表
        #[inline]
        fn new_root() -> Self {
            Self(NonNull::new(Self::page_alloc(1)).unwrap())
        }

        /// 获取根页表的物理页号
        #[inline]
        fn root_ppn(&self) -> PPN<Sv39> {
            PPN::new(self.0.as_ptr() as usize >> Sv39::PAGE_BITS)
        }

        /// 获取根页表的指针
        #[inline]
        fn root_ptr(&self) -> NonNull<Pte<Sv39>> {
            self.0
        }

        /// 物理页号转虚拟地址（恒等映射下直接转换）
        #[inline]
        fn p_to_v<T>(&self, ppn: PPN<Sv39>) -> NonNull<T> {
            unsafe { NonNull::new_unchecked(VPN::<Sv39>::new(ppn.val()).base().as_mut_ptr()) }
        }

        /// 虚拟地址转物理页号（恒等映射下直接转换）
        #[inline]
        fn v_to_p<T>(&self, ptr: NonNull<T>) -> PPN<Sv39> {
            PPN::new(VAddr::<Sv39>::new(ptr.as_ptr() as _).floor().val())
        }

        /// 检查页表项是否由内核分配
        #[inline]
        fn check_owned(&self, pte: Pte<Sv39>) -> bool {
            pte.flags().contains(Self::OWNED)
        }

        /// 分配物理页面并标记为内核所有
        #[inline]
        fn allocate(&mut self, len: usize, flags: &mut VmFlags<Sv39>) -> NonNull<u8> {
            *flags |= Self::OWNED;
            NonNull::new(Self::page_alloc(len)).unwrap()
        }

        fn deallocate(&mut self, _pte: Pte<Sv39>, _len: usize) -> usize {
            todo!()
        }

        fn drop_root(&mut self) {
            todo!()
        }
    }

    // ─── 控制台实现 ───

    /// 控制台输出实现，通过 SBI 接口逐字符输出
    pub struct Console;

    impl tg_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            tg_sbi::console_putchar(c);
        }
    }

    // ─── 系统调用实现 ───

    /// 系统调用上下文，实现 IO、Process、Scheduling、Clock、Memory 等 trait
    pub struct SyscallContext;

    const PIPE_FD_BASE: usize = 3;
    const PIPE_BUF_CAP: usize = 1024;

    struct Pipe {
        buf: VecDeque<u8>,
        read_open: bool,
        write_open: bool,
    }

    struct PipeManager {
        pipes: Vec<Option<Pipe>>,
    }

    impl PipeManager {
        fn new() -> Self {
            Self { pipes: Vec::new() }
        }

        #[inline]
        fn decode_fd(fd: usize) -> Option<(usize, bool)> {
            if fd < PIPE_FD_BASE {
                return None;
            }
            let x = fd - PIPE_FD_BASE;
            Some((x / 2, (x & 1) != 0))
        }

        fn alloc_pipe(&mut self) -> (usize, usize) {
            let idx = if let Some((idx, _)) = self
                .pipes
                .iter()
                .enumerate()
                .find(|(_, slot)| slot.is_none())
            {
                idx
            } else {
                self.pipes.push(None);
                self.pipes.len() - 1
            };

            self.pipes[idx] = Some(Pipe {
                buf: VecDeque::with_capacity(PIPE_BUF_CAP),
                read_open: true,
                write_open: true,
            });

            (PIPE_FD_BASE + idx * 2, PIPE_FD_BASE + idx * 2 + 1)
        }

        fn write_pipe(&mut self, fd: usize, input: &[u8]) -> isize {
            let Some((idx, is_write_end)) = Self::decode_fd(fd) else {
                return -1;
            };
            if !is_write_end {
                return -1;
            }
            let Some(pipe) = self.pipes.get_mut(idx).and_then(Option::as_mut) else {
                return -1;
            };
            if !pipe.write_open || !pipe.read_open {
                return -1;
            }

            let mut wrote = 0usize;
            for &b in input {
                if pipe.buf.len() >= PIPE_BUF_CAP {
                    break;
                }
                pipe.buf.push_back(b);
                wrote += 1;
            }

            if wrote == 0 {
                -2
            } else {
                wrote as isize
            }
        }

        fn read_pipe(&mut self, fd: usize, output: &mut [u8]) -> isize {
            let Some((idx, is_write_end)) = Self::decode_fd(fd) else {
                return -1;
            };
            if is_write_end {
                return -1;
            }
            let Some(pipe) = self.pipes.get_mut(idx).and_then(Option::as_mut) else {
                return -1;
            };
            if !pipe.read_open {
                return -1;
            }

            let mut read = 0usize;
            while read < output.len() {
                if let Some(v) = pipe.buf.pop_front() {
                    output[read] = v;
                    read += 1;
                } else {
                    break;
                }
            }

            if read > 0 {
                return read as isize;
            }
            if !pipe.write_open {
                0
            } else {
                -2
            }
        }

        fn close_fd(&mut self, fd: usize) -> isize {
            let Some((idx, is_write_end)) = Self::decode_fd(fd) else {
                return -1;
            };
            let Some(pipe) = self.pipes.get_mut(idx).and_then(Option::as_mut) else {
                return -1;
            };

            if is_write_end {
                pipe.write_open = false;
            } else {
                pipe.read_open = false;
            }

            if !pipe.read_open && !pipe.write_open && pipe.buf.is_empty() {
                self.pipes[idx] = None;
            }
            0
        }
    }

    static PIPE_MANAGER: Lazy<Mutex<PipeManager>> = Lazy::new(|| Mutex::new(PipeManager::new()));

    /// IO 系统调用实现：write 和 read
    impl IO for SyscallContext {
        /// write 系统调用：将数据写入标准输出
        ///
        /// 需要通过 `translate()` 将用户虚拟地址翻译为物理地址，
        /// 并检查可读权限后才能访问用户缓冲区。
        fn write(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    const READABLE: VmFlags<Sv39> = build_flags("RV");
                    if let Some(ptr) = PROCESSOR
                        .get_mut()
                        .current()
                        .unwrap()
                        .address_space
                        .translate::<u8>(VAddr::new(buf), READABLE)
                    {
                        print!("{}", unsafe {
                            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                                ptr.as_ptr(),
                                count,
                            ))
                        });
                        count as _
                    } else {
                        log::error!("ptr not readable");
                        -1
                    }
                }
                _ => {
                    const READABLE: VmFlags<Sv39> = build_flags("RV");
                    if let Some(ptr) = PROCESSOR
                        .get_mut()
                        .current()
                        .unwrap()
                        .address_space
                        .translate::<u8>(VAddr::new(buf), READABLE)
                    {
                        let data = unsafe { core::slice::from_raw_parts(ptr.as_ptr(), count) };
                        PIPE_MANAGER.lock().write_pipe(fd, data)
                    } else {
                        log::error!("ptr not readable");
                        -1
                    }
                }
            }
        }

        /// read 系统调用：从标准输入读取数据
        ///
        /// 通过 SBI console_getchar 接口逐字符读取，
        /// 同样需要地址翻译和可写权限检查。
        #[inline]
        fn read(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            if fd == STDIN {
                const WRITEABLE: VmFlags<Sv39> = build_flags("W_V");
                if let Some(mut ptr) = PROCESSOR
                    .get_mut()
                    .current()
                    .unwrap()
                    .address_space
                    .translate::<u8>(VAddr::new(buf), WRITEABLE)
                {
                    let mut ptr = unsafe { ptr.as_mut() } as *mut u8;
                    for _ in 0..count {
                        let c = tg_sbi::console_getchar() as u8;
                        unsafe {
                            *ptr = c;
                            ptr = ptr.add(1);
                        }
                    }
                    count as _
                } else {
                    log::error!("ptr not writeable");
                    -1
                }
            } else {
                const WRITEABLE: VmFlags<Sv39> = build_flags("W_V");
                if let Some(ptr) = PROCESSOR
                    .get_mut()
                    .current()
                    .unwrap()
                    .address_space
                    .translate::<u8>(VAddr::new(buf), WRITEABLE)
                {
                    let out = unsafe { core::slice::from_raw_parts_mut(ptr.as_ptr(), count) };
                    PIPE_MANAGER.lock().read_pipe(fd, out)
                } else {
                    log::error!("ptr not writeable");
                    -1
                }
            }
        }

        fn pipe(&self, _caller: Caller, pipe: usize) -> isize {
            const WRITEABLE: VmFlags<Sv39> = build_flags("W_V");
            let current = PROCESSOR.get_mut().current().unwrap();

            let Some(mut read_ptr) = current
                .address_space
                .translate::<usize>(VAddr::new(pipe), WRITEABLE)
            else {
                return -1;
            };
            let Some(mut write_ptr) = current.address_space.translate::<usize>(
                VAddr::new(pipe + core::mem::size_of::<usize>()),
                WRITEABLE,
            ) else {
                return -1;
            };

            let (read_fd, write_fd) = PIPE_MANAGER.lock().alloc_pipe();
            unsafe {
                *read_ptr.as_mut() = read_fd;
                *write_ptr.as_mut() = write_fd;
            }
            0
        }

        fn close(&self, _caller: Caller, fd: usize) -> isize {
            PIPE_MANAGER.lock().close_fd(fd)
        }

        fn console_getchar_nonblocking(&self, _caller: Caller) -> isize {
            #[cfg(target_arch = "riscv64")]
            {
                let lsr = unsafe { core::ptr::read_volatile((0x1000_0000 + 5) as *const u8) };
                if lsr & 1 != 0 {
                    let ch = unsafe { core::ptr::read_volatile(0x1000_0000 as *const u8) };
                    ch as isize
                } else {
                    -1
                }
            }
            #[cfg(not(target_arch = "riscv64"))]
            {
                -1
            }
        }

        fn fb_info(&self, _caller: Caller, width_out: usize, height_out: usize) -> isize {
            const WRITEABLE: VmFlags<Sv39> = build_flags("W_V");
            let current = PROCESSOR.get_mut().current().unwrap();
            let width_ptr = current
                .address_space
                .translate::<u32>(VAddr::new(width_out), WRITEABLE);
            let height_ptr = current
                .address_space
                .translate::<u32>(VAddr::new(height_out), WRITEABLE);

            if let (Some(mut wp), Some(mut hp)) = (width_ptr, height_ptr) {
                unsafe {
                    *wp.as_mut() = crate::FB_WIDTH;
                    *hp.as_mut() = crate::FB_HEIGHT;
                }
                0
            } else {
                -1
            }
        }

        fn fb_flush(&self, _caller: Caller, buf: usize, len: usize) -> isize {
            if unsafe { crate::FB_LEN } == 0 || unsafe { crate::FB_PTR }.is_null() {
                return -1;
            }
            if len != unsafe { crate::FB_LEN } {
                return -1;
            }

            const READABLE: VmFlags<Sv39> = build_flags("RV");
            let current = PROCESSOR.get_mut().current().unwrap();
            let mut remaining = len;
            let mut src = buf;
            let mut dst = unsafe { crate::FB_PTR } as usize;
            while remaining > 0 {
                if let Some(src_ptr) = current
                    .address_space
                    .translate::<u8>(VAddr::new(src), READABLE)
                {
                    let page_end = (src & !0xfff) + 0x1000;
                    let copy_len = core::cmp::min(remaining, page_end - src);
                    unsafe {
                        core::ptr::copy_nonoverlapping(src_ptr.as_ptr(), dst as *mut u8, copy_len)
                    };
                    src += copy_len;
                    dst += copy_len;
                    remaining -= copy_len;
                } else {
                    return -1;
                }
            }

            unsafe {
                if let Some(gpu) = (&raw mut crate::GPU_DEV).as_mut() {
                    if let Some(gpu) = gpu.as_mut() {
                        match gpu.flush() {
                            Ok(_) => {
                                if !crate::FB_FLUSH_OK_LOGGED {
                                    crate::FB_FLUSH_OK_LOGGED = true;
                                    log::info!("GPU fb_flush OK");
                                }
                            }
                            Err(e) => {
                                if crate::FB_FLUSH_ERR_COUNT < 8 {
                                    crate::FB_FLUSH_ERR_COUNT += 1;
                                    log::warn!("GPU fb_flush failed: {:?}", e);
                                }
                            }
                        }
                    }
                }
            }
            0
        }
    }

    /// 进程管理系统调用实现
    impl Process for SyscallContext {
        /// exit 系统调用：退出当前进程
        ///
        /// 返回 exit_code，由 ProcManager 标记为僵尸状态，
        /// 等待父进程通过 wait 回收。
        #[inline]
        fn exit(&self, _caller: Caller, exit_code: usize) -> isize {
            exit_code as isize
        }

        /// fork 系统调用：创建子进程
        ///
        /// 复制父进程的完整地址空间（深拷贝页表和物理页面），
        /// 父进程返回子进程 PID，子进程返回 0。
        fn fork(&self, _caller: Caller) -> isize {
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };
            let parent_pid = current.pid; // 保存父进程 PID
            let mut child_proc = current.fork().unwrap();
            let pid = child_proc.pid;
            let context = &mut child_proc.context.context;
            // 子进程的 a0 寄存器设为 0（fork 的返回值）
            *context.a_mut(0) = 0 as _;
            // 将子进程加入进程管理器，父进程 PID 用于维护进程树
            unsafe { (*processor).add(pid, child_proc, parent_pid) };
            // 父进程返回子进程 PID
            pid.get_usize() as isize
        }

        /// exec 系统调用：加载并执行新程序
        ///
        /// 根据用户传入的程序名（需地址翻译），查找对应的 ELF 数据，
        /// 替换当前进程的地址空间。
        fn exec(&self, _caller: Caller, path: usize, count: usize) -> isize {
            const READABLE: VmFlags<Sv39> = build_flags("RV");
            let current = PROCESSOR.get_mut().current().unwrap();
            current
                .address_space
                .translate::<u8>(VAddr::new(path), READABLE)
                .map(|ptr| unsafe {
                    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr.as_ptr(), count))
                })
                .and_then(|name| APPS.get(name))
                .and_then(|input| ElfFile::new(input).ok())
                .map_or_else(
                    || {
                        log::error!("unknown app, select one in the list: ");
                        APPS.keys().for_each(|app| println!("{app}"));
                        println!();
                        -1
                    },
                    |data| {
                        current.exec(data);
                        0
                    },
                )
        }

        /// wait 系统调用：等待子进程退出
        ///
        /// - pid == -1：等待任意子进程
        /// - pid > 0：等待指定 PID 的子进程
        /// 返回值：成功返回子进程 PID，无子进程返回 -1
        fn wait(&self, _caller: Caller, pid: isize, exit_code_ptr: usize) -> isize {
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            if let Some((dead_pid, exit_code)) =
                unsafe { (*processor).wait(ProcId::from_usize(pid as usize)) }
            {
                // 将退出码写入用户空间指针（需地址翻译）
                if let Some(mut ptr) = current
                    .address_space
                    .translate::<i32>(VAddr::new(exit_code_ptr), WRITABLE)
                {
                    unsafe { *ptr.as_mut() = exit_code as i32 };
                }
                return dead_pid.get_usize() as isize;
            } else {
                // 等待的子进程不存在
                return -1;
            }
        }

        /// getpid 系统调用：获取当前进程 PID
        fn getpid(&self, _caller: Caller) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            current.pid.get_usize() as _
        }

        /// spawn 系统调用：创建新进程并直接执行指定程序
        ///
        /// 与 fork+exec 不同，spawn 直接从 ELF 创建新进程，
        /// 无需复制父进程地址空间。
        ///
        /// TODO: 实现 spawn 系统调用（练习题）
        fn spawn(&self, _caller: Caller, path: usize, count: usize) -> isize {
            const READABLE: VmFlags<Sv39> = build_flags("RV");
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };

            let name = current
                .address_space
                .translate::<u8>(VAddr::new(path), READABLE)
                .map(|ptr| unsafe {
                    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr.as_ptr(), count))
                });

            let name = match name {
                Some(n) => n,
                None => return -1,
            };

            let data = match APPS.get(name) {
                Some(d) => d,
                None => return -1,
            };

            let elf = match ElfFile::new(data) {
                Ok(e) => e,
                Err(_) => return -1,
            };

            if let Some(process) = ProcStruct::from_elf(elf) {
                let pid = process.pid;
                unsafe { (*processor).add(pid, process, current.pid) };
                pid.get_usize() as isize
            } else {
                -1
            }
        }

        /// sbrk 系统调用：调整进程堆空间大小
        ///
        /// - size > 0：扩展堆
        /// - size < 0：收缩堆
        /// - size == 0：返回当前堆顶地址
        fn sbrk(&self, _caller: Caller, size: i32) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            if let Some(old_brk) = current.change_program_brk(size as isize) {
                old_brk as isize
            } else {
                -1
            }
        }
    }

    /// 调度系统调用实现
    impl Scheduling for SyscallContext {
        /// sched_yield 系统调用：主动让出 CPU
        #[inline]
        fn sched_yield(&self, _caller: Caller) -> isize {
            0
        }

        /// set_priority 系统调用：设置当前进程优先级
        ///
        /// TODO: 实现 set_priority 系统调用（练习题：stride 调度算法）
        fn set_priority(&self, _caller: Caller, prio: isize) -> isize {
            if prio < 2 {
                return -1;
            }
            let current = PROCESSOR.get_mut().current().unwrap();
            current.priority = prio as usize;
            prio
        }
    }

    /// 时钟系统调用实现
    impl Clock for SyscallContext {
        /// clock_gettime 系统调用：获取系统时间
        ///
        /// 读取 RISC-V time 寄存器，转换为纳秒级时间戳，
        /// 通过地址翻译写入用户空间的 TimeSpec 结构。
        #[inline]
        fn clock_gettime(&self, _caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    if let Some(mut ptr) = PROCESSOR
                        .get_mut()
                        .current()
                        .unwrap()
                        .address_space
                        .translate::<TimeSpec>(VAddr::new(tp), WRITABLE)
                    {
                        let time = riscv::register::time::read() * 10000 / 125;
                        *unsafe { ptr.as_mut() } = TimeSpec {
                            tv_sec: time / 1_000_000_000,
                            tv_nsec: time % 1_000_000_000,
                        };
                        0
                    } else {
                        log::error!("ptr not readable");
                        -1
                    }
                }
                _ => -1,
            }
        }
    }

    /// 内存管理系统调用实现
    impl Memory for SyscallContext {
        /// mmap 系统调用：映射匿名内存
        ///
        /// TODO: 实现 mmap 系统调用（练习题）
        fn mmap(
            &self,
            _caller: Caller,
            addr: usize,
            len: usize,
            prot: i32,
            _flags: i32,
            _fd: i32,
            _offset: usize,
        ) -> isize {
            if addr % 4096 != 0 || (prot & !0x7) != 0 || prot == 0 {
                return -1;
            }
            let mut pte_flags: u8 = 0x11; // V_U
            if (prot & 1) != 0 {
                pte_flags |= 2;
            } // R
            if (prot & 2) != 0 {
                pte_flags |= 4;
            } // W
            if (prot & 4) != 0 {
                pte_flags |= 8;
            } // X
            let vm_flags = unsafe { VmFlags::from_raw(pte_flags as usize) };
            let vpn_start = VPN::<Sv39>::new(addr >> 12);
            let pages_needed = (len + 4095) / 4096;
            let vpn_end = VPN::<Sv39>::new(vpn_start.val() + pages_needed);
            let range = vpn_start..vpn_end;
            let ptr = PROCESSOR.get_mut().current().unwrap();
            for area in &ptr.address_space.areas {
                if area.start < range.end && area.end > range.start {
                    return -1;
                }
            }
            ptr.address_space.map(range, &[], 0, vm_flags);
            0
        }

        /// munmap 系统调用：取消内存映射
        ///
        /// TODO: 实现 munmap 系统调用（练习题）
        fn munmap(&self, _caller: Caller, addr: usize, len: usize) -> isize {
            if addr % 4096 != 0 {
                return -1;
            }
            let vpn_start = VPN::<Sv39>::new(addr >> 12);
            let pages_needed = (len + 4095) / 4096;
            let vpn_end = VPN::<Sv39>::new(vpn_start.val() + pages_needed);
            let target_range = vpn_start..vpn_end;
            let ptr = PROCESSOR.get_mut().current().unwrap();
            let mut is_fully_mapped = false;
            for area in &ptr.address_space.areas {
                if target_range.start >= area.start && target_range.end <= area.end {
                    is_fully_mapped = true;
                    break;
                }
            }
            if !is_fully_mapped {
                return -1;
            }
            ptr.address_space.unmap(target_range);
            0
        }
    }
}

/// 非 RISC-V64 架构的占位实现
///
/// 在主机平台（如 x86_64）上提供占位符，使代码可以通过编译检查，
/// 方便在 IDE 中进行开发和语法检查。
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    use tg_kernel_vm::page_table::{MmuMeta, VmFlags};

    /// Sv39 占位类型（仅用于主机平台编译检查）
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
    pub struct Sv39;

    impl MmuMeta for Sv39 {
        const P_ADDR_BITS: usize = 56;
        const PAGE_BITS: usize = 12;
        const LEVEL_BITS: &'static [usize] = &[9, 9, 9];
        const PPN_POS: usize = 10;

        #[inline]
        fn is_leaf(value: usize) -> bool {
            value & 0b1110 != 0
        }
    }

    /// 构建 VmFlags 占位实现
    pub const fn build_flags(_s: &str) -> VmFlags<Sv39> {
        unsafe { VmFlags::from_raw(0) }
    }

    /// 解析 VmFlags 占位实现
    pub fn parse_flags(_s: &str) -> Result<VmFlags<Sv39>, ()> {
        Ok(unsafe { VmFlags::from_raw(0) })
    }

    /// 主机平台占位入口
    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    /// libc 启动占位
    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    /// 异常处理占位
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}
