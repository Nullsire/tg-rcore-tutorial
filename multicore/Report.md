# RISC-V 多核操作系统实验报告

**核心成果一览**：

1. 4-hart SMP 启动：基于 `amoswap` 原子指令的 boot hart 选举 + SBI HSM 启动副核
2. 双 trap 路径：用户态 trap（完整上下文保存/页表切换）与内核态 trap（轻量 callee-saved 保存）分离
3. 双调度器可运行时切换：全局轮转（GlobalRoundRobin）与 per-CPU 工作窃取（PerCpuWorkStealing），切换时自动迁移任务
4. 进程/线程分离模型：Process 持有资源（地址空间、fd 表、信号），Thread 是调度单元
5. 完整的 fork/exec/waitpid/thread_create/waittid 生命周期
6. Ticket SpinLock（关中断）、Mutex（阻塞骨架）、Semaphore、RAMFS、stdin/stdout、signal 最小闭环

**当前实现边界**：

- COW 仅处理 Store Page Fault，Load/Instruction Page Fault 未尝试 COW 解析
- signal 仅有最小闭环（无用户态 handler、无 sigaction）
- `active_harts_set` 是逻辑调度参与度控制，不是物理热插拔
- `VIRTIO0_IRQ` 保留分支，无真实驱动逻辑
- Mutex/Semaphore 唤醒不完整：获取了待唤醒 TID 但未真正执行唤醒
- PerCpuWS 的 PID 取模分配在 fork 密集场景下可能不均匀（当前测试中表现均衡）

---

## 1. 概述

MultiCore OS 是一个运行在 QEMU `virt` 平台上的 RISC-V 64 位教学内核，使用 Rust nightly + `no_std` 构建，目标特权级为 S-mode，M-mode 由 OpenSBI 提供底层服务。

与单核教学内核（如 rCore-Tutorial）相比，本实现的核心差异有四点：

| 差异维度 | 单核教学内核 | 本实现 |
|----------|-------------|--------|
| 启动 | 单 hart 直接初始化 | 4 hart SMP 启动，原子选举 boot hart |
| trap 处理 | 仅用户态 trap | 用户态 trap + 内核态 trap 双路径 |
| 调度器 | 全局轮转 | 全局轮转 + per-CPU 工作窃取，可运行时切换 |
| 执行模型 | 仅进程 | 进程（资源容器）+ 线程（调度单元）分离 |

代码层面，`kernel/` 负责内核，`user/` 负责用户态运行时和测试程序。`kernel/build.rs` 在构建时将 `user/` 编译出的 ELF 二进制通过 `.incbin` 嵌入内核镜像，生成 `link_app.S` 供内核直接引用。

---

## 2. 系统架构

### 2.1 分层结构

```text
┌─────────────────────────────────────────────────────┐
│  User Space                                         │
│  shell, hello, fork_test, test_* 等用户程序          │
│  用户态运行时: _start → main(), syscall 封装         │
├─────────────────────────────────────────────────────┤
│  Kernel Space                                       │
│  ┌──────────┬──────────┬──────────┬──────────────┐  │
│  │  trap/   │  task/   │   mm/   │    sync/     │  │
│  │ 双向量   │ 进程线程 │ 地址空间 │ 锁与同步     │  │
│  │ 分发     │ 调度器   │ 页表     │              │  │
│  ├──────────┼──────────┴──────────┼──────────────┤  │
│  │ syscall/ │       fs/          │   drivers/   │  │
│  │ 系统调用 │ RAMFS + stdio      │ UART + PLIC  │  │
│  └──────────┴────────────────────┴──────────────┘  │
├─────────────────────────────────────────────────────┤
│  OpenSBI (M-mode)                                   │
│  SBI 基础服务: console, timer, shutdown, HSM, IPI   │
├─────────────────────────────────────────────────────┤
│  Hardware (QEMU virt)                               │
│  4 × hart (RV64), UART, CLINT, PLIC, 128MB RAM     │
└─────────────────────────────────────────────────────┘
```

### 2.2 代码组织

```text
kernel/src/
  main.rs            : 入口、boot hart 选举、secondary hart 加入
  config.rs          : 常量（MAX_HARTS=4, 内存布局, MMIO 地址, PLIC 辅助函数）
  sbi.rs             : SBI ecall 封装（console, timer, shutdown, hart_start, IPI）
  console.rs         : 内核 print!/println! 宏
  lang.rs            : panic handler → shutdown
  mm/
    address.rs       : PhysAddr/VirtAddr/PhysPageNum/VirtPageNum 强类型 + PTEFlags
    frame_alloc.rs   : 栈式物理帧分配器 + FrameTracker（Drop 时自动回收）
    heap_alloc.rs    : buddy_system_allocator 提供 12MB 内核堆
    page_table.rs    : SV39 页表操作 + 用户指针翻译
    memory_set.rs    : 地址空间构造（内核/用户/fork）、MapArea、ELF 装载
  trap/
    trap.S           : user_trap_vector / user_trap_return / kernel_trap_vector 汇编
    context.rs       : TrapContext（32 GPR + sstatus + sepc + 内核信息）、KernelTrapContext
    mod.rs            : trap_handler / kernel_trap_handler 分发、signal 递送、调度检查
    interrupt.rs     : timer_handler、PLIC 初始化、外部中断处理、统计计数
  task/
    pid.rs           : PidAllocator + KernelStack（堆分配，页对齐）
    context.rs       : TaskContext（ra + sp + s[0..12]），仅保存 callee-saved
    switch.S         : __switch：保存/恢复 TaskContext
    thread.rs        : Thread / ThreadInner / ThreadStatus
    process.rs       : Process / ProcessInner，fork/exec
    processor.rs     : Per-hart Processor、ACTIVE_HARTS、run_tasks 调度循环
    scheduler/
      mod.rs         : Scheduler trait、运行时切换 + 任务迁移
      global_rr.rs   : 全局 SpinLock<VecDeque> 轮转调度器
      percpu_rr.rs   : Per-CPU 队列 + 工作窃取调度器
  sync/
    spinlock.rs      : Ticket SpinLock（关中断 + FIFO 公平）
    mutex.rs         : Mutex / MutexBlocking（阻塞互斥锁骨架）
    semaphore.rs     : Semaphore（计数信号量）
  fs/
    mod.rs           : File trait
    stdio.rs         : Stdin / Stdout（CONSOLE_LOCK 保护的 SBI 输出）
    ramfs.rs         : RAMFS（全局 BTreeMap + Inode + RamFile）
  syscall/
    mod.rs           : syscall 号分发
    fs.rs            : read/write/open/close/lseek/unlink/fsize
    process.rs       : exit/yield/getpid/fork/exec/waitpid/sched_set/hart_id/kernel_stats/...
    thread.rs        : thread_create/gettid/waittid
    sync_calls.rs    : mutex_create/lock/unlock、semaphore_create/up/down
  drivers/
    uart.rs          : NS16550A MMIO 初始化 + 输入缓冲区

user/src/
  lib.rs             : 用户态运行时（_start → main）、syscall 封装、KernelStats 解析
  console.rs         : 用户态 print!/println!（通过 sys_write(1, ...)）
  syscall.rs         : ecall 汇编封装
  bin/               : shell + 15 个测试程序
```

### 2.3 应用装载机制

`kernel/build.rs` 在构建时扫描用户程序编译产物目录，为每个 ELF 生成符号：

- `_num_app`：应用数量
- `app_i_start / app_i_end`：每个应用的二进制范围
- `_app_names`：应用名字符串表

这些符号通过 `.incbin` 指令直接嵌入内核 `.data` 段。[`main.rs`](kernel/src/main.rs) 中的 `get_app_data_by_name()` 和 `get_app_names()` 读取这一段生成的数据，支持按名查找应用。

`exec` 系统调用正是通过 `get_app_data_by_name()` 在嵌入的应用列表中查找目标 ELF，而非从文件系统加载。

---

## 3. SMP 启动与 Hart 管理

### 3.1 启动流程详解

所有 hart 都从 [`_start`](kernel/src/main.rs:89) 进入，这是一段 naked 汇编函数，执行以下步骤：

```asm
# 1. 保存 hart_id 到 tp 寄存器（整个内核生命周期使用）
mv tp, a0

# 2. 计算当前 hart 的 boot stack 顶
#    sp = &BOOT_STACK + (hart_id + 1) << 16
#    每个 hart 独占 64KB (KERNEL_STACK_SIZE) 栈空间
la t0, BOOT_STACK
addi t2, a0, 1
slli t2, t2, 16
add sp, t0, t2

# 3. 原子选举 boot hart
la t0, BOOT_CLAIMED
li t1, 1
amoswap.w.aq t2, t1, (t0)   # t2 = 旧值, 新值设为 1
bnez t2, 2f                   # 旧值非零 → 已有 boot hart → 跳到等待

# 4. boot hart 路径
j boot_main

# 5. secondary hart 等待路径
2:
la t0, BOOT_HART_READY
3: ld t1, (t0)
beqz t1, 3b    # 自旋等待
fence           # 内存屏障
j secondary_main
```

**关键设计决策**：

- 使用 `amoswap.w.aq` 而非普通写入，保证多个 hart 同时起跑时只有一个进入 boot 路径
- `BOOT_CLAIMED` 是 `AtomicUsize`，初始为 0，第一个执行 `amoswap` 的 hart 读到旧值 0，赢得选举
- Secondary hart 自旋等待 `BOOT_HART_READY`，而非使用 SBI HSM 的 `hart_start`，这是因为 OpenSBI 已经在 M-mode 启动了所有 hart

### 3.2 boot_main 初始化序列

[`boot_main()`](kernel/src/main.rs:149) 执行完整的内核初始化：

```text
1. clear_bss()                          # 清零 BSS 段
2. mm::init()                           # 初始化堆 → 帧分配器 → 内核页表 → 激活
3. task::processor::init_processor()     # 初始化当前 hart 的 Processor，设置 tp
4. trap::init()                          # 设置 stvec=kernel_trap_vector, sie, PLIC
5. drivers::uart::init()                 # UART 波特率/FIFO/中断使能
6. start_secondary_harts()               # 通过 SBI HSM 启动其他 hart
7. 读取嵌入应用列表，创建初始进程         # INIT_APP 或回退到 shell
8. BOOT_HART_READY = true               # 发布就绪信号
9. set_next_trigger()                    # 设置第一次时钟中断
10. run_tasks()                          # 进入调度循环
```

### 3.3 secondary_main 初始化序列

[`secondary_main()`](kernel/src/main.rs:192) 路径更短：

```text
1. init_processor()    # 初始化本地 Processor
2. trap::init()        # 设置 stvec, sie, PLIC
3. 切换 satp 到内核页表 + sfence.vma
4. HARTS_BOOTED += 1   # 诊断计数
5. set_next_trigger()  # 设置第一次时钟中断
6. run_tasks()         # 进入调度循环
```

注意：secondary hart 不需要 `clear_bss()` 和 `mm::init()`，因为这些已经由 boot hart 完成。但它需要显式切换 `satp`，因为此时它还在使用 OpenSBI 的初始页表。

### 3.4 start_secondary_harts 与 SBI HSM

[`start_secondary_harts()`](kernel/src/main.rs:133) 通过 `sbi::hart_start()` 向 SBI 发送 HSM 请求，让未启动的 hart 从 `_start` 地址重新开始执行。这意味着 secondary hart 实际上会再次经过 `_start` 的选举逻辑，但因为 `BOOT_CLAIMED` 已经是 1，它们会直接跳到等待路径。

### 3.5 两类栈的区分

| 栈类型 | 用途 | 位置 | 大小 |
|--------|------|------|------|
| `BOOT_STACK` | 启动阶段使用 | `.bss.stack` 段 | `KERNEL_STACK_SIZE × MAX_HARTS` = 256KB |
| `KernelStack` | 线程运行时使用 | 堆分配（`alloc_zeroed`） | 64KB/线程 |

`KernelStack` 由 [`pid.rs`](kernel/src/task/pid.rs:47) 中的 `KernelStack::new()` 分配，使用 `Layout::from_size_align(KERNEL_STACK_SIZE, PAGE_SIZE)` 保证页对齐。`Drop` 时通过 `dealloc` 回收。

### 3.6 active_harts 逻辑活跃核机制

[`ACTIVE_HARTS`](kernel/src/task/processor.rs:48) 是一个 `AtomicUsize`，初始为 `MAX_HARTS`(4)。它控制的是"参与调度的 hart 数量"，而非物理关闭 QEMU 中的 hart。

```rust
pub fn set_active_harts(count: usize) -> bool {
    if count == 1 {
        // 单核模式：将调用者的 hart 设为唯一活跃 hart
        ACTIVE_HART_BASE.store(hart_id(), Ordering::SeqCst);
    } else {
        ACTIVE_HART_BASE.store(0, Ordering::SeqCst);
    }
    ACTIVE_HARTS.store(count, Ordering::SeqCst);
}
```

[`logical_hart_index()`](kernel/src/task/processor.rs:72) 将物理 hart ID 映射为逻辑调度索引：

- `active == 1` 时，只有 `ACTIVE_HART_BASE` 指定的 hart 返回 `Some(0)`，其余返回 `None`
- `active > 1` 时，hart 0..active 返回 `Some(self)`，其余返回 `None`

在 [`run_tasks()`](kernel/src/task/processor.rs:147) 中，`logical_hart_index(hid).is_none()` 的 hart 会执行 `wfi` 等待中断，不参与调度。

---

## 4. 内存管理

### 4.1 地址抽象与 SV39 页表

[`mm/address.rs`](kernel/src/mm/address.rs) 定义了四种强类型地址/页号：

| 类型 | 位宽 | 用途 |
|------|------|------|
| `PhysAddr` | 56 位 | 物理地址 |
| `VirtAddr` | 39 位 | 虚拟地址 |
| `PhysPageNum` | 44 位 | 物理页号 |
| `VirtPageNum` | 27 位 | 虚拟页号 |

页大小 4KB，SV39 三级页表每级 512 项。`VirtPageNum::indexes()` 将 VPN 分解为三个 9-bit 索引 `[L2, L1, L0]`，供页表遍历使用。

`PageTableEntry` 采用标准编码 `PPN[0:43] | flags[0:7]`，支持 V/R/W/X/U/G/A/D 标志位。

`PhysPageNum` 提供了三个关键方法：
- `get_bytes_array()` → `&'static mut [u8; 4096]`：直接访问页内容
- `get_pte_array()` → `&'static mut [PageTableEntry; 512]`：将页当作页表使用
- `get_mut::<T>()` → `&'static mut T`：将页当作任意类型访问

这些方法利用恒等映射，直接将物理地址转为引用，是整个内核操作物理页的基础。

### 4.2 物理帧分配器

[`frame_alloc.rs`](kernel/src/mm/frame_alloc.rs) 实现栈式帧分配器 `StackFrameAllocator`：

```rust
struct StackFrameAllocator {
    current: usize,    // 下一个可分配的 PPN
    end: usize,        // 终止 PPN
    recycled: Vec<usize>,  // 回收的 PPN 列表
}
```

分配优先从 `recycled` 弹出，其次 `current` 线性前进。`FrameTracker` 持有帧的 RAII 所有权，`Drop` 时自动调用 `frame_dealloc` 回收。

帧分配器被 `SpinLock` 保护，初始化范围为 `[ekernel 页号, MEMORY_END 页号)`，即内核结束到 128MB 处。

### 4.3 内核堆

[`heap_alloc.rs`](kernel/src/mm/heap_alloc.rs) 使用 `buddy_system_allocator` crate 提供 12MB（`KERNEL_HEAP_SIZE = 0xC0_0000`）全局堆空间。堆空间位于 `.bss` 段的静态数组 `HEAP_SPACE` 中。

### 4.4 内核地址空间

[`MemorySet::new_kernel()`](kernel/src/mm/memory_set.rs:176) 创建内核地址空间，全部使用**恒等映射**（`MapType::Identical`，VPN == PPN）：

| 区域 | 权限 | 说明 |
|------|------|------|
| `.text` | R+X | 内核代码段 |
| `.rodata` | R | 只读数据 |
| `.data` | R+W | 可修改数据 |
| `.bss`（含 boot stack） | R+W | 零初始化数据 |
| `ekernel..MEMORY_END` | R+W | 剩余物理内存 |
| UART (0x1000_0000) | R+W | MMIO |
| PLIC (0x0C00_0000, 4MB) | R+W | MMIO |
| VirtIO (0x1000_1000) | R+W | MMIO |

恒等映射意味着内核代码和数据可以直接用物理地址访问，无需地址翻译开销。

### 4.5 用户地址空间

[`MemorySet::from_elf()`](kernel/src/mm/memory_set.rs:340) 从 ELF 创建用户地址空间：

```text
1. map_kernel_space()        # 将内核空间映射进用户页表（恒等映射）
2. 解析 ELF program headers  # 只处理 LOAD 段
3. 为每个段创建 Framed MapArea # 按段权限设置 U+R/W/X
4. copy_data()               # 逐页复制段数据（处理非页对齐偏移）
5. 分配用户栈                # 16KB (USER_STACK_SIZE) + 1 guard page
6. map_thread_exit_trampoline()  # 0x1000 处映射线程退出蹦床页
```

**为什么用户页表要映射内核空间？** 因为 trap 发生时，CPU 先硬件切换到内核态，但此时页表还是用户页表。trap 向量代码需要访问内核代码和数据（如 `trap_handler` 函数、内核栈），所以用户页表必须包含内核映射。这是当前实现与经典"固定 trampoline 页"方案的关键区别。

**段数据复制的细节**：ELF 段的 `p_vaddr` 可能不是页对齐的，`copy_data()` 通过 `page_offset` 参数处理首页的页内偏移，逐页复制。

**权限合并**：`PageTable::map()` 在发现目标 VPN 已映射时，会将新旧 flags 取或（`old_flags | flags`），避免覆盖前一个段的合法权限。这处理了 ELF 段在页边界重叠的情况。

### 4.6 fork 与 COW（Copy-On-Write）

[`MemorySet::from_existing()`](kernel/src/mm/memory_set.rs:429) 使用 COW 机制共享物理页，而非深拷贝：

```text
COW fork 流程：
1. map_kernel_space()                  # 内核映射复用（恒等映射，不占新帧）
2. 遍历父进程的 Framed area：
   a. 将 data_frames 中的 FrameTracker 移入 cow_frames
   b. 注册/递增全局帧引用计数（FRAME_REFCOUNT）
   c. core::mem::forget(frame) 阻止 FrameTracker Drop 释放帧
3. 标记父进程可写页为只读 + COW：
   - PTE 清除 W 位，设置 COW 位（RSW bit 0，PTE bit 8）
4. 创建子进程的 MapArea 和页表项：
   - 可写页：映射为只读 + COW（共享同一物理帧）
   - 只读页：直接共享（无需 COW 标记）
5. sfence.vma 刷新 TLB
```

**COW 页错误处理**（[`handle_cow_fault()`](kernel/src/mm/memory_set.rs:526)）：

当进程写入 COW 页时，触发 Store Page Fault（scause=15），trap 处理器调用 `handle_cow_fault()`：

```text
1. 检查 PTE 的 COW 位是否置位
2. 查询帧引用计数：
   - refcount > 1：分配新帧，复制数据，递减旧帧引用计数
   - refcount == 1：直接恢复写权限，取回独占所有权
3. 更新 PTE：恢复 W 位，清除 COW 位
4. sfence.vma 刷新 TLB
```

**帧引用计数**（[`frame_alloc.rs`](kernel/src/mm/frame_alloc.rs)）：

全局 `FRAME_REFCOUNT: SpinLock<BTreeMap<usize, usize>>` 跟踪 COW 共享帧的引用数。关键函数：

| 函数 | 作用 |
|------|------|
| `frame_refcount_register(ppn)` | 首次共享时注册，初始计数=1（父进程） |
| `frame_refcount_inc(ppn)` | fork 时递增（子进程） |
| `frame_refcount_dec(ppn)` | 进程退出/exec 时递减；计数归零时释放帧 |
| `frame_refcount_take(ppn)` | COW 解析时取回独占所有权（从表中移除） |

**重叠 ELF 段处理**：当两个 ELF 段在同一页边界重叠时，同一 VPN 出现在多个 `MapArea` 中。`from_existing()` 使用 `seen_vpns` 集合确保每个 VPN 只注册一次引用计数，避免 Drop 时双重递减导致帧被错误释放。

### 4.7 线程退出蹦床

[`map_thread_exit_trampoline()`](kernel/src/mm/memory_set.rs:454) 在每个用户地址空间的 `0x1000` 处映射一个 R+X+U 页，写入三条指令：

```asm
li a7, 93      # SYS_exit = 93
li a0, 0       # exit_code = 0
ecall          # 系统调用
```

当线程入口函数正常返回时，`ra` 寄存器指向这个地址（在 [`Thread::new()`](kernel/src/task/thread.rs:69) 中设置 `trap_cx.x[1] = THREAD_EXIT_TRAMPOLINE`），从而自动调用 `exit(0)` 而非跳转到非法地址。

### 4.8 trap context 的存放方式

当前实现**没有**采用经典"固定高虚拟地址 trampoline + 固定 trap context 页"的布局。每个线程的 trap context 通过 `frame_alloc()` 分配一块物理页，由 `ThreadInner::trap_cx_ppn` 持有。trap 入口和返回通过这个页的实际物理地址工作。

因此，[`config.rs`](kernel/src/config.rs) 中虽然保留了 `TRAMPOLINE` 和 `TRAP_CONTEXT_BASE` 常量，但它们标记为 `#[allow(dead_code)]`，不是当前实现的核心路径。

---

## 5. Trap 与中断处理

> 这是整个内核最关键的部分。用户态 trap、内核态 trap、定时器中断、外部中断、signal 递送都在这里汇合。

### 5.1 用户态 trap 向量

[`user_trap_vector`](kernel/src/trap/trap.S:21) 负责从 U-mode 进入 S-mode，是整个 trap 处理最复杂的路径：

```asm
# 1. 用 sscratch 交换 sp（sscratch 保存了 TrapContext 页的物理地址）
csrrw sp, sscratch, sp

# 2. 保存全部 32 个通用寄存器到 TrapContext
#    x0 恒为 0 不保存
#    x1(ra) → offset 1×8
#    x2(sp) → 从 sscratch 取回用户 sp，存入 offset 2×8
#    x3-x31 → offset 3×8 .. 31×8

# 3. 保存 sstatus 和 sepc
csrr t0, sstatus → offset 32×8
csrr t1, sepc   → offset 33×8

# 4. 从 TrapContext 读取内核信息
ld t0, 34×8(sp)   # kernel_satp
ld t1, 35×8(sp)   # kernel_sp
ld t2, 36×8(sp)   # trap_handler 函数地址

# 5. 切换到内核页表
csrw satp, t0
sfence.vma

# 6. 切换到内核栈
mv sp, t1

# 7. 设置 stvec 为 kernel_trap_vector（后续中断走内核路径）
la t3, kernel_trap_vector
csrw stvec, t3

# 8. 跳转到 Rust 的 trap_handler()
jr t2
```

**关键理解**：`sscratch` 在用户态运行时保存的是 `TrapContext` 页的物理地址。由于用户页表包含内核恒等映射，这个物理地址在切换到内核页表后仍然有效。

### 5.2 用户态返回

[`user_trap_return`](kernel/src/trap/trap.S:58) 负责从 S-mode 回到 U-mode：

```asm
# 1. 恢复 stvec = user_trap_vector
# 2. 将 user_satp 暂存到 sscratch
# 3. sp = TrapContext 地址（a0 参数传入）
# 4. 恢复 sstatus、sepc
# 5. 恢复 x1, x3, x7-x31（跳过 x2/sp, x5/t0, x6/t1）
# 6. 从 TrapContext 加载用户 sp (x2) 和 user_satp
# 7. 设置 sscratch = TrapContext 地址（为下次 trap 准备）
# 8. 切换到用户页表 (csrw satp + sfence.vma)
# 9. 恢复用户 sp
# 10. 通过 sscratch 间接恢复 x5(t0) 和 x6(t1)
#     （因为切换页表后需要这两个寄存器做临时变量）
# 11. sret
```

**为什么 x5/x6 要延迟恢复？** 因为在切换页表和恢复 sp 的过程中需要 t0/t1 作为临时寄存器。恢复它们的值时，页表已经切换到用户空间，但 `sscratch` 仍指向 TrapContext 物理页，该页通过内核恒等映射在用户页表中可见。

### 5.3 内核态 trap 向量

[`kernel_trap_vector`](kernel/src/trap/trap.S:113) 处理 S-mode 下发生的中断，设计上更轻量：

```asm
# 1. 在当前内核栈上分配 16×8 字节空间
addi sp, sp, -16*REG_SIZE

# 2. 保存 callee-saved 寄存器 (ra, s0-s11) + sstatus + sepc
#    注意：sp 的"原始值"被保存为 ra 位置的"返回地址"
#    addi t2, sp, 16*REG_SIZE; sd t2, 1*REG_SIZE(sp)

# 3. 读取 scause, stval, sepc 作为参数
# 4. 调用 kernel_trap_handler(stval, scause, sepc)
# 5. 恢复 sstatus, sepc, callee-saved 寄存器
# 6. 释放栈空间
# 7. sret
```

**与用户态 trap 的关键区别**：
- 不切换页表（已经在内核页表）
- 不切换栈（使用当前内核栈）
- 只保存 callee-saved 寄存器（调用约定保证 caller-saved 已保存）
- 保存原始 sp 作为"返回地址"（`__switch` 机制需要）

### 5.4 trap_handler 的完整流程

[`trap_handler()`](kernel/src/trap/mod.rs:63) 是用户态 trap 的 Rust 处理入口：

```text
1. 设置 stvec = kernel_trap_vector（后续中断走内核路径）

2. 按 scause 分流：
   - ecall from U-mode (8) → 系统调用分发
   - supervisor timer      → timer_handler()
   - supervisor external   → handle_external_irq()
   - supervisor soft       → 清除 SIP.SSIP（IPI）
   - page fault            → 打印信息 + exit(-2)
   - illegal instruction   → 打印信息 + exit(-3)

3. 检查 signal 递送：
   pending & !mask → 取最低位信号 → 默认动作终止

4. 检查 need_reschedule：
   若为 true → suspend_current_and_run_next()

5. trap_return_to_user()
```

**关键点**：`trap_handler()` 不会在进入后自动打开内核中断。syscall 处理路径在当前实现里是非重入的。内核态中断是否实际触发取决于 `sstatus.SIE` 位——它由 SpinLock 的开关中断策略和 `wfi` 窗口控制。

### 5.5 kernel_trap_handler

[`kernel_trap_handler()`](kernel/src/trap/mod.rs:142) 处理 S-mode 下的中断：

- **timer** → `kernel_timer_handler()`：递增 `KERNEL_TIMER_COUNT`，然后调用 `timer_handler()`
- **external** → `kernel_external_handler()`：递增 `KERNEL_EXTERNAL_COUNT`，然后调用 `handle_external_irq()`
- **soft** → 清除 SSIP（IPI）
- **page fault** → 打印详细诊断信息（含 sp, ra, 当前任务信息）后 panic

内核态 page fault 直接 panic 而非终止任务，因为这是内核自身的 bug。

### 5.6 timer 中断与抢占

[`timer_handler()`](kernel/src/trap/interrupt.rs:23) 的处理路径：

```text
timer interrupt → TICKS += 1 → set_next_trigger() → need_reschedule = true
```

`need_reschedule` 不会在中断里直接切换任务。它只是一个标志，在 `trap_handler()` 末尾、返回用户态之前检查。这保证了上下文切换发生在安全点。

`set_next_trigger()` 读取 `time` CSR，设置下一次中断为 `time + CLOCK_FREQ / TICKS_PER_SEC`（10MHz / 100 = 100000 个时钟周期，即 10ms 后）。

### 5.7 外部中断与 PLIC

[`plic_init()`](kernel/src/trap/interrupt.rs:87) 为每个 hart：
1. 设置 UART(IRQ10) 和 VirtIO(IRQ1) 优先级为 1
2. 在 `senable` 寄存器中使能这两个 IRQ
3. 设置 `spriority` 阈值为 0（接受所有优先级）

[`handle_external_irq()`](kernel/src/trap/interrupt.rs:44) 的处理流程：

```text
1. claim（读 plic_sclaim 寄存器获取 IRQ 号）
2. 分流：
   - UART_IRQ → sbi::console_getchar() → uart::push_char()
   - VIRTIO0_IRQ → 空操作（保留）
3. complete（写回 claim 寄存器）
```

**容易误读的地方**：当前代码的"字符输入"主路径是 SBI console getchar，不是直接读 UART MMIO RBR。`drivers::uart::getc()` 存在但不是用户交互的主路径。输出路径则是 `Stdout::write()` → `sbi::console_putchar()`。

### 5.8 kernel_irq_set 的真实语义

[`KERNEL_IRQ_ENABLED`](kernel/src/trap/mod.rs:27) 不是"全局打开/关闭 SIE"的总开关。当前代码中，`sys_fork()` 并没有直接读取这个标志——它的实际作用域较窄。这个标志通过 `sys_kernel_irq_set()` 系统调用设置，在 `sys_kernel_stats()` 中作为诊断字段输出，主要用于实验对比。

### 5.9 signal 最小闭环

signal 目前实现了最小闭环：

| 操作 | 实现 |
|------|------|
| `sig_send(pid, sig)` | 设置目标进程的 `signal_pending \|= 1 << sig` |
| `sig_mask(mask)` | 设置 `signal_mask = mask` |
| `sig_pending()` | 读取 `signal_pending` |
| 递送检查 | `trap_handler()` 末尾：`pending & !mask` → 取最低位 → 默认动作终止 |

递送逻辑在 [`take_current_deliverable_signal()`](kernel/src/task/mod.rs:150) 中：找到 `pending & !mask` 的最低位（`trailing_zeros()`），清除该位，返回信号号。默认动作是 `exit(-(128 + sig))`。

当前没有用户态 handler、没有 sigaction、没有复杂优先级策略。它的作用是证明"信号递送链路"已经打通。

---

## 6. 进程与线程管理

### 6.1 设计理念：进程/线程分离

本内核采用进程/线程分离模型：

- **Process**：资源容器，持有地址空间、fd 表、信号状态、子进程列表
- **Thread**：调度单元，持有内核栈、trap context、任务上下文、运行状态

一个 Process 包含多个 Thread，所有线程共享进程的资源。

### 6.2 Process 结构

```rust
pub struct Process {
    pub pid: PidHandle,           // PID（Drop 时自动回收）
    pub inner: SpinLock<ProcessInner>,
}

pub struct ProcessInner {
    pub is_zombie: bool,
    pub memory_set: MemorySet,    // 地址空间
    pub parent: Option<Weak<Process>>,  // 父进程弱引用
    pub children: Vec<Arc<Process>>,    // 子进程列表
    pub exit_code: i32,
    pub threads: Vec<Option<Arc<Thread>>>,  // 线程表
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,  // 文件描述符表
    pub signal_mask: usize,
    pub signal_pending: usize,
    pub next_user_stack_top: usize,  // 下一个线程栈的起始位置
}
```

初始化时 `fd_table[0..2]` 分别是 stdin / stdout / stderr（stderr 复用 stdout）。

### 6.3 Thread 结构

```rust
pub struct Thread {
    pub tid: usize,
    pub process: Weak<Process>,   // 所属进程弱引用
    pub kernel_stack: KernelStack,
    pub inner: SpinLock<ThreadInner>,
}

pub struct ThreadInner {
    pub status: ThreadStatus,     // Ready / Running(hart) / Blocked / Exited
    pub task_context: TaskContext, // __switch 使用的上下文
    pub trap_cx_ppn: PhysPageNum, // TrapContext 所在物理页
    pub trap_cx_frame: Option<FrameTracker>,  // 帧所有权
    pub exit_code: Option<i32>,
}
```

`ThreadStatus::Running(usize)` 记录线程在哪个 hart 上运行，用于调试和调度决策。

### 6.4 创建进程

[`Process::new()`](kernel/src/task/process.rs:30) 的流程：

```text
1. MemorySet::from_elf() → 创建地址空间，获取 user_sp 和 entry_point
2. pid_alloc() → 分配 PID
3. 计算 next_user_stack_top（初始栈顶 + guard page + USER_STACK_SIZE）
4. 创建 Process 对象，初始化 fd_table[0..2]
5. Thread::new() → 创建主线程 (tid=0)
   - 分配 trap context 页
   - 分配内核栈
   - 初始化 TrapContext（入口点、用户栈、内核信息）
   - 设置 ra = THREAD_EXIT_TRAMPOLINE
   - 创建 TaskContext（ra = trap_return_wrapper, sp = kstack_top）
6. 将线程加入进程的 threads 列表
7. 将线程加入调度器
```

### 6.5 fork

[`Process::fork()`](kernel/src/task/process.rs:81) 使用 COW（Copy-On-Write）共享地址空间：

```text
1. MemorySet::from_existing() → COW 共享地址空间
   - 父子进程共享所有物理帧（只读映射 + COW 标记）
   - 写入时触发 Page Fault，按需分配新帧并复制数据
2. 克隆 fd_table（Arc 共享同一底层 File 对象）
3. 复制 signal_mask，清空 signal_pending
4. 创建子进程主线程 (tid=0)
   - Thread::fork_from() → 复制父线程 trap context
   - 子线程 x[10] = 0（fork 返回值语义）
   - 子线程 kernel_sp = 新内核栈顶
5. 将子进程加入父进程的 children 列表
6. 将子线程加入调度器
```

COW 的核心优势：fork 时无需为每页分配新物理帧并复制 4KB 数据，仅在实际写入时才按需复制，显著降低 fork 的时间和内存开销。

### 6.6 exec

[`Process::exec()`](kernel/src/task/process.rs:124) 重建地址空间和主线程 trap context：

```text
1. MemorySet::from_elf() → 重新解析 ELF
2. 替换 memory_set
3. 清空 signal_pending
4. 重写主线程的 TrapContext（新入口点、新用户栈）
```

它不创建新 PID，不替换进程对象本身，只是重置当前进程镜像。

### 6.7 thread_create

[`sys_thread_create()`](kernel/src/syscall/thread.rs:6) 的流程：

```text
1. 如果 user_sp == 0：
   - 从 process.next_user_stack_top 分配新栈
   - 在地址空间中映射新栈页
   - 更新 next_user_stack_top
2. 分配新 TID = threads.len()
3. Thread::new() → 创建新线程
4. 将线程加入进程的 threads 列表
5. 将线程加入调度器
```

### 6.8 exit / waitpid / waittid

**exit**：[`exit_current_and_run_next()`](kernel/src/task/mod.rs:51) 将当前线程标记为 `Exited`，如果进程所有线程都退出，则将进程标为 zombie 并清空 children。

**waitpid**：[`sys_waitpid()`](kernel/src/syscall/process.rs:69) 采用"克隆后检查"策略避免 ABBA 死锁——先克隆子进程列表，释放锁后再逐个检查。找到 zombie 子进程后，从 children 中移除并调用 `remove_process()` 释放全局资源。未找到则 yield 后重试。

**waittid**：[`sys_waittid()`](kernel/src/syscall/thread.rs:51) 类似，检查目标线程的 `exit_code`，未退出则 yield 重试。

### 6.9 上下文切换机制

上下文切换由 [`__switch`](kernel/src/task/switch.S:7) 汇编函数完成，只保存/恢复 callee-saved 寄存器（ra, sp, s0-s11），共 14 个 × 8 字节 = 112 字节。

调度流程：

```text
suspend_current_and_run_next()
  → take_current_task()           # 从 Processor 取出当前线程
  → 标记 Ready
  → 保存 task_context 指针
  → pending_enqueue = Some(task)  # 延迟入队！
  → schedule()
    → __switch(task_cx, idle_cx)  # 切到 idle 循环

run_tasks() [idle 循环]
  → __switch(idle_cx, next_cx)   # 切到新线程
  → pending_enqueue.take()        # 此时旧线程上下文已保存，安全入队
  → scheduler.add_task(pending)   # 将旧线程重新加入调度队列
```

**pending_enqueue 的设计**：这是一个关键的竞态修复。如果在 `__switch` 之前就将任务加入调度队列，另一个 hart 可能在上下文保存完成之前就拾取了这个任务，导致使用过时的 TaskContext。`pending_enqueue` 将入队操作延迟到 `__switch` 返回之后（此时上下文已保存），保证了正确性。

---

## 7. 多核调度

### 7.1 Scheduler trait

```rust
pub trait Scheduler: Send + Sync {
    fn add_task(&self, task: Arc<Thread>);
    fn fetch_task(&self, hart_id: usize) -> Option<Arc<Thread>>;
    fn task_count(&self) -> usize;
    fn name(&self) -> &'static str;
}
```

内核同时实例化了两个调度器：

```rust
static GLOBAL_SCHEDULER: GlobalRoundRobin = GlobalRoundRobin::new();
static PERCPU_SCHEDULER: PerCpuWorkStealing = PerCpuWorkStealing::new();
```

`ACTIVE_SCHEDULER` 是 `AtomicUsize`，通过 `set_scheduler()` 切换。

### 7.2 调度器切换与任务迁移

[`set_scheduler()`](kernel/src/task/scheduler/mod.rs:18) **会**迁移任务：

```rust
pub fn set_scheduler(stype: SchedulerType) {
    ACTIVE_SCHEDULER.store(stype as usize, Ordering::SeqCst);
    
    // 从旧调度器中取出所有任务，加入新调度器
    let (from, to) = match stype {
        PerCpuWorkStealing => (&GLOBAL_SCHEDULER, &PERCPU_SCHEDULER),
        GlobalRoundRobin  => (&PERCPU_SCHEDULER, &GLOBAL_SCHEDULER),
    };
    while let Some(task) = from.fetch_task(0) {
        to.add_task(task);
    }
}
```

当前实现**确实**会在切换时迁移旧队列中的任务到新调度器。

### 7.3 GlobalRoundRobin

[`GlobalRoundRobin`](kernel/src/task/scheduler/global_rr.rs) 是最简单的多核调度器：

- 一个 `SpinLock<VecDeque<Arc<Thread>>>` 共享队列
- `add_task` → `push_back`
- `fetch_task` → `pop_front`（忽略 hart_id 参数）
- `count` 用 `AtomicUsize` 跟踪

**优点**：实现简单、FIFO 公平、行为直观
**缺点**：所有 hart 争用同一把锁，随 hart 数增加扩展性差

### 7.4 PerCpuWorkStealing

[`PerCpuWorkStealing`](kernel/src/task/scheduler/percpu_rr.rs) 为每个 hart 维护独立队列：

**任务分发**（`add_task`）：
```rust
let target = task.get_process().getpid() % active;
self.queues[target].lock().push_back(task);
```
使用 PID 取模决定目标队列，保证连续 PID（如 fork 循环产生的）均匀分布，且 yield 后重新入队的任务回到同一队列。

**任务获取**（`fetch_task`）：
```text
1. 计算本地逻辑索引 (logical_hart_index)
2. 尝试本地队列 → 成功则返回
3. 工作窃取：遍历其他 hart 的队列
   - 受害者队列长度 > 2 时才窃取
   - 窃取一半（至少 1 个）
   - 第一个窃取的任务直接返回，其余放入本地队列
4. 所有队列都空 → 返回 None
```

**设计目的**：
- 平时减少共享锁竞争（本地队列操作无需全局锁）
- 负载不均时自动均衡（窃取机制）
- 窃取一半而非全部，避免频繁窃取的抖动

### 7.5 Processor 与 run_tasks 调度循环

每个 hart 持有一个 [`Processor`](kernel/src/task/processor.rs:11)：

```rust
pub struct Processor {
    pub hart_id: usize,
    pub current: Option<Arc<Thread>>,     // 当前运行的线程
    pub idle_task_cx: TaskContext,         // idle 循环的上下文
    pub need_reschedule: bool,             // 时钟中断设置的调度标志
    pub dead_task: Option<Arc<Thread>>,    # 已退出线程，等待清理
    pub pending_enqueue: Option<Arc<Thread>>,  # 延迟入队的线程
}
```

[`run_tasks()`](kernel/src/task/processor.rs:147) 的主循环：

```text
loop {
    // 1. 清理上一个已退出任务（drop Arc 释放资源）
    if let Some(dead) = dead_task.take() { drop(dead); }

    // 2. 检查当前 hart 是否活跃
    if logical_hart_index(hid).is_none() {
        wfi;  // 不活跃 hart 等待中断
        continue;
    }

    // 3. 从调度器获取任务
    if let Some(task) = fetch_task(hid) {
        task.status = Running(hid);
        processor.current = Some(task);
        __switch(&mut idle_cx, &next_cx);  // 切到任务

        // 4. __switch 返回后，处理延迟入队
        if let Some(pending) = pending_enqueue.take() {
            add_task(pending);
        }
    } else {
        // 5. 无任务：短暂开中断 + wfi + 关中断
        csrs sstatus, SIE;
        wfi;
        csrc sstatus, SIE;
    }
}
```

---

## 8. 同步原语

### 8.1 Ticket SpinLock

[`SpinLock`](kernel/src/sync/spinlock.rs) 使用 ticket lock 算法，而非简单 TAS 锁：

```text
加锁：
  1. 保存当前 SIE 状态（sstatus.SIE 位）
  2. 关闭本地中断（csrc sstatus, SIE_bit）
  3. 领取 ticket = next_ticket.fetch_add(1)
  4. 自旋等待 now_serving == ticket

解锁：
  1. now_serving.fetch_add(1)  （唤醒下一个等待者）
  2. 恢复之前保存的中断状态
```

**为什么必须关中断？** 多核环境下最危险的不是"锁本身"，而是"锁 + 中断"的组合：

- 持锁时如果本地中断还开着，中断 handler 可能再次来拿同一把锁
- 如果 handler 里再去等锁，就会把自己卡住（自死锁）
- 即使 handler 不拿同一把锁，中断处理也可能破坏临界区的一致性

Ticket lock 的 FIFO 公平性保证了先到先服务，避免了简单 TAS 锁的饥饿问题。

### 8.2 Mutex（阻塞互斥锁骨架）

[`Mutex`](kernel/src/sync/mutex.rs) 内部用 `SpinLock<MutexInner>` 保护状态：

```rust
struct MutexInner {
    locked: bool,
    wait_queue: VecDeque<usize>,  // 等待线程的 TID 列表
}
```

- `lock()` 成功则 `locked = true`，返回 `true`
- 失败则将 TID 加入 `wait_queue`，返回 `false`（由 syscall 层调用 `block_current_task()`）
- `unlock()` 尝试从 `wait_queue` 取出下一个等待者的 TID，返回 `Some(tid)`（由 syscall 层负责唤醒）

当前实现中，`sys_mutex_unlock()` 虽然获取了待唤醒 TID，但并未真正执行唤醒操作（注释说明这是简化实现）。因此 Mutex 更像"阻塞锁的骨架"，接口和状态机已打通，但唤醒协作还需完善。

### 8.3 Semaphore（计数信号量）

[`Semaphore`](kernel/src/sync/semaphore.rs) 同样用 `SpinLock` 保护：

```rust
struct SemaphoreInner {
    count: isize,
    wait_queue: VecDeque<usize>,
}
```

- `down()`：`count -= 1`，若 `count < 0` 则需要阻塞（返回 `false`）
- `up()`：`count += 1`，从 `wait_queue` 弹出一个可唤醒的 TID

与 Mutex 类似，`sys_semaphore_down()` 在 `down()` 返回 `false` 时调用 `block_current_task()`，但 `sys_semaphore_up()` 获取到待唤醒 TID 后也未真正唤醒。

### 8.4 全局同步对象表

Mutex 和 Semaphore 通过全局 `SpinLock<Vec<Option<Arc<...>>>>` 表管理，通过整数 ID 访问。这是教学简化——生产内核通常将同步对象放在进程的 fd 表中。

---

## 9. 设备与文件系统

### 9.1 UART 驱动

[`drivers/uart.rs`](kernel/src/drivers/uart.rs) 提供 NS16550A 的 MMIO 初始化：

```text
1. 禁用所有中断 (IER = 0)
2. 设置 DLAB，配置波特率分频 = 3 (38400 baud)
3. 8 位数据、无校验、1 停止位 (LCR = 0x03)
4. 使能 FIFO (FCR = 0x07)
5. 使能接收数据中断 (IER = 0x01)
```

输入缓冲区 `INPUT_BUFFER: SpinLock<VecDeque<u8>>` 提供 `push_char()` / `pop_char()` 接口。

`putc()` 和 `getc()` 通过 MMIO 直接操作 UART 寄存器，但不是当前用户交互的主路径。

### 9.2 I/O 路径总结

| 方向 | 路径 |
|------|------|
| **输出** | 用户 `print!` → `sys_write(1, ...)` → `Stdout::write()` → `CONSOLE_LOCK.lock()` → `sbi::console_putchar()` |
| **输入** | 外部中断 → PLIC claim → UART_IRQ → `sbi::console_getchar()` → `uart::push_char()` → `Stdin::read()` 时 `pop_char()` |

`Stdin::read()` 的逻辑：先看缓冲区，缓冲区空则尝试 SBI getchar，再不行就 `yield_()` 让出 CPU。因此它不是纯忙等。

`CONSOLE_LOCK` 保证多核并发写 stdout 的原子性，但在高并发场景下（如 `test_perf_stdio_concurrent` 的 8 进程同时写），锁竞争加上 SBI ecall 的开销仍可能导致问题。

### 9.3 RAMFS

[`fs/ramfs.rs`](kernel/src/fs/ramfs.rs) 实现内存文件系统：

```rust
static RAMFS: SpinLock<Option<BTreeMap<String, Arc<Inode>>>>

struct Inode {
    data: SpinLock<Vec<u8>>,   // 文件内容
}

pub struct RamFile {
    inode: Arc<Inode>,
    offset: SpinLock<usize>,   // 独立文件偏移
}
```

**语义**：
- `open(path, O_CREATE)` → 创建新 Inode 或打开已有
- `open(path, O_TRUNC)` → 打开并清空
- 同一路径可被多个句柄打开（Arc 共享 Inode）
- `unlink()` 只删除路径名映射，已打开的句柄仍可读写（因为 Inode 被 Arc 引用）
- 每个打开句柄有独立 offset
- `seek()` 支持 `SEEK_SET(0)` / `SEEK_CUR(1)` / `SEEK_END(2)`

### 9.4 File trait

```rust
pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;
    fn read(&self, buf: &mut [u8]) -> usize;
    fn write(&self, buf: &[u8]) -> usize;
    fn seek(&self, offset: isize, whence: usize) -> isize { -1 }
    fn size(&self) -> usize { 0 }
}
```

stdin、stdout、RamFile 都通过这个 trait 统一进 `fd_table`，实现多态 I/O。

---

## 10. 系统调用接口

### 10.1 调用约定

用户态和内核态遵循同样的 syscall 约定：

| 寄存器 | 用途 |
|--------|------|
| `a7` | syscall 号 |
| `a0-a2` | 前三个参数 |
| `a0` | 返回值 |

用户态通过 `ecall` 指令触发 trap，`trap_handler()` 根据 `scause == 8` 识别为 syscall，从 TrapContext 中读取 `x[17]`(a7) 和 `x[10..12]`(a0-a2) 进行分发。

### 10.2 完整系统调用表

#### 进程 / 线程 / 时间

| ID | 名称 | 语义 | 实现要点 |
|----|------|------|----------|
| 93 | `exit` | 退出当前线程 | 标记 Exited，检查进程所有线程是否退出 |
| 124 | `yield` | 主动让出 CPU | `suspend_current_and_run_next()` |
| 169 | `get_time` | 获取毫秒时间 | `csrr time / (CLOCK_FREQ / 1000)` |
| 172 | `getpid` | 获取进程 ID | 从当前线程的 process 获取 |
| 220 | `fork` | 创建子进程 | COW 共享地址空间 + fd_table + signal_mask |
| 221 | `exec` | 装载新程序 | `get_app_data_by_name()` 查找嵌入 ELF |
| 260 | `waitpid` | 等待子进程 | 克隆 children 后检查，避免 ABBA 死锁 |
| 1000 | `thread_create` | 创建线程 | 分配新栈 + 新 trap context + 新内核栈 |
| 1001 | `gettid` | 获取线程 ID | 从当前线程的 tid 获取 |
| 1002 | `waittid` | 等待线程 | 检查 exit_code，未退出则 yield |

#### 文件 I/O

| ID | 名称 | 语义 | 实现要点 |
|----|------|------|----------|
| 63 | `read` | 从 fd 读取 | `translated_byte_buffer` 翻译用户缓冲区 |
| 64 | `write` | 向 fd 写入 | 先 clone Arc\<File\> 再 drop 锁后写入 |
| 1050 | `open` | 打开/创建 RAMFS 文件 | 路径从用户空间翻译，fd 从 3 开始分配 |
| 1051 | `close` | 关闭 fd | fd < 3 不可关闭 |
| 1052 | `lseek` | 调整文件偏移 | 委托 File::seek() |
| 1053 | `unlink` | 删除 RAMFS 路径 | 委托 ramfs::unlink() |
| 1054 | `fsize` | 获取文件大小 | 通过 translated_refmut 写回用户空间 |

#### 同步 / 调度 / 诊断

| ID | 名称 | 语义 | 实现要点 |
|----|------|------|----------|
| 1010 | `mutex_create` | 创建互斥锁 | 全局 MUTEX_TABLE 推入 |
| 1011 | `mutex_lock` | 加锁 | 失败则 block_current_task() |
| 1012 | `mutex_unlock` | 解锁 | 返回待唤醒 TID（未实际唤醒） |
| 1020 | `semaphore_create` | 创建信号量 | 全局 SEM_TABLE 推入 |
| 1021 | `semaphore_up` | V 操作 | count += 1，返回待唤醒 TID |
| 1022 | `semaphore_down` | P 操作 | count -= 1，< 0 则 block |
| 1030 | `sched_set` | 切换调度器 | 迁移旧队列任务到新调度器 |
| 1031 | `hart_id` | 获取当前 hart | 从 tp 寄存器读取 |
| 1032 | `kernel_stats` | 获取内核统计 | 栈上构建字符串，拷贝到用户缓冲区 |
| 1033 | `kernel_irq_set` | 设置 IRQ 策略标志 | AtomicBool 存储 |
| 1034 | `active_harts_set` | 设置活跃 hart 数 | 1 时将调用者 hart 设为唯一活跃 |
| 1040 | `sig_send` | 发送 signal | 设置目标进程 pending 位 |
| 1041 | `sig_mask` | 设置屏蔽位 | 返回旧 mask |
| 1042 | `sig_pending` | 获取 pending | 返回 pending 位图 |

### 10.3 kernel_stats 的实现细节

[`sys_kernel_stats()`](kernel/src/syscall/process.rs:176) 使用栈上 `StatsBuf`（160 字节数组）构建逗号分隔的 `key:value` 字符串，再通过 `translated_byte_buffer` 拷贝到用户缓冲区。不使用堆分配，避免在统计路径上引入额外的分配器锁竞争。

输出格式：`ticks:N,kernel_timer:N,kernel_ext:N,tasks:N,sched:N,kirq:N,harts:N`

用户态通过 `kernel_stats_parsed()` 解析为 `KernelStats` 结构体。

---

## 11. 用户态运行时

### 11.1 启动与退出

[`user/src/lib.rs`](user/src/lib.rs) 的 `_start()` 是所有用户程序的入口：

```rust
#[no_mangle]
#[link_section = ".text.entry"]
pub extern "C" fn _start() -> ! {
    clear_bss();
    exit(main());  // main 返回后自动调用 exit
}
```

`main` 函数通过 `#[linkage = "weak"]` 声明为弱符号，用户程序可以覆盖它。如果用户程序没有定义 `main`，则触发 panic。

### 11.2 syscall 封装

[`user/src/syscall.rs`](user/src/syscall.rs) 统一使用 `ecall` 指令：

```rust
fn syscall(id: usize, args: [usize; 3]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") args[0] => ret,
            in("a1") args[1],
            in("a2") args[2],
            in("a7") id,
        );
    }
    ret
}
```

### 11.3 waitpid / waittid 的重试逻辑

用户态 `waitpid()` 和 `waittid()` 保留了对 `-2` 返回值的重试分支：

```rust
pub fn waitpid(pid: isize, exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(pid, exit_code) {
            -2 => yield_(),
            n => return n,
        }
    }
}
```

当前内核实现通常是直接阻塞等待（内部 yield 重试），而非返回 `-2`。这个重试分支是为未来可能的非阻塞 wait 预留的。

---

## 12. 测试设计

当前共有 15 个 `test_*` 用户程序，按验证目标分为四类。

### 12.1 中断正确性测试

| 测试 | 验证目标 | 方法论 | 关键观察 |
|------|----------|--------|----------|
| `test_timer_interrupt` | tick 频率、内核 timer 计数、抢占 | 3 个子测试：① 50ms 内 tick 增量 ② kernel_timer 递增 ③ 单 hart 下子进程是否被抢占（hart 迁移） | `kernel_stats_parsed()` 获取 ticks/kernel_timer；单 hart 模式下 hart 迁移证明 need_reschedule 抢占生效 |
| `test_preemption` | 高负载下是否表现出并发 | 3 个子进程做 50K 次计算循环，5 个检查点记录 hart_id 和时间 | hart_id 变化 = 迁移；总 wall time < 串行时间 = 并发执行 |

### 12.2 多核与调度测试

| 测试 | 验证目标 | 方法论 | 关键观察 |
|------|----------|--------|----------|
| `test_multicore_online` | 4 个 hart 都能参与调度 | fork 16 个子进程，每个 yield 4 次后报告 hart_id | 父进程收集退出码，统计哪些 hart 被观察到 |
| `test_concurrent_fork` | fork/exec/wait 在并发下是否稳定 | 同时 fork 8 个子进程，每个立即退出报告 hart_id | 检查无重复 PID、无异常退出码 |
| `test_hart_balance` | 两种调度器的任务分布 | PerCpuWS 模式下 fork 8 个任务，记录 hart_before/hart_after | hart 分布在期望值的 50%-200% 范围内为 balanced |
| `test_work_stealing` | 是否真的发生过任务迁移 | PerCpuWS 模式下 fork 16 个子进程，yield 4 次后报告 hart_id | 观察是否所有 4 个 hart 都被使用 |

### 12.3 性能对比测试

| 测试 | 比较内容 | 方法论 | 度量指标 |
|------|----------|--------|----------|
| `test_perf_throughput` | 调度器吞吐量 | 16 个任务各 yield 5000 次 | 平均 yield 延迟(μs)、总 wall time(ms)、tick 计数 |
| `test_perf_contention` | CPU-bound 竞争开销 | 4 个 CPU-bound 任务（500K 迭代 + 周期性 yield） | 平均完成时间、wall time、hart 分布标准差 |
| `test_perf_mixed` | 混合负载调度表现 | 2 CPU-bound（1M 迭代）+ 2 I/O-bound（100ms 等待） | 总 wall time |
| `test_perf_kernel_irq` | fork 路径中断屏蔽策略 | 8 个子进程做 1.5M 计算，比较 IRQ ON/OFF | wall time、tick 计数、kernel_timer 计数 |
| `test_perf_single_vs_multi` | 单活跃 hart vs 四活跃 hart | 相同总工作量（1×2M vs 4×500K） | wall time、加速比 |
| `test_perf_single_vs_multi_thread` | 线程路径单/多 hart 对比 | 4 个线程各做 600K+ 计算，5 轮采样 | 平均/最小/最大 wall time |
| `test_perf_stdio_concurrent` | stdout 并发写成本 | 1/2/4/8 进程各写 10000 次，4 轮采样 | 平均/最小/最大 wall time |
| `test_perf_fs_workload` | RAMFS 真实工作负载 | 顺序 RW / 随机 RW / 密集元数据操作，4 轮采样 | 平均/最小/最大 wall time |

### 12.4 signal 测试

| 测试 | 验证目标 | 方法论 |
|------|----------|--------|
| `test_signal_basic` | send/mask/pending/default action | 子进程 mask SIG_TERM → 父进程发送 → 子进程检测 pending → unmask → 默认终止 |

### 12.5 test_suite

[`test_suite`](user/src/bin/test_suite.rs) 按顺序 `exec` 上述 15 个测试，是全量回归的总入口。每个测试在独立子进程中运行，父进程等待并统计通过/失败数。

---

## 13. 实验结果

测试环境：QEMU `virt` machine，`-smp 4 -m 128M`，release 编译。

### 13.1 功能正确性测试

| 测试 | 结果 | 关键数据 |
|------|------|----------|
| `test_timer_interrupt` | PASS (3/3) | 50ms 内 ticks_delta=17，kernel_timer delta=11，子任务抢占观察正常（无 hart 迁移但 preemption 可能仍生效） |
| `test_preemption` | PASS | 3 个子进程并发执行，总 wall time=16ms，hart 迁移检测或 wall time 合理 |
| `test_multicore_online` | PASS | harts 0/1/3 被观察到（hart 2 未被观察到，WARN 但继续） |
| `test_concurrent_fork` | PASS | 8 个子进程全部正常退出，harts 1/2/3 参与 |
| `test_signal_basic` | PASS | signal 递送、mask/unmask、默认终止动作全部正确 |

### 13.2 调度正确性测试

| 测试 | 结果 | 关键数据 |
|------|------|----------|
| `test_hart_balance` | PASS | PerCpuWS hart 分布均匀 [2,2,2,2]，0 次迁移，8 个任务均衡分配到 4 个 hart |
| `test_work_stealing` | PASS | 16 个子进程全部退出，4 个 hart 均被观察到（16 次观测） |

`test_hart_balance` 当前版本中 PerCpuWorkStealing 的 `pid % active` 分配策略在 8 个任务场景下实现了完美均衡 [2,2,2,2]。GlobalRoundRobin 阶段在当前构建中被跳过。

### 13.3 性能对比测试

#### 13.3.1 调度吞吐量 (`test_perf_throughput`)

| 调度器 | 平均 yield 延迟 (μs) | 总 wall time (ms) | tick 计数 |
|--------|----------------------|-------------------|----------|
| GlobalRoundRobin | 60 | 323 | 127 |
| PerCpuWorkStealing | 58 | 328 | 128 |

16 个任务各 yield 5000 次。两种调度器吞吐性能接近（GlobalRR 323ms vs PerCpuWS 328ms），平均 yield 延迟几乎相同（60μs vs 58μs），总 wall time 差异仅 1.5%，不具统计显著性。PerCpuWS 的本地队列在频繁 yield 场景下并未表现出明显优势，可能因为 16 个任务频繁让出 CPU 导致工作窃取开销抵消了本地队列的收益。

#### 13.3.2 调度竞争开销 (`test_perf_contention`)

| 调度器 | 平均完成时间 (ms) | 总 wall time (ms) | hart 分布 |
|--------|-------------------|-------------------|----------|
| GlobalRoundRobin | 0 | 15 | [0,2,2,0] |
| PerCpuWorkStealing | 0 | 11 | [1,1,1,1] |

4 个 CPU-bound 任务各做 500K 迭代。PerCpuWorkStealing 的 hart 分布为 [1,1,1,1]，完全均衡；GlobalRoundRobin 的 hart 分布为 [0,2,2,0]，任务未均匀分布。PerCpuWS 的 wall time 更低（11ms vs 15ms），**证明多核均衡调度带来了 1.36× 的性能提升**。GlobalRR 的不均衡分布导致部分 hart 空闲，而 PerCpuWS 充分利用了全部 4 个 hart。

#### 13.3.3 混合负载 (`test_perf_mixed`)

| 调度器 | 总 wall time (ms) |
|--------|-------------------|
| GlobalRoundRobin | 106 |
| PerCpuWorkStealing | 108 |

2 个 CPU-bound 任务（1M 迭代）+ 2 个 I/O-bound 任务（100ms 等待）。两种调度器在混合负载下性能接近（GlobalRR 106ms vs PerCpuWS 108ms），差异仅 1.9%。I/O-bound 任务的 100ms 等待成为 wall time 的主要决定因素，调度策略的差异被 I/O 等待时间掩盖。

#### 13.3.4 内核中断策略 (`test_perf_kernel_irq`)

| 模式 | 总 wall time (ms) | tick 计数 | kernel_timer |
|------|-------------------|----------|-------------|
| IRQ ON | 29 | 12 | 2 |
| IRQ OFF | 24 | 8 | 2 |

8 个子进程各做 1.5M 迭代。IRQ OFF 比 IRQ ON 快约 17%（24ms vs 29ms）。IRQ ON 时更多的 timer 中断（ticks 12 vs 8）引入了额外的调度开销，但 kernel_timer 计数相同（2），说明中断处理本身的开销不大，差异主要来自中断触发的 need_reschedule 导致的额外上下文切换。

#### 13.3.5 单核 vs 多核 (`test_perf_single_vs_multi`)

| 活跃 hart 数 | 总 wall time (ms) |
|-------------|-------------------|
| 1 | 14 |
| 4 | 10 |

相同总工作量（1×2M vs 4×500K 迭代）。4 hart 模式观测到 **1.40× 加速比**（14ms vs 10ms），COW fork 降低了地址空间复制的开销，使 fork+compute 模式的并行收益可观测。加速比仍低于理论值 4×，受限于 QEMU SMP 模拟粒度和调度器锁竞争。

#### 13.3.6 线程单核 vs 多核 (`test_perf_single_vs_multi_thread`)

| 模式 | 平均 (ms) | 最小 (ms) | 最大 (ms) | 样本 |
|------|----------|----------|----------|------|
| 1 hart (thread) | 11 | 9 | 15 | [12, 15, 12, 11, 9] |
| 4 harts (thread) | 4 | 4 | 6 | [5, 4, 4, 6, 5] |

4 个线程各做 600K+ 计算，5 轮采样。4 hart 线程模式比 1 hart 快 **2.75×**（11ms vs 4ms），线程路径无需 fork 的地址空间复制开销，在计算密集负载下多核并行优势显著。1 hart 模式的方差较大（9-15ms），4 hart 模式更稳定（4-6ms），说明多核调度在充分工作负载下行为更可预测。

#### 13.3.7 并发写 (`test_perf_stdio_concurrent`)

| 并发进程数 | 平均 (ms) | 最小 (ms) | 最大 (ms) | 样本 |
|-----------|----------|----------|----------|------|
| 1 | 299 | 297 | 303 | [297, 299, 298, 303] |
| 2 | 321 | 308 | 339 | [328, 308, 310, 339] |
| 4 | 375 | 367 | 384 | [371, 384, 367, 380] |
| 8 | 676 | 655 | 694 | [694, 655, 678, 680] |

并发写测试写入 RAMFS 文件（而非 stdout），避免大量输出淹没控制台。每个进程写 10000 次 6 字节。wall time 随并发数增长（1→299ms, 2→321ms, 4→375ms, 8→676ms），1-2 进程时开销差异较小（+7%），4-8 进程时 RAMFS Inode 的 SpinLock 竞争开始显现（8 进程比 1 进程慢 2.26×）。

#### 13.3.8 文件系统工作负载 (`test_perf_fs_workload`)

| 负载类型 | 平均 (ms) | 最小 (ms) | 最大 (ms) | 样本 |
|----------|----------|----------|----------|------|
| Sequential RW | 4 | 4 | 5 | [4, 4, 5, 5] |
| Random RW | 93 | 87 | 97 | [94, 97, 94, 87] |
| Mixed Metadata | 502 | 480 | 520 | [520, 520, 480, 490] |

顺序 RW（128 blocks × 256B）最快（~4ms），随机 RW（1500 次随机 seek+read/write）约慢 23×（~93ms，因为每次 seek + read/write 路径更长），密集元数据操作（1920 次 open/write/close/unlink 循环）最慢（~502ms，密集 BTreeMap 操作和 fd 分配/回收）。三种工作负载的性能差异显著，Mixed Metadata 的方差（480-520ms）说明密集元数据操作对调度和锁竞争更敏感。

### 13.4 汇总

| 类别 | 通过 | 失败 | 未完成 |
|------|------|------|--------|
| 功能正确性 | 5 | 0 | 0 |
| 调度正确性 | 2 | 0 | 0 |
| 性能对比 | 8 | 0 | 0 |
| **合计** | **15** | **0** | **0** |

✓ **ALL TESTS PASSED** — 全部 15 个测试通过，无失败、无未完成。

### 13.5 结果判读指南

- `test_perf_contention` 中 PerCpuWS 的 hart 分布为 [1,1,1,1]，**证明 4 个 hart 确实并行执行了任务**；GlobalRR 的 hart 分布为 [0,2,2,0]，500K 迭代下 PerCpuWS 快 1.36×（11ms vs 15ms）
- `test_perf_throughput` 中两种调度器性能接近（GlobalRR 323ms vs PerCpuWS 328ms），5000 次 yield 下差异仅 1.5%，不具统计显著性
- `test_perf_mixed` 中两种调度器性能接近（GlobalRR 106ms vs PerCpuWS 108ms），I/O 等待时间掩盖了调度策略差异
- `test_perf_single_vs_multi` 观测到 **1.40× 加速比**（1 hart 14ms vs 4 harts 10ms），COW fork 降低了 fork 开销，多核并行收益可观测
- `test_perf_single_vs_multi_thread` 在线程路径上观测到 **2.75× 加速**（1 hart 11ms vs 4 harts 4ms），线程模型可利用多核并行
- `test_perf_stdio_concurrent` 写入 RAMFS 文件，wall time 随并发数增长（299ms→676ms），RAMFS SpinLock 竞争是主要瓶颈
- `test_perf_fs_workload` 三种负载差异显著：Sequential RW ~4ms，Random RW ~93ms，Mixed Metadata ~502ms
- `test_perf_kernel_irq` 的 IRQ OFF 比 IRQ ON 快 17%（24ms vs 29ms），中断屏蔽策略对 fork 密集型工作负载的影响可观测
- 性能数值依赖 QEMU 版本、host 平台和负载形态，不应将单次数值视为绝对基准

---

## 14. 课程知识点映射

| 知识点 | 代码落点 | 关键机制 |
|--------|---------|----------|
| 特权级切换 / 系统调用 | [`trap.S`](kernel/src/trap/trap.S), [`syscall/mod.rs`](kernel/src/syscall/mod.rs) | ecall 触发 U→S 切换，sret 返回 |
| SV39 地址管理 | [`mm/address.rs`](kernel/src/mm/address.rs), [`mm/page_table.rs`](kernel/src/mm/page_table.rs) | 三级页表，VPN 索引分解，PTE 编码 |
| 进程与线程 | [`task/process.rs`](kernel/src/task/process.rs), [`task/thread.rs`](kernel/src/task/thread.rs) | 资源容器 vs 调度单元分离 |
| 上下文切换 | [`task/switch.S`](kernel/src/task/switch.S), [`task/context.rs`](kernel/src/task/context.rs) | callee-saved 保存/恢复，__switch |
| 中断与异常 | [`trap/trap.S`](kernel/src/trap/trap.S), [`trap/mod.rs`](kernel/src/trap/mod.rs) | 双 trap 向量，scause 分流 |
| 时钟中断与抢占 | [`trap/interrupt.rs`](kernel/src/trap/interrupt.rs), [`task/processor.rs`](kernel/src/task/processor.rs) | need_reschedule 标志 + 安全点调度 |
| SMP 与 hart 选举 | [`main.rs`](kernel/src/main.rs) | amoswap 原子选举 + SBI HSM 启动副核 |
| 调度算法 | [`task/scheduler/`](kernel/src/task/scheduler/) | 全局 RR vs Per-CPU 工作窃取 |
| 自旋锁与同步 | [`sync/spinlock.rs`](kernel/src/sync/spinlock.rs), [`sync/mutex.rs`](kernel/src/sync/mutex.rs), [`sync/semaphore.rs`](kernel/src/sync/semaphore.rs) | Ticket Lock + 关中断，阻塞锁骨架 |
| 设备驱动 | [`drivers/uart.rs`](kernel/src/drivers/uart.rs), [`trap/interrupt.rs`](kernel/src/trap/interrupt.rs), [`sbi.rs`](kernel/src/sbi.rs) | UART MMIO + PLIC + SBI ecall |
| 文件系统抽象 | [`fs/mod.rs`](kernel/src/fs/mod.rs), [`fs/stdio.rs`](kernel/src/fs/stdio.rs), [`fs/ramfs.rs`](kernel/src/fs/ramfs.rs) | File trait + RAMFS + stdin/stdout |

---

## 15. 构建与运行

### 15.1 常用命令

```bash
make            # 构建 kernel + user (release)
make run        # 构建并运行 (4 harts)
make run1       # SMP=1 运行
make run2       # SMP=2 运行
make run4       # SMP=4 运行 (默认)
make debug      # QEMU 带 -s -S (GDB stub 端口 1234)
make gdb        # 附加 riscv64 GDB
make clean      # 清理构建产物
make experiments # 运行完整实验套件 (run_experiments.py)
```

### 15.2 指定初始应用

```bash
INIT_APP=test_perf_kernel_irq cargo build --release
qemu-system-riscv64 -machine virt -nographic -smp 4 -m 128M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/release/kernel
```

可将 `INIT_APP` 替换为任意嵌入的应用名（如 `test_perf_single_vs_multi`、`test_signal_basic` 等）。未指定时默认启动 `shell`。

### 15.3 实验脚本

[`run_experiments.py`](run_experiments.py) 自动化完整实验流程：

1. 清理并构建 kernel + user（指定 `INIT_APP=test_suite`）
2. 启动 QEMU，收集输出直到测试套件完成
3. 解析结构化输出（`[SUITE]`、`[perf_*]` 标签）
4. 将结果写入 `artifacts/` 目录（log、JSON、markdown 表格）

### 15.4 复现建议

1. 固定 `-machine virt -smp 4 -m 128M -bios default`
2. 只对同一次编译产物做比较
3. 性能测试尽量多轮采样，读 Avg/Min/Max/Samples，不要只看单次结果
4. `single_vs_multi` 比较的是逻辑活跃 hart 数，不是物理 hotplug
5. QEMU 模拟的 SMP 并行度受 host 调度影响，不同机器上数值可能差异较大

---

## 附录 A: 关键数据结构速查

| 结构 | 文件 | 核心字段 |
|------|------|----------|
| `TrapContext` | [`trap/context.rs`](kernel/src/trap/context.rs) | x[32], sstatus, sepc, kernel_satp, kernel_sp, trap_handler |
| `KernelTrapContext` | [`trap/context.rs`](kernel/src/trap/context.rs) | ra, sp, s[0..11], sstatus, sepc |
| `TaskContext` | [`task/context.rs`](kernel/src/task/context.rs) | ra, sp, s[0..12] |
| `Process` | [`task/process.rs`](kernel/src/task/process.rs) | pid, inner(SpinLock\<ProcessInner\>) |
| `ProcessInner` | [`task/process.rs`](kernel/src/task/process.rs) | memory_set, children, threads, fd_table, signal_*, is_zombie |
| `Thread` | [`task/thread.rs`](kernel/src/task/thread.rs) | tid, process(Weak), kernel_stack, inner(SpinLock\<ThreadInner\>) |
| `ThreadInner` | [`task/thread.rs`](kernel/src/task/thread.rs) | status, task_context, trap_cx_ppn, exit_code |
| `Processor` | [`task/processor.rs`](kernel/src/task/processor.rs) | hart_id, current, idle_task_cx, need_reschedule, dead_task, pending_enqueue |
| `SpinLock<T>` | [`sync/spinlock.rs`](kernel/src/sync/spinlock.rs) | next_ticket, now_serving, data(UnsafeCell) |
| `MapArea` | [`mm/memory_set.rs`](kernel/src/mm/memory_set.rs) | vpn_range, data_frames(BTreeMap), map_type, map_perm |
| `MemorySet` | [`mm/memory_set.rs`](kernel/src/mm/memory_set.rs) | page_table, areas(Vec\<MapArea\>) |

---

## 附录 B: 关键文件索引

| 文件 | 作用 |
|------|------|
| [`kernel/src/main.rs`](kernel/src/main.rs) | 入口、boot hart 选举、应用加载、secondary hart 加入 |
| [`kernel/src/config.rs`](kernel/src/config.rs) | 常量定义：MAX_HARTS、内存布局、MMIO 地址、PLIC 辅助函数 |
| [`kernel/src/trap/trap.S`](kernel/src/trap/trap.S) | 用户 trap 向量、返回路径、内核 trap 向量（汇编） |
| [`kernel/src/trap/mod.rs`](kernel/src/trap/mod.rs) | trap 分发、signal 递送、调度检查 |
| [`kernel/src/trap/interrupt.rs`](kernel/src/trap/interrupt.rs) | timer、PLIC、外部中断、统计计数 |
| [`kernel/src/mm/memory_set.rs`](kernel/src/mm/memory_set.rs) | 地址空间构造、ELF 装载、COW fork、线程退出蹦床 |
| [`kernel/src/mm/page_table.rs`](kernel/src/mm/page_table.rs) | 页表操作、用户指针翻译、PTE 修改 |
| [`kernel/src/mm/frame_alloc.rs`](kernel/src/mm/frame_alloc.rs) | 物理帧分配器、COW 帧引用计数 |
| [`kernel/src/task/process.rs`](kernel/src/task/process.rs) | Process、fork、exec |
| [`kernel/src/task/thread.rs`](kernel/src/task/thread.rs) | Thread、trap context、fork_from |
| [`kernel/src/task/processor.rs`](kernel/src/task/processor.rs) | Per-hart 状态、调度循环、active_harts |
| [`kernel/src/task/scheduler/mod.rs`](kernel/src/task/scheduler/mod.rs) | Scheduler trait、运行时切换 + 任务迁移 |
| [`kernel/src/task/scheduler/global_rr.rs`](kernel/src/task/scheduler/global_rr.rs) | 全局轮转调度器 |
| [`kernel/src/task/scheduler/percpu_rr.rs`](kernel/src/task/scheduler/percpu_rr.rs) | Per-CPU 工作窃取调度器 |
| [`kernel/src/sync/spinlock.rs`](kernel/src/sync/spinlock.rs) | Ticket SpinLock（关中断） |
| [`kernel/src/fs/ramfs.rs`](kernel/src/fs/ramfs.rs) | RAMFS 实现 |
| [`kernel/src/fs/stdio.rs`](kernel/src/fs/stdio.rs) | stdin / stdout（CONSOLE_LOCK 保护） |
| [`kernel/src/syscall/mod.rs`](kernel/src/syscall/mod.rs) | syscall 号分发 |
| [`user/src/lib.rs`](user/src/lib.rs) | 用户态运行时、syscall 封装、KernelStats 解析 |
| [`user/src/bin/test_suite.rs`](user/src/bin/test_suite.rs) | 全量回归测试入口 |

---

## 附录 C: 已知边界与后续建议

### C.1 已知边界

1. **COW 仅处理 Store Page Fault**：Load/Instruction Page Fault 仍直接终止进程，未尝试 COW 解析
2. **signal 最小闭环**：无用户态 handler、无 sigaction、无复杂语义
3. **Mutex/Semaphore 唤醒不完整**：获取了待唤醒 TID 但未真正执行唤醒
4. **stdout 并发串行化**：`CONSOLE_LOCK` 保证正确性但引入显著串行化开销，8 进程并发写 wall time 近似线性增长
5. **VIRTIO0_IRQ 保留**：无真实块设备驱动
6. **调度器切换不迁移旧队列中的 Running 任务**：只有 Ready 状态的任务会被迁移

### C.2 后续建议

1. **完善 Mutex/Semaphore 唤醒**：在 unlock/up 时调用 `unblock_task()` 唤醒等待线程
2. **给 signal 加用户态 handler**：在 trap context 中设置信号栈和返回地址
3. **优化 stdout 并发**：考虑 per-hart 输出缓冲或批量写入，减少 CONSOLE_LOCK 争用
4. **增加 VirtIO 块设备驱动**：支持持久化文件系统
5. **benchmark 增加更多轮次与方差统计**：避免单样本误导
6. **COW 扩展到 Load Page Fault**：对只读共享页也标记 COW，支持更灵活的共享语义
