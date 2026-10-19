#!/usr/bin/env python3
"""Summarize isolated diagnostic events without publishing object identities."""
from collections import Counter, defaultdict
import json
from pathlib import Path

root = Path(__file__).resolve().parent
path = root / 'raw/results/diagnostic/stderr.txt'
reads = Counter()
promotions = Counter()
evictions = Counter()
last_promotion_eviction = {}
capacity_pairs = Counter()
requests_per_read_range = defaultdict(set)
request_bins = defaultdict(Counter)
read_bins = defaultdict(Counter)
summary = Counter()

def bucket(size):
    for limit in [32768, 262144, 1048576, 8388608, 67108864]:
        if size <= limit:
            return str(limit)
    return 'larger'

for line in path.open():
    parts = line.strip().split(',')
    if not parts[0].startswith('RETENTION_'):
        continue
    event, *values = parts
    v = list(map(int, values))
    key = tuple(v[:3])
    if event == 'RETENTION_READ':
        size = v[2] - v[1]
        reads[key] += 1
        read_bins[bucket(size)].update(calls=1, bytes=size)
        summary.update(read_calls=1, read_bytes=size)
        if reads[key] > 1:
            summary.update(repeated_read_calls=1, repeated_read_bytes=size)
    elif event == 'RETENTION_EVICT':
        evictions[key] += 1
        summary.update(evictions=1, eviction_charge=v[3], eviction_payload=v[2]-v[1])
        if 4 * (v[2]-v[1]) <= 3 * v[3]:
            summary.update(evictions_with_quarter_charge_slack=1,
                           eviction_quarter_charge_slack=v[3]-(v[2]-v[1]))
    elif event == 'RETENTION_PROMOTION':
        _, start, end, read_start, read_end, before, after = v
        size = end-start
        requests_per_read_range[(v[0], read_start, read_end)].add((start, end))
        summary.update(promotions=1, promoted_payload=size, promotion_charge=after,
                       selected_read_bytes=read_end-read_start, selected_read_capacity=before)
        request_bins[bucket(size)].update(promotions=1, payload=size, charge=after)
        capacity_pairs[(before, after)] += 1
        if before != after:
            summary.update(shrunk_promotions=1, shrunk_payload=size, shrink_charge_saved=before-after)
        if promotions[key]:
            summary.update(repeated_promotions=1, repeated_promoted_payload=size)
            if evictions[key] > last_promotion_eviction[key]:
                summary.update(repeated_promotions_after_exact_eviction=1,
                               repeated_payload_after_exact_eviction=size)
        promotions[key] += 1
        last_promotion_eviction[key] = evictions[key]
        if read_end-read_start > 8388608 and size <= 32768:
            summary.update(small_request_large_read_promotions=1,
                           small_request_large_read_payload=size,
                           small_request_large_read_selected_bytes=read_end-read_start,
                           small_request_large_read_charge=after)

for key, count in reads.items():
    if count <= 1:
        continue
    if requests_per_read_range[key] == {(key[1], key[2])}:
        summary.update(repeated_exact_request_read_calls=count-1,
                       repeated_exact_request_read_bytes=(count-1)*(key[2]-key[1]))
    if len(requests_per_read_range[key]) > 1:
        summary.update(repeated_multi_request_range_read_calls=count-1,
                       repeated_multi_request_range_read_bytes=(count-1)*(key[2]-key[1]))
summary.update(unique_read_ranges=len(reads), unique_promoted_ranges=len(promotions),
               unique_evicted_ranges=len(evictions))
result = dict(totals=dict(summary), read_size_bins=dict(read_bins), promotion_size_bins=dict(request_bins),
              capacity_pairs=[dict(before=b, after=a, count=n) for (b,a),n in capacity_pairs.most_common()],
              most_repeated_read_ranges=[dict(calls=n, bytes_per_read=k[2]-k[1], total_bytes=n*(k[2]-k[1]), distinct_requested_ranges=len(requests_per_read_range[k]))
                                         for k,n in reads.most_common(20)],
              caveats=['Selected read bytes are counted per promotion and include shared reads.',
                       'Repeated physical reads do not imply identical requested slices.',
                       'Retention of every previously read range is not capacity feasible.',
                       'Promotions are disk payload hits only, excluding pending-buffer hits.',
                       'Evictions count sampled victims, not contained replacements or compaction removals.',
                       'Event logging changes timing. This is a diagnostic run, not a speed comparison.'])
(root / 'raw/event-summary.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result['totals'], indent=2))
