# Experiment Summary

- Passed: 15
- Failed: 0
- Total: 15

## Benchmark Tables

### Throughput

| Scheduler | Avg yield (us) | Wall time (ms) | Ticks |
|---|---|---|---|
| GlobalRoundRobin | 0 | 323 | 127 |
| PerCpuWorkStealing | 0 | 328 | 128 |

### Contention

| Scheduler | Avg completion (ms) | Wall time (ms) | Hart balance stddev x100 |
|---|---|---|---|
| GlobalRoundRobin | 0 | 15 | 10 |
| PerCpuWorkStealing | 0 | 11 | 0 |

### Mixed

| Scheduler | Wall time (ms) |
|---|---|
| GlobalRoundRobin | 106 |
| PerCpuWorkStealing | 108 |

### Kernel IRQ

| Mode | Wall time (ms) | Ticks | KernelTimer |
|---|---|---|---|
| IRQ ON | 29 | 12 | 2 |
| IRQ OFF | 24 | 8 | 2 |

### Single vs Multi

| Active harts | Wall time (ms) |
|---|---|
| 1 | 14 |
| 4 | 10 |
| 1 | 299 |
| 4 | 375 |

### Single vs Multi Thread

| Mode | Avg(ms) | Min(ms) | Max(ms) | Samples |
|---|---|---|---|---|
| 1 hart (thread) | 11 | 9 | 15 | [12, 15, 12, 11, 9] |
| 4 harts (thread) | 4 | 4 | 6 | [5, 4, 4, 6, 5] |

### Stdio Concurrent

| QueueDepth | Avg(ms) | Min(ms) | Max(ms) | Samples |
|---|---|---|---|---|
| 1 | 299 | 297 | 303 | [297, 299, 298, 303] |
| 2 | 321 | 308 | 339 | [328, 308, 310, 339] |
| 4 | 375 | 367 | 384 | [371, 384, 367, 380] |
| 8 | 676 | 655 | 694 | [694, 655, 678, 680] |

### FS Workload

| Workload | Avg(ms) | Min(ms) | Max(ms) | Samples |
|---|---|---|---|---|
| Sequential RW | 4 | 4 | 5 | [4, 4, 5, 5] |
| Random RW | 93 | 87 | 97 | [94, 97, 94, 87] |
| Mixed Metadata | 502 | 480 | 520 | [520, 520, 480, 490] |
