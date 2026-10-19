#!/usr/bin/env python3
"""Inventory captured metrics and summarize one replay, without comparing revisions."""
import csv
from collections import defaultdict
import json
import math
from pathlib import Path
import re
import sys

PATTERN = re.compile(r'([\w:]+)(\{.*\})?\s+(\S+)(?:\s+(\d+))?')


def parse(path):
    types = {}
    values = {}
    frames = defaultdict(dict)
    for line in path.read_text().splitlines():
        if line.startswith("# TYPE "):
            _, _, name, kind = line.split()
            types[name] = kind
        elif line and not line.startswith("#"):
            name, labels, value, stamp = PATTERN.fullmatch(line).groups()
            tags = tuple(sorted(re.findall(r'(\w+)="([^"]*)"', labels or "")))
            key = name, tags
            values[key] = float(value)
            if stamp:
                frames[int(stamp)][key] = float(value)
    return types, values, frames


def label(key):
    name, tags = key
    return name + ("{" + ",".join(f'{k}="{v}"' for k, v in tags) + "}" if tags else "")


def quantile_bounds(buckets, count, fraction):
    if not count:
        return None
    lower = 0
    for upper, cumulative in sorted(buckets):
        if cumulative >= fraction * count:
            return [lower, upper if math.isfinite(upper) else None]
        lower = upper
    raise AssertionError("histogram quantile missing")


def main(directory):
    types, before, _ = parse(directory / "before.prom")
    _, finished, _ = parse(directory / "requests-finished.prom")
    _, settled, _ = parse(directory / "settled.prom")
    _, _, frames = parse(directory / "metrics.prom")
    keys = sorted(before.keys() | finished.keys() | settled.keys())
    with (directory / "metric-inventory.csv").open("w") as output:
        writer = csv.writer(output)
        writer.writerow(["metric", "before", "requests_finished", "settled", "requests_delta", "tail_delta", "sampled_max"])
        for key in keys:
            a, b, c = [data.get(key, 0) for data in (before, finished, settled)]
            writer.writerow([label(key), a, b, c, b - a, c - b,
                             max((frame.get(key, 0) for frame in frames.values()), default=0)])
    lines = ["# Captured metric inventory", "",
             "One run, not a revision comparison. Phase counter deltas exclude opening and input loading.",
             "Gauge changes are not operation counts. Sampled peaks can miss short spikes. CPU counters have one-second resolution.",
             "Every captured sample, including histogram buckets, is retained in `metric-inventory.csv` and `metrics.prom`.", "",
             "## Scalar metrics", "",
             "| Metric | Type | During requests / gauge at request finish | Observation tail / gauge settled | Sampled maximum |",
             "| --- | --- | ---: | ---: | ---: |"]
    for key in keys:
        kind = types.get(key[0])
        if kind not in ("gauge", "counter"):
            continue
        a, b, c = [data.get(key, 0) for data in (before, finished, settled)]
        peak = max((frame.get(key, 0) for frame in frames.values()), default=0)
        first, last = (b - a, c - b) if kind == "counter" else (b, c)
        lines.append(f"| `{label(key)}` | {kind} | {first:g} | {last:g} | {peak:g} |")
    lines += ["", "## Histograms during requests", "",
              "Quantiles are bucket intervals, not exact percentiles. Means use histogram sum/count deltas.", "",
              "| Histogram | Count | Mean | p50 interval | p95 interval | p99 interval |",
              "| --- | ---: | ---: | --- | --- | --- |"]
    histograms = []
    for name, kind in sorted(types.items()):
        if kind != "histogram":
            continue
        for key in keys:
            if key[0] != name + "_count":
                continue
            tags = key[1]
            count = finished.get(key, 0) - before.get(key, 0)
            sum_key = name + "_sum", tags
            total = finished.get(sum_key, 0) - before.get(sum_key, 0)
            buckets = []
            for bucket in keys:
                if bucket[0] == name + "_bucket" and tuple((k, v) for k, v in bucket[1] if k != "le") == tags:
                    upper = float(dict(bucket[1])["le"])
                    buckets.append((upper, finished.get(bucket, 0) - before.get(bucket, 0)))
            bounds = [quantile_bounds(buckets, count, q) for q in (0.5, 0.95, 0.99)]
            mean = total / count if count else None
            item = {"metric": label((name, tags)), "count": count, "mean": mean,
                    "p50_bucket_interval": bounds[0], "p95_bucket_interval": bounds[1], "p99_bucket_interval": bounds[2]}
            histograms.append(item)
            rendered = ["n/a" if bound is None else f"({bound[0]:g}, {'inf' if bound[1] is None else format(bound[1], 'g')}]" for bound in bounds]
            lines.append(f"| `{item['metric']}` | {count:g} | {'n/a' if mean is None else format(mean, 'g')} | {' | '.join(rendered)} |")
    sorted_frames = sorted(frames.values(), key=lambda frame: frame.get(("replay_elapsed_seconds", ()), 0))
    windows = []
    def counter(frame, name, **tags):
        return frame.get((name, tuple(sorted(tags.items()))), 0)
    end = json.loads((directory / "report.json").read_text())["execution_seconds"]
    boundaries = [value for value in (0, 30, 90, 180, 900, 1800, 2700) if value < end] + [end]
    for lo, hi in zip(boundaries, boundaries[1:]):
        left = before if lo == 0 else min(sorted_frames, key=lambda frame: abs(counter(frame, "replay_elapsed_seconds") - lo))
        right = finished if hi == end else min(sorted_frames, key=lambda frame: abs(counter(frame, "replay_elapsed_seconds") - hi))
        item = {"nominal_seconds": [lo, hi],
                "actual_right_elapsed": end if hi == end else counter(right, "replay_elapsed_seconds"),
                "caller_backlog_at_right": counter(right, "replay_queued_requests"),
                "active_lookups_at_right": counter(right, "replay_active_requests")}
        for outcome in ("memory_hit", "disk_hit", "callback"):
            item[outcome] = counter(right, "feuer_lookup_total", outcome=outcome) - counter(left, "feuer_lookup_total", outcome=outcome)
        for operation in ("read", "write"):
            item[operation + "_bytes"] = counter(right, "feuer_disk_io_bytes_total", operation=operation) - counter(left, "feuer_disk_io_bytes_total", operation=operation)
        windows.append(item)
    lines += ["", "## Startup versus later samples", "",
              "Windows use nearest one-second samples and are approximate, unlike the final exactly-once totals.", "",
              "| Nominal seconds | Memory hits | Disk/buffered hits | Callbacks | Read GiB | Write GiB | Unstarted at end | Active at end |",
              "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for item in windows:
        lo, hi = item["nominal_seconds"]
        lines.append(f"| {lo}–{hi} | {item['memory_hit']:g} | {item['disk_hit']:g} | {item['callback']:g} | {item['read_bytes']/2**30:.3f} | {item['write_bytes']/2**30:.3f} | {item['caller_backlog_at_right']:g} | {item['active_lookups_at_right']:g} |")
    (directory / "METRICS.md").write_text("\n".join(lines) + "\n")
    (directory / "histograms-and-windows.json").write_text(json.dumps({"histograms": histograms, "windows": windows}, indent=2) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
