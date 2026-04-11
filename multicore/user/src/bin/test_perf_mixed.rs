#![no_std]
#![no_main]

extern crate user;

use user::*;

struct MixedResult {
    wall_time_ms: isize,
}

fn run_mixed_test(sched_mode: usize) -> MixedResult {
    let sched_name = if sched_mode == 0 {
        "GlobalRR"
    } else {
        "PerCpuWS"
    };
    println!("[perf_mixed] Running with {}...", sched_name);

    sched_set(sched_mode);

    let wall_start = get_time();

    let mut child_pids = [0isize; 4];

    // 2 CPU-bound tasks
    for i in 0..2 {
        let pid = fork();
        if pid == 0 {
            // CPU-bound: computation + yields
            let mut sum = 0u64;
            for j in 0..1_000_000 {
                sum = sum.wrapping_add(j);
                sum = core::hint::black_box(sum);
            }
            // Do some yields
            for _ in 0..10 {
                yield_();
            }
            exit(0);
        } else {
            child_pids[i] = pid;
        }
    }

    // 2 I/O-bound tasks (light waiting then exit)
    for i in 2..4 {
        let pid = fork();
        if pid == 0 {
            // Simulate I/O with a short wait/yield loop.
            let start = get_time();
            while get_time() - start < 100 {
                yield_();
            }
            exit(0);
        } else {
            child_pids[i] = pid;
        }
    }

    // Wait for all children
    for i in 0..4 {
        let mut exit_code: i32 = -1;
        waitpid(child_pids[i], &mut exit_code);
    }

    let wall_end = get_time();

    let wall_time_ms = wall_end - wall_start;
    println!(
        "[perf_mixed]   {}: wall_time={}ms",
        sched_name, wall_time_ms
    );

    MixedResult { wall_time_ms }
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_mixed] Mixed workload test (2 CPU-bound + 2 I/O-bound)");
    println!();

    let rr = run_mixed_test(0);
    let ws = run_mixed_test(1);

    println!();
    println!("| Scheduler         | Wall time (ms) |");
    println!("|-------------------|----------------|");
    println!(
        "| GlobalRoundRobin  | {:>14} |",
        rr.wall_time_ms
    );
    println!(
        "| PerCpuWorkStealing| {:>14} |",
        ws.wall_time_ms
    );

    if ws.wall_time_ms < rr.wall_time_ms {
        println!(
            "[perf_mixed] PerCpuWorkStealing was faster by {}ms",
            rr.wall_time_ms - ws.wall_time_ms
        );
    } else {
        println!(
            "[perf_mixed] GlobalRoundRobin was faster by {}ms",
            ws.wall_time_ms - rr.wall_time_ms
        );
    }

    0 // informational
}
