#![no_std]
#![no_main]

extern crate user;

use user::*;

struct ContentionResult {
    avg_completion_ms: isize,
    wall_time_ms: isize,
    hart_histo: [usize; 4],
}

fn run_contention_test(sched_mode: usize) -> ContentionResult {
    let sched_name = if sched_mode == 0 {
        "GlobalRR"
    } else {
        "PerCpuWS"
    };
    println!("[perf_contention] Running with {}...", sched_name);

    sched_set(sched_mode);

    let num_children = 4usize;
    let work_iters = 500_000usize;
    let yield_interval = (work_iters / 4).max(1);

    let wall_start = get_time();
    let mut child_pids = [0isize; 4];

    for i in 0..num_children {
        let pid = fork();
        if pid == 0 {
            let t0 = get_time();
            let h0 = hart_id() as usize;

            let mut sum = 0u64;
            for j in 0..work_iters {
                sum = sum.wrapping_add(j as u64);
                sum = core::hint::black_box(sum);
                if j % yield_interval == 0 {
                    yield_();
                }
            }

            let elapsed = get_time() - t0;
            let h1 = hart_id() as usize;
            let encoded = ((elapsed as usize / 10) << 16) | (h0 << 8) | h1;
            exit(encoded as i32);
        } else {
            child_pids[i] = pid;
        }
    }

    let mut total_completion: isize = 0;
    let mut hart_histo = [0usize; 4];
    for i in 0..num_children {
        let mut exit_code: i32 = -1;
        waitpid(child_pids[i], &mut exit_code);
        let code = exit_code as u32 as usize;
        let elapsed_10ms = code >> 16;
        let h0 = (code >> 8) & 0xFF;
        total_completion += elapsed_10ms as isize * 10;
        if h0 < 4 {
            hart_histo[h0] += 1;
        }
    }

    let wall_time_ms = get_time() - wall_start;
    let avg_completion_ms = total_completion / num_children as isize;

    println!(
        "[perf_contention]   {}: avg_completion={}ms, wall={}ms",
        sched_name, avg_completion_ms, wall_time_ms
    );
    println!(
        "[perf_contention]   {}: hart_dist=[{}, {}, {}, {}]",
        sched_name, hart_histo[0], hart_histo[1], hart_histo[2], hart_histo[3]
    );

    ContentionResult {
        avg_completion_ms,
        wall_time_ms,
        hart_histo,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_contention] High contention test (4 CPU-bound tasks)");
    println!();

    let rr = run_contention_test(0);
    let ws = run_contention_test(1);

    println!();
    println!("| Scheduler         | Avg completion (ms) | Wall time (ms) |");
    println!("|-------------------|---------------------|----------------|");
    println!(
        "| GlobalRoundRobin  | {:>19} | {:>14} |",
        rr.avg_completion_ms, rr.wall_time_ms
    );
    println!(
        "| PerCpuWorkStealing| {:>19} | {:>14} |",
        ws.avg_completion_ms, ws.wall_time_ms
    );

    let rr_mean = 1isize;
    let mut rr_var: isize = 0;
    let mut ws_var: isize = 0;
    for h in 0..4 {
        let rr_dev = rr.hart_histo[h] as isize - rr_mean;
        rr_var += rr_dev * rr_dev;
        let ws_dev = ws.hart_histo[h] as isize - rr_mean;
        ws_var += ws_dev * ws_dev;
    }

    let rr_var_x100 = rr_var * 100 / 4;
    let ws_var_x100 = ws_var * 100 / 4;

    fn isqrt(n: isize) -> isize {
        if n <= 0 {
            return 0;
        }
        let mut x = n;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + n / x) / 2;
        }
        x
    }

    let rr_stddev = isqrt(rr_var_x100);
    let ws_stddev = isqrt(ws_var_x100);
    println!(
        "| Hart balance stddev (x100): RR={}, WS={} |",
        rr_stddev, ws_stddev
    );

    0
}
