#!/usr/bin/env python3
"""Summarize one baseline/candidate pair, retaining all captured metric families."""
import csv
import importlib.util
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
cases = ["baseline", "candidate"]
reports = [json.loads((root / "results" / case / "report.json").read_text()) for case in cases]

def number(value):
    if isinstance(value, float):
        return f"{value:,.6g}"
    return str(value)

def table(title, rows, destination):
    destination.extend(["", "## " + title, "", "| Metric | Baseline | Partial checksums | Change |", "| --- | ---: | ---: | ---: |"])
    for label, values in rows:
        delta = f"{(values[1] / values[0] - 1) * 100:+.2f}%" if isinstance(values[0], (int, float)) and values[0] else "—"
        destination.append(f"| {label} | {number(values[0])} | {number(values[1])} | {delta} |")

def values(fn):
    return [fn(report) for report in reports]

lines = ["# Partial-read checksums: 10-minute replay comparison", "",
         "One baseline/candidate pair on `m8g-32cpu-local-ssd-2`, baseline first, fresh cache each time.",
         "Baseline `033284d264062e438010784cb6a2118568130786`, candidate is that snapshot plus the saved working-tree patch.",
         "Range mode, 100,000,000,000-byte memory target, 500,000,000,000-byte disk, immediate independent exact-range sources.",
         "Matched jemalloc, 32 runtime threads, 64 shards, default scoring/history/eviction settings, first 600 seconds only.",
         "No expanded downloads, source coalescing, simulated S3 delay, or full-object transformation.",
         "See `replay-settings.json` for all settings and `raw/replay/` for source snapshots, commands, hashes, and telemetry."]
rows = []
for label, fn in [
    ("Requests", lambda r: r["requests"]),
    ("Execution seconds", lambda r: r["execution_seconds"]),
    ("Backlog-drain seconds", lambda r: r["backlog_drain_seconds"]),
    ("Public request hit rate (%)", lambda r: 100 * r["request_hit_ratio"]),
    ("Public byte hit rate (%)", lambda r: 100 * r["byte_hit_ratio"]),
    ("Memory hits", lambda r: r["request_outcomes"]["memory_hit"]),
    ("Disk/buffered hits", lambda r: r["request_outcomes"]["disk_hit"]),
    ("Source callbacks", lambda r: r["source_gets"]),
    ("Memory-returned GB", lambda r: r["returned_bytes"]["memory"] / 1e9),
    ("Disk/buffer-returned GB", lambda r: r["returned_bytes"]["disk"] / 1e9),
    ("Source GB", lambda r: r["source_bytes"] / 1e9),
    ("CPU seconds during requests", lambda r: r["cpu_seconds_during_requests"]),
    ("Average CPU cores", lambda r: r["average_cores_during_requests"]),
    ("Sampled peak RSS GB", lambda r: r["sampled_peaks"]["process_resident_memory_bytes"] / 1e9),
    ("OS maximum RSS GB", lambda r: r["os_time"]["max_rss_kib"] * 1024 / 1e9),
    ("Peak cache-accounted GB", lambda r: r["sampled_peaks"]["feuer_memory_used_bytes"] / 1e9),
    ("Peak pending payload GB", lambda r: r["sampled_peaks"]["feuer_disk_write_pending_bytes"] / 1e9),
    ("Peak unstarted requests", lambda r: r["sampled_peaks"]["replay_queued_requests"]),
    ("Peak active lookups", lambda r: r["sampled_peaks"]["replay_active_requests"]),
    ("Source queue mean ms", lambda r: r["source_queue_mean_seconds"] * 1e3),
    ("Source service mean ms", lambda r: r["source_service_mean_seconds"] * 1e3),
]:
    rows.append((label, values(fn)))
table("Requests, CPU, and memory", rows, lines)
rows = []
for operation in ["read", "write"]:
    for field, scale, unit in [("bytes", 1e9, "GB"), ("calls", 1, "calls"), ("errors", 1, "errors")]:
        rows.append((f"DataFile {operation} {unit}", values(lambda r: r["io_including_observation_tail"][operation][field] / scale)))
    rows.append((f"Device {operation} GB", values(lambda r: r["whole_device_io"][operation + "_bytes"] / 1e9)))
    rows.append((f"Device {operation} await ms", values(lambda r: r["device_iostat"][operation[0] + "_await_ms_request_weighted"])))
for label, fn in [
    ("Mean DataFile read size (KiB)", lambda r: r["io_including_observation_tail"]["read"]["bytes"] / r["io_including_observation_tail"]["read"]["calls"] / 1024),
    ("DataFile read bytes / disk-and-buffer-returned byte", lambda r: r["io_including_observation_tail"]["read"]["bytes"] / r["returned_bytes"]["disk"]),
    ("Total DataFile transfer GB", lambda r: r["total_disk_transfer_bytes"] / 1e9),
    ("Device average queue size", lambda r: r["device_iostat"]["aqu-sz"]),
    ("Device average utilization (%)", lambda r: r["device_iostat"]["%util"]),
    ("Memory eviction-triggering insertions", lambda r: r["memory_eviction_triggering_insertions"]),
    ("Disk eviction-triggering insertions", lambda r: r["disk_eviction_triggering_insertions"]),
    ("Disk no-capacity outcomes", lambda r: r["disk_no_capacity"]),
    ("Disk queue-full skips", lambda r: r["disk_write_queue_full"]),
    ("Pending payload bytes after observation tail", lambda r: r["pending_payload_bytes_after_tail"]),
    ("Tail read bytes", lambda r: r["io_observation_tail"]["read"]["bytes"]),
    ("Tail write bytes", lambda r: r["io_observation_tail"]["write"]["bytes"]),
    ("Tail CPU seconds", lambda r: r["cpu_seconds_in_observation_tail"]),
    ("OS user seconds", lambda r: r["os_time"]["user_seconds"]),
    ("OS system seconds", lambda r: r["os_time"]["system_seconds"]),
    ("Process read bytes (/usr/bin/time inputs × 512)", lambda r: r["os_time"]["file_inputs"] * 512),
    ("Process written bytes (/usr/bin/time outputs × 512)", lambda r: r["os_time"]["file_outputs"] * 512),
]:
    rows.append((label, values(fn)))
table("I/O and admission", rows, lines)
rows = []
for group in ["write_admission", "write_terminal"]:
    for outcome in reports[0][group]:
        rows.append((group + ": " + outcome, values(lambda r: r[group][outcome])))
for phase in ["finished", "settled"]:
    for field in reports[0]["memory"][phase]:
        rows.append((phase + " " + field + " (GB)", values(lambda r: r["memory"][phase][field] / 1e9)))
table("Write outcomes and final memory accounting", rows, lines)
rows = []
for metric in ["sender_lateness", "queue_wait", "service", "scheduled_to_complete"]:
    for quantile in ["mean", "p50", "p95", "p99", "max"]:
        rows.append((metric + " " + quantile + " (ms)", values(lambda r: r[metric + "_" + quantile + "_ns"] / 1e6)))
table("Exact request latency distributions", rows, lines)
lines += ["", "## Validation and interpretation", ""]
for case, report in zip(cases, reports):
    lines.append(f"- {case}: validation {report['validation']}.")
lines += ["", "DataFile counters count API calls and requested payload bytes, not kernel submissions or physical-device bytes.",
          "The read/returned-byte ratio includes buffered disk-tier hits in its denominator, so it is not a pure SSD-read amplification ratio.",
          "Phase counter deltas exclude opening/input loading. OS process totals include startup and final reporting.",
          "Device totals include other processes. Inspect the retained host-activity, iostat, and pidstat files.",
          "All requests must finish exactly once. The five-second observation tail is not a forced flush or durability guarantee.",
          "CPU phase counters have one-second resolution. Sampled peaks can miss short spikes.",
          "One pair does not establish a small speedup or regression, especially for scheduling and latency differences.",
          "Full scalar metrics, histograms (with quantile bucket intervals), per-bucket buffer memory, and cold/later windows are in `raw/replay/METRICS.md`."]
(root / "COMPARISON.md").write_text("\n".join(lines) + "\n")

spec = importlib.util.spec_from_file_location("inventory", root / "report-metrics.py")
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)
parsed = []
for case in cases:
    directory = root / "results" / case
    types, before, _, _ = inventory.parse(directory / "before.prom")
    _, finished, _, _ = inventory.parse(directory / "requests-finished.prom")
    _, settled, _, _ = inventory.parse(directory / "settled.prom")
    _, _, _, peaks = inventory.parse(directory / "metrics.prom")
    parsed.append((types, before, finished, settled, peaks))
keys = sorted(set().union(*(set(part[1]) | set(part[2]) | set(part[3]) for part in parsed)))
metric_lines = ["# Complete captured metric comparison", "",
                "Counters are request-phase deltas, gauges are request-finish values. Peaks are one-second samples.",
                "Each series, including all histogram buckets and observation-tail deltas, is retained in `metric-comparison.csv`.", "",
                "| Scalar metric | Baseline | Candidate | Baseline tail / settled | Candidate tail / settled | Baseline peak | Candidate peak |",
                "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
with (root / "metric-comparison.csv").open("w") as out:
    writer = csv.writer(out)
    writer.writerow(["metric", "type", "baseline_requests", "candidate_requests", "baseline_tail", "candidate_tail", "baseline_peak", "candidate_peak"])
    for key in keys:
        kind = next((part[0].get(key[0]) for part in parsed if part[0].get(key[0])), "histogram_component")
        request, tail, peak = [], [], []
        for _, before, finished, settled, peaks in parsed:
            a, b, c = (data.get(key, 0) for data in [before, finished, settled])
            request.append(b if kind == "gauge" else b - a)
            tail.append(c if kind == "gauge" else c - b)
            peak.append(peaks.get(key, 0))
        row = [inventory.label(key), kind, *request, *tail, *peak]
        writer.writerow(row)
        if kind in ["gauge", "counter"]:
            metric_lines.append("| " + " | ".join(["`" + row[0] + "`", *(number(v) for v in row[2:])]) + " |")
items = [json.loads((root / "results" / case / "histograms-and-windows.json").read_text()) for case in cases]
histograms = [{h["metric"]: h for h in item["histograms"]} for item in items]
metric_lines += ["", "## Histogram request-phase count, mean, and quantile bucket intervals", "",
                 "Intervals are histogram bucket intervals, not exact percentiles.", "",
                 "| Histogram | Baseline | Candidate |", "| --- | --- | --- |"]
for name in sorted(histograms[0].keys() | histograms[1].keys()):
    descriptions = []
    for data in histograms:
        h = data[name]
        descriptions.append(f"count={h['count']:g}, mean={h['mean']}, p50={h['p50_bucket_interval']}, p95={h['p95_bucket_interval']}, p99={h['p99_bucket_interval']}")
    metric_lines.append(f"| `{name}` | " + " | ".join(descriptions) + " |")
metric_lines += ["", "## Startup and later request windows", "",
                 "Nearest one-second snapshots, not exact interval counts. Each case's exact overall totals remain in its report.", "",
                 "| Case | Seconds | Memory hits | Disk/buffer hits | Callbacks | Read GB | Write GB | Unstarted | Active |",
                 "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
for case, item in zip(cases, items):
    for window in item["windows"]:
        metric_lines.append("| " + " | ".join(map(str, [case, window['nominal_seconds'], window['memory_hit'], window['disk_hit'], window['callback'], window['read_bytes']/1e9, window['write_bytes']/1e9, window['caller_backlog_at_right'], window['active_lookups_at_right']])) + " |")
    metric_lines += ["", case + " eviction timing:", "```json", json.dumps(item['eviction_timing'], indent=2), "```", ""]
(root / "METRICS.md").write_text("\n".join(metric_lines) + "\n")
