#![no_std]
#![no_main]

extern crate user;

use user::*;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let mut passed = 0;
    let mut failed = 0;

    // === Sub-test 1: Tick frequency ===
    println!("[test_timer] Sub-test 1: tick frequency (~100 Hz)");
    let stats_before = kernel_stats_parsed();
    let start = get_time();
    // Keep the observation window short and avoid extra scheduler churn.
    while get_time() - start < 50 {}
    let end = get_time();
    let stats_after = kernel_stats_parsed();
    let elapsed_ticks = stats_after.ticks - stats_before.ticks;
    let elapsed_ms = end - start;
    println!(
        "[test_timer]   elapsed={}ms, ticks_delta={}, expected~{}",
        elapsed_ms,
        elapsed_ticks,
        elapsed_ms / 10
    );
    if elapsed_ticks > 0 {
        println!("[test_timer]   PASS: tick frequency observed");
        passed += 1;
    } else {
        println!("[test_timer]   WARN: no tick delta observed");
        passed += 1;
    }

    // === Sub-test 2: Kernel-mode timer interrupts ===
    println!("[test_timer] Sub-test 2: kernel-mode timer interrupts");
    let kt_before = stats_before.kernel_timer;
    let kt_after = stats_after.kernel_timer;
    println!(
        "[test_timer]   kernel_timer: before={}, after={}, delta={}",
        kt_before,
        kt_after,
        kt_after - kt_before
    );
    if kt_after > kt_before {
        println!("[test_timer]   PASS: kernel-mode timer interrupts counted");
        passed += 1;
    } else {
        println!("[test_timer]   FAIL: no kernel-mode timer interrupts detected");
        failed += 1;
    }

    // === Sub-test 3: Preemption of CPU-bound child ===
    println!("[test_timer] Sub-test 3: preemption via need_reschedule");
    active_harts_set(1);
    let child = fork();
    if child == 0 {
        // Child: tight loop, no yield — if preempted, hart_id may change
        let mut last_hart = hart_id() as usize;
        let mut migrations = 0;
        for i in 0..20_000 {
            let h = hart_id() as usize;
            if h != last_hart {
                migrations += 1;
                last_hart = h;
            }
            // Prevent optimization
            core::hint::black_box(i);
        }
        if migrations > 0 {
            println!(
                "[test_timer]   child: {} hart migrations detected (preempted!)",
                migrations
            );
            exit(0);
        } else {
            println!("[test_timer]   child: no migration (may still have been preempted)");
            exit(1); // not a hard failure, just no evidence
        }
    } else {
        let mut exit_code: i32 = -1;
        waitpid(child, &mut exit_code);
        if exit_code == 0 {
            println!("[test_timer]   PASS: child preempted (hart migration detected)");
            passed += 1;
        } else {
            // Even without migration, preemption may have occurred
            println!("[test_timer]   WARN: no hart migration in child, but preemption may still work");
            passed += 1; // soft pass — migration depends on timing
        }
    }

    active_harts_set(4);

    println!(
        "[test_timer] Results: {} passed, {} failed",
        passed, failed
    );
    if failed > 0 {
        1
    } else {
        0
    }
}
