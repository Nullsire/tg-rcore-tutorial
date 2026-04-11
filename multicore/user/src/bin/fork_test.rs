#![no_std]
#![no_main]

use user::{println, fork, getpid, wait, exit, hart_id};

#[no_mangle]
fn main() -> i32 {
    println!("[fork_test] parent pid = {}", getpid());
    let pid = fork();
    if pid == 0 {
        // Child
        println!("[fork_test] child: pid={}, hart={}", getpid(), hart_id());
        exit(42);
    } else {
        // Parent
        println!("[fork_test] forked child pid = {}", pid);
        let mut exit_code: i32 = 0;
        let waited = wait(&mut exit_code);
        println!("[fork_test] child {} exited with code {}", waited, exit_code);
    }
    0
}
