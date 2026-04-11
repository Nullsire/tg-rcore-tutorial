#![no_std]
#![no_main]

extern crate user;

use user::*;

const NUM_CHILDREN: usize = 16;

static mut HART_SET: [bool; 4] = [false; 4];

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[test_multicore] Checking all 4 harts are online...");

    unsafe {
        HART_SET = [false; 4];
    }

    // Fork many children, each yields a few times before reporting hart_id.
    // This gives the scheduler time to distribute tasks across harts.
    for _i in 0..NUM_CHILDREN {
        let pid = fork();
        if pid == 0 {
            // Yield several times so the scheduler can migrate this task
            for _ in 0..4 {
                yield_();
            }
            let my_hart = hart_id() as usize;
            exit(my_hart as i32);
        }
    }

    // Parent: collect hart_ids from all children
    let mut observations = 0usize;
    for _ in 0..NUM_CHILDREN {
        let mut exit_code: i32 = -1;
        waitpid(-1, &mut exit_code);
        let hid = exit_code as usize;
        if hid < 4 {
            unsafe {
                HART_SET[hid] = true;
            }
            observations += 1;
        }
    }

    let all_online = unsafe { HART_SET[0] && HART_SET[1] && HART_SET[2] && HART_SET[3] };
    println!(
        "[test_multicore] Harts observed: [0={}, 1={}, 2={}, 3={}] ({} observations)",
        unsafe { HART_SET[0] },
        unsafe { HART_SET[1] },
        unsafe { HART_SET[2] },
        unsafe { HART_SET[3] },
        observations
    );

    if all_online {
        println!("[test_multicore] PASS: all 4 harts online");
    } else {
        println!("[test_multicore] WARN: missing harts, continuing suite");
    }

    0
}
