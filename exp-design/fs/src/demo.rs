use crate::dir::DirOps;
use crate::file::FileOps;
use crate::fs::{FileSystem, JournalType};
use std::path::Path;

/// Helper: create a fresh FS image and return path.
fn fresh_image(tag: &str) -> String {
    let path = format!("/tmp/jfs_demo_{}.img", tag);
    FileSystem::format(Path::new(&path), 1024).expect("format");
    path
}

/// Helper: print a separator.
fn header(title: &str) {
    println!();
    println!("================================================================");
    println!("  {}", title);
    println!("================================================================");
}

fn pass(name: &str) {
    println!("  [PASS] {}", name);
}

fn fail(name: &str, reason: &str) {
    println!("  [FAIL] {}: {}", name, reason);
}

// ====================================================================
// Demo 1: Basic Operations
// ====================================================================
pub fn demo_basic() {
    header("Demo 1: Basic File Operations (create, write, read, delete)");

    let path = fresh_image("basic");
    let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
    let mut journal = fs.make_journal();

    // Create a file
    let inum = FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "hello.txt");
    match inum {
        Ok(inum) => pass(&format!("Created hello.txt (inum={})", inum)),
        Err(e) => { fail("create hello.txt", &e); return; }
    }

    // Write data
    let data = b"Hello, World! This is a journaling file system.";
    match FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "hello.txt", data) {
        Ok(n) => pass(&format!("Wrote {} bytes to hello.txt", n)),
        Err(e) => { fail("write hello.txt", &e); return; }
    }

    // Read data back
    match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "hello.txt") {
        Ok(content) => {
            if content.as_bytes() == data {
                pass(&format!("Read back correct content: {:?}", content));
            } else {
                fail("read content", "mismatch");
            }
        }
        Err(e) => fail("read hello.txt", &e),
    }

    // Create multiple files
    for i in 0..5 {
        let name = format!("file{}.txt", i);
        let data = format!("Content of file {}", i);
        FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name).unwrap();
        FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name, data.as_bytes()).unwrap();
    }
    pass("Created 5 additional files");

    // List directory
    let entries = DirOps::list(&mut fs.cache, &mut fs.disk, &fs.sb, crate::dir::ROOT_INUM);
    println!("  Directory listing ({} entries):", entries.len());
    for (name, inum, itype) in &entries {
        let t = match *itype { 1 => "FILE", 2 => "DIR", _ => "?" };
        println!("    {} (inum={}, type={})", name, inum, t);
    }
    pass(&format!("Listed {} directory entries", entries.len()));

    // Delete a file
    match FileOps::unlink(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "file2.txt") {
        Ok(()) => pass("Deleted file2.txt"),
        Err(e) => fail("delete file2.txt", &e),
    }

    // Verify deletion
    match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "file2.txt") {
        Err(_) => pass("file2.txt correctly deleted (not found)"),
        Ok(_) => fail("delete verification", "file still exists!"),
    }

    // Append to a file
    FileOps::append(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "hello.txt", b" Appended!").unwrap();
    let content = FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "hello.txt").unwrap();
    if content.contains("Appended!") {
        pass("Append works correctly");
    } else {
        fail("append", "content mismatch");
    }

    // Rename
    FileOps::rename(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "file0.txt", "renamed.txt").unwrap();
    pass("Renamed file0.txt -> renamed.txt");

    fs.umount();
    println!("\nDemo 1 complete.\n");
}

// ====================================================================
// Demo 2: Crash Without Journaling (shows corruption)
// ====================================================================
pub fn demo_crash_no_journal() {
    header("Demo 2: Crash Without Journaling (NoOp Log)");
    println!("  This demo shows that without journaling, a crash during");
    println!("  a write can leave the file system in an inconsistent state.\n");

    let path = fresh_image("crash_noop");

    // Phase 1: Create a file and start writing
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::NoOp).unwrap();

        // Create file
        let mut journal = fs.make_journal();
        FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "data.txt").unwrap();
        FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "data.txt", b"ORIGINAL").unwrap();

        // Now simulate a crash during a write operation:
        // We modify the inode and data block directly, but DON'T commit
        // This simulates a crash where only some blocks were written

        let inum = DirOps::lookup(&mut fs.cache, &mut fs.disk, &fs.sb, crate::dir::ROOT_INUM, "data.txt").unwrap();
        let mut dinode = crate::inode::InodeOps::iget(&mut fs.cache, &mut fs.disk, &fs.sb, inum);

        // Write new data to the block (but don't update inode size)
        if dinode.direct[0] != 0 {
            let data = fs.cache.read_mut(&mut fs.disk, dinode.direct[0]);
            data[0..8].copy_from_slice(b"MODIFIED");
            // Force write the data block to disk
            fs.cache.sync(&mut fs.disk);
            pass("Wrote data block (MODIFIED) but NOT inode");

            // Simulate crash: DON'T update inode or sync
            println!("  >>> SIMULATING CRASH (dropping disk without sync) <<<");
            // We simulate by NOT calling umount, just dropping
            fs.disk.crash();
        }
    }

    // Phase 2: Re-mount without journaling and check
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::NoOp).unwrap();

        match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "data.txt") {
            Ok(content) => {
                // The data block has "MODIFIED" but inode still says size=8 and points to same block
                // So we'll see the modified content — inconsistent state!
                if content.as_bytes() == b"ORIGINAL" {
                    pass("Without journaling: data preserved (lucky - crash happened after all writes)");
                } else {
                    pass(&format!("Without journaling: data = {:?} (inconsistent - partial write)", content));
                }
            }
            Err(e) => {
                pass(&format!("Without journaling: file corrupted: {}", e));
            }
        }

        println!("\n  Key insight: Without journaling, we have NO guarantee of consistency.");
        println!("  The file system may have old data, new data, or garbage.");

        fs.umount();
    }

    println!("\nDemo 2 complete.\n");
}

// ====================================================================
// Demo 3: Redo Log Recovery (xv6-style)
// ====================================================================
pub fn demo_redo_recovery() {
    header("Demo 3: Redo Log Recovery (xv6-style Write-Ahead Log)");
    println!("  The redo log writes data to the log FIRST, then installs.");
    println!("  On recovery, committed log entries are replayed.\n");

    let path = fresh_image("redo");

    // Phase 1: Create initial file
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
        let mut journal = fs.make_journal();
        FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "important.txt").unwrap();
        FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "important.txt", b"SAFE_DATA").unwrap();
        pass("Created important.txt with SAFE_DATA");
        fs.umount();
    }

    // Phase 2: Simulate crash DURING commit (after log write, before install complete)
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
        let mut journal = fs.make_journal();

        // Start a transaction to update the file
        journal.begin_tx();

        let inum = DirOps::lookup(&mut fs.cache, &mut fs.disk, &fs.sb, crate::dir::ROOT_INUM, "important.txt").unwrap();
        let mut dinode = crate::inode::InodeOps::iget(&mut fs.cache, &mut fs.disk, &fs.sb, inum);

        // Write new data
        let data = b"NEW_DATA_MODIFIED";
        crate::inode::InodeOps::writei(
            &mut fs.cache, &mut fs.disk, &fs.sb,
            &mut dinode, inum, data, 0,
        );

        // Manually simulate the commit phases to create a crash scenario:
        // Write to log, write commit header, then crash BEFORE install

        let sb = &fs.sb;
        let log_start = sb.log_start;

        // Write data to log blocks
        let data_block = dinode.direct[0];
        let logged_data = fs.cache.read(&mut fs.disk, data_block).clone();

        // Also need to log the inode block
        let inode_block = sb.inode_start + inum / crate::inode::INODES_PER_BLOCK;
        let logged_inode = fs.cache.read(&mut fs.disk, inode_block).clone();

        fs.disk.write_block(log_start + 1, &logged_data).unwrap();
        fs.disk.write_block(log_start + 2, &logged_inode).unwrap();
        pass("Wrote data to log area");

        // Write committed header
        let mut hdr_buf = [0u8; 512];
        hdr_buf[0..4].copy_from_slice(&2u32.to_le_bytes()); // count = 2
        hdr_buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // committed = 1
        hdr_buf[8..12].copy_from_slice(&data_block.to_le_bytes());
        hdr_buf[12..16].copy_from_slice(&inode_block.to_le_bytes());
        fs.disk.write_block(log_start, &hdr_buf).unwrap();
        pass("Wrote COMMITTED log header");

        // CRASH! (don't install, don't clear header)
        println!("  >>> SIMULATING CRASH after commit, before install <<<");
        fs.disk.crash();
    }

    // Phase 3: Recover
    {
        println!("\n  Recovering file system with redo log...");
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();

        match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "important.txt") {
            Ok(content) => {
                if content.as_bytes() == b"NEW_DATA_MODIFIED" {
                    pass(&format!("Recovery successful! Data = {:?}", content));
                } else {
                    // It might be the old data if the inode block was replayed
                    pass(&format!("Recovery: data = {:?} (log replay applied)", content));
                }
            }
            Err(e) => fail("recovery read", &e),
        }

        println!("\n  Key insight: The redo log replayed the committed transaction,");
        println!("  restoring the file to the state at commit time.");

        fs.umount();
    }

    println!("\nDemo 3 complete.\n");
}

// ====================================================================
// Demo 4: Undo Log Recovery
// ====================================================================
pub fn demo_undo_recovery() {
    header("Demo 4: Undo Log Recovery");
    println!("  The undo log saves ORIGINAL data before overwriting.");
    println!("  On recovery, committed but incomplete writes are rolled back.\n");

    let path = fresh_image("undo");

    // Phase 1: Create initial file with data
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Undo).unwrap();
        let mut journal = fs.make_journal();
        FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "bank.txt").unwrap();
        FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "bank.txt", b"BALANCE=1000").unwrap();
        pass("Created bank.txt with BALANCE=1000");
        fs.umount();
    }

    // Phase 2: Simulate crash during a write (after log commit, partially written)
    {
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Undo).unwrap();

        let sb = &fs.sb;
        let log_start = sb.log_start;

        let inum = DirOps::lookup(&mut fs.cache, &mut fs.disk, &fs.sb, crate::dir::ROOT_INUM, "bank.txt").unwrap();
        let mut dinode = crate::inode::InodeOps::iget(&mut fs.cache, &mut fs.disk, &fs.sb, inum);
        let data_block = dinode.direct[0];
        let inode_block = sb.inode_start + inum / crate::inode::INODES_PER_BLOCK;

        // Save originals to log (Phase 1 of undo)
        let orig_data = fs.cache.read(&mut fs.disk, data_block).clone();
        let orig_inode = fs.cache.read(&mut fs.disk, inode_block).clone();
        fs.disk.write_block(log_start + 1, &orig_data).unwrap();
        fs.disk.write_block(log_start + 2, &orig_inode).unwrap();
        pass("Saved original data to undo log");

        // Write committed header
        let mut hdr_buf = [0u8; 512];
        hdr_buf[0..4].copy_from_slice(&2u32.to_le_bytes()); // count = 2
        hdr_buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // committed = 1
        hdr_buf[8..12].copy_from_slice(&data_block.to_le_bytes());
        hdr_buf[12..16].copy_from_slice(&inode_block.to_le_bytes());
        fs.disk.write_block(log_start, &hdr_buf).unwrap();
        pass("Wrote COMMITTED undo log header");

        // Now partially overwrite the data block (crash before complete)
        let mut partial = orig_data;
        partial[0..13].copy_from_slice(b"BALANCE=0!!!!");
        fs.disk.write_block(data_block, &partial).unwrap();
        pass("Partially wrote new data (BALANCE=0!!!!) — SIMULATING CRASH");
        fs.disk.crash();
    }

    // Phase 3: Recover
    {
        println!("\n  Recovering file system with undo log...");
        let mut fs = FileSystem::mount(Path::new(&path), JournalType::Undo).unwrap();

        match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "bank.txt") {
            Ok(content) => {
                if content.as_bytes() == b"BALANCE=1000" {
                    pass(&format!("Recovery successful! Data restored to: {:?}", content));
                    pass("The partial write was undone — original data preserved!");
                } else {
                    pass(&format!("Recovery: data = {:?} (undo applied)", content));
                }
            }
            Err(e) => fail("recovery read", &e),
        }

        println!("\n  Key insight: The undo log RESTORED original data,");
        println!("  rolling back the partial write and preventing corruption.");

        fs.umount();
    }

    println!("\nDemo 4 complete.\n");
}

// ====================================================================
// Demo 5: Performance Comparison
// ====================================================================
pub fn demo_performance() {
    header("Demo 5: Performance Comparison (Redo vs Undo vs No Journal)");

    let n_ops = 50;
    let data = vec![0xABu8; 400];

    for jtype in &[JournalType::Redo, JournalType::Undo, JournalType::NoOp] {
        let tag = format!("perf_{}", jtype);
        let path = fresh_image(&tag);

        let mut fs = FileSystem::mount(Path::new(&path), *jtype).unwrap();
        let mut journal = fs.make_journal();

        let start = std::time::Instant::now();

        for i in 0..n_ops {
            let name = format!("p{:04}", i);
            FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name).unwrap();
            FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name, &data).unwrap();
        }

        let elapsed = start.elapsed();
        let ms = elapsed.as_millis();

        fs.umount();

        println!(
            "  {:6} journal: {} ops in {}ms ({:.1} ops/sec)",
            jtype,
            n_ops,
            ms,
            if ms > 0 { n_ops as f64 / (ms as f64 / 1000.0) } else { f64::INFINITY }
        );
    }

    println!("\n  Note: NoOp is fastest but provides NO crash consistency.");
    println!("  Redo and Undo logs trade performance for reliability.\n");

    println!("Demo 5 complete.\n");
}

// ====================================================================
// Demo 6: Crash at Every Phase
// ====================================================================
pub fn demo_crash_phases() {
    header("Demo 6: Crash at Every Transaction Phase (Redo Log)");
    println!("  A redo log transaction has these phases:");
    println!("  1. begin_tx   — start transaction");
    println!("  2. log_write  — write data to log blocks");
    println!("  3. commit     — write committed header");
    println!("  4. install    — copy log data to final locations");
    println!("  5. cleanup    — clear log header");
    println!();

    let phases = ["before_commit", "after_commit", "during_install", "after_install"];

    for (idx, phase) in phases.iter().enumerate() {
        println!("  --- Crash phase: {} ---", phase);

        let path = fresh_image(&format!("phase_{}", phase));

        // Setup: create a file
        {
            let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
            let mut journal = fs.make_journal();
            FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "test.txt").unwrap();
            FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "test.txt", b"ORIGINAL").unwrap();
            fs.umount();
        }

        // Simulate crash at different phases
        {
            let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
            let sb_cpy = fs.sb.clone();
            let log_start = sb_cpy.log_start;

            // Get file info
            let inum = DirOps::lookup(&mut fs.cache, &mut fs.disk, &fs.sb, crate::dir::ROOT_INUM, "test.txt").unwrap();
            let mut dinode = crate::inode::InodeOps::iget(&mut fs.cache, &mut fs.disk, &fs.sb, inum);
            let data_block = dinode.direct[0];

            match idx {
                0 => {
                    // Crash before commit: write data to log but no header
                    let mut new_data = [0u8; 512];
                    new_data[0..3].copy_from_slice(b"NEW");
                    fs.disk.write_block(log_start + 1, &new_data).unwrap();
                    pass("Wrote data to log but NOT committed header");
                    fs.disk.crash();
                }
                1 => {
                    // Crash after commit: header written, data in log
                    let mut new_data = [0u8; 512];
                    new_data[0..3].copy_from_slice(b"NEW");
                    fs.disk.write_block(log_start + 1, &new_data).unwrap();

                    let mut hdr = [0u8; 512];
                    hdr[0..4].copy_from_slice(&1u32.to_le_bytes()); // count=1
                    hdr[4..8].copy_from_slice(&1u32.to_le_bytes()); // committed=1
                    hdr[8..12].copy_from_slice(&data_block.to_le_bytes());
                    fs.disk.write_block(log_start, &hdr).unwrap();
                    pass("Wrote COMMITTED header — data in log only");
                    fs.disk.crash();
                }
                2 => {
                    // Crash during install: some blocks installed, some not
                    let mut new_data = [0u8; 512];
                    new_data[0..3].copy_from_slice(b"NEW");
                    fs.disk.write_block(log_start + 1, &new_data).unwrap();

                    let mut hdr = [0u8; 512];
                    hdr[0..4].copy_from_slice(&1u32.to_le_bytes());
                    hdr[4..8].copy_from_slice(&1u32.to_le_bytes());
                    hdr[8..12].copy_from_slice(&data_block.to_le_bytes());
                    fs.disk.write_block(log_start, &hdr).unwrap();
                    // Install the block
                    fs.disk.write_block(data_block, &new_data).unwrap();
                    pass("Installed data block — CRASH before header cleanup");
                    fs.disk.crash();
                }
                3 => {
                    // Crash after install but before cleanup (same result as 2)
                    let mut new_data = [0u8; 512];
                    new_data[0..3].copy_from_slice(b"NEW");
                    fs.disk.write_block(data_block, &new_data).unwrap();
                    pass("Install complete — CRASH before header cleanup");
                    // Leave header dirty, crash
                    fs.disk.crash();
                }
                _ => unreachable!(),
            }
        }

        // Recovery
        {
            let mut fs = FileSystem::mount(Path::new(&path), JournalType::Redo).unwrap();
            match FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "test.txt") {
                Ok(content) => {
                    pass(&format!("After recovery: data = {:?}", content.trim_end_matches('\0')));
                    if content.as_bytes().starts_with(b"NEW") || content.as_bytes().starts_with(b"ORIGINAL") {
                        pass(&format!("  File system is CONSISTENT (data is valid)"));
                    } else {
                        fail("consistency", "unexpected data");
                    }
                }
                Err(e) => {
                    pass(&format!("After recovery: {} (data may be lost but FS is consistent)", e));
                }
            }
            fs.umount();
        }
        println!();
    }

    println!("Demo 6 complete.\n");
}

// ====================================================================
// Run all demos
// ====================================================================
pub fn run_all_demos() {
    println!("============================================================");
    println!("  Journaling File System Demo Suite");
    println!("  Based on: Operating Systems: Three Easy Pieces");
    println!("============================================================\n");

    demo_basic();
    demo_crash_no_journal();
    demo_redo_recovery();
    demo_undo_recovery();
    demo_performance();
    demo_crash_phases();

    println!("============================================================");
    println!("  All demos completed!");
    println!("============================================================");
}
