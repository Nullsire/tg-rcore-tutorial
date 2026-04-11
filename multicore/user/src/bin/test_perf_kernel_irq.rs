#![no_std]
#![no_main]

extern crate user;

use user::*;

struct RunResult {
    wall_time_ms: isize,
    ticks_delta: usize,
    kernel_timer_delta: usize,
}

fn run_workload(kernel_irq_enabled: bool) -> RunResult {
    let mode = if kernel_irq_enabled { "ON" } else { "OFF" };
    println!("[perf_kernel_irq] Run with kernel IRQ {}", mode);

    // Keep other factors fixed for fair comparison.
    active_harts_set(4);
    sched_set(0); // GlobalRoundRobin
    kernel_irq_set(kernel_irq_enabled);
    println!("[perf_kernel_irq] Config applied: active_harts=4, sched=0, kirq={}", mode);

    let children = 8;
    let mut pids = [0isize; 8];

    let stats_before = kernel_stats_parsed();
    let start = get_time();
    println!("[perf_kernel_irq] Starting fork phase ({ } children)", children);

    for i in 0..children {
        let pid = fork();
        if pid == 0 {
            let mut sum = 0u64;
            for j in 0..1_500_000 {
                sum = sum.wrapping_add(j as u64);
                if j % 50_000 == 0 {
                    yield_();
                }
                core::hint::black_box(&sum);
            }
            exit(0);
        } else {
            pids[i] = pid;
            if i == 0 {
                println!("[perf_kernel_irq] First child forked: pid={}", pid);
            }
        }
    }

    println!("[perf_kernel_irq] All children forked, starting wait phase");

    for i in 0..children {
        let mut code = -1;
        waitpid(pids[i], &mut code);
    }

    let end = get_time();
    let stats_after = kernel_stats_parsed();

    RunResult {
        wall_time_ms: end - start,
        ticks_delta: stats_after.ticks - stats_before.ticks,
        kernel_timer_delta: stats_after.kernel_timer - stats_before.kernel_timer,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_kernel_irq] Compare kernel IRQ ON vs OFF");

    let on = run_workload(true);
    let off = run_workload(false);

    // Restore default behavior for subsequent tests.
    kernel_irq_set(true);
    active_harts_set(4);

    println!();
    println!("| Mode         | Wall(ms) | Ticks | KernelTimer |");
    println!("|--------------|----------|-------|-------------|");
    println!(
        "| IRQ ON       | {:>8} | {:>5} | {:>11} |",
        on.wall_time_ms, on.ticks_delta, on.kernel_timer_delta
    );
    println!(
        "| IRQ OFF      | {:>8} | {:>5} | {:>11} |",
        off.wall_time_ms, off.ticks_delta, off.kernel_timer_delta
    );

    if on.wall_time_ms <= off.wall_time_ms {
        println!(
            "[perf_kernel_irq] IRQ ON faster/equal by {}ms",
            off.wall_time_ms - on.wall_time_ms
        );
    } else {
        println!(
            "[perf_kernel_irq] IRQ OFF faster by {}ms (workload-dependent)",
            on.wall_time_ms - off.wall_time_ms
        );
    }

    0
}
