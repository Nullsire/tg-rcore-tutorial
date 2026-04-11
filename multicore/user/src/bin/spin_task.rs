#![no_std]
#![no_main]

use user::{println, hart_id, get_time};

/// CPU-bound task for testing preemptive scheduling
#[no_mangle]
fn main() -> i32 {
    let start = get_time();
    let mut sum: u64 = 0;
    for i in 0..1_000_000u64 {
        sum = sum.wrapping_add(i);
    }
    let end = get_time();
    println!(
        "[spin_task] hart={}, sum={}, time={}ms",
        hart_id(), sum, end - start
    );
    0
}
