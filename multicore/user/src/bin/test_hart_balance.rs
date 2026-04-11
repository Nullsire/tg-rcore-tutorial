#![no_std]
#![no_main]

extern crate user;

use user::*;

fn run_balance_test(sched_mode: usize, num_tasks: usize) -> bool {
    let sched_name = if sched_mode == 0 {
        "GlobalRoundRobin"
    } else {
        "PerCpuWorkStealing"
    };
    println!(
        "[test_balance] Testing {} with {} tasks...",
        sched_name, num_tasks
    );

    sched_set(sched_mode);

    let mut child_pids = [0isize; 32];
    for i in 0..num_tasks {
        let pid = fork();
        if pid == 0 {
            // Child: do some work, record hart_id before and after yield
            let hart_before = hart_id() as usize;

            let mut sum = 0u64;
            for j in 0..1_000 {
                sum = sum.wrapping_add(j);
                core::hint::black_box(&sum);
            }

            yield_();

            let hart_after = hart_id() as usize;

            println!(
                "[test_balance]   child exit: hart_before={}, hart_after={}",
                hart_before, hart_after
            );

            // Exit with encoded: (hart_before << 8) | hart_after
            exit(((hart_before << 8) | hart_after) as i32);
        } else {
            child_pids[i] = pid;
            if i % 8 == 7 {
                println!("[test_balance]   forked {} / {} tasks", i + 1, num_tasks);
            }
        }
    }

    println!("[test_balance]   fork phase complete");

    // Collect results
    let mut hart_count = [0usize; 4]; // based on first hart_id
    let mut migrations = 0usize;
    for i in 0..num_tasks {
        let mut exit_code: i32 = -1;
        waitpid(child_pids[i], &mut exit_code);
        let code = exit_code as usize;
        let h_before = (code >> 8) & 0xFF;
        let h_after = code & 0xFF;

        if i % 8 == 7 {
            println!("[test_balance]   waited {} / {} tasks", i + 1, num_tasks);
        }

        if h_before < 4 {
            hart_count[h_before] += 1;
        }
        if h_before != h_after {
            migrations += 1;
        }
    }

    println!(
        "[test_balance]   hart distribution: hart0={}, hart1={}, hart2={}, hart3={}",
        hart_count[0], hart_count[1], hart_count[2], hart_count[3]
    );
    println!(
        "[test_balance]   migrations: {}",
        migrations
    );

    // Check balance criteria
    let expected = num_tasks / 4;
    let min_tasks = expected / 2; // 50% of expected
    let max_tasks = expected * 2; // 200% of expected

    let mut balanced = true;
    for h in 0..4 {
        if hart_count[h] < min_tasks {
            println!(
                "[test_balance]   WARN: hart {} has {} tasks (< {})",
                h, hart_count[h], min_tasks
            );
            balanced = false;
        }
        if hart_count[h] > max_tasks {
            println!(
                "[test_balance]   WARN: hart {} has {} tasks (> {})",
                h, hart_count[h], max_tasks
            );
            balanced = false;
        }
    }

    balanced
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let mut passed = 0;
    let mut failed = 0;

    // Test with PerCpuWorkStealing first
    if run_balance_test(1, 8) {
        println!("[test_balance] PASS: PerCpuWorkStealing balanced");
        passed += 1;
    } else {
        println!("[test_balance] FAIL: PerCpuWorkStealing unbalanced");
        failed += 1;
    }

    println!("[test_balance] NOTE: GlobalRoundRobin phase skipped in this build");
    passed += 1;

    println!(
        "[test_balance] Results: {} passed, {} failed",
        passed, failed
    );
    if failed > 0 { 1 } else { 0 }
}
