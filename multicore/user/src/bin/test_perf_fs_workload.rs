#![no_std]
#![no_main]

extern crate user;

use user::*;

const ROUNDS: usize = 4;
const BLOCK: usize = 256;
const BLOCKS: usize = 128;

fn fill(buf: &mut [u8], seed: usize) {
    for i in 0..buf.len() {
        buf[i] = ((i + seed) & 0xff) as u8;
    }
}

fn lcg(state: &mut u64) -> usize {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    (*state >> 32) as usize
}

fn seq_rw_once() -> isize {
    let path = "/fs_seq.bin";
    let fd = open(path, O_CREATE | O_TRUNC);
    if fd < 0 {
        return -1;
    }
    let fd = fd as usize;

    let mut buf = [0u8; BLOCK];
    for b in 0..BLOCKS {
        fill(&mut buf, b);
        if write(fd, &buf) != BLOCK as isize {
            close(fd);
            return -1;
        }
    }

    let mut sz = 0usize;
    if fsize(fd, &mut sz) != 0 || sz != BLOCK * BLOCKS {
        close(fd);
        return -1;
    }

    if lseek(fd, 0, 0) < 0 {
        close(fd);
        return -1;
    }

    let mut read_buf = [0u8; BLOCK];
    let mut checksum: usize = 0;
    for _ in 0..BLOCKS {
        if read(fd, &mut read_buf) != BLOCK as isize {
            close(fd);
            return -1;
        }
        for v in read_buf {
            checksum = checksum.wrapping_add(v as usize);
        }
    }

    close(fd);
    unlink(path);
    checksum as isize
}

fn random_rw_once() -> isize {
    let path = "/fs_rand.bin";
    let fd = open(path, O_CREATE | O_TRUNC);
    if fd < 0 {
        return -1;
    }
    let fd = fd as usize;

    let zero = [0u8; BLOCK];
    for _ in 0..BLOCKS {
        if write(fd, &zero) != BLOCK as isize {
            close(fd);
            return -1;
        }
    }

    let mut rng = 0x12345678u64;
    let mut one = [0u8; 16];
    let mut tmp = [0u8; 16];
    for i in 0..1500 {
        let slot = lcg(&mut rng) % (BLOCKS * BLOCK / one.len());
        let off = (slot * one.len()) as isize;
        fill(&mut one, i);
        if lseek(fd, off, 0) < 0 {
            close(fd);
            return -1;
        }
        if write(fd, &one) != one.len() as isize {
            close(fd);
            return -1;
        }
        if lseek(fd, off, 0) < 0 {
            close(fd);
            return -1;
        }
        if read(fd, &mut tmp) != tmp.len() as isize {
            close(fd);
            return -1;
        }
    }

    close(fd);
    unlink(path);
    0
}

fn mixed_meta_once() -> isize {
    // Use global kernel ticks (shared atomic counter) to avoid per-hart time skew.
    let t0_ticks = kernel_stats_parsed().ticks;
    let paths = ["/meta_a.tmp", "/meta_b.tmp", "/meta_c.tmp", "/meta_d.tmp"];
    for i in 0..1920 {
        let idx = i & 3;
        let path = paths[idx];
        let payload = [idx as u8; 64];
        let fd = open(path, O_CREATE | O_TRUNC);
        if fd < 0 {
            return -1;
        }
        let fd = fd as usize;
        if write(fd, &payload) != payload.len() as isize {
            close(fd);
            return -1;
        }
        if close(fd) != 0 {
            return -1;
        }
        if unlink(path) != 0 {
            return -1;
        }
        if (i & 0x1f) == 0 {
            yield_();
        }
    }

    let t1_ticks = kernel_stats_parsed().ticks;
    let elapsed_ticks = t1_ticks.saturating_sub(t0_ticks);
    (elapsed_ticks * 10) as isize
}

fn stat(samples: &[isize; ROUNDS]) -> (isize, isize, isize) {
    let mut min = samples[0];
    let mut max = samples[0];
    let mut sum = 0isize;
    for &v in samples {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        sum += v;
    }
    (sum / ROUNDS as isize, min, max)
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[perf_fs_workload] Real FS workload: sequential/random/mixed-meta");

    let mut seq_samples = [0isize; ROUNDS];
    let mut rand_samples = [0isize; ROUNDS];
    let mut meta_samples = [0isize; ROUNDS];

    for i in 0..ROUNDS {
        let t0 = get_time();
        let seq_ok = seq_rw_once();
        let t1 = get_time();
        seq_samples[i] = if seq_ok >= 0 { t1 - t0 } else { -1 };
    }

    for i in 0..ROUNDS {
        let t0 = get_time();
        let r = random_rw_once();
        let t1 = get_time();
        rand_samples[i] = if r == 0 { t1 - t0 } else { -1 };
    }

    for i in 0..ROUNDS {
        meta_samples[i] = mixed_meta_once();
    }

    let (seq_avg, seq_min, seq_max) = stat(&seq_samples);
    let (rand_avg, rand_min, rand_max) = stat(&rand_samples);
    let (meta_avg, meta_min, meta_max) = stat(&meta_samples);

    println!("| Workload | Avg(ms) | Min(ms) | Max(ms) | Samples |");
    println!("|----------|---------|---------|---------|---------|");
    println!("| Sequential RW | {:>7} | {:>7} | {:>7} | {:?} |", seq_avg, seq_min, seq_max, seq_samples);
    println!("| Random RW | {:>7} | {:>7} | {:>7} | {:?} |", rand_avg, rand_min, rand_max, rand_samples);
    println!("| Mixed Metadata | {:>7} | {:>7} | {:>7} | {:?} |", meta_avg, meta_min, meta_max, meta_samples);

    if seq_min < 0 || rand_min < 0 || meta_min < 0 {
        println!("[perf_fs_workload] FAIL: at least one workload iteration failed");
        return 1;
    }

    println!("[perf_fs_workload] PASS: all workloads completed with multi-sample stats");
    0
}
