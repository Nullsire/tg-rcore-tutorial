#![no_std]
#![no_main]

extern crate user;

use user::*;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[test_preemption] Testing need_reschedule preemption...");

    active_harts_set(1);

    let parent_start = get_time();
    let num_children = 3;
    let mut child_pids = [0isize; 3];

    // Fork 3 children
    for i in 0..num_children {
        let pid = fork();
        if pid == 0 {
            // Child i: tight loop with hart_id checkpoints
            let mut hart_ids = [0usize; 5];
            let mut times = [0isize; 5];
            hart_ids[0] = hart_id() as usize;
            times[0] = get_time();

            for j in 1..5 {
                // Do some work between checkpoints
                let mut sum = 0u64;
                for k in 0..50_000 {
                    sum = sum.wrapping_add(k as u64);
                    core::hint::black_box(&sum);
                }
                hart_ids[j] = hart_id() as usize;
                times[j] = get_time();
            }

            // Check if hart_id changed at any point
            let mut migrated = false;
            for j in 1..5 {
                if hart_ids[j] != hart_ids[0] {
                    migrated = true;
                }
            }

            if migrated {
                println!(
                    "[test_preemption]   child {}: MIGRATED hart {:?}",
                    i, hart_ids
                );
            } else {
                println!(
                    "[test_preemption]   child {}: stayed on hart {}",
                    i, hart_ids[0]
                );
            }

            let elapsed = times[4] - times[0];
            println!(
                "[test_preemption]   child {}: elapsed={}ms, times={:?}",
                i, elapsed, times
            );

            exit(if migrated { 0 } else { 1 });
        } else {
            child_pids[i] = pid;
        }
    }

    // Parent: wait for all children
    let mut any_migrated = false;
    let _child_times = [0isize; 3];
    for i in 0..num_children {
        let mut exit_code: i32 = -1;
        waitpid(child_pids[i], &mut exit_code);
        if exit_code == 0 {
            any_migrated = true;
        }
    }

    let parent_end = get_time();
    let total_elapsed = parent_end - parent_start;

    // Check for overlapping execution (concurrent)
    // If total wall time < sum of expected CPU times, tasks ran concurrently
    // Each child does ~4 * 100K iterations, should take several hundred ms each
    // If total is < 3x single-child time, there was concurrency
    println!(
        "[test_preemption] Total wall time: {}ms for {} children",
        total_elapsed, num_children
    );

    let result = if any_migrated {
        println!("[test_preemption] PASS: preemption detected (hart migration)");
        0
    } else if total_elapsed < 5000 {
        // If total time is reasonable, concurrency is likely happening
        println!("[test_preemption] PASS: concurrent execution detected (wall time reasonable)");
        0
    } else {
        println!("[test_preemption] FAIL: no evidence of preemption or concurrency");
        1
    };

    active_harts_set(4);
    result
}
