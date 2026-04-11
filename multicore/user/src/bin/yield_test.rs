#![no_std]
#![no_main]

use user::{println, yield_, hart_id, getpid};

#[no_mangle]
fn main() -> i32 {
    for i in 0..5 {
        println!(
            "[yield_test] pid={}, hart={}, iteration={}",
            getpid(), hart_id(), i
        );
        yield_();
    }
    0
}
