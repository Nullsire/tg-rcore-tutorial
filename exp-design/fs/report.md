# 实验报告：设计支持日志功能的文件系统

## 1 实验目标

设计并实现一个支持**日志（Journaling）功能**的文件系统，使得在意外掉电后能够**快速恢复**到一致性状态。本实验：

- 参考 xv6 的日志文件系统进行设计，使用 Rust 语言实现
- 提供两种日志策略：**Redo Log**（重做日志，xv6 风格）和 **Undo Log**（撤销日志）
- 通过自动化演示和交互式 Shell 展示 *Operating Systems: Three Easy Pieces* 教材中的崩溃一致性示例

---

## 2 背景知识

### 2.1 崩溃一致性问题

文件系统在执行一个逻辑操作（如创建文件）时，往往需要更新磁盘上的多个块：数据块、inode 块、目录块、位图块等。如果在更新过程中发生掉电，磁盘上可能只有部分块被成功写入，导致文件系统处于**不一致状态**。

*Three Easy Pieces* 教材以"银行转账"为例说明了这一问题：如果系统在写入 A 账户余额和 B 账户余额之间崩溃，可能出现"钱凭空消失"或"凭空出现"的错误。

### 2.2 日志（Journaling）机制

日志的核心思想是 **Write-Ahead Logging（WAL）**：在修改真实数据之前，先将修改意图记录到日志中。这样，无论崩溃发生在哪个阶段，恢复时都可以通过日志将文件系统恢复到一致性状态。

两种经典策略：

| 策略 | 原理 | 恢复行为 |
|------|------|----------|
| **Redo Log** | 先将新数据写入日志，再写入最终位置 | 重放已提交的日志条目 |
| **Undo Log** | 先将原始数据保存到日志，再覆盖 | 回滚未完成的写入，恢复原始数据 |

### 2.3 xv6 日志文件系统

xv6 的日志系统采用 Redo Log，事务流程如下：

```
begin_tx() → log_write(block, data) × N → commit() → install() → cleanup()
```

1. **begin_tx**：开始一个事务
2. **log_write**：将待写入的块缓存在内存中
3. **commit**：将所有修改写入日志区域，写入提交标记（commit point）
4. **install**：将日志中的数据复制到磁盘上的最终位置
5. **cleanup**：清除日志头部，表示事务完成

---

## 3 系统设计

### 3.1 磁盘布局

磁盘被划分为固定大小的块（512 字节），布局如下：

```
+--------+------------+-------+--------------+-------------+--------------+--------------+
| Block  | Superblock | Log   | Inode Bitmap | Data Bitmap | Inode Blocks | Data Blocks  |
| 0      | 1          | 2..65 | 66           | 67          | 68..92       | 93..end      |
+--------+------------+-------+--------------+-------------+--------------+--------------+
```

| 区域 | 起始块号 | 大小 | 说明 |
|------|---------|------|------|
| Boot Block | 0 | 1 块 | 保留未使用 |
| Superblock | 1 | 1 块 | 文件系统元信息 |
| Log Area | 2 | 64 块 | 日志头部 + 数据 |
| Inode Bitmap | 66 | 1 块 | inode 分配位图 |
| Data Bitmap | 67 | 1 块 | 数据块分配位图 |
| Inode Blocks | 68 | 25 块 | inode 表（每 inode 64 字节，每块 8 个） |
| Data Blocks | 93 | 剩余 | 文件数据块 |

### 3.2 关键数据结构

#### Superblock（超级块，512 字节）

```rust
struct Superblock {
    magic: u32,        // 魔数 0x4A465331 ("JFS1")
    version: u32,      // 版本号
    nblocks: u32,      // 磁盘总块数
    nlogblocks: u32,   // 日志区块数
    log_start: u32,    // 日志区起始块号
    ninodes: u32,      // inode 总数（200）
    inode_start: u32,  // inode 表起始块号
    inode_bitmap: u32, // inode 位图块号
    data_bitmap: u32,  // 数据位图块号
    data_start: u32,   // 数据区起始块号
}
```

#### DiskInode（磁盘 inode，64 字节）

```rust
struct DiskInode {
    itype: u16,          // 类型：0=空, 1=文件, 2=目录
    nlink: u16,          // 硬链接计数
    size: u32,           // 文件大小（字节）
    direct: [u32; 12],   // 12 个直接数据块指针
}
```

每个 inode 64 字节，一个 512 字节的块可容纳 8 个 inode。最大文件大小 = 12 × 512 = 6144 字节。

#### 目录项（32 字节）

```rust
struct DirEntry {
    inum: u32,          // inode 编号（4 字节）
    name: [u8; 28],     // 文件名（28 字节）
}
```

每个 512 字节的块可容纳 16 个目录项。根目录的 inode 编号固定为 1。

#### 日志头部（512 字节）

```
+----------+-----------+------------------+
| count    | committed | block_numbers[N] |
| 4 bytes  | 4 bytes   | N × 4 bytes      |
+----------+-----------+------------------+
```

- `count`：日志中记录的块数
- `committed`：是否已提交（1=已提交，0=未提交）
- `block_numbers[]`：各块的目标块号

### 3.3 日志接口设计

```rust
trait Journal {
    fn begin_tx(&mut self);                                    // 开始事务
    fn log_write(&mut self, block_no: u32, data: &[u8; 512]);  // 记录写入
    fn commit(&mut self, disk, cache) -> Result<()>;           // 提交事务
    fn recover(&mut self, disk, cache) -> Result<()>;          // 崩溃恢复
}
```

三种实现：

1. **RedoLog**（重做日志）：`log_write` → 写入日志 → `commit` 写头部 → `install` 复制到最终位置
2. **UndoLog**（撤销日志）：`log_write` → 保存原始数据 → `commit` 写头部 → 覆盖最终位置
3. **NoOpLog**（无日志）：不做任何日志操作，用于对比演示

### 3.4 Redo Log 事务流程

```
                    崩溃安全点
                       ↓
    ┌──────────────────┼──────────────────┐
    │ log_write        │ commit           │ install          │ cleanup
    │ 写数据到日志块    │ 写committed头部   │ 复制到最终位置     │ 清除头部
    │                  │ ← 安全点         │ ← 安全点          │
    └──────────────────┴──────────────────┘

    崩溃场景：
    • commit 前崩溃 → 无 committed 标记，忽略日志，数据不变
    • commit 后、install 前崩溃 → 重放日志，安装新数据
    • install 后、cleanup 前崩溃 → 重放日志（幂等操作），数据不变
```

### 3.5 Undo Log 事务流程

```
    ┌──────────────────┐──────────────────┐
    │ save_originals   │ commit           │ write_new        │ cleanup
    │ 保存原始数据到日志 │ 写committed头部   │ 写新数据到目标位置  │ 清除头部
    │                  │ ← 安全点         │                  │
    └──────────────────┘──────────────────┘

    崩溃场景：
    • commit 前崩溃 → 无 committed 标记，数据不变
    • commit 后、write_new 中崩溃 → 从日志恢复原始数据（回滚）
```

---

## 4 代码框架与实现

### 4.1 项目结构

```
fs/
├── Cargo.toml               # 项目配置（依赖 clap）
├── src/
│   ├── main.rs      (604行) # CLI 入口 + 交互 Shell + 集成测试
│   ├── disk.rs       (95行) # 文件模拟的块设备 + 崩溃模拟
│   ├── superblock.rs (137行)# 超级块读写
│   ├── block_cache.rs(149行)# 内存块缓存（脏标记、同步）
│   ├── bitmap.rs     (109行)# inode/数据块位图分配
│   ├── log.rs        (475行)# 核心：RedoLog + UndoLog + NoOpLog
│   ├── inode.rs      (271行)# inode 分配/读写/数据块管理
│   ├── dir.rs        (277行)# 目录操作（查找/链接/删除）
│   ├── file.rs       (173行)# 高层文件操作（创建/读写/删除/重命名）
│   ├── fs.rs         (163行)# 文件系统挂载/卸载/恢复
│   └── demo.rs       (538行)# 6 个演示场景
└── report.md                 # 本报告
```

总计约 **2991 行** Rust 代码。

### 4.2 各模块职责

#### disk.rs — 块设备层

使用本地文件模拟物理磁盘，支持读写指定块号的数据：

```rust
pub struct FileDisk {
    file: File,
    pub nblocks: u32,
}
```

关键方法：
- `create(path, nblocks)`：创建指定大小的磁盘镜像文件
- `read_block(block_no, buf)` / `write_block(block_no, buf)`：读写指定块
- `crash(self)`：**模拟掉电**——直接 drop 文件句柄，不执行 sync

#### block_cache.rs — 块缓存层

```rust
pub struct BlockCache {
    cache: HashMap<u32, CachedBlock>,  // 块号 → 缓存数据
}
```

缓存中每个块维护 `dirty`（是否修改）和 `pinned`（是否被日志锁定）状态。`sync()` 方法将所有脏块写回磁盘。

#### log.rs — 日志层（核心）

`Journal` trait 定义了统一接口，三种实现共享同一接口：

```rust
pub trait Journal {
    fn begin_tx(&mut self);
    fn log_write(&mut self, block_no: u32, data: &Block);
    fn commit(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String>;
    fn recover(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String>;
}
```

**RedoLog 的 commit 实现**（简化伪代码）：

```
1. 将所有修改的块数据写入日志数据区
2. 写入日志头部（count + committed=1）  ← 这是原子提交点
3. 将日志数据复制到最终磁盘位置（install）
4. 清除日志头部（cleanup）
```

**UndoLog 的 commit 实现**：

```
1. 读取并保存所有将被修改的块的原始内容到日志
2. 写入日志头部（count + committed=1）  ← 原子提交点
3. 将新数据写入最终磁盘位置
4. 清除日志头部
```

#### inode.rs — inode 层

提供 `ialloc`（分配 inode）、`iget`（读取 inode）、`iupdate`（更新 inode）、`readi`/`writei`（文件数据读写）等操作。

#### dir.rs — 目录层

目录以文件形式存储，每个目录项 32 字节（4 字节 inode 号 + 28 字节文件名）。提供 `lookup`、`link`、`unlink`、`list` 操作。根目录 inode 编号固定为 1。

#### file.rs — 文件操作层

每个修改操作（create/write/unlink/rename）都被包装在 `begin_tx()` 和 `commit()` 之间，确保原子性：

```rust
pub fn create(journal, cache, disk, sb, name) -> Result<u32, String> {
    journal.begin_tx();
    let inum = ialloc(cache, disk, sb, INODE_FILE)?;
    iupdate(cache, disk, sb, inum, &dinode);
    dir_link(cache, disk, sb, ROOT_INUM, name, inum)?;
    journal.commit(disk, cache)?;
    Ok(inum)
}
```

#### fs.rs — 文件系统管理

```rust
pub struct FileSystem {
    pub disk: FileDisk,
    pub cache: BlockCache,
    pub sb: Superblock,
    pub journal_type: JournalType,
}
```

提供 `format`（格式化）、`mount`（挂载 + 自动恢复）、`umount`（卸载）操作。

### 4.3 交互式 Shell

`main.rs` 中的 `run_shell` 函数提供了一个交互式命令行界面，支持用户直接操作文件系统：

```
$ cargo run -- shell fs.img
jfs> create hello.txt
jfs> write hello.txt Hello_World
jfs> read hello.txt
jfs> stat hello.txt
jfs> crash          ← 模拟掉电
$ cargo run -- shell fs.img    ← 重新挂载，日志自动恢复
jfs> read hello.txt   ← 数据完好
```

---

## 5 实验结果

### 5.1 Demo 1：基本文件操作

验证文件系统的基本 CRUD 功能：

- 创建文件 `hello.txt` 并写入 47 字节数据
- 读回数据，验证内容正确
- 批量创建 5 个文件，列出目录
- 删除文件后验证确实不存在
- 追加（append）和重命名（rename）操作

```
[PASS] Created hello.txt (inum=2)
[PASS] Wrote 47 bytes to hello.txt
[PASS] Read back correct content: "Hello, World! This is a journaling file system."
[PASS] Deleted file2.txt
[PASS] file2.txt correctly deleted (not found)
[PASS] Append works correctly
[PASS] Renamed file0.txt -> renamed.txt
```

### 5.2 Demo 2：无日志时的崩溃（对照组）

**目的**：展示没有日志保护时，崩溃会导致数据不一致。

**过程**：修改数据块但不更新 inode，模拟掉电。重新挂载后发现数据已被意外修改：

```
[PASS] Wrote data block (MODIFIED) but NOT inode
>>> SIMULATING CRASH <<<
[PASS] Without journaling: data = "MODIFIED" (inconsistent - partial write)
```

**结论**：无日志时，文件系统无法保证崩溃后的一致性。

### 5.3 Demo 3：Redo Log 崩溃恢复

**过程**：在 commit 之后、install 之前模拟崩溃：

```
[PASS] Wrote COMMITTED log header
>>> SIMULATING CRASH after commit, before install <<<

[RedoLog] Recovering 2 committed blocks...
  [RedoLog] Replayed log block 0 -> disk block 94
  [RedoLog] Replayed log block 1 -> disk block 68
[RedoLog] Recovery complete.
[PASS] Recovery successful! Data = "NEW_DATA_MODIFIED"
```

**结论**：Redo Log 在恢复时**重放**已提交的事务，将数据安装到最终位置，保证了已提交事务的持久性。

### 5.4 Demo 4：Undo Log 崩溃恢复

**过程**：模拟银行场景，`BALANCE=1000` 被部分覆盖为 `BALANCE=0!!!!`，中途崩溃：

```
[PASS] Saved original data to undo log
[PASS] Partially wrote new data (BALANCE=0!!!!) — SIMULATING CRASH

[UndoLog] Recovering: undoing 2 partially written blocks...
  [UndoLog] Restored block 94 from log
  [UndoLog] Restored block 68 from log
[UndoLog] Recovery complete (undid partial writes).
[PASS] Recovery successful! Data restored to: "BALANCE=1000"
[PASS] The partial write was undone — original data preserved!
```

**结论**：Undo Log 在恢复时**回滚**未完成的写入，恢复到事务开始前的状态，保证数据不被部分写入破坏。

### 5.5 Demo 5：性能对比

对比三种日志策略的吞吐量：

```
  redo journal: 50 ops in 1ms (50000.0 ops/sec)
  undo journal: 50 ops in 1ms (50000.0 ops/sec)
  noop journal: 50 ops in 1ms (50000.0 ops/sec)
```

> 注：由于使用内存文件系统（Linux page cache），三种策略在本实验中性能差异不明显。在真实的物理磁盘上，Redo Log 通常比 Undo Log 稍慢（因为 install 阶段需要额外的写操作），但差异很小。NoOp 最快但不提供任何保护。

### 5.6 Demo 6：事务各阶段崩溃测试

在 Redo Log 事务的 4 个关键阶段分别模拟崩溃，验证恢复正确性：

| 崩溃阶段 | 日志状态 | 恢复行为 | 恢复后数据 |
|----------|---------|---------|-----------|
| commit 前 | 无 committed 标记 | 忽略日志 | `ORIGINAL`（不变） |
| commit 后 | 已提交，数据在日志 | 重放日志 | `NEW`（新数据） |
| install 中 | 已提交 + 部分安装 | 重放日志（幂等） | `NEW`（新数据） |
| install 后 | 数据已安装 | 重放日志（幂等） | `NEW`（新数据） |

```
[PASS] Crash phase: before_commit  → data = "ORIGINAL", CONSISTENT
[PASS] Crash phase: after_commit   → data = "NEW",      CONSISTENT
[PASS] Crash phase: during_install → data = "NEW",      CONSISTENT
[PASS] Crash phase: after_install  → data = "NEW",      CONSISTENT
```

**结论**：Redo Log 在事务的任何阶段崩溃都能保证文件系统一致性。

### 5.7 集成测试结果

```
Test 1: Create, write, read file...        PASS
Test 2: Redo log crash recovery...          PASS (recovered: "AFTER1")
Test 3: Multiple file operations...         PASS
Test 4: Undo log crash recovery...          PASS (undo restored original data)
Test 5: Delete and recreate file...         PASS

Results: 5 passed, 0 failed
```

---

## 6 复现方法

### 6.1 环境要求

- Rust 工具链（`rustc` + `cargo`），建议 1.70 或更高版本
- Linux / macOS / WSL 环境

### 6.2 编译

```bash
cd fs
cargo build --release
```

### 6.3 运行演示

```bash
# 运行全部 6 个演示
cargo run -- demo

# 运行集成测试（5 个测试）
cargo run -- test

# 格式化一个新的磁盘镜像
cargo run -- format mydisk.img
cargo run -- format mydisk.img --blocks 8192

# 查看文件系统信息
cargo run -- info mydisk.img
cargo run -- info mydisk.img -j undo
```

### 6.4 交互式 Shell

```bash
# 启动交互 Shell（使用 Redo Log）
cargo run -- shell fs.img

# 使用 Undo Log
cargo run -- shell fs.img -j undo

# 不使用日志（对照实验）
cargo run -- shell fs.img -j noop
```

Shell 中的可用命令：

| 命令 | 格式 | 说明 |
|------|------|------|
| `ls` | `ls` | 列出根目录下的文件 |
| `create` | `create <文件名>` | 创建空文件 |
| `write` | `write <文件名> <内容>` | 写入文件（覆盖） |
| `append` | `append <文件名> <内容>` | 追加内容 |
| `read` | `read <文件名>` | 读取文件内容 |
| `rm` | `rm <文件名>` | 删除文件 |
| `mv` | `mv <旧名> <新名>` | 重命名 |
| `stat` | `stat <文件名>` | 查看文件详细信息 |
| `touch` | `touch <文件名>` | 创建文件（如不存在） |
| `stats` | `stats` | 文件系统统计 |
| `crash` | `crash` | **模拟掉电** |
| `journal` | `journal` | 查看日志类型 |
| `help` | `help` | 帮助信息 |
| `exit` | `exit` | 安全卸载并退出 |

### 6.5 崩溃恢复实验步骤

以下是手动复现崩溃恢复的完整步骤：

```bash
# 步骤 1：创建镜像并写入数据
cargo run -- shell test.img
jfs> create bank.txt
jfs> write bank.txt BALANCE=1000
jfs> read bank.txt        # 输出: BALANCE=1000
jfs> crash                # 模拟正常写入后的掉电

# 步骤 2：重新挂载，验证恢复
cargo run -- shell test.img
jfs> read bank.txt        # 输出: BALANCE=1000（数据完好）
jfs> exit
```

### 6.6 运行单元测试

```bash
cargo test
```

包含 2 个单元测试（磁盘读写、位图分配）和 5 个集成测试。

---

## 7 总结与思考

### 7.1 实验收获

1. **理解了崩溃一致性问题的本质**：文件系统操作涉及多个磁盘块的原子更新，掉电可能导致不一致
2. **掌握了 WAL（Write-Ahead Logging）机制**：先记录日志再修改数据，保证可恢复
3. **对比了 Redo Log 和 Undo Log**：前者保证已提交事务的持久性，后者保证未完成事务的原子性
4. **理解了幂等性在恢复中的作用**：Redo Log 的 install 操作是幂等的，重复执行结果相同

### 7.2 设计取舍

| 决策 | 选择 | 原因 |
|------|------|------|
| 单线程 | 无锁设计 | 简化实现，聚焦日志机制本身 |
| 固定日志区 | 64 块 | 足够覆盖单个事务 |
| 仅直接块 | 无间接块 | 简化实现，6KB 文件足够演示 |
| 位图分配 | 替代空闲链表 | 实现简单，O(n) 分配 |
| 文件模拟磁盘 | 本地文件 | 跨平台，便于调试和崩溃模拟 |

### 7.3 可能的改进

1. **间接块支持**：添加一级间接块指针，支持更大的文件
2. **并发事务**：支持多个并发事务，增加日志空间利用率
3. **日志检查点**：定期创建检查点，减少恢复时间
4. **实际磁盘 I/O**：直接操作块设备文件（如 `/dev/sda`），测试真实磁盘性能
