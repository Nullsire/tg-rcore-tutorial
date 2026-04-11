#![no_std]
#![no_main]

extern crate user;

use core::sync::atomic::{AtomicIsize, Ordering};
use user::*;

const WORKERS: usize = 4;
const ROUNDS: usize = 5;
static THREAD_TIME_MS: [AtomicIsize; WORKERS] = [
    AtomicIsize::new(0),
    AtomicIsize::new(0),
    AtomicIsize::new(0),
    AtomicIsize::new(0),
];

#[no_mangle]
fn perf_thread_worker(arg: usize) -> i32 {
    let idx = arg & 0xff;
    let loops = 600_000 + (idx * 40_000);
    let t0 = get_time();
    let mut sum = 0u64;
    for j in 0..loops {
        sum = sum.wrapping_add((j + idx) as u64);
        if j % 40_000 == 0 {
            yield_();
        }
        core::hint::black_box(&sum);
    }
    let t1 = get_time();
    THREAD_TIME_MS[idx].store(t1 - t0, Ordering::Relaxed);
    0
}

fn run_case(harts: usize) -> isize {
    active_harts_set(harts);
    sched_set(0);
    kernel_irq_set(true);

    let begin = get_time();
    let mut tids = [0usize; WORKERS];
    for i in 0..WORKERS {
        THREAD_TIME_MS[i].store(0, Ordering::Relaxed);
        let tid = thread_create(perf_thread_worker, i) as usize;
        tids[i] = tid;
    }
    for i in 0..WORKERS {
        waittid(tids[i]);
    }
    let end = get_time();
    end - begin
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_single_multi_thread] Thread-based single-core vs multi-core");

    let mut one_samples = [0isize; ROUNDS];
    let mut four_samples = [0isize; ROUNDS];

    for i in 0..ROUNDS {
        one_samples[i] = run_case(1);
    }
    for i in 0..ROUNDS {
        four_samples[i] = run_case(4);
    }

    active_harts_set(4);

    let one_sum: isize = one_samples.iter().sum();
    let four_sum: isize = four_samples.iter().sum();
    let one_avg = one_sum / ROUNDS as isize;
    let four_avg = four_sum / ROUNDS as isize;

    let one_min = *one_samples.iter().min().unwrap_or(&0);
    let one_max = *one_samples.iter().max().unwrap_or(&0);
    let four_min = *four_samples.iter().min().unwrap_or(&0);
    let four_max = *four_samples.iter().max().unwrap_or(&0);

    println!("| Mode | Avg(ms) | Min(ms) | Max(ms) | Samples |");
    println!("|------|---------|---------|---------|---------|");
    println!(
        "| 1 hart (thread) | {:>7} | {:>7} | {:>7} | {:?} |",
        one_avg, one_min, one_max, one_samples
    );
    println!(
        "| 4 harts (thread)| {:>7} | {:>7} | {:>7} | {:?} |",
        four_avg, four_min, four_max, four_samples
    );

    if four_avg < one_avg {
        let speedup_x100 = one_avg * 100 / four_avg;
        println!(
            "[perf_single_multi_thread] 4-hart faster, speedup ~= {}.{:02}x",
            speedup_x100 / 100,
            speedup_x100 % 100
        );
    } else {
        println!("[perf_single_multi_thread] no speedup observed (workload-dependent)");
    }

    0
}
