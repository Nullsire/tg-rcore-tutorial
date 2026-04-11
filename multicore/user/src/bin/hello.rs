#![no_std]
#![no_main]

use user::{println, hart_id, getpid};

#[no_mangle]
fn main() -> i32 {
    println!("Hello from user space!");
    println!("  PID = {}, running on hart {}", getpid(), hart_id());
    0
}
