#![no_std]
#![no_main]

extern crate user;

use user::*;

struct PerfResult {
    avg_yield_ms: isize,
    wall_time_ms: isize,
    ticks: usize,
}

fn run_throughput_test(sched_mode: usize) -> PerfResult {
    let sched_name = if sched_mode == 0 {
        "GlobalRR"
    } else {
        "PerCpuWS"
    };
    println!("[perf_throughput] Running with {}...", sched_name);

    sched_set(sched_mode);

    let num_children = 16;
    let yields_per_child = 5000;

    let wall_start = get_time();
    let stats_before = kernel_stats_parsed();

    let mut child_pids = [0isize; 16];
    for i in 0..num_children {
        let pid = fork();
        if pid == 0 {
            let t0 = get_time();
            for _ in 0..yields_per_child {
                yield_();
            }
            let t1 = get_time();
            let elapsed = t1 - t0;
            // Exit with encoded elapsed time (max ~30s = 30000ms fits in i32)
            exit(elapsed as i32);
        } else {
            child_pids[i] = pid;
        }
    }

    // Collect results
    let mut total_elapsed: isize = 0;
    for i in 0..num_children {
        let mut exit_code: i32 = -1;
        waitpid(child_pids[i], &mut exit_code);
        total_elapsed += exit_code as isize;
    }

    let wall_end = get_time();
    let stats_after = kernel_stats_parsed();

    let avg_yield_us = total_elapsed * 1000 / (num_children as isize * yields_per_child as isize);
    let wall_time_ms = wall_end - wall_start;
    let ticks = stats_after.ticks - stats_before.ticks;

    println!(
        "[perf_throughput]   {}: avg_yield={}us, wall={}ms, ticks={}",
        sched_name,
        avg_yield_us,
        wall_time_ms,
        ticks
    );

    PerfResult {
        avg_yield_ms: avg_yield_us / 1000,
        wall_time_ms,
        ticks,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_throughput] Scheduler throughput comparison (16 tasks, 5000 yields each)");
    println!();

    let rr = run_throughput_test(0);
    let ws = run_throughput_test(1);

    println!();
    println!("| Scheduler         | Avg yield (us) | Wall time (ms) | Ticks |");
    println!("|-------------------|----------------|----------------|-------|");
    println!(
        "| GlobalRoundRobin  | {:>14} | {:>14} | {:>5} |",
        rr.avg_yield_ms * 1000,
        rr.wall_time_ms,
        rr.ticks
    );
    println!(
        "| PerCpuWorkStealing| {:>14} | {:>14} | {:>5} |",
        ws.avg_yield_ms * 1000,
        ws.wall_time_ms,
        ws.ticks
    );

    // PerCpuWS should be faster (lower latency, lower wall time)
    if ws.avg_yield_ms <= rr.avg_yield_ms {
        println!("[perf_throughput] PerCpuWorkStealing has lower or equal yield latency");
    } else {
        println!("[perf_throughput] GlobalRoundRobin was faster (may happen with few tasks)");
    }

    0 // informational test, always "passes"
}
