#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import re
import select
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent
USER_DIR = ROOT / "user"
KERNEL_DIR = ROOT / "kernel"
KERNEL_ELF = ROOT / "target" / "riscv64gc-unknown-none-elf" / "release" / "kernel"
ARTIFACTS_DIR = ROOT / "artifacts"
LOG_PATH = ARTIFACTS_DIR / "test_suite.log"
JSON_PATH = ARTIFACTS_DIR / "performance_results.json"
MD_PATH = ARTIFACTS_DIR / "performance_tables.md"


def run_command(command: list[str], cwd: Path, env: dict[str, str] | None = None) -> None:
    print(f"[run] ({cwd.name}) {' '.join(command)}", flush=True)
    completed = subprocess.run(command, cwd=cwd, env=env)
    if completed.returncode != 0:
        raise RuntimeError(f"command failed with exit code {completed.returncode}: {' '.join(command)}")


def build_kernel(init_app: str) -> None:
    build_env = os.environ.copy()
    build_env["INIT_APP"] = init_app

    run_command(["cargo", "clean"], USER_DIR)
    run_command(["cargo", "clean"], KERNEL_DIR)
    run_command(["cargo", "build", "--release"], USER_DIR)
    run_command(["cargo", "build", "--release"], KERNEL_DIR, env=build_env)


def run_qemu(init_app: str, timeout_sec: int, idle_grace_sec: float) -> str:
    if not KERNEL_ELF.exists():
        raise FileNotFoundError(f"kernel image not found: {KERNEL_ELF}")

    qemu_command = [
        "qemu-system-riscv64",
        "-machine",
        "virt",
        "-nographic",
        "-smp",
        "4",
        "-m",
        "128M",
        "-bios",
        "default",
        "-kernel",
        str(KERNEL_ELF),
    ]

    print(f"[run] booting INIT_APP={init_app}")
    process = subprocess.Popen(
        qemu_command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )

    assert process.stdout is not None
    log_lines: list[str] = []
    start = time.monotonic()
    summary_seen = False
    last_output = time.monotonic()

    try:
        while True:
            now = time.monotonic()
            if now - start > timeout_sec:
                raise TimeoutError(f"QEMU run exceeded {timeout_sec} seconds")

            ready, _, _ = select.select([process.stdout], [], [], 0.25)
            if ready:
                line = process.stdout.readline()
                if line:
                    sys.stdout.write(line)
                    sys.stdout.flush()
                    log_lines.append(line)
                    last_output = time.monotonic()
                    if "COMPREHENSIVE TEST RESULTS" in line or "Total Tests:" in line:
                        summary_seen = True
                elif process.poll() is not None:
                    break
            else:
                if summary_seen and (now - last_output) >= idle_grace_sec:
                    break

        return "".join(log_lines)
    finally:
        if process.poll() is None:
            try:
                process.terminate()
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def extract_table_rows(text: str, label: str, row_regex: str) -> list[list[str]]:
    start = text.find(label)
    if start == -1:
        return []
    section = text[start:]
    matches = []
    for match in re.finditer(row_regex, section, re.MULTILINE):
        matches.append([group.strip() for group in match.groups()])
    return matches


def parse_results(text: str) -> dict[str, object]:
    suite_results = []
    for test_name, status, code in re.findall(r"^\[SUITE\]\s+([^\s]+)\s+(PASSED|FAILED)\s*\(exit code:\s*([^\)]+)\)", text, re.MULTILINE):
        suite_results.append({"test": test_name, "status": status, "exit_code": code.strip()})

    summary = {
        "passed": None,
        "failed": None,
        "total": None,
    }
    passed_match = re.search(r"Total Passed:\s*(\d+)", text)
    failed_match = re.search(r"Total Failed:\s*(\d+)", text)
    total_match = re.search(r"Total Tests:\s*(\d+)", text)
    if passed_match:
        summary["passed"] = int(passed_match.group(1))
    if failed_match:
        summary["failed"] = int(failed_match.group(1))
    if total_match:
        summary["total"] = int(total_match.group(1))

    throughput_rows = extract_table_rows(
        text,
        "[perf_throughput]",
        r"^\|\s*(GlobalRoundRobin|PerCpuWorkStealing)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|",
    )

    contention_rows = extract_table_rows(
        text,
        "[perf_contention]",
        r"^\|\s*(GlobalRoundRobin|PerCpuWorkStealing)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|",
    )

    contention_stddev = None
    stddev_match = re.search(r"Hart balance stddev \(x100\): RR=([0-9]+), WS=([0-9]+)", text)
    if stddev_match:
        contention_stddev = {
            "rr": stddev_match.group(1),
            "ws": stddev_match.group(2),
        }

    mixed_rows = extract_table_rows(
        text,
        "[perf_mixed]",
        r"^\|\s*(GlobalRoundRobin|PerCpuWorkStealing)\s*\|\s*([0-9]+)\s*\|",
    )

    kernel_irq_rows = extract_table_rows(
        text,
        "[perf_kernel_irq]",
        r"^\|\s*(IRQ ON|IRQ OFF)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|",
    )

    single_multi_rows = extract_table_rows(
        text,
        "[perf_single_multi]",
        r"^\|\s*([14])\s*\|\s*([0-9]+)\s*\|",
    )

    thread_multi_rows = extract_table_rows(
        text,
        "[perf_single_multi_thread]",
        r"^\|\s*(1 hart \(thread\)|4 harts \(thread\))\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*(\[.*\])\s*\|",
    )

    stdio_rows = extract_table_rows(
        text,
        "[perf_stdio_concurrent]",
        r"^\|\s*([1248])\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*([0-9]+)\s*\|\s*(\[.*\])\s*\|",
    )

    fs_rows = extract_table_rows(
        text,
        "[perf_fs_workload]",
        r"^\|\s*(Sequential RW|Random RW|Mixed Metadata)\s*\|\s*([0-9-]+)\s*\|\s*([0-9-]+)\s*\|\s*([0-9-]+)\s*\|\s*(\[.*\])\s*\|",
    )

    preemption_pass = len(re.findall(r"\[test_preemption\].*PASS", text))

    return {
        "suite": summary,
        "suite_results": suite_results,
        "benchmarks": {
            "throughput": throughput_rows,
            "contention": contention_rows,
            "contention_stddev": contention_stddev,
            "mixed": mixed_rows,
            "kernel_irq": kernel_irq_rows,
            "single_vs_multi": single_multi_rows,
            "single_vs_multi_thread": thread_multi_rows,
            "stdio_concurrent": stdio_rows,
            "fs_workload": fs_rows,
            "preemption_pass_count": preemption_pass,
        },
    }


def write_outputs(text: str, data: dict[str, object]) -> None:
    ARTIFACTS_DIR.mkdir(parents=True, exist_ok=True)
    LOG_PATH.write_text(text, encoding="utf-8")
    JSON_PATH.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")

    lines = []
    suite = data["suite"]  # type: ignore[assignment]
    lines.append("# Experiment Summary")
    lines.append("")
    lines.append(f"- Passed: {suite['passed']}")
    lines.append(f"- Failed: {suite['failed']}")
    lines.append(f"- Total: {suite['total']}")
    lines.append("")
    lines.append("## Benchmark Tables")
    lines.append("")

    def add_table(title: str, headers: list[str], rows: list[list[str]]) -> None:
        lines.append(f"### {title}")
        lines.append("")
        lines.append("| " + " | ".join(headers) + " |")
        lines.append("|" + "|".join(["---"] * len(headers)) + "|")
        for row in rows:
            lines.append("| " + " | ".join(row) + " |")
        lines.append("")

    benchmarks = data["benchmarks"]  # type: ignore[assignment]
    add_table("Throughput", ["Scheduler", "Avg yield (us)", "Wall time (ms)", "Ticks"], benchmarks["throughput"])

    contention_rows_with_stddev = []
    for scheduler, avg_ms, wall_ms in benchmarks["contention"]:
        if scheduler == "GlobalRoundRobin":
            stddev = benchmarks["contention_stddev"]["rr"] if benchmarks["contention_stddev"] else ""
        else:
            stddev = benchmarks["contention_stddev"]["ws"] if benchmarks["contention_stddev"] else ""
        contention_rows_with_stddev.append([scheduler, avg_ms, wall_ms, stddev])
    add_table("Contention", ["Scheduler", "Avg completion (ms)", "Wall time (ms)", "Hart balance stddev x100"], contention_rows_with_stddev)
    add_table("Mixed", ["Scheduler", "Wall time (ms)"], benchmarks["mixed"])
    add_table("Kernel IRQ", ["Mode", "Wall time (ms)", "Ticks", "KernelTimer"], benchmarks["kernel_irq"])
    add_table("Single vs Multi", ["Active harts", "Wall time (ms)"], benchmarks["single_vs_multi"])
    add_table("Single vs Multi Thread", ["Mode", "Avg(ms)", "Min(ms)", "Max(ms)", "Samples"], benchmarks["single_vs_multi_thread"])
    add_table("Stdio Concurrent", ["QueueDepth", "Avg(ms)", "Min(ms)", "Max(ms)", "Samples"], benchmarks["stdio_concurrent"])
    add_table("FS Workload", ["Workload", "Avg(ms)", "Min(ms)", "Max(ms)", "Samples"], benchmarks["fs_workload"])

    MD_PATH.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Build and run all multicore OS experiments.")
    parser.add_argument("--init-app", default="test_suite", help="kernel INIT_APP to boot")
    parser.add_argument("--timeout", type=int, default=7200, help="overall QEMU timeout in seconds")
    parser.add_argument("--idle-grace", type=float, default=3.0, help="idle grace period after final summary")
    args = parser.parse_args()

    build_kernel(args.init_app)
    log_text = run_qemu(args.init_app, timeout_sec=args.timeout, idle_grace_sec=args.idle_grace)
    data = parse_results(log_text)
    write_outputs(log_text, data)

    suite = data["suite"]  # type: ignore[assignment]
    print("")
    print(f"[done] passed={suite['passed']} failed={suite['failed']} total={suite['total']}")
    print(f"[done] log={LOG_PATH}")
    print(f"[done] json={JSON_PATH}")
    print(f"[done] markdown={MD_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())