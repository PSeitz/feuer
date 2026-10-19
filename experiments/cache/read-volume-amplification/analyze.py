#!/usr/bin/env python3
"""Classify missing writer checksums in conditional exact-range entry geometry.

This is not cache simulation. Every uncovered arrival immediately publishes an
entry, entries never evict, and every covered arrival reads and learns serially.
Only counts and byte geometry are reported, not predicted replay savings.
"""
import collections
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
INPUT = ROOT / "experiments/cache/partial-read-checksums/raw/static-analysis/checksum-learning/requests.jsonl"
OUT = pathlib.Path(__file__).resolve().parent / "raw"
AGE = 262144
LARGE = 8 * 1024 * 1024


def contains(outer, inner):
    return outer[0] <= inner[0] and inner[1] <= outer[1]


def analyze(rows):
    objects = collections.defaultdict(list)
    for i, (time, request_id, key, start, end) in enumerate(rows):
        objects[key].append((i, (start, end)))
    counts = collections.Counter(objects=len(objects))
    counts['distinct_object_ranges'] = sum(len({r for _, r in v}) for v in objects.values())
    counts['maximum_distinct_ranges_per_object'] = max(len({r for _, r in v}) for v in objects.values())
    classes = collections.defaultdict(collections.Counter)
    for keyrows in objects.values():
        events = collections.deque(maxlen=64)
        prior = set()
        entries = []
        for clock, request in keyrows:
            events.append((clock, request))
            prior.add(request)
            covering = [e for e in entries if contains(e['range'], request)]
            if not covering:
                entries = [e for e in entries if not contains(request, e['range'])]
                eligible = {r for r in prior if r[0] < r[1] and r != request and contains(request, r)}
                recent = {r for i, r in events if clock - i <= AGE} & eligible
                last64 = {r for _, r in events} & eligible
                entries.append(dict(range=request, checksums={request} | recent,
                                    prior=eligible, recent=recent, last64=last64))
                counts['publications'] += 1
                if request[1] - request[0] > LARGE:
                    counts['large_publications'] += 1
                    counts['large_recent_seeds'] += len(recent)
                    counts['large_all_prior_seeds'] += len(eligible)
                    counts['large_extra_prior_hash_bytes'] += sum(b-a for a,b in eligible-recent)
                continue
            entry = max(covering, key=lambda e: e['range'][0])
            if request == entry['range']:
                continue
            counts['proper_covered_requests'] += 1
            selected = min((r for r in entry['checksums'] if contains(r, request)), key=lambda r: r[1]-r[0])
            if selected == entry['range']:
                if request in entry['recent']:
                    raise AssertionError('An exact seeded range cannot select the whole entry')
                if request in entry['last64']:
                    reason = 'prior_range_lost_to_age'
                elif request in entry['prior']:
                    reason = 'prior_range_lost_to_event_limit'
                else:
                    reason = 'range_not_seen_before_publication'
                for group in ['all'] + (['large'] if selected[1]-selected[0] > LARGE else []):
                    c = classes[group + '/' + reason]
                    c['first_unseeded_requests'] += 1
                    c['requested_bytes'] += request[1]-request[0]
                    c['selected_entry_bytes'] += selected[1]-selected[0]
                    c['aligned_selected_bytes'] += ((selected[1]-selected[0]+4095)//4096)*4096
                    relative = request[0]-entry['range'][0]
                    c['aligned_exact_bytes'] += ((relative % 4096 + request[1]-request[0]+4095)//4096)*4096
            entry['checksums'].add(request)
    return dict(counts=counts, first_unseeded_geometry=dict(classes))


def self_test():
    # Repetition erases one observed slice from the 64-event history.
    rows = [(0, 0, 1, 4, 8)] + [(i, i, 1, 20, 24) for i in range(1, 65)]
    rows += [(65, 65, 1, 0, 100), (66, 66, 1, 4, 8), (67, 67, 1, 4, 8),
             (68, 68, 1, 30, 34)]
    result = analyze(rows)['first_unseeded_geometry']
    assert result['all/prior_range_lost_to_event_limit']['first_unseeded_requests'] == 1
    assert result['all/range_not_seen_before_publication']['first_unseeded_requests'] == 1
    assert result['all/prior_range_lost_to_event_limit']['selected_entry_bytes'] == 100
    seeded = analyze([(0, 0, 1, 4, 8), (1, 1, 1, 0, 100), (2, 2, 1, 4, 8)])
    assert seeded['first_unseeded_geometry'] == {}


if __name__ == '__main__':
    self_test()
    rows = [json.loads(line) for line in INPUT.open()]
    assert len(rows) == 1606645
    assert sum(r[4]-r[3] for r in rows) == 2800015397526
    assert all(rows[i][:2] <= rows[i+1][:2] for i in range(len(rows)-1))
    result = analyze(rows)
    result['validation'] = dict(requests=len(rows), requested_bytes=sum(r[4]-r[3] for r in rows),
                                input_sha256=hashlib.sha256(INPUT.read_bytes()).hexdigest(),
                                self_test='passed')
    OUT.mkdir(exist_ok=True)
    (OUT / 'geometry.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
