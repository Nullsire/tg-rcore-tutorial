#![allow(dead_code)]

mod event;
mod experiment;
mod kernel;
mod mutex;
mod scheduler;
mod task;

use experiment::*;

const N_RUNS: usize = 100;
const SIM_DURATION: u32 = 200;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║  Real-Time Scheduling Experiment: Priority Inversion & PI Protocol ║");
    println!("║  Scheduler: EDF (Earliest Deadline First)                          ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");

    print_scenario();

    // --- Part 1: Detailed single run with PI OFF ---
    println!("\n{}", "#".repeat(70));
    println!("  PART 1: Single Run — PI OFF (detailed trace)");
    println!("{}", "#".repeat(70));

    let single_off = run_single(false, SIM_DURATION);
    print_timeline(&single_off.tick_runner, 80);
    print_events(&single_off.events, 50);

    // --- Part 2: Detailed single run with PI ON ---
    println!("\n{}", "#".repeat(70));
    println!("  PART 2: Single Run — PI ON (detailed trace)");
    println!("{}", "#".repeat(70));

    let single_on = run_single(true, SIM_DURATION);
    print_timeline(&single_on.tick_runner, 80);
    print_events(&single_on.events, 50);

    // --- Part 3: Batch runs (deterministic, N=100) ---
    println!("\n{}", "#".repeat(70));
    println!("  PART 3: Reproducibility — {} runs (deterministic)", N_RUNS);
    println!("{}", "#".repeat(70));

    let batch_off = run_batch(N_RUNS, false, SIM_DURATION);
    let batch_on = run_batch(N_RUNS, true, SIM_DURATION);

    print_batch_summary("Results without PI", &batch_off);
    print_batch_summary("Results with PI", &batch_on);
    print_comparison(&batch_off, &batch_on);

    // --- Part 4: Batch runs with jitter ---
    println!("\n{}", "#".repeat(70));
    println!("  PART 4: Robustness — {} runs with ±10% jitter", N_RUNS);
    println!("{}", "#".repeat(70));

    let batch_jitter_off = run_batch_with_jitter(N_RUNS, false, SIM_DURATION, 0.10);
    let batch_jitter_on = run_batch_with_jitter(N_RUNS, true, SIM_DURATION, 0.10);

    print_batch_summary("Jitter results without PI", &batch_jitter_off);
    print_batch_summary("Jitter results with PI", &batch_jitter_on);
    print_comparison(&batch_jitter_off, &batch_jitter_on);

    println!("Experiment complete.");
}

fn print_scenario() {
    println!("\n  Task Configuration:");
    println!("  {}", "-".repeat(60));
    println!("  {:>4}  {:>8}  {:>6}  {:>6}  {:>6}  {}",
        "Task", "Priority", "Period", "DL", "WCET", "Segments");
    println!("  {:>4}  {:>8}  {:>6}  {:>6}  {:>6}  {}",
        "L", "3 (low)", "200", "200", "40",
        "Compute(5) → CS(20,mutex) → Compute(15)");
    println!("  {:>4}  {:>8}  {:>6}  {:>6}  {:>6}  {}",
        "H", "1 (high)", "60", "60", "20",
        "Compute(5) → CS(5,mutex) → Compute(10)");
    println!("  {:>4}  {:>8}  {:>6}  {:>6}  {:>6}  {}",
        "M", "2 (med)", "100", "100", "30",
        "Compute(30)");
    println!("  {}", "-".repeat(60));
    println!("  Shared Mutex: mutex_0 (L and H both access)");
    println!("  First releases: L@t=0, H@t=10, M@t=15");
    println!("  Simulation duration: {} ticks", SIM_DURATION);
}
