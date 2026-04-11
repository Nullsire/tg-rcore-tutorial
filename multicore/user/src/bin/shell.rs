#![no_std]
#![no_main]

use user::{println, print, read, fork, exec, waitpid, exit, getpid, hart_id};

const LF: u8 = 0x0au8;
const CR: u8 = 0x0du8;
const BS: u8 = 0x08u8;
const DL: u8 = 0x7fu8;

#[no_mangle]
fn main() -> i32 {
    println!("MultiCore OS Shell (pid={}, hart={})", getpid(), hart_id());
    println!("Type 'help' for available commands");
    let mut line = [0u8; 256];
    loop {
        print!("$ ");
        let mut pos = 0;
        loop {
            let mut c = [0u8; 1];
            read(0, &mut c);
            match c[0] {
                LF | CR => {
                    println!();
                    break;
                }
                BS | DL => {
                    if pos > 0 {
                        pos -= 1;
                        print!("{}", 8 as char);
                        print!(" ");
                        print!("{}", 8 as char);
                    }
                }
                _ => {
                    if pos < line.len() - 1 {
                        line[pos] = c[0];
                        pos += 1;
                        print!("{}", c[0] as char);
                    }
                }
            }
        }
        line[pos] = 0;
        let cmd = unsafe { core::str::from_utf8_unchecked(&line[..pos]) };
        if cmd.is_empty() {
            continue;
        }
        match cmd {
            "help" => {
                println!("Available commands:");
                println!("  hello        - print hello");
                println!("  fork_test    - test fork/wait");
                println!("  thread_test  - test threads");
                println!("  multicore_test - test multi-core");
                println!("  spin_task    - CPU-bound task");
                println!("  yield_test   - yield test");
                println!("  sched 0/1    - set scheduler (0=global, 1=percpu)");
                println!("  --- Test Suite ---");
                println!("  test_timer_interrupt  - timer interrupt correctness");
                println!("  test_preemption       - preemption via need_reschedule");
                println!("  test_multicore_online - verify all 4 harts active");
                println!("  test_concurrent_fork  - concurrent fork/exec stress");
                println!("  test_hart_balance     - load distribution balance");
                println!("  test_work_stealing    - work stealing verification");
                println!("  --- Performance ---");
                println!("  test_perf_throughput  - scheduler throughput comparison");
                println!("  test_perf_contention  - high contention comparison");
                println!("  test_perf_mixed       - mixed workload comparison");
                println!("  exit         - exit shell");
            }
            "exit" => {
                println!("Bye!");
                return 0;
            }
            _ if cmd.starts_with("sched ") => {
                let n = cmd.as_bytes()[6] - b'0';
                user::sched_set(n as usize);
                println!("Scheduler set to {}", n);
            }
            _ => {
                // Try to exec the command as a program name
                let pid = fork();
                if pid == 0 {
                    if exec(cmd) == -1 {
                        println!("Unknown command: {}", cmd);
                        exit(-1);
                    }
                    unreachable!()
                } else {
                    let mut exit_code: i32 = 0;
                    waitpid(pid, &mut exit_code);
                }
            }
        }
    }
}
