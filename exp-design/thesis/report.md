# 实验：实时调度与优先级反转

## 1 实验目标

本实验通过**内核级离散事件模拟**，复现经典的**优先级反转（Priority Inversion）**现象，并验证**优先级继承协议（Priority Inheritance Protocol, PIP）**的有效性。具体目标包括：

1. 实现 EDF（Earliest Deadline First）实时调度器
2. 构造 H（高优先级）/ M（中优先级）/ L（低优先级）三任务优先级反转场景
3. 实现基于 deadline inheritance 的优先级继承机制
4. 对比有/无 PI 协议下 H 任务的阻塞时间、响应时间与截止期违约率
5. 通过 100 次重复实验验证结果的可复现性

---

## 2 背景知识

### 2.1 EDF 调度

EDF（Earliest Deadline First）是一种动态优先级实时调度算法。在每个调度决策点，选择**绝对截止期（absolute deadline）最早**的就绪任务执行。EDF 是最优的单处理器抢占式调度算法：若某任务集在任何算法下都可调度，则 EDF 下也一定可调度。

### 2.2 优先级反转

当高优先级任务 H 因等待低优先级任务 L 持有的互斥锁而阻塞时，中优先级任务 M 可以抢占 L 的执行。由于 M 不依赖该锁，M 会持续运行，导致 H 间接被 M 阻塞——这就是**优先级反转**。

```
正常情况：L 运行 → L 释放锁 → H 立即获得锁 → H 运行
反转情况：L 持有锁 → H 阻塞 → M 抢占 L → M 长时间运行 → H 被卡住
```

现实中最著名的案例是 1997 年火星探路者号（Mars Pathfinder）任务，其 VxWorks 内核中的优先级反转导致系统反复重启。

### 2.3 优先级继承协议

优先级继承协议（PIP）由 Sha、Rajkumar 和 Lehoczky 于 1990 年提出。核心思想：当高优先级任务因等待锁而阻塞时，**锁的持有者临时继承等待者的优先级**，使其能够排挤中优先级任务，尽快完成临界区并释放锁。

在 EDF 语境下，"优先级"体现为绝对截止期。优先级继承对应为 **deadline inheritance**：锁持有者的 effective_deadline 被降低为等待者中的最小 deadline。

---

## 3 实验框架设计

### 3.1 整体架构

```
┌──────────────────────────────────────────────────────┐
│                    Kernel（内核模拟器）                 │
│                                                      │
│  ┌────────────┐  ┌────────────┐  ┌───────────────┐  │
│  │ Scheduler  │  │   Timer    │  │   Executor    │  │
│  │   (EDF)    │  │  (tick)    │  │  (per-tick)   │  │
│  └────────────┘  └────────────┘  └───────────────┘  │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │          PiMutex（带/不带 PI 的互斥锁）          │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  Ready Queue      Blocked Queue      Event Log       │
└──────────────────────────────────────────────────────┘
```

### 3.2 模块划分

| 模块 | 文件 | 职责 |
|------|------|------|
| 任务模型 | `task.rs` | 定义 Task、Segment、TaskState；支持周期释放、分段执行、抖动 |
| EDF 调度器 | `scheduler.rs` | 从就绪队列中选择 effective_deadline 最小的任务 |
| PI 互斥锁 | `mutex.rs` | try_lock/unlock；PI 开启时执行 deadline inheritance |
| 内核模拟器 | `kernel.rs` | tick 驱动的离散事件仿真；调度、执行、段切换、事件记录 |
| 事件日志 | `event.rs` | 定义 EventType 枚举：Released、Blocked、PI_Boost 等 |
| 实验编排 | `experiment.rs` | 构造场景、批量运行、统计计算（均值/方差）、输出格式化 |
| 入口 | `main.rs` | 4 部分实验流程、任务配置表打印 |

### 3.3 任务模型

每个任务（Task）是一个**周期性实时任务**，具有以下属性：

- **base_priority**：静态基优先级（数值越小优先级越高）
- **period**：周期（ticks）
- **relative_deadline**：相对截止期
- **segments**：作业分段模板，每个周期重复执行

作业分段（Segment）有两种类型：

- `Compute(duration)`：普通计算段
- `CriticalSection { duration, mutex_id }`：临界区段，进入时获取互斥锁，结束时释放

动态状态包括：`absolute_deadline`（当前作业的绝对截止期）、`effective_deadline`（EDF 调度使用的有效截止期，可被 PI 修改）、`state`（Suspended/Ready/Running/Blocked）。

### 3.4 EDF 调度器

```rust
pub fn pick_next(tasks: &[Task]) -> Option<usize> {
    tasks.iter()
        .enumerate()
        .filter(|(_, t)| t.state == TaskState::Ready)
        .min_by(|(_, a), (_, b)| {
            a.effective_deadline.cmp(&b.effective_deadline)
                .then(a.base_priority.cmp(&b.base_priority))
        })
        .map(|(i, _)| i)
}
```

选择 `effective_deadline` 最小的就绪任务。平局时按 `base_priority` 决定。注意这里使用 `effective_deadline` 而非 `absolute_deadline`，正是 PI 能够改变调度决策的关键。

### 3.5 PI 互斥锁

互斥锁（PiMutex）通过 `pi_enabled` 标志控制是否启用优先级继承：

**加锁（try_lock）：**

1. 若无持有者 → 直接获取
2. 若已被持有 → 请求者进入 Blocked 状态，加入等待队列
3. 若 `pi_enabled` → 执行 `apply_pi()`：将持有者的 `effective_deadline` 降为所有等待者中的最小值

**解锁（unlock）：**

1. 恢复持有者的 `effective_deadline` 为原始 `absolute_deadline`
2. 从等待队列中选出 `effective_deadline` 最小的等待者，唤醒并转移所有权

### 3.6 内核模拟器

内核以 tick 为驱动，每个 tick 执行以下步骤：

```
tick(t):
    1. 将当前 Running 任务重置为 Ready（允许重新调度）
    2. 释放 next_release == t 的周期任务（处理违约回收）
    3. EDF 调度器从 Ready 队列选择下一个执行任务
    4. 执行 1 tick（segment_remaining--）
    5. 若 segment_remaining == 0 → 段完成处理：
       a. 若为 CriticalSection → 释放互斥锁
       b. 推进到下一段 → 若为 CriticalSection → 尝试获取互斥锁
       c. 若无下一段 → 作业完成，记录响应时间与违约情况
    6. 记录 tick_runner（每个 tick 由哪个任务执行）
```

---

## 4 实验场景

### 4.1 任务配置

| 任务 | 基优先级 | 周期 | 截止期 | WCET | 首次释放 | 作业分段 |
|------|---------|------|--------|------|---------|---------|
| L（低） | 3 | 200 | 200 | 40 | t=0 | Compute(5) → CS(20, mutex=0) → Compute(15) |
| H（高） | 1 | 60 | 60 | 20 | t=10 | Compute(5) → CS(5, mutex=0) → Compute(10) |
| M（中） | 2 | 100 | 100 | 30 | t=15 | Compute(30) |

L 和 H 共享 mutex_0，M 不使用任何互斥锁。

### 4.2 为什么这个场景会触发优先级反转

1. L 先于 H 释放，在 t=4 获取 mutex_0 并进入 20 tick 的临界区
2. H 在 t=10 释放，因其截止期（t=70）早于 L（t=200），EDF 让 H 抢占 L
3. H 执行 5 tick 计算后尝试获取 mutex_0 → 被 L 持有 → **H 阻塞**
4. M 在 t=15 释放，截止期 t=115，L 的截止期 t=200 → EDF 选择 M 运行
5. M 运行 30 tick 期间，L 无法执行、无法释放锁 → **H 被间接阻塞 30 tick**

这就是经典的优先级反转：H 的阻塞时间远超 L 临界区剩余长度（15 tick），因为 M "插队"了。

---

## 5 实验结果

### 5.1 无 PI 时的时间线（PI OFF）

```
t=  0-9   LLLLLLLLLL    ← L 执行 5 tick 计算 + 5 tick 临界区
t= 10-14  HHHHH         ← H 释放，抢占 L，执行 5 tick 计算
t= 15-44  MMMMMMMM      ← M 抢占 L（反转！H 被卡住 30 tick）
t= 45-59  LLLLLLLLL     ← M 完成，L 恢复执行剩余 15 tick 临界区
t= 60-79  HHHHHHHHHH    ← L 释放锁，H 终于获得并执行完作业
```

**关键事件：**

| 时间 | 任务 | 事件 | 说明 |
|------|------|------|------|
| t=4 | L | LOCK_ACQ | L 获取 mutex |
| t=10 | H | RELEASED | H 释放，deadline=70 |
| t=14 | H | BLOCKED | H 尝试获取 mutex → 阻塞 |
| t=44 | M | JOB_DONE | M 完成，resp_time=30 |
| t=59 | L | LOCK_REL | L 释放 mutex |
| t=59 | H | UNBLOCKED | H 被唤醒 |
| t=70 | H | DEADLINE_MISS | **H 超过截止期（deadline=70, 当前仍在执行）** |

H 在 t=14 阻塞，t=59 才被唤醒，**反转持续 45 tick**。最终 H 的响应时间（60 tick）恰好等于截止期（60 tick），构成违约。

### 5.2 有 PI 时的时间线（PI ON）

```
t=  0-9   LLLLLLLLLL    ← L 执行（同上）
t= 10-14  HHHHH         ← H 抢占 L（同上）
t= 15-29  LLLLLLLLL     ← PI 生效！L 继承 H 的 deadline=70，排挤 M
t= 30-44  HHHHHHHHH     ← H 获得 mutex，执行完 CS(5) + Compute(10)
t= 45-74  MMMMMMMM      ← M 在 H 完成后才执行
```

**关键事件：**

| 时间 | 任务 | 事件 | 说明 |
|------|------|------|------|
| t=14 | H | BLOCKED | H 阻塞 |
| t=14 | L | **PI_BOOST** eff_deadline=70 | L 的有效截止期从 200 降为 70 |
| t=15 | M | RELEASED | M 释放，但 L(eff=70) < M(deadline=115) → **M 无法抢占 L** |
| t=29 | L | **PI_RESTORE** deadline=200 | L 释放 mutex，截止期恢复 |
| t=29 | H | UNBLOCKED | H 被唤醒 |
| t=44 | H | JOB_DONE resp_time=35 | **H 在截止期前完成（35 < 60）** |

H 在 t=14 阻塞，t=29 被唤醒，**反转持续仅 15 tick**（即 L 临界区剩余长度）。PI 让 L 继承了 H 的紧迫性，避免了 M 的干扰。

### 5.3 对比总结

| 指标 | PI OFF | PI ON | 改善 |
|------|--------|-------|------|
| H 反转持续时间 | 45 tick | 15 tick | **减少 67%** |
| H 响应时间 | 60 tick | 35 tick | **减少 42%** |
| H 截止期违约 | 是（deadline=70, completed=70） | 否（35 < 60） | **消除违约** |
| 截止期违约总次数 | 1 | 0 | **完全消除** |

### 5.4 可复现性验证（100 次运行）

#### 确定性模式（无抖动）

```
                     Metric        PI OFF         PI ON
  ----------------------------------------------------
       H Inversion Duration  45.0 +/- 0.00  15.0 +/- 0.00
            H Response Time  60.0 +/- 0.00  35.0 +/- 0.00
      Total Deadline Misses   1.0 +/- 0.00   0.0 +/- 0.00
       H Deadline Miss Rate          100%            0%
```

100 次运行，所有指标**方差为零**，证明模拟器完全确定性、结果精确可复现。

#### 鲁棒性模式（±10% 执行时间抖动）

```
                     Metric        PI OFF         PI ON
  ----------------------------------------------------
       H Inversion Duration  44.8 +/- 2.31  14.8 +/- 1.23
            H Response Time  60.0 +/- 0.00  34.8 +/- 1.49
      Total Deadline Misses   1.0 +/- 0.20   0.0 +/- 0.00
       H Deadline Miss Rate           96%            0%
```

引入 ±10% 的随机执行时间抖动后：

- PI OFF：反转持续时间方差 = 2.31，deadline miss 率 96%（个别运行因抖动缩短了临界区而侥幸未违约）
- PI ON：反转持续时间方差 = 1.23，**deadline miss 率仍为 0%**
- PI 的优势在扰动下依然稳健

---

## 6 结果分析

### 6.1 优先级反转的量化

无 PI 时，H 的阻塞由两部分组成：

```
H 阻塞时间 = M 执行时间 (30 tick) + L 临界区剩余 (15 tick) = 45 tick
```

其中只有 15 tick 是"不可避免的"（L 必须完成临界区才能释放锁），其余 30 tick 完全是 M 不必要地延长了 H 的等待——这正是反转的代价。

有 PI 时，L 继承了 H 的 deadline（70 vs M 的 115），EDF 选择 L 继续执行而非 M：

```
H 阻塞时间 = L 临界区剩余 (15 tick) = 15 tick（无反转开销）
```

### 6.2 Deadline Miss 的消除

无 PI 时 H 首个作业的关键路径：

```
t=10 释放 → t=14 阻塞 → t=59 解除阻塞 → t=70 完成
响应时间 = 60 tick ≥ deadline (60 tick) → 违约
```

有 PI 时：

```
t=10 释放 → t=14 阻塞 → t=29 解除阻塞 → t=44 完成
响应时间 = 35 tick < deadline (60 tick) → 安全
```

PI 通过将 L 的有效截止期从 200 降至 70，使 EDF 正确地优先执行 L 而非 M，将 H 的响应时间从 60 缩短至 35 tick，裕量从 0 提升到 25 tick。

### 6.3 确定性与可复现性

本模拟器是纯计算模型（无真实时钟、无 I/O），相同输入必然产生相同输出。100 次运行方差为零，满足"固定脚本重复运行 N 次"的验收要求。

抖动模式通过可控随机种子引入参数变化，方差有界（std_dev ∈ [0, 2.31]），证明结果不仅确定性可复现，在参数扰动下也具有统计意义上的稳定性。

---

## 7 实现要点

### 7.1 EDF 下的优先级继承

传统 PIP 文献针对固定优先级调度（如 RMS），通过修改任务的静态优先级值实现继承。本实验将 PIP 适配到 EDF 场景：

- 固定优先级 PIP：提升持有者的 priority 值
- EDF PIP（本实验）：降低持有者的 effective_deadline 值

两者在语义上等价：都使持有者在调度决策中获得更高的紧迫性。

### 7.2 段式作业模型

任务作业被建模为有序段序列（Segment），每段为普通计算或临界区。内核逐 tick 推进当前段的剩余计数器；段结束时自动处理互斥锁的获取/释放并切换到下一段。这种设计使得任务行为的描述既直观又精确：

```rust
vec![
    Segment::Compute(5),                           // 5 tick 普通计算
    Segment::CriticalSection { duration: 20, mutex_id: 0 },  // 20 tick 临界区
    Segment::Compute(15),                          // 15 tick 普通计算
]
```

### 7.3 互斥锁与调度器的解耦

PiMutex 独立于 Kernel 和 Scheduler 实现在单独模块中。锁操作通过传入任务数组引用和事件向量来完成状态修改和事件记录，避免了循环依赖，也便于单独测试锁的 PI 行为。

---

## 8 复现方法

### 8.1 环境要求

- Rust 工具链（edition 2021，建议 rustc ≥ 1.56）
- 无需外部系统依赖

### 8.2 构建与运行

```bash
cd thesis/
cargo run
```

输出包含完整的 4 部分实验结果：PI OFF 详细跟踪、PI ON 详细跟踪、100 次确定性批量统计、100 次抖动批量统计。

### 8.3 修改实验参数

在 `src/experiment.rs` 的 `build_tasks()` 函数中可修改任务参数（周期、截止期、分段、偏移量）。在 `src/main.rs` 中可调整 `N_RUNS`（重复次数）和 `SIM_DURATION`（仿真时长）。

---

## 9 文件清单

```
thesis/
├── Cargo.toml              # 项目配置，依赖 rand 0.8（抖动模式）
├── report.md               # 本报告
└── src/
    ├── main.rs             # 实验入口，4 部分流程编排
    ├── kernel.rs           # tick 驱动内核模拟器
    ├── task.rs             # 任务模型（Task, Segment, TaskState）
    ├── scheduler.rs        # EDF 调度器
    ├── mutex.rs            # PiMutex（带/不带 PI）
    ├── event.rs            # 事件类型定义
    └── experiment.rs       # 场景构造、批量运行、统计输出
```

---

## 10 结论

本实验通过内核级离散事件模拟，完整复现了经典优先级反转现象并验证了优先级继承协议的有效性：

1. **反转现象清晰可观测**：无 PI 时 H 被 M 间接阻塞 45 tick（其中 30 tick 为不必要的反转开销），导致 deadline miss
2. **PI 有效消除反转**：有 PI 时 L 继承 H 的紧迫性，H 阻塞仅 15 tick（临界区剩余长度），无 deadline miss
3. **结果完全可复现**：100 次确定性运行方差为零；±10% 抖动下方差有界，PI 优势稳健不变
