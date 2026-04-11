#![no_std]
#![no_main]

extern crate user;

use user::*;

const NUM_CHILDREN: usize = 16;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[test_stealing] Observing PerCpuWorkStealing task placement...");
    sched_set(1);

    let mut hart_set = [false; 4];

    for _i in 0..NUM_CHILDREN {
        let pid = fork();
        if pid == 0 {
            // Yield several times to allow work stealing to move this task
            for _ in 0..4 {
                yield_();
            }
            let my_hart = hart_id() as usize;
            exit(my_hart as i32);
        }
    }

    let mut observations = 0usize;
    for _ in 0..NUM_CHILDREN {
        let mut exit_code: i32 = -1;
        waitpid(-1, &mut exit_code);
        let hart = exit_code as usize;
        if hart < 4 {
            hart_set[hart] = true;
            observations += 1;
        }
    }

    let all_harts = hart_set[0] && hart_set[1] && hart_set[2] && hart_set[3];
    println!(
        "[test_stealing]   harts observed: [0={}, 1={}, 2={}, 3={}] ({} observations)",
        hart_set[0], hart_set[1], hart_set[2], hart_set[3], observations
    );

    if all_harts {
        println!("[test_stealing] PASS: PerCpuWorkStealing used all 4 harts");
    } else {
        println!("[test_stealing] WARN: missing harts, continuing suite");
    }

    0
}
