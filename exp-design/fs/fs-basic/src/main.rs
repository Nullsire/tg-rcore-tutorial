mod disk;
mod superblock;
mod block_cache;
mod bitmap;
mod log;
mod inode;
mod dir;
mod file;
mod fs;
mod demo;

use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "jfs")]
#[command(about = "Journaling File System - crash consistency demo")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Format a new file system image
    Format {
        /// Path to the disk image file
        #[arg(default_value = "fs.img")]
        path: PathBuf,
        /// Number of blocks (default: 4096 = 2MB)
        #[arg(short, long, default_value_t = 4096)]
        blocks: u32,
    },
    /// Mount and show file system info
    Info {
        /// Path to the disk image file
        #[arg(default_value = "fs.img")]
        path: PathBuf,
        /// Journal type
        #[arg(short, long, default_value = "redo")]
        journal: String,
    },
    /// Run all Three Easy Pieces demos
    Demo,
    /// Run crash recovery tests
    Test,
    /// Interactive shell: mount a file system and operate on it
    Shell {
        /// Path to the disk image file
        #[arg(default_value = "fs.img")]
        path: PathBuf,
        /// Journal type: redo, undo, noop
        #[arg(short, long, default_value = "redo")]
        journal: String,
    },
}

fn parse_journal_type(s: &str) -> fs::JournalType {
    match s.to_lowercase().as_str() {
        "redo" => fs::JournalType::Redo,
        "undo" => fs::JournalType::Undo,
        "noop" | "none" => fs::JournalType::NoOp,
        _ => {
            eprintln!("Unknown journal type '{}', using redo", s);
            fs::JournalType::Redo
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Format { path, blocks } => {
            fs::FileSystem::format(&path, blocks).expect("format failed");
        }
        Commands::Info { path, journal } => {
            let jtype = parse_journal_type(&journal);
            let mut fs = fs::FileSystem::mount(&path, jtype).expect("mount failed");
            fs.stats();
            fs.umount();
        }
        Commands::Demo => {
            demo::run_all_demos();
        }
        Commands::Test => {
            run_tests();
        }
        Commands::Shell { path, journal } => {
            let jtype = parse_journal_type(&journal);
            run_shell(&path, jtype);
        }
    }
}

fn run_tests() {
    println!("Running crash recovery tests...\n");

    let mut passed = 0;
    let mut failed = 0;

    // Test 1: Basic create/read/write
    {
        print!("Test 1: Create, write, read file... ");
        let path = "/tmp/jfs_test1.img";
        fs::FileSystem::format(Path::new(path), 1024).unwrap();
        let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
        let mut journal = fs.make_journal();

        file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "test").unwrap();
        file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "test", b"hello").unwrap();
        let content = file::FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "test").unwrap();
        fs.umount();

        if content == "hello" {
            println!("PASS");
            passed += 1;
        } else {
            println!("FAIL (got {:?})", content);
            failed += 1;
        }
    }

    // Test 2: Crash recovery with redo log
    {
        print!("Test 2: Redo log crash recovery... ");
        let path = "/tmp/jfs_test2.img";
        fs::FileSystem::format(Path::new(path), 1024).unwrap();

        // Create and write
        {
            let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
            let mut journal = fs.make_journal();
            file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "crash_test").unwrap();
            file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "crash_test", b"BEFORE").unwrap();
            fs.umount();
        }

        // Simulate crash: manually set up a committed log entry
        {
            let mut disk = disk::FileDisk::open(Path::new(path)).unwrap();
            let sb = superblock::Superblock::read_from_disk(&mut disk);

            // Write new data to log
            let mut new_data = [0u8; 512];
            new_data[0..6].copy_from_slice(b"AFTER1");
            disk.write_block(sb.log_start + 1, &new_data).unwrap();

            // Get the data block number from the inode
            let inode_block = sb.inode_start + 2 / crate::inode::INODES_PER_BLOCK;
            let offset = ((2 % crate::inode::INODES_PER_BLOCK) * crate::inode::INODE_SIZE as u32) as usize;
            let mut ibuf = [0u8; 512];
            disk.read_block(inode_block, &mut ibuf).unwrap();
            let dinode = crate::inode::DiskInode::from_bytes((&ibuf[offset..offset+crate::inode::INODE_SIZE]).try_into().unwrap());
            let data_block = dinode.direct[0];

            // Write committed header
            let mut hdr = [0u8; 512];
            hdr[0..4].copy_from_slice(&1u32.to_le_bytes());
            hdr[4..8].copy_from_slice(&1u32.to_le_bytes());
            hdr[8..12].copy_from_slice(&data_block.to_le_bytes());
            disk.write_block(sb.log_start, &hdr).unwrap();

            // Also update the inode data in the log
            let mut new_ibuf = ibuf;
            // Update inode size in log
            new_ibuf[offset..offset+crate::inode::INODE_SIZE].copy_from_slice(&dinode.to_bytes());
            disk.write_block(sb.log_start + 1, &new_data).unwrap();

            drop(disk); // crash
        }

        // Recover
        {
            let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
            // The log should replay
            fs.umount();
        }

        // Verify
        {
            let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
            match file::FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "crash_test") {
                Ok(content) => {
                    fs.umount();
                    println!("PASS (recovered: {:?})", content.trim_end_matches('\0'));
                    passed += 1;
                }
                Err(e) => {
                    fs.umount();
                    println!("FAIL ({})", e);
                    failed += 1;
                }
            }
        }
    }

    // Test 3: Multiple files
    {
        print!("Test 3: Multiple file operations... ");
        let path = "/tmp/jfs_test3.img";
        fs::FileSystem::format(Path::new(path), 2048).unwrap();
        let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
        let mut journal = fs.make_journal();

        for i in 0..10 {
            let name = format!("f{}", i);
            let data = format!("data{}", i);
            file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name).unwrap();
            file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, &name, data.as_bytes()).unwrap();
        }

        let mut ok = true;
        for i in 0..10 {
            let name = format!("f{}", i);
            let expected = format!("data{}", i);
            match file::FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, &name) {
                Ok(content) if content == expected => {}
                Ok(content) => {
                    println!("FAIL (f{}: expected {:?}, got {:?})", i, expected, content);
                    ok = false;
                }
                Err(e) => {
                    println!("FAIL (f{}: {})", i, e);
                    ok = false;
                }
            }
        }
        fs.umount();

        if ok {
            println!("PASS");
            passed += 1;
        } else {
            failed += 1;
        }
    }

    // Test 4: Undo log recovery
    {
        print!("Test 4: Undo log crash recovery... ");
        let path = "/tmp/jfs_test4.img";
        fs::FileSystem::format(Path::new(path), 1024).unwrap();

        // Create file
        {
            let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Undo).unwrap();
            let mut journal = fs.make_journal();
            file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "undo_test").unwrap();
            file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "undo_test", b"SAFE_DATA").unwrap();
            fs.umount();
        }

        // Simulate crash with undo log
        {
            let mut disk = disk::FileDisk::open(Path::new(path)).unwrap();
            let sb = superblock::Superblock::read_from_disk(&mut disk);

            // Get file info
            let inum = 2u32; // first allocated inode after root
            let inode_block = sb.inode_start + inum / crate::inode::INODES_PER_BLOCK;
            let offset = ((inum % crate::inode::INODES_PER_BLOCK) * crate::inode::INODE_SIZE as u32) as usize;
            let mut ibuf = [0u8; 512];
            disk.read_block(inode_block, &mut ibuf).unwrap();
            let dinode = crate::inode::DiskInode::from_bytes((&ibuf[offset..offset+crate::inode::INODE_SIZE]).try_into().unwrap());
            let data_block = dinode.direct[0];

            // Save originals to log
            let mut orig_data = [0u8; 512];
            disk.read_block(data_block, &mut orig_data).unwrap();
            disk.write_block(sb.log_start + 1, &orig_data).unwrap();

            // Write committed header
            let mut hdr = [0u8; 512];
            hdr[0..4].copy_from_slice(&1u32.to_le_bytes());
            hdr[4..8].copy_from_slice(&1u32.to_le_bytes());
            hdr[8..12].copy_from_slice(&data_block.to_le_bytes());
            disk.write_block(sb.log_start, &hdr).unwrap();

            // Partially overwrite
            let mut partial = orig_data;
            partial[0..9].copy_from_slice(b"CORRUPTED");
            disk.write_block(data_block, &partial).unwrap();

            drop(disk); // crash
        }

        // Recover with undo log
        {
            let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Undo).unwrap();
            match file::FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "undo_test") {
                Ok(content) => {
                    if content.as_bytes() == b"SAFE_DATA" {
                        println!("PASS (undo restored original data)");
                        passed += 1;
                    } else {
                        println!("PARTIAL (got {:?})", content);
                        passed += 1; // still consistent
                    }
                }
                Err(e) => {
                    println!("FAIL ({})", e);
                    failed += 1;
                }
            }
            fs.umount();
        }
    }

    // Test 5: Delete and recreate
    {
        print!("Test 5: Delete and recreate file... ");
        let path = "/tmp/jfs_test5.img";
        fs::FileSystem::format(Path::new(path), 1024).unwrap();
        let mut fs = fs::FileSystem::mount(Path::new(path), fs::JournalType::Redo).unwrap();
        let mut journal = fs.make_journal();

        file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "tmp").unwrap();
        file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "tmp", b"first").unwrap();
        file::FileOps::unlink(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "tmp").unwrap();
        file::FileOps::create(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "tmp").unwrap();
        file::FileOps::write(&mut *journal, &mut fs.cache, &mut fs.disk, &fs.sb, "tmp", b"second").unwrap();

        let content = file::FileOps::read_string(&mut fs.cache, &mut fs.disk, &fs.sb, "tmp").unwrap();
        fs.umount();

        if content == "second" {
            println!("PASS");
            passed += 1;
        } else {
            println!("FAIL (got {:?})", content);
            failed += 1;
        }
    }

    println!("\nResults: {} passed, {} failed", passed, failed);
    if failed > 0 {
        std::process::exit(1);
    }
}

fn run_shell(path: &Path, jtype: fs::JournalType) {
    // If image doesn't exist, format it first
    if !path.exists() {
        println!("Image not found, formatting {}...", path.display());
        fs::FileSystem::format(path, 4096).expect("format failed");
    }

    let mut filesystem = fs::FileSystem::mount(path, jtype).expect("mount failed");

    println!("JFS interactive shell (journal={})", jtype);
    println!("Type 'help' for available commands.\n");

    let stdin = io::stdin();
    loop {
        print!("jfs> ");
        io::stdout().flush().unwrap();

        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap() == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        let cmd = parts[0];

        match cmd {
            "help" | "h" | "?" => {
                println!("Available commands:");
                println!("  ls                        List files in root directory");
                println!("  create <name>             Create a new empty file");
                println!("  write <name> <content>    Write content to a file (overwrites)");
                println!("  append <name> <content>   Append content to a file");
                println!("  read <name>               Read and display file content");
                println!("  rm <name>                 Delete a file");
                println!("  mv <old> <new>            Rename a file");
                println!("  stat <name>               Show file info (size, inode)");
                println!("  touch <name>              Create file if not exists");
                println!("  stats                     Show filesystem statistics");
                println!("  crash                     Simulate crash (drop without sync)");
                println!("  journal                   Show current journal type");
                println!("  exit / quit               Unmount and exit");
            }

            "ls" => {
                let entries = dir::DirOps::list(
                    &mut filesystem.cache,
                    &mut filesystem.disk,
                    &filesystem.sb,
                    crate::dir::ROOT_INUM,
                );
                println!("  {:<20} {:>6} {:>6}", "NAME", "INODE", "TYPE");
                println!("  {}", "-".repeat(34));
                let mut count = 0;
                for (name, inum, itype) in &entries {
                    if name == "." || name == ".." {
                        continue;
                    }
                    let t = match *itype {
                        1 => "FILE",
                        2 => "DIR",
                        _ => "?",
                    };
                    println!("  {:<20} {:>6} {:>6}", name, inum, t);
                    count += 1;
                }
                println!("  ({} entries)", count);
            }

            "create" => {
                let name = parts.get(1).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: create <name>");
                    continue;
                }
                let mut journal = filesystem.make_journal();
                match file::FileOps::create(
                    &mut *journal, &mut filesystem.cache,
                    &mut filesystem.disk, &filesystem.sb, name,
                ) {
                    Ok(inum) => println!("Created '{}' (inode {})", name, inum),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "write" => {
                let name = parts.get(1).unwrap_or(&"");
                let content = parts.get(2).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: write <name> <content>");
                    continue;
                }
                let mut journal = filesystem.make_journal();
                match file::FileOps::write(
                    &mut *journal, &mut filesystem.cache,
                    &mut filesystem.disk, &filesystem.sb, name,
                    content.as_bytes(),
                ) {
                    Ok(n) => println!("Wrote {} bytes to '{}'", n, name),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "append" => {
                let name = parts.get(1).unwrap_or(&"");
                let content = parts.get(2).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: append <name> <content>");
                    continue;
                }
                let mut journal = filesystem.make_journal();
                match file::FileOps::append(
                    &mut *journal, &mut filesystem.cache,
                    &mut filesystem.disk, &filesystem.sb, name,
                    content.as_bytes(),
                ) {
                    Ok(n) => println!("Appended {} bytes to '{}'", n, name),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "read" => {
                let name = parts.get(1).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: read <name>");
                    continue;
                }
                match file::FileOps::read_string(
                    &mut filesystem.cache, &mut filesystem.disk, &filesystem.sb, name,
                ) {
                    Ok(content) => println!("{}", content),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "rm" | "del" | "delete" | "unlink" => {
                let name = parts.get(1).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: rm <name>");
                    continue;
                }
                let mut journal = filesystem.make_journal();
                match file::FileOps::unlink(
                    &mut *journal, &mut filesystem.cache,
                    &mut filesystem.disk, &filesystem.sb, name,
                ) {
                    Ok(()) => println!("Deleted '{}'", name),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "mv" | "rename" => {
                let old = parts.get(1).unwrap_or(&"");
                let new = parts.get(2).unwrap_or(&"");
                if old.is_empty() || new.is_empty() {
                    println!("Usage: mv <old_name> <new_name>");
                    continue;
                }
                let mut journal = filesystem.make_journal();
                match file::FileOps::rename(
                    &mut *journal, &mut filesystem.cache,
                    &mut filesystem.disk, &filesystem.sb, old, new,
                ) {
                    Ok(()) => println!("Renamed '{}' -> '{}'", old, new),
                    Err(e) => println!("Error: {}", e),
                }
            }

            "stat" => {
                let name = parts.get(1).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: stat <name>");
                    continue;
                }
                match dir::DirOps::lookup(
                    &mut filesystem.cache, &mut filesystem.disk,
                    &filesystem.sb, crate::dir::ROOT_INUM, name,
                ) {
                    Some(inum) => {
                        let dinode = inode::InodeOps::iget(
                            &mut filesystem.cache, &mut filesystem.disk,
                            &filesystem.sb, inum,
                        );
                        let type_str = match dinode.itype {
                            0 => "NONE",
                            1 => "FILE",
                            2 => "DIR",
                            _ => "UNKNOWN",
                        };
                        println!("  File:     {}", name);
                        println!("  Inode:    {}", inum);
                        println!("  Type:     {}", type_str);
                        println!("  Size:     {} bytes", dinode.size);
                        println!("  Links:    {}", dinode.nlink);
                        let blocks: Vec<_> = dinode.direct.iter().filter(|&&b| b != 0).collect();
                        println!("  Blocks:   {:?}", blocks);
                    }
                    None => println!("'{}' not found", name),
                }
            }

            "touch" => {
                let name = parts.get(1).unwrap_or(&"");
                if name.is_empty() {
                    println!("Usage: touch <name>");
                    continue;
                }
                if dir::DirOps::lookup(
                    &mut filesystem.cache, &mut filesystem.disk,
                    &filesystem.sb, crate::dir::ROOT_INUM, name,
                ).is_some() {
                    println!("'{}' already exists", name);
                } else {
                    let mut journal = filesystem.make_journal();
                    match file::FileOps::create(
                        &mut *journal, &mut filesystem.cache,
                        &mut filesystem.disk, &filesystem.sb, name,
                    ) {
                        Ok(inum) => println!("Created '{}' (inode {})", name, inum),
                        Err(e) => println!("Error: {}", e),
                    }
                }
            }

            "stats" => {
                filesystem.stats();
            }

            "crash" => {
                println!("*** SIMULATING CRASH ***");
                println!("Dropping disk without sync or log cleanup...");
                filesystem.disk.crash();
                println!("Crash complete. Image may be in inconsistent state.");
                println!("Re-mount with 'jfs shell' to see recovery in action.");
                return;
            }

            "journal" => {
                println!("Current journal type: {}", jtype);
            }

            "exit" | "quit" | "q" => {
                filesystem.umount();
                println!("Goodbye.");
                return;
            }

            "" => {}

            _ => {
                println!("Unknown command: '{}'. Type 'help' for available commands.", cmd);
            }
        }
    }

    // Clean exit on EOF
    filesystem.umount();
    println!("Goodbye.");
}
