#!/usr/bin/env python3
"""Summarize diagnostic read lifetimes without treating arrivals as SSD reads."""
import collections
import csv
import json
from pathlib import Path
import sys


def union_size(ranges):
    total, end = 0, 0
    for start, stop in sorted(ranges):
        total += max(0, stop - max(start, end))
        end = max(end, stop)
    return total


def analyze(path):
    rows = []
    with path.open() as source:
        for row in csv.DictReader(source):
            rows.append({k: v if k == 'kind' else int(v) for k, v in row.items()})
    counts = collections.Counter(r['kind'] for r in rows)
    for kind in ('io_eintr', 'io_short', 'submit_eagain'):
        counts.setdefault(kind, 0)
    starts, ends, selected, by_task = {}, {}, collections.defaultdict(list), {}
    for r in rows:
        if r['kind'] == 'selected':
            selected[r['read']].append(r)
            assert r['task'] not in by_task
            by_task[r['task']] = r
        elif r['kind'] == 'read_start':
            assert r['read'] not in starts, 'initializer restarted'
            starts[r['read']] = r
        elif r['kind'] == 'read_end':
            assert r['read'] not in ends
            ends[r['read']] = r
    assert starts.keys() == ends.keys()
    assert starts.keys() == selected.keys()
    assert all(r['flag'] for r in ends.values())
    assert counts['selected'] == counts['disk_return']
    assert counts['selected'] + counts['buffer_hit'] == counts['promotion_begin'] == counts['promotion_end']
    assert all(by_task[s['task']]['read'] == rid for rid, s in starts.items())
    stats = collections.Counter({name: 0 for name in (
        'memory_visible_at_read_start_calls', 'memory_visible_at_read_start_bytes',
        'memory_visible_at_selection_calls', 'memory_visible_at_selection_requested_bytes',
        'read_while_covering_promotion_pending_calls', 'read_while_covering_promotion_pending_bytes',
    )})
    examples = collections.defaultdict(list)
    active = collections.defaultdict(dict)
    active_physical = {}
    pending_promotions = collections.defaultdict(dict)
    outstanding_promotions = {}
    for r in rows:
        kind, key, task, read = r['kind'], r['key'], r['task'], r['read']
        if kind == 'disk_return':
            pending_promotions[key][task] = r
        elif kind == 'promotion_end':
            if task in by_task:
                prior = pending_promotions[key].pop(task)
                outstanding_promotions[task] = r['time_ns'] - prior['time_ns']
                stats['redundant_disk_read_promotions'] += not r['flag']
            stats['redundant_promotions'] += not r['flag']
        elif kind == 'read_start':
            request = by_task[task]
            length = r['end'] - r['start']
            stats['read_bytes'] += length
            stats['read_calls'] += 1
            if r['flag']:
                stats['memory_visible_at_read_start_calls'] += 1
                stats['memory_visible_at_read_start_bytes'] += length
                examples['memory_visible_at_read_start'].append(r)
            overlaps = [(max(r['start'], a['start']), min(r['end'], a['end']))
                        for a in active[key].values()
                        if max(r['start'], a['start']) < min(r['end'], a['end'])]
            if overlaps:
                stats['reads_overlapping_live_read_calls'] += 1
                stats['reads_overlapping_live_read_bytes'] += length
                stats['concurrent_object_overlap_bytes'] += union_size(overlaps)
                examples['concurrent_overlap'].append(r)
            covering = [a for a in active[key].values()
                        if a['start'] <= request['start'] and request['end'] <= a['end']]
            if covering:
                stats['read_with_live_read_covering_request_calls'] += 1
                stats['read_with_live_read_covering_request_bytes'] += length
                examples['live_read_covering_request'].append(dict(read=r, selected=request, covering=covering))
            pending = [p for p in pending_promotions[key].values()
                       if p['start'] <= request['start'] and request['end'] <= p['end']]
            if pending:
                stats['read_while_covering_promotion_pending_calls'] += 1
                stats['read_while_covering_promotion_pending_bytes'] += length
                examples['promotion_pending'].append(r)
            physical_start = r['address'] // 4096 * 4096
            physical_end = (r['address'] + max(length, 1) + 4095) // 4096 * 4096
            physical_overlaps = [(max(physical_start, a), min(physical_end, b))
                                 for a, b in active_physical.values()
                                 if max(physical_start, a) < min(physical_end, b)]
            if physical_overlaps:
                stats['reads_overlapping_live_physical_read_calls'] += 1
                stats['concurrent_physical_overlap_bytes'] += union_size(physical_overlaps)
                examples['physical_overlap'].append(r)
            stats['aligned_read_bytes'] += physical_end - physical_start
            active_physical[read] = (physical_start, physical_end)
            active[key][read] = r
        elif kind == 'read_end':
            del active[key][read]
            del active_physical[read]
        elif kind == 'selected' and r['flag']:
            stats['memory_visible_at_selection_calls'] += 1
            stats['memory_visible_at_selection_requested_bytes'] += r['end'] - r['start']
    assert all(not v for v in active.values())
    assert all(not v for v in pending_promotions.values())
    for read, group in selected.items():
        length = starts[read]['end'] - starts[read]['start']
        stats['sharing_extra_callers'] += len(group) - 1
        stats['sharing_selected_read_byte_equivalent'] += length * (len(group) - 1)
        stats['reads_with_multiple_callers'] += len(group) > 1
        stats['shared_callers_requested_bytes'] += sum(r['end'] - r['start'] for r in group if r['task'] != starts[read]['task'])
        stats['max_callers_one_read'] = max(stats['max_callers_one_read'], len(group))
    completed_by_key = collections.defaultdict(list)
    for read, start in sorted(starts.items(), key=lambda pair: pair[1]['time_ns']):
        request = by_task[start['task']]
        for window in [100_000, 1_000_000, 10_000_000, 100_000_000]:
            stats.setdefault(f'read_after_covering_completion_within_{window}_ns_calls', 0)
            stats.setdefault(f'read_after_covering_completion_within_{window}_ns_bytes', 0)
            previous = [r for r in completed_by_key[start['key']]
                        if 0 <= start['time_ns'] - r['time_ns'] <= window
                        and r['start'] <= request['start'] and request['end'] <= r['end']]
            if previous:
                stats[f'read_after_covering_completion_within_{window}_ns_calls'] += 1
                stats[f'read_after_covering_completion_within_{window}_ns_bytes'] += start['end'] - start['start']
        completed_by_key[start['key']].append(ends[read])
    durations = sorted(outstanding_promotions.values())
    durations_summary = {name: durations[min(len(durations) - 1, int(q * len(durations)))]
                         for name, q in [('p50_ns', .5), ('p95_ns', .95), ('p99_ns', .99), ('max_ns', 1.)]}
    durations_summary['mean_ns'] = sum(durations) / len(durations)
    return dict(event_counts=dict(counts), measurements=dict(stats),
                disk_return_to_memory_publication=durations_summary,
                caveats=[
                    'One instrumented replay, not a baseline/candidate speed comparison.',
                    'Observed memory probes do not suppress reads or retain memory results.',
                    'Read lifetime includes checksum computation, not just kernel I/O.',
                    'Sharing byte equivalent is not a measured no-sharing counterfactual.',
                    'Rapid rereads can follow eviction or a smaller promoted range, not just races.',
                    'Object overlap is not necessarily overlap at the same physical SSD address.',
                    'Physical overlap uses 4-KiB alignment across all live payload API reads. Individual 1-MiB kernel pieces can already have finished.'
                ]), examples


if __name__ == '__main__':
    path = Path(sys.argv[1])
    summary, examples = analyze(path)
    report_path = path.with_name('report.json')
    if report_path.exists():
        report = json.loads(report_path.read_text())
        assert report['validation']['passed']
        observed = summary['measurements']
        io = report['io_including_observation_tail']['read']
        assert observed['read_calls'] == io['calls']
        assert observed['read_bytes'] == io['bytes']
        events = summary['event_counts']
        assert events['selected'] + events['buffer_hit'] == report['request_outcomes']['disk_hit']
        assert events['memory_miss'] == report['request_outcomes']['disk_hit'] + report['request_outcomes']['callback']
        summary['telemetry_cross_checks_passed'] = True
    path.with_name('duplicates.json').write_text(json.dumps(summary, indent=2) + '\n')
    path.with_name('duplicate-witnesses.json').write_text(json.dumps(examples, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
