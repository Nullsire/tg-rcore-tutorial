#![no_std]
#![no_main]

extern crate user;

use user::*;

const SIG_TERM: usize = 1;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[test_signal_basic] minimal signal loop test");

    let child = fork();
    if child == 0 {
        println!("[test_signal_basic] child {} masking SIG_TERM", getpid());
        sig_mask(1usize << SIG_TERM);

        let start = get_time();
        let mut seen_pending = false;
        while get_time() - start < 150 {
            let pending = sig_pending() as usize;
            if (pending & (1usize << SIG_TERM)) != 0 {
                seen_pending = true;
                break;
            }
            yield_();
        }

        if !seen_pending {
            println!("[test_signal_basic] child did not observe pending signal");
            exit(2);
        }

        println!("[test_signal_basic] child unmasking SIG_TERM, expecting default action");
        sig_mask(0);
        yield_();
        println!("[test_signal_basic] ERROR: child survived delivered SIG_TERM");
        exit(3);
    }

    let parent_delay_start = get_time();
    while get_time() - parent_delay_start < 30 {
        yield_();
    }

    let send_ret = sig_send(child as usize, SIG_TERM);
    if send_ret != 0 {
        println!("[test_signal_basic] FAIL: sig_send returned {}", send_ret);
        return 1;
    }

    let mut code = -1;
    waitpid(child, &mut code);

    if code == -(128 + SIG_TERM as i32) {
        println!("[test_signal_basic] PASS: signal delivered with default terminate action");
        0
    } else {
        println!("[test_signal_basic] FAIL: unexpected child exit code {}", code);
        1
    }
}
