"""Check overlap arithmetic independently and count earlier differently shaped ranges."""
import collections
import importlib.util
import json
import random
from pathlib import Path

import numpy as np

ROOT = Path(__file__).parent
spec = importlib.util.spec_from_file_location("ranges", ROOT / "analyze.py")
ranges = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ranges)
rng = random.Random(1)
for _ in range(10000):
    entries = {tuple(sorted(rng.sample(range(30), 2))) for _ in range(rng.randrange(8))}
    start, end = sorted(rng.sample(range(30), 2))
    byte_set = set().union(*(set(range(a, b)) for a, b in entries))
    covered = len(byte_set.intersection(range(start, end)))
    assert ranges.overlap(entries, start, end) == covered
    expected = ("exact" if (start, end) in entries else
                "single_cover" if any(a <= start and end <= b for a, b in entries) else
                "fragments_cover" if covered == end - start else
                "partial" if covered else "disjoint")
    assert ranges.classify(entries, start, end) == (expected, covered)

# This independent prior analysis decoded the same capture and prefix.
requests_path = ROOT.parent / "partial-read-checksums/raw/static-analysis/promotion/requests.npy"
requests = np.load(requests_path)
requests.sort(order=["time", "id"])
prior = collections.defaultdict(set)
stats = collections.Counter()
noncovering_stats = collections.Counter()
exact_counts = collections.Counter()
fragment_sizes = []
partial_sizes = []
for row in requests:
    key = int(row["lo"]), int(row["hi"])
    start, end = int(row["start"]), int(row["end"])
    others = prior[key] - {(start, end)}
    category, covered = ranges.classify(others, start, end)
    ranges.count(stats, category, end - start, covered)
    noncovering = {r for r in others if not (r[0] <= start and end <= r[1])}
    category, covered = ranges.classify(noncovering, start, end)
    ranges.count(noncovering_stats, category, end - start, covered)
    exact_counts[(key, start, end)] += 1
    if category == "fragments_cover":
        fragment_sizes.append(end - start)
    if category == "partial":
        partial_sizes.append((end - start, covered))
    prior[key].add((start, end))

static_noncovering = collections.Counter()
for (key, start, end), frequency in exact_counts.items():
    entries = {r for r in prior[key] if not (r[0] <= start and end <= r[1])}
    category, covered = ranges.classify(entries, start, end)
    static_noncovering[category + "_distinct_ranges"] += 1
    static_noncovering[category + "_requests"] += frequency
    static_noncovering[category + "_requested_bytes"] += (end - start) * frequency
    static_noncovering[category + "_covered_bytes"] += covered * frequency

result = json.loads((ROOT / "raw/trace-prefix.json").read_text())
assert len(requests) == result["requests"] == 1606645
assert int((requests["end"] - requests["start"]).sum()) == result["requested_bytes"] == 2800015397526
for name in ("chronological_prior_starts_no_eviction", "prior_completed_successful_requests_no_eviction",
             "all_other_distinct_ranges_ignoring_time_and_residency"):
    totals = result[name]
    assert sum(v for k, v in totals.items() if k.endswith("_requests")) == result["requests"]
    assert sum(v for k, v in totals.items() if k.endswith("_requested_bytes")) == result["requested_bytes"]
    if name == "chronological_prior_starts_no_eviction":
        assert sum(totals[k.replace("_requested_bytes", "_covered_bytes")] for k in totals if k.endswith("_requested_bytes")) == result["requested_bytes"] - result["union_bytes"]
assert sum(b - a for entries in prior.values() for a, b in entries) == result["distinct_exact_range_bytes"]
assert sum(sum(b - a for a, b in ranges.merge(entries)) for entries in prior.values()) == result["union_bytes"]
output = {"random_byte_set_checks": 10000, "request_counts_and_byte_sums": "passed",
          "distinct_range_and_union_check_against_independent_decoding": "passed",
          "prior_differently_shaped_request_ranges_no_eviction": dict(stats),
          "prior_ranges_excluding_all_individual_covers": dict(noncovering_stats),
          "all_ranges_excluding_all_individual_covers": dict(static_noncovering),
          "fragment_request_size_min_max": [min(fragment_sizes), max(fragment_sizes)] if fragment_sizes else [],
          "partial_request_max_bytes": max(partial_sizes) if partial_sizes else None,
          "requests_source": str(requests_path)}
(ROOT / "raw/validation.json").write_text(json.dumps(output, indent=2) + "\n")
print(json.dumps(output, indent=2))
