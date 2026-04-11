#![no_std]
#![no_main]

use user::{println, fork, getpid, wait, exit, hart_id, yield_};

/// Test multi-core by forking multiple processes that run on different harts
#[no_mangle]
fn main() -> i32 {
    println!("[multicore_test] starting on hart {}", hart_id());

    // Fork 4 child processes
    for i in 0..4 {
        let pid = fork();
        if pid == 0 {
            // Child: busy loop to observe hart assignment
            let my_pid = getpid();
            for j in 0..3 {
                println!(
                    "[multicore_test] child {}: pid={}, hart={}, iter={}",
                    i, my_pid, hart_id(), j
                );
                yield_();
            }
            exit(0);
        }
    }

    // Parent: wait for all children
    let mut exit_code = 0;
    for _ in 0..4 {
        let pid = wait(&mut exit_code);
        println!("[multicore_test] child {} exited", pid);
    }
    println!("[multicore_test] all children done, test passed!");
    0
}
