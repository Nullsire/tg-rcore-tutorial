use crate::event::{Event, EventType};
use crate::kernel::Kernel;
use crate::mutex::PiMutex;
use crate::task::{Segment, Task};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SingleRunResult {
    pub pi_enabled: bool,
    /// H task blocking duration (ticks from block to unblock).
    pub h_inversion_duration: Option<u32>,
    /// H task response time (release → completion) for first job.
    pub h_response_time: Option<u32>,
    /// Whether H missed its deadline in the first job.
    pub h_deadline_missed: bool,
    /// Total deadline misses across all tasks.
    pub total_deadline_misses: u32,
    /// Per-task deadline misses: (task_name, misses).
    pub per_task_misses: Vec<(String, u32)>,
    /// Per-task average response time.
    pub per_task_avg_response: Vec<(String, f64)>,
    /// Full event log.
    pub events: Vec<Event>,
    /// Per-tick runner: task_names[task_id] for each tick.
    pub tick_runner: Vec<Option<usize>>,
}

#[derive(Debug, Clone)]
pub struct BatchResult {
    pub pi_enabled: bool,
    pub n_runs: usize,
    pub h_inversion_durations: Vec<Option<u32>>,
    pub h_response_times: Vec<Option<u32>>,
    pub h_deadline_miss_count: u32,
    pub total_deadline_misses: Vec<u32>,
    pub first_run_events: Vec<Event>,
    pub first_run_tick_runner: Vec<Option<usize>>,
}

impl BatchResult {
    pub fn h_inversion_stats(&self) -> Option<Stats> {
        let vals: Vec<u32> = self.h_inversion_durations.iter().filter_map(|&x| x).collect();
        Stats::from_slice(&vals)
    }

    pub fn h_response_stats(&self) -> Option<Stats> {
        let vals: Vec<u32> = self.h_response_times.iter().filter_map(|&x| x).collect();
        Stats::from_slice(&vals)
    }

    pub fn total_misses_stats(&self) -> Option<Stats> {
        Stats::from_slice(&self.total_deadline_misses)
    }
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub mean: f64,
    pub variance: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
}

impl Stats {
    pub fn from_slice(vals: &[u32]) -> Option<Stats> {
        if vals.is_empty() {
            return None;
        }
        let n = vals.len() as f64;
        let mean = vals.iter().sum::<u32>() as f64 / n;
        let variance = if vals.len() > 1 {
            vals.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n
        } else {
            0.0
        };
        Some(Stats {
            mean,
            variance,
            std_dev: variance.sqrt(),
            min: vals.iter().map(|&x| x as f64).fold(f64::INFINITY, f64::min),
            max: vals.iter().map(|&x| x as f64).fold(f64::NEG_INFINITY, f64::max),
        })
    }
}

// ---------------------------------------------------------------------------
// Scenario configuration
// ---------------------------------------------------------------------------

/// Task L (Low):     period=200, deadline=200, wcet=40, offset=0
/// Task H (High):    period=60,  deadline=60,  wcet=20, offset=10
/// Task M (Medium):  period=100, deadline=100, wcet=30, offset=15
fn build_tasks() -> Vec<Task> {
    let task_l = Task::new(
        0, "L", 3, 200, 200,
        vec![
            Segment::Compute(5),
            Segment::CriticalSection { duration: 20, mutex_id: 0 },
            Segment::Compute(15),
        ],
        0,
    );

    let task_h = Task::new(
        1, "H", 1, 60, 60,
        vec![
            Segment::Compute(5),
            Segment::CriticalSection { duration: 5, mutex_id: 0 },
            Segment::Compute(10),
        ],
        10,
    );

    let task_m = Task::new(
        2, "M", 2, 100, 100,
        vec![Segment::Compute(30)],
        15,
    );

    vec![task_l, task_h, task_m]
}

fn build_tasks_with_jitter(jitter_pct: f64, seed: u64) -> Vec<Task> {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    let base = build_tasks();
    base.into_iter()
        .map(|t| t.with_jitter(jitter_pct, &mut || rng.r#gen::<f64>()))
        .collect()
}

// ---------------------------------------------------------------------------
// Single run
// ---------------------------------------------------------------------------

pub fn run_single(pi_enabled: bool, sim_duration: u32) -> SingleRunResult {
    run_single_with_jitter(pi_enabled, sim_duration, 0.0, 0)
}

pub fn run_single_with_jitter(
    pi_enabled: bool,
    sim_duration: u32,
    jitter_pct: f64,
    seed: u64,
) -> SingleRunResult {
    let tasks = if jitter_pct > 0.0 {
        build_tasks_with_jitter(jitter_pct, seed)
    } else {
        build_tasks()
    };

    let mutex = PiMutex::new(0, pi_enabled);
    let mut kernel = Kernel::new(tasks, vec![mutex], sim_duration);
    kernel.run();

    let events: Vec<Event> = kernel.events.clone();
    let tick_runner: Vec<Option<usize>> = kernel.tick_runner.clone();
    let tasks = &kernel.tasks;

    // Measure H's inversion duration: time from Blocked to Unblocked
    let mut h_block_time: Option<u32> = None;
    let mut h_unblock_time: Option<u32> = None;
    for ev in &events {
        if ev.task_id == 1 {
            match &ev.event_type {
                EventType::Blocked(_) => h_block_time = Some(ev.time),
                EventType::Unblocked(_) => h_unblock_time = Some(ev.time),
                _ => {}
            }
        }
    }
    let h_inversion_duration = match (h_block_time, h_unblock_time) {
        (Some(b), Some(u)) => Some(u - b),
        _ => None,
    };

    let h_task = &tasks[1];
    let h_response_time = h_task.response_times.first().copied();
    let h_deadline_missed = h_task.deadline_misses > 0;

    let total_deadline_misses: u32 = tasks.iter().map(|t| t.deadline_misses).sum();

    let per_task_misses: Vec<(String, u32)> = tasks
        .iter()
        .map(|t| (t.name.clone(), t.deadline_misses))
        .collect();

    let per_task_avg_response: Vec<(String, f64)> = tasks
        .iter()
        .map(|t| {
            let avg = if t.response_times.is_empty() {
                0.0
            } else {
                t.response_times.iter().sum::<u32>() as f64 / t.response_times.len() as f64
            };
            (t.name.clone(), avg)
        })
        .collect();

    SingleRunResult {
        pi_enabled,
        h_inversion_duration,
        h_response_time,
        h_deadline_missed,
        total_deadline_misses,
        per_task_misses,
        per_task_avg_response,
        events,
        tick_runner,
    }
}

// ---------------------------------------------------------------------------
// Batch runs
// ---------------------------------------------------------------------------

pub fn run_batch(n_runs: usize, pi_enabled: bool, sim_duration: u32) -> BatchResult {
    run_batch_with_jitter(n_runs, pi_enabled, sim_duration, 0.0)
}

pub fn run_batch_with_jitter(
    n_runs: usize,
    pi_enabled: bool,
    sim_duration: u32,
    jitter_pct: f64,
) -> BatchResult {
    let mut h_inversion_durations = Vec::with_capacity(n_runs);
    let mut h_response_times = Vec::with_capacity(n_runs);
    let mut total_deadline_misses = Vec::with_capacity(n_runs);
    let mut first_run_events = Vec::new();
    let mut first_run_tick_runner = Vec::new();
    let mut h_deadline_miss_count = 0u32;

    for i in 0..n_runs {
        let seed = if jitter_pct > 0.0 { (i + 1) as u64 * 7919 } else { 0 };
        let result = run_single_with_jitter(pi_enabled, sim_duration, jitter_pct, seed);

        if i == 0 {
            first_run_events = result.events;
            first_run_tick_runner = result.tick_runner;
        }

        h_inversion_durations.push(result.h_inversion_duration);
        h_response_times.push(result.h_response_time);
        total_deadline_misses.push(result.total_deadline_misses);
        if result.h_deadline_missed {
            h_deadline_miss_count += 1;
        }
    }

    BatchResult {
        pi_enabled,
        n_runs,
        h_inversion_durations,
        h_response_times,
        h_deadline_miss_count,
        total_deadline_misses,
        first_run_events,
        first_run_tick_runner,
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

const TASK_NAMES: [&str; 3] = ["L", "H", "M"];

pub fn print_timeline(tick_runner: &[Option<usize>], max_ticks: usize) {
    println!("\n  Execution Timeline:");
    println!("  {}", "-".repeat(72));

    // Group consecutive ticks by task
    let limit = max_ticks.min(tick_runner.len());
    let mut groups: Vec<(String, u32, u32)> = Vec::new();
    for tick in 0..limit {
        let name = match tick_runner[tick] {
            Some(id) => TASK_NAMES.get(id).unwrap_or(&"?").to_string(),
            None => "-".to_string(),
        };
        if let Some(last) = groups.last_mut() {
            if last.0 == name && last.2 + 1 == tick as u32 {
                last.2 = tick as u32;
            } else {
                groups.push((name, tick as u32, tick as u32));
            }
        } else {
            groups.push((name, tick as u32, tick as u32));
        }
    }

    for (task, start, end) in &groups {
        let bar_len = (*end - *start + 1) as usize;
        let bar: String = match task.as_str() {
            "H" => "H".repeat(bar_len),
            "M" => "M".repeat(bar_len),
            "L" => "L".repeat(bar_len),
            _ => ".".repeat(bar_len),
        };
        println!("  t={:>3}-{:<3} {} ({})", start, end, bar, task);
    }
    println!("  {}", "-".repeat(72));
}

pub fn print_events(events: &[Event], max_events: usize) {
    println!("\n  Key Events:");
    println!("  {:>5}  {:>4}  {:>14}  {}", "Time", "Task", "Event", "Detail");
    println!("  {}", "-".repeat(60));

    let mut count = 0;
    for ev in events {
        // Skip SCHEDULED events (too many, one per tick)
        if matches!(ev.event_type, EventType::Scheduled) {
            continue;
        }
        if count >= max_events {
            println!("  ... ({} more events)", events.len() - max_events);
            break;
        }
        let name = TASK_NAMES.get(ev.task_id).unwrap_or(&"?");
        let (ev_name, detail) = format_event(&ev.event_type);
        println!("  {:>5}  {:>4}  {:>14}  {}", ev.time, name, ev_name, detail);
        count += 1;
    }
}

fn format_event(et: &EventType) -> (&'static str, String) {
    match et {
        EventType::Released => ("RELEASED", String::new()),
        EventType::Scheduled => ("SCHEDULED", String::new()),
        EventType::Preempted => ("PREEMPTED", String::new()),
        EventType::Blocked(mid) => ("BLOCKED", format!("mutex={}", mid)),
        EventType::Unblocked(mid) => ("UNBLOCKED", format!("mutex={}", mid)),
        EventType::MutexAcquired(mid) => ("LOCK_ACQ", format!("mutex={}", mid)),
        EventType::MutexReleased(mid) => ("LOCK_REL", format!("mutex={}", mid)),
        EventType::JobCompleted { response_time } => {
            ("JOB_DONE", format!("resp_time={}", response_time))
        }
        EventType::DeadlineMissed { deadline, completed_at } => (
            "DEADLINE_MISS",
            format!("deadline={}, completed={}", deadline, completed_at),
        ),
        EventType::PriorityInherited { new_effective_deadline } => (
            "PI_BOOST",
            format!("eff_deadline={}", new_effective_deadline),
        ),
        EventType::PriorityRestored { original_deadline } => (
            "PI_RESTORE",
            format!("deadline={}", original_deadline),
        ),
    }
}

pub fn print_batch_summary(label: &str, result: &BatchResult) {
    println!("\n{}", "=".repeat(70));
    println!("  {} (PI {})", label, if result.pi_enabled { "ON" } else { "OFF" });
    println!("  Runs: {}", result.n_runs);
    println!("{}", "=".repeat(70));

    if let Some(stats) = result.h_inversion_stats() {
        println!("  H Inversion Duration: mean={:.2}  std_dev={:.4}  min={:.0}  max={:.0}",
            stats.mean, stats.std_dev, stats.min, stats.max);
    } else {
        println!("  H Inversion Duration: N/A (no inversion detected)");
    }

    if let Some(stats) = result.h_response_stats() {
        println!("  H Response Time:      mean={:.2}  std_dev={:.4}  min={:.0}  max={:.0}",
            stats.mean, stats.std_dev, stats.min, stats.max);
    }

    if let Some(stats) = result.total_misses_stats() {
        println!("  Total Deadline Misses: mean={:.2}  std_dev={:.4}  min={:.0}  max={:.0}",
            stats.mean, stats.std_dev, stats.min, stats.max);
    }

    println!("  H Deadline Miss Rate:  {}/{} runs ({:.1}%)",
        result.h_deadline_miss_count, result.n_runs,
        result.h_deadline_miss_count as f64 / result.n_runs as f64 * 100.0);
}

pub fn print_comparison(no_pi: &BatchResult, with_pi: &BatchResult) {
    println!("\n{}", "=".repeat(70));
    println!("  COMPARISON: PI OFF vs PI ON");
    println!("{}", "=".repeat(70));
    println!("  {:>25}  {:>12}  {:>12}", "Metric", "PI OFF", "PI ON");
    println!("  {}", "-".repeat(52));

    let fmt = |s: Option<Stats>| -> String {
        match s {
            Some(st) => format!("{:.1} +/- {:.2}", st.mean, st.std_dev),
            None => "N/A".into(),
        }
    };

    println!("  {:>25}  {:>12}  {:>12}",
        "H Inversion Duration",
        fmt(no_pi.h_inversion_stats()),
        fmt(with_pi.h_inversion_stats()));

    println!("  {:>25}  {:>12}  {:>12}",
        "H Response Time",
        fmt(no_pi.h_response_stats()),
        fmt(with_pi.h_response_stats()));

    println!("  {:>25}  {:>12}  {:>12}",
        "Total Deadline Misses",
        fmt(no_pi.total_misses_stats()),
        fmt(with_pi.total_misses_stats()));

    println!("  {:>25}  {:>12}  {:>12}",
        "H Deadline Miss Rate",
        format!("{:.0}%", no_pi.h_deadline_miss_count as f64 / no_pi.n_runs as f64 * 100.0),
        format!("{:.0}%", with_pi.h_deadline_miss_count as f64 / with_pi.n_runs as f64 * 100.0));

    println!();
}
