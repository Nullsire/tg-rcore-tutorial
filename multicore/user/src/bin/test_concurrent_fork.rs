#![no_std]
#![no_main]

extern crate user;

use user::*;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[test_concurrent_fork] Stress testing concurrent fork/wait...");

    let num_children = 8;
    let mut child_pids = [0isize; 8];
    let mut child_harts = [false; 4];

    // Fork 8 children simultaneously
    for i in 0..num_children {
        let pid = fork();
        if pid == 0 {
            let h = hart_id() as usize;
            println!("[test_concurrent_fork]   child {}: hart={}", i, h);
            exit(h as i32);
        } else {
            child_pids[i] = pid;
        }
    }

    // Parent: wait for all children
    let mut all_ok = true;
    let mut seen_pids = [0isize; 8];
    for i in 0..num_children {
        let mut exit_code: i32 = -1;
        let pid = waitpid(child_pids[i], &mut exit_code);
        seen_pids[i] = pid;
        if exit_code >= 0 && (exit_code as usize) < child_harts.len() {
            child_harts[exit_code as usize] = true;
        }
        if exit_code < 0 {
            println!(
                "[test_concurrent_fork]   child {} (pid={}): exit_code={}",
                i, pid, exit_code
            );
            all_ok = false;
        }
    }

    // Check for duplicate PIDs (shouldn't happen, but verify)
    for i in 0..num_children {
        for j in (i + 1)..num_children {
            if seen_pids[i] == seen_pids[j] && seen_pids[i] > 0 {
                println!(
                    "[test_concurrent_fork]   duplicate pid: {}",
                    seen_pids[i]
                );
                all_ok = false;
            }
        }
    }

    if all_ok {
        println!(
            "[test_concurrent_fork] Harts observed: [0={}, 1={}, 2={}, 3={}]",
            child_harts[0], child_harts[1], child_harts[2], child_harts[3]
        );
        println!("[test_concurrent_fork] PASS: all {} children exited normally", num_children);
        0
    } else {
        println!("[test_concurrent_fork] FAIL: some children failed");
        1
    }
}
