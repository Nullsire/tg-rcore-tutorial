#![no_std]
#![no_main]

use user::{println, thread_create, waittid, hart_id, gettid};

fn thread_fn(arg: usize) -> i32 {
    println!(
        "[thread_test] thread {} on hart {}, arg={}",
        gettid(), hart_id(), arg
    );
    0
}

#[no_mangle]
fn main() -> i32 {
    println!("[thread_test] main thread on hart {}", hart_id());
    let mut tids = [0usize; 4];
    for i in 0..4 {
        let tid = thread_create(thread_fn, i) as usize;
        println!("[thread_test] created thread tid={}", tid);
        tids[i] = tid;
    }
    for tid in tids {
        waittid(tid);
        println!("[thread_test] thread {} joined", tid);
    }
    println!("[thread_test] all threads completed");
    0
}
