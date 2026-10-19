"""Count request-range reuse and recorded download expansion, without simulating a cache."""
import argparse
import collections
import hashlib
import json
import subprocess
import tarfile
from pathlib import Path

import numpy as np

EVENT = np.dtype({"names": ["kind", "id", "time", "lo", "hi", "start", "end", "download_start", "download_end"],
                  "formats": ["u1"] + ["<u8"] * 8,
                  "offsets": [0, 1, 9, 17, 25, 33, 41, 49, 57], "itemsize": 65})


def packages(path):
    process = subprocess.Popen(["zstd", "-dc", str(path)], stdout=subprocess.PIPE)
    with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
        for member in archive:
            if member.name.endswith(".feuer"):
                data = archive.extractfile(member).read()
                assert data[:8] == b"FETR\x01\0\0\0" and (len(data) - 8) % 65 == 0
                yield np.frombuffer(data, dtype=EVENT, offset=8)
    assert process.wait() == 0


def merge(ranges):
    result = []
    for start, end in sorted(ranges):
        if result and start <= result[-1][1]:
            result[-1] = (result[-1][0], max(end, result[-1][1]))
        else:
            result.append((start, end))
    return result


def overlap(ranges, start, end):
    return sum(max(0, min(end, b) - max(start, a)) for a, b in merge(ranges))


def classify(ranges, start, end):
    if (start, end) in ranges:
        return "exact", end - start
    if any(a <= start and end <= b for a, b in ranges):
        return "single_cover", end - start
    covered = overlap(ranges, start, end)
    if covered == end - start and end > start:
        return "fragments_cover", covered
    return ("partial" if covered else "disjoint"), covered


def count(counter, category, length, covered):
    counter[category + "_requests"] += 1
    counter[category + "_requested_bytes"] += length
    counter[category + "_covered_bytes"] += covered


def summarize(requests, outcomes):
    requests.sort(order=["time", "id"])
    exact = collections.Counter()
    prior = collections.defaultdict(set)
    chronological = collections.Counter()
    sizes = []
    for row in requests:
        key = (int(row["lo"]), int(row["hi"]))
        start, end = int(row["start"]), int(row["end"])
        category, covered = classify(prior[key], start, end)
        count(chronological, category, end - start, covered)
        prior[key].add((start, end))
        exact[(key, start, end)] += 1
        sizes.append(end - start)
    by_object = collections.defaultdict(list)
    for key, start, end in exact:
        by_object[key].append((start, end))
    static = collections.Counter()
    for (key, start, end), frequency in exact.items():
        others = [r for r in by_object[key] if r != (start, end)]
        category, covered = classify(others, start, end)
        static[category + "_distinct_ranges"] += 1
        static[category + "_requests"] += frequency
        static[category + "_requested_bytes"] += (end - start) * frequency
        static[category + "_covered_bytes"] += covered * frequency
    recorded = collections.Counter()
    callback_ranges = collections.defaultdict(set)
    success = []
    for row in requests:
        outcome = outcomes.get(int(row["id"]))
        if outcome is None:
            recorded["unmatched_starts"] += 1
            continue
        kind, timestamp, dstart, dend = outcome
        recorded[f"kind_{kind}_requests"] += 1
        recorded[f"kind_{kind}_requested_bytes"] += int(row["end"]) - int(row["start"])
        if kind in (1, 2, 3):
            key = (int(row["lo"]), int(row["hi"]))
            success.append((timestamp, key, int(row["start"]), int(row["end"])))
        if kind == 3:
            start, end = int(row["start"]), int(row["end"])
            recorded["callback_downloaded_bytes"] += dend - dstart
            shape = "exact" if (start, end) == (dstart, dend) else "expanded"
            recorded[f"callback_{shape}_requests"] += 1
            recorded[f"callback_{shape}_requested_bytes"] += end - start
            recorded[f"callback_{shape}_downloaded_bytes"] += dend - dstart
            callback_ranges[key].add((dstart, dend))
    success.sort()
    completed = collections.defaultdict(set)
    completed_stats = collections.Counter()
    index = 0
    for row in requests:
        while index < len(success) and success[index][0] < int(row["time"]):
            _, key, start, end = success[index]
            completed[key].add((start, end))
            index += 1
        key = (int(row["lo"]), int(row["hi"]))
        start, end = int(row["start"]), int(row["end"])
        category, covered = classify(completed[key], start, end)
        count(completed_stats, category, end - start, covered)
    lengths = np.array(sizes, dtype=np.uint64)
    return {
        "requests": len(requests), "requested_bytes": sum(sizes),
        "objects": len(by_object), "distinct_exact_ranges": len(exact),
        "distinct_exact_range_bytes": sum(end - start for _, start, end in exact),
        "union_bytes": sum(sum(b - a for a, b in merge(ranges)) for ranges in by_object.values()),
        "request_size_bytes": {str(p): float(np.percentile(lengths, p)) for p in (0, 50, 90, 95, 99, 100)},
        "chronological_prior_starts_no_eviction": dict(chronological),
        "prior_completed_successful_requests_no_eviction": dict(completed_stats),
        "all_other_distinct_ranges_ignoring_time_and_residency": dict(static),
        "recorded_outcomes": dict(recorded),
        "distinct_callback_download_union_bytes": sum(sum(b - a for a, b in merge(ranges)) for ranges in callback_ranges.values()),
    }


def self_test():
    assert merge([(0, 3), (2, 5), (5, 6), (8, 9)]) == [(0, 6), (8, 9)]
    assert classify({(0, 4), (4, 8)}, 0, 8) == ("fragments_cover", 8)
    assert classify({(0, 4)}, 2, 6) == ("partial", 2)
    assert classify({(0, 8)}, 2, 6) == ("single_cover", 4)
    assert classify({(0, 8)}, 0, 8) == ("exact", 8)
    assert classify({(0, 4)}, 4, 8) == ("disjoint", 0)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--seconds", type=int, default=600)
    args = parser.parse_args()
    self_test()
    first = None
    for rows in packages(args.input):
        starts = rows[rows["kind"] == 0]
        if len(starts):
            earliest = int(starts["time"].min())
            first = earliest if first is None else min(first, earliest)
    cutoff = first + args.seconds * 1_000_000_000
    parts = []
    for rows in packages(args.input):
        starts = rows[(rows["kind"] == 0) & (rows["time"] < cutoff)]
        if len(starts):
            parts.append(starts.copy())
    requests = np.concatenate(parts)
    ids = set(map(int, requests["id"]))
    assert len(ids) == len(requests)
    outcomes = {}
    for rows in packages(args.input):
        for row in rows[rows["kind"] != 0]:
            request_id = int(row["id"])
            if request_id in ids:
                assert request_id not in outcomes
                outcomes[request_id] = tuple(int(row[k]) for k in ("kind", "time", "download_start", "download_end"))
    result = summarize(requests, outcomes)
    digest = hashlib.sha256()
    with args.input.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    result.update(input_sha256=digest.hexdigest(),
                  first_start_ns=first, prefix_seconds=args.seconds,
                  method="Offline FETR range analysis, not cache residency or disk I/O measurement")
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
