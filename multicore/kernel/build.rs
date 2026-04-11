use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    // Linker script
    let ld_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=../user/src/");

    // Find user ELF binaries (not stripped - we need ELF headers for loading)
    let elf_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/riscv64gc-unknown-none-elf/release");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let app_names = [
        "fork_test",
        "hello",
        "multicore_test",
        "shell",
        "spin_task",
        "thread_test",
        "yield_test",
        "test_timer_interrupt",
        "test_preemption",
        "test_multicore_online",
        "test_concurrent_fork",
        "test_hart_balance",
        "test_work_stealing",
        "test_perf_throughput",
        "test_perf_contention",
        "test_perf_mixed",
        "test_perf_kernel_irq",
        "test_perf_single_vs_multi",
        "test_perf_single_vs_multi_thread",
        "test_perf_stdio_concurrent",
        "test_perf_fs_workload",
        "test_signal_basic",
        "test_suite",
    ];

    let mut apps: Vec<String> = Vec::new();
    for name in &app_names {
        let elf_path = elf_dir.join(name);
        if elf_path.exists() {
            apps.push(name.to_string());
        }
    }

    // Generate link_app.S
    let mut f = fs::File::create(format!("{}/link_app.S", out_dir)).unwrap();

    writeln!(f, "    .align 3").unwrap();
    writeln!(f, "    .section .data").unwrap();
    writeln!(f, "    .global _num_app").unwrap();
    writeln!(f, "_num_app:").unwrap();
    writeln!(f, "    .quad {}", apps.len()).unwrap();

    for i in 0..=apps.len() {
        writeln!(f, "    .quad app_{}_start", i).unwrap();
    }

    // App names (null-terminated strings)
    writeln!(f, "    .global _app_names").unwrap();
    writeln!(f, "_app_names:").unwrap();
    for app in &apps {
        writeln!(f, "    .string \"{}\"", app).unwrap();
    }

    // App data (include ELF files, not stripped binaries)
    for (i, app) in apps.iter().enumerate() {
        let elf_path = elf_dir.join(app);
        let abs_path = fs::canonicalize(&elf_path)
            .unwrap_or_else(|_| elf_path.clone());
        writeln!(f, "    .section .data").unwrap();
        writeln!(f, "    .global app_{}_start", i).unwrap();
        writeln!(f, "    .global app_{}_end", i).unwrap();
        writeln!(f, "    .align 3").unwrap();
        writeln!(f, "app_{}_start:", i).unwrap();
        writeln!(f, "    .incbin \"{}\"", abs_path.display()).unwrap();
        writeln!(f, "app_{}_end:", i).unwrap();
    }
    // Sentinel
    writeln!(f, "    .global app_{}_start", apps.len()).unwrap();
    writeln!(f, "app_{}_start:", apps.len()).unwrap();
}
