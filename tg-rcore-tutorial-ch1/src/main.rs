//! # 第一章：应用程序与基本执行环境
//!
//! 本章实现了一个最简单的 RISC-V S 态裸机程序，展示操作系统的最小执行环境。
//!
//! ## 关键概念
//!
//! - `#![no_std]`：不使用 Rust 标准库，改用不依赖操作系统的核心库 `core`
//! - `#![no_main]`：不使用标准的 `main` 入口，自定义裸函数 `_start` 作为入口
//! - 裸函数（naked function）：不生成函数序言/尾声，可在无栈环境下执行
//! - SBI（Supervisor Binary Interface）：S 态软件向 M 态固件请求服务的标准接口
//!
//! 教程阅读建议：
//!
//! - 先看 `_start`：理解无运行时情况下的最小启动流程；
//! - 再看 `rust_main`：理解最小 I/O 路径（SBI 输出 + 关机）；
//! - 最后看 `panic_handler`：理解 no_std 程序的异常收口方式。

// 不使用标准库，因为裸机环境没有操作系统提供系统调用支持
#![no_std]
// 不使用标准入口，因为裸机环境没有 C runtime 进行初始化
#![no_main]
// RISC-V64 架构下启用严格警告和文档检查
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 非 RISC-V64 架构允许死代码（用于 cargo publish --dry-run 在主机上通过编译）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]

extern crate alloc;

// 引入 SBI 调用库，提供 console_putchar（输出字符）和 shutdown（关机）功能
// 启用 nobios 特性后，tg_sbi 内建了 M-mode 启动代码，无需外部 SBI 固件
use tg_sbi::{console_putchar, shutdown};

fn print_str(s: &str) {
    for b in s.bytes() {
        console_putchar(b);
    }
}

/// S 态程序入口点。
///
/// 这是一个裸函数（naked function），放置在 `.text.entry` 段，
/// 链接脚本将其安排在地址 `0x80200000`。
///
/// 裸函数不生成函数序言和尾声，因此可以在没有栈的情况下执行。
/// 它完成两件事：
/// 1. 设置栈指针 `sp`，指向栈顶（栈从高地址向低地址增长）
/// 2. 跳转到 Rust 主函数 `rust_main`
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    // 栈大小：4 KiB
    const STACK_SIZE: usize = 4096;

    // 在 .bss.uninit 段中分配栈空间
    #[unsafe(link_section = ".bss.uninit")]
    static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}", // 将 sp 设置为栈顶地址
        "j  {main}",                     // 跳转到 rust_main
        stack_size = const STACK_SIZE,
        stack      =   sym STACK,
        main       =   sym rust_main,
    )
}

use core::ptr::NonNull;
use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

use buddy_system_allocator::LockedHeap;
#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();
static mut HEAP_SPACE: [u8; 0x400000] = [0; 0x400000];

mod tangram;
mod gpu;
use gpu::VirtioHal;

/// S 态内核主函数
///
/// 本函数不仅建立了操作系统的最小执行环境，还通过初始化并驱动 VirtIO GPU 设备，
/// 直观展示了裸机操作系统如何直接与硬件交互并进行数据处理。
/// 核心考察点与机制展示包含：
/// 1. 动态内存管理：通过伙伴系统（Buddy System）分配大块内存（内核堆），满足 VirtIO 复杂协议要求。
/// 2. 外设探测（MMIO Probing）：通过扫描预先约定的物理连续地址段寻找特征魔数以探测接入外围设备，并在内存总线上完成设备挂载。
/// 3. 帧缓冲（Framebuffer）抽象与 DMA 控制机制：申请建立显示设备的连续物理驱动内存窗口，绕过直接汇编进行画面到像素的渲染，并借助 `flush` 同步绘制。
/// 4. 生命周期抑制防收回与挂机策略（WFI）：通过显式泄漏 `core::mem::forget` 绕开 Rust 的清理策略以防止显存黑屏；基于汇编陷入低耗能态（Wait For Interrupt）。
extern "C" fn rust_main() -> ! {
    // 1. 初始化内核堆（全局内存分配器）
    // 大量外部硬件结构（如 VirtIO 控制队列）由于数量不可静态推测，需在运行时通过堆内存获取载体。
    unsafe {
        HEAP_ALLOCATOR.lock().init(
            core::ptr::addr_of_mut!(HEAP_SPACE).cast::<u8>() as usize,
            0x400000,
        );
    }

    print_str("[OS] Starting Kernel and VirtIO probing...\n");
    let mut gpu_dev = None;
    
    // 2. 硬件设备探测（通过遍历 MMIO 区域空间实现挂载）
    // VirtIO 通过特定的物理地址区间（0x1000_1000 始，每跨度 0x1000 即为一个设备条目）将本身状态暴露至系统主存。
    for addr in (0x1000_1000..=0x1000_8000).step_by(0x1000) {
        // 读取特征识别码（魔数 `"virt"`），拦截所有真实接入且支持协议的总线响应
        if unsafe { core::ptr::read_volatile(addr as *const u32) } != 0x74726976 {
            continue;
        }

        let header = NonNull::new(addr as *mut VirtIOHeader).unwrap();
        // 尝试封装基于底层传输层协议的对象
        if let Ok(transport) = unsafe { MmioTransport::new(header) } {
            // 校验到外设设备类型为 GPU 图形输出组件
            if transport.device_type() == DeviceType::GPU {
                print_str("[OS] VirtIO GPU discovered.\n");
                // 将基于平台定制的 DMA 和地址转换接口（VirtioHal）注入驱动并构建高级操控句柄
                match VirtIOGpu::<VirtioHal, MmioTransport>::new(transport) {
                    Ok(g) => {
                        gpu_dev = Some(g);
                        print_str("[OS] GPU initialized.\n");
                    }
                    Err(_e) => {
                        print_str("[OS] GPU Init Failed.\n");
                    }
                }
                break;
            }
        }
    }

    // 3. 画面挂载与核心图层建立
    if let Some(mut gpu) = gpu_dev {
        print_str("[OS] Setting up Framebuffer...\n");
        let (width, _height) = match gpu.resolution() {
            Ok(res) => res,
            Err(_) => {
                print_str("[OS] Resolution query failed.\n");
                (0, 0)
            }
        };
        
        // 分配请求实际用于储存逐个像素颜色点、与物理显示接口等高同宽的显存区域（Framebuffer）
        let fb = match gpu.setup_framebuffer() {
            Ok(fb) => fb,
            Err(_) => {
                print_str("[OS] Framebuffer attach failed.\n");
                &mut []
            }
        };
        
        print_str("[OS] Drawing Tangram Geometry Array...\n");
        // 背景设定纯白 (通过直接写全等 255 实现 RGBA 位满像素投递)
        fb.fill(255);

        // 调用算法引擎处理图形转换空间坐标并直接写入上一步拿到的像素内存中
        for poly in tangram::TANGRAM_O.iter() {
            tangram::fill_polygon(&poly.vertices, poly.color, fb, width);
        }
        for poly in tangram::TANGRAM_S.iter() {
            tangram::fill_polygon(&poly.vertices, poly.color, fb, width);
        }

        print_str("[OS] Flushing pixels to hardware...\n");
        // 指针推刷，强迫图形设备直接在显现矩阵中对缓存变化重新进行取样和光栅应用
        let _ = gpu.flush();
        
        // 关键规避技术：抑制 RAII 释放
        // 退出该区域会将 gpu 对象析构回收，清空控制链并进而黑屏，因此显式命令程序不予回收处理
        core::mem::forget(gpu);
        print_str("[OS] Rendering Complete, system entering halt.\n");
    } else {
        print_str("[OS] GPU Missing! Launch missing '-device virtio-gpu-device' parameter?_\n");
    }

    // 4. 下旋节能状态维持
    loop {
        // 利用直接汇编中断唤醒命令 (Wait For Interrupt)，停止取指流周期从而达到挂机睡眠显示而非硬退出
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

/// panic 处理函数。
///
/// `#![no_std]` 环境下必须自行实现。发生 panic 时以异常状态关机。
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    shutdown(true)
}

/// 非 RISC-V64 架构的占位模块。
///
/// 提供 `main` 等符号，使得在主机平台（如 x86_64）上也能通过编译，
/// 满足 `cargo publish --dry-run` 和 `cargo test` 的需求。
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    /// 主机平台占位入口
    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    /// C 运行时占位
    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    /// Rust 异常处理人格占位
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}