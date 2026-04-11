#![no_std]
#![no_main]

extern crate user;

use user::*;

const SAMPLE_ROUNDS: usize = 4;
const PAYLOAD: &[u8] = b"hello\n";
const WRITES_PER_WORKER: usize = 10_000;

fn run_depth(depth: usize) -> isize {
    let begin = get_time();
    let mut pids = [0isize; 8];

    for i in 0..depth {
        let pid = fork();
        if pid == 0 {
            // Write to a RAMFS file instead of stdout to avoid flooding console
            let fd = open("sink", 1); // O_CREATE = 1
            let out_fd = if fd > 0 { fd as usize } else { 1 }; // fallback to stdout
            for _ in 0..WRITES_PER_WORKER {
                write(out_fd, PAYLOAD);
                yield_();
            }
            if fd > 0 {
                close(fd as usize);
            }
            exit(0);
        } else {
            pids[i] = pid;
        }
    }

    for i in 0..depth {
        let mut code = -1;
        waitpid(pids[i], &mut code);
    }

    get_time() - begin
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_stdio_concurrent] Concurrent write benchmark");
    println!(
        "[perf_stdio_concurrent] payload={} bytes, each worker writes {} times",
        PAYLOAD.len(),
        WRITES_PER_WORKER
    );

    let depths = [1usize, 2, 4, 8];
    println!("| QueueDepth | Avg(ms) | Min(ms) | Max(ms) | Samples |");
    println!("|------------|---------|---------|---------|---------|");

    for depth in depths {
        let mut samples = [0isize; SAMPLE_ROUNDS];
        for i in 0..SAMPLE_ROUNDS {
            samples[i] = run_depth(depth);
        }
        let sum: isize = samples.iter().sum();
        let avg = sum / SAMPLE_ROUNDS as isize;
        let min = *samples.iter().min().unwrap_or(&0);
        let max = *samples.iter().max().unwrap_or(&0);

        println!(
            "| {:>10} | {:>7} | {:>7} | {:>7} | {:?} |",
            depth, avg, min, max, samples
        );
    }

    0
}
