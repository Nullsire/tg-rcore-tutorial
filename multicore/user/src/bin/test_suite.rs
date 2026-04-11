#![no_std]
#![no_main]

extern crate user;

use user::*;

fn sleep(ms: isize) {
    let start = get_time();
    while get_time() - start < ms {
        yield_();
    }
}

#[no_mangle]
fn main() -> i32 {
    println!("\n\n");
    println!("88b  88 88888 88b  888 88  88 888888 888b 88 888888 .o.");
    println!("88Yb88   88   88Yb 88  88  88 88__   88 Y88 88__   Yo");
    println!("88 Y88   88   88 Y8888  8888 888 88     88  88 88  88");
    println!("88  Y8   88   88  Y888  88  88 \"\"8 88____88 888888 888");
    println!("\nMultiCore OS - Comprehensive Test Suite\n");
    
    let tests = [
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
    ];
    
    let mut passed = 0;
    let mut failed = 0;
    
    for test in &tests {
        let child = fork();
        if child == 0 {
            // Child: run the test direct
            exec(test);
            exit(1);
        } else {
            // Parent: wait
            let mut exit_code: i32 = -1;
            waitpid(child, &mut exit_code);
            
            if exit_code == 0 {
                println!("[SUITE] {} PASSED", test);
                passed += 1;
            } else {
                println!("[SUITE] {} FAILED (exit code: {})", test, exit_code);
                failed += 1;
            }
            
            sleep(20);
        }
    }
    
    println!("\n\n================================================================================");
    println!("COMPREHENSIVE TEST RESULTS");
    println!("================================================================================");
    println!("Total Passed: {}", passed);
    println!("Total Failed: {}", failed);
    println!("Total Tests: {}", passed + failed);
    
    if failed == 0 {
        println!("✓ ALL TESTS PASSED!");
    } else {
        println!("✗ Some tests failed");
    }
    
    0
}
