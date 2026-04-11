#![no_std]
#![no_main]

extern crate user;

use user::*;

fn run_case(harts: usize) -> isize {
    println!("[perf_single_multi] Run with active_harts={}", harts);

    active_harts_set(harts);
    kernel_irq_set(true);
    sched_set(0); // GlobalRoundRobin
    println!("[perf_single_multi] Config applied: active_harts={}", harts);

    let start = get_time();
    let (children, work_iters, yield_every) = if harts == 1 {
        (1, 2_000_000, 100_000)
    } else {
        (4, 500_000, 25_000)
    };
    let mut pids = [0isize; 4];

    if harts == 1 {
        println!("[perf_single_multi] Running sequential workload on 1 hart");
        for i in 0..children {
            let pid = fork();
            if pid == 0 {
                let mut sum = 0u64;
                for j in 0..work_iters {
                    sum = sum.wrapping_add((j + i) as u64);
                    if j % yield_every == 0 {
                        yield_();
                    }
                    core::hint::black_box(&sum);
                }
                exit(0);
            } else {
                let mut code = -1;
                waitpid(pid, &mut code);
            }
        }
    } else {
        println!("[perf_single_multi] Running concurrent workload on 4 harts");
        for i in 0..children {
            let pid = fork();
            if pid == 0 {
                let mut sum = 0u64;
                for j in 0..work_iters {
                    sum = sum.wrapping_add((j + i) as u64);
                    if j % yield_every == 0 {
                        yield_();
                    }
                    core::hint::black_box(&sum);
                }
                exit(0);
            } else {
                pids[i] = pid;
                if i == 0 {
                    println!("[perf_single_multi] First child forked: pid={}", pid);
                }
            }
        }

        println!("[perf_single_multi] All children forked, starting wait phase");

        for i in 0..children {
            let mut code = -1;
            waitpid(pids[i], &mut code);
        }
    }

    get_time() - start
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_single_multi] Compare single-core vs multi-core");

    let t1 = run_case(1);
    let t4 = run_case(4);

    // Restore default for following tests.
    active_harts_set(4);

    println!();
    println!("| Active Harts | Wall time (ms) |");
    println!("|--------------|----------------|");
    println!("| 1            | {:>14} |", t1);
    println!("| 4            | {:>14} |", t4);

    if t4 < t1 {
        let speedup_x100 = (t1 * 100) / t4;
        println!(
            "[perf_single_multi] 4-core faster, speedup ~= {}.{:02}x",
            speedup_x100 / 100,
            speedup_x100 % 100
        );
    } else {
        println!("[perf_single_multi] no speedup observed (workload/platform-dependent)");
    }

    0
}
