#!/usr/bin/env python3
"""Report DIR CASE_ID MODE MEMORY_COST DISK_COST HALF_LIFE as JSON on stdout.

A validation failure still emits JSON and exits 1. Retain the raw metric inventory.
Disk transfer costs include the observation tail. Conditional disk hit ratios
exclude memory hits from their denominators. Source totals count exact callbacks.
"""
import csv
import json
import re
import sys
from pathlib import Path


def samples(path):
    for line in path.read_text().splitlines():
        if not line or line.startswith('#'):
            continue
        name, labels, value = re.fullmatch(r'([\w:]+)(\{.*\})?\s+(\S+)(?:\s+\d+)?', line).groups()
        yield name, dict(re.findall(r'(\w+)="([^"]*)"', labels or '')), float(value)


def metric(data, name, **labels):
    return sum(value for key, tags, value in data
               if key == name and all(tags.get(k) == v for k, v in labels.items()))


def report(directory, scenario, mode, memory_cost, disk_cost, half_life):
    summary = {name: (value if name in ('scheduling', 'source') else float(value))
               for name, value in list(csv.reader((directory / 'summary.csv').open()))[1:]}
    errors = []

    def check(condition, message):
        if not condition:
            errors.append(message)

    check((directory / 'exit-status.txt').read_text().strip() == '0', 'replay exit status is not zero')
    check(summary['scheduling'] == 'task_per_arrival', 'unexpected scheduling')
    check(summary['workers'] == 32, 'expected 32 workers')
    check(summary['requests'] == 1606645, 'unexpected request count')
    before = list(samples(directory / 'before.prom'))
    finished = list(samples(directory / 'requests-finished.prom'))
    settled = list(samples(directory / 'settled.prom'))
    delta = lambda data, name, **labels: metric(data, name, **labels) - metric(before, name, **labels)
    requests = summary['requests']
    lookups = {outcome: delta(finished, 'feuer_lookup_total', outcome=outcome)
               for outcome in ['memory_hit', 'disk_hit', 'callback', 'callback_error', 'invalid_download']}
    returned = {source: delta(finished, 'feuer_lookup_bytes_total', source=source)
                for source in ['memory', 'disk', 'callback']}
    check(sum(lookups.values()) == requests, 'lookup counts do not match the trace')
    check(metric(finished, 'replay_completed_total') == metric(settled, 'replay_completed_total') == requests, 'incomplete replay')
    summary['request_outcomes'] = lookups
    summary['request_hit_ratio'] = (lookups['memory_hit'] + lookups['disk_hit']) / requests
    summary['returned_bytes'] = returned
    summary['byte_hit_ratio'] = (returned['memory'] + returned['disk']) / sum(returned.values())
    summary['cpu_seconds_during_requests'] = delta(finished, 'process_cpu_seconds_total')
    summary['average_cores_during_requests'] = summary['cpu_seconds_during_requests'] / summary['execution_seconds']
    summary['cpu_seconds_in_observation_tail'] = metric(settled, 'process_cpu_seconds_total') - metric(finished, 'process_cpu_seconds_total')
    summary['io_including_observation_tail'] = {
        operation: {'calls': delta(settled, 'feuer_disk_io_total', operation=operation),
                    'errors': delta(settled, 'feuer_disk_io_total', operation=operation, outcome='error'),
                    'bytes': delta(settled, 'feuer_disk_io_bytes_total', operation=operation)}
        for operation in ['read', 'write']}
    summary['disk_eviction_triggering_insertions'] = delta(settled, 'feuer_disk_eviction_triggering_insertions_total')
    summary['disk_no_capacity'] = delta(settled, 'feuer_disk_write_entries_total', outcome='no_capacity')
    summary['disk_write_queue_full'] = delta(settled, 'feuer_disk_write_queue_total', outcome='queue_full')
    summary['pending_payload_bytes_after_tail'] = metric(settled, 'feuer_disk_write_pending_bytes')
    summary['scenario'] = scenario
    summary['disk_eviction_observed'] = summary['disk_eviction_triggering_insertions'] > 0
    check(summary['memory_bytes'] == 100000000000, 'unexpected memory capacity')
    check(summary['disk_bytes'] == 500000000000, 'unexpected disk capacity')
    check(mode in ('range', 'full_objects'), 'unknown mode')
    check(summary['full_objects'] == int(mode == 'full_objects'), 'unexpected full_objects setting')
    check(sum(returned.values()) == 2800015397526, 'unexpected returned byte count')
    for gauge in ('replay_queued_requests', 'replay_active_requests',
                  'replay_source_waiting_requests', 'replay_source_active_requests'):
        check(metric(finished, gauge) == metric(settled, gauge) == 0, gauge)
    check(summary['pending_payload_bytes_after_tail'] == 0, 'pending payload makes disk costs incomplete')
    for name, expected in [('memory_cost_per_request', float(memory_cost)),
                           ('disk_cost_per_request', float(disk_cost)),
                           ('access_count_half_life', int(half_life)),
                           ('memory_cost_per_requested_byte', 1),
                           ('disk_cost_per_requested_byte', 1), ('simulate_s3', 1),
                           ('source_slots', 48), ('source_latency_seconds', 0.108599),
                           ('source_bytes_per_second', 84101038.08)]:
        check(summary[name] == expected, 'unexpected ' + name)
    summary['mode'] = mode
    summary['source_gets'] = delta(finished, 'replay_source_gets_total')
    summary['source_bytes'] = delta(finished, 'replay_source_bytes_total')
    check(summary['source_gets'] == lookups['callback'], 'source GET count does not match callbacks')
    check(summary['source_bytes'] == returned['callback'], 'source bytes do not match exact downloads')
    for phase in ('queue', 'service'):
        name = f'replay_source_{phase}_seconds'
        count = delta(finished, name + '_count')
        check(count == summary['source_gets'], 'incomplete source ' + phase + ' observations')
        summary[f'source_{phase}_mean_seconds'] = delta(finished, name + '_sum') / count if count else None
    disk_requests = lookups['disk_hit'] + lookups['callback']
    disk_bytes = returned['disk'] + returned['callback']
    summary['conditional_disk_request_hit_ratio'] = lookups['disk_hit'] / disk_requests if disk_requests else None
    summary['conditional_disk_byte_hit_ratio'] = returned['disk'] / disk_bytes if disk_bytes else None
    summary['total_disk_transfer_bytes'] = sum(item['bytes'] for item in summary['io_including_observation_tail'].values())
    successful = sum(lookups[k] for k in ('memory_hit', 'disk_hit', 'callback'))
    summary['tier_request_hit_percent'] = {tier: lookups[outcome] / successful * 100
                                           for tier, outcome in [('memory', 'memory_hit'), ('disk', 'disk_hit')]}
    summary['tier_byte_hit_percent'] = {tier: returned[tier] / sum(returned.values()) * 100
                                        for tier in ('memory', 'disk')}
    summary['memory_eviction_triggering_insertions'] = delta(settled, 'feuer_memory_eviction_triggering_insertions_total')
    summary['write_admission'] = {outcome: delta(settled, 'feuer_disk_write_queue_total', outcome=outcome)
                                  for outcome in ('queued', 'queue_full', 'queue_closed', 'canceled', 'already_covered', 'redundant')}
    summary['write_terminal'] = {outcome: delta(settled, 'feuer_disk_write_entries_total', outcome=outcome)
                                 for outcome in ('published', 'already_covered', 'no_capacity', 'failed', 'canceled')}
    summary['memory'] = {}
    for phase, data in [('finished', finished), ('settled', settled)]:
        idle = metric(data, 'feuer_io_buffer_pool_bytes', status='idle')
        used = metric(data, 'feuer_memory_used_bytes')
        summary['memory'][phase] = {'resident_charge_bytes': used - idle,
                                    'idle_bytes': idle, 'used_bytes': used}
    summary['io_observation_tail'] = {operation: {
        field: metric(settled, name, operation=operation) - metric(finished, name, operation=operation)
        for field, name in [('calls', 'feuer_disk_io_total'), ('bytes', 'feuer_disk_io_bytes_total')]}
        for operation in ('read', 'write')}
    check(lookups['callback_error'] == lookups['invalid_download'] == 0, 'lookup errors')
    check(all(item['errors'] == 0 for item in summary['io_including_observation_tail'].values()), 'disk I/O errors')
    check(summary['write_terminal']['failed'] == summary['write_terminal']['canceled'] == 0, 'failed or canceled disk writes')
    check(summary['write_admission']['queue_closed'] == summary['write_admission']['canceled'] == 0, 'closed or canceled write admission')
    peaks = {'replay_queued_requests': 0, 'replay_active_requests': 0, 'process_resident_memory_bytes': 0,
             'feuer_memory_used_bytes': 0,
             'feuer_disk_write_pending_bytes': 0,
             'replay_source_waiting_requests': 0, 'replay_source_active_requests': 0}
    for name, _, value in samples(directory / 'metrics.prom'):
        if name in peaks:
            peaks[name] = max(peaks[name], value)
    summary['sampled_peaks'] = peaks
    # Whole-device counters include other processes and count kernel requests, not DataFile calls.
    device_before = list(map(int, (directory / 'device-before.txt').read_text().split()))
    device_after = list(map(int, (directory / 'device-after.txt').read_text().split()))
    summary['whole_device_io'] = {name: (device_after[index] - device_before[index]) * scale
                                 for name, index, scale in [('reads', 0, 1), ('read_bytes', 2, 512),
                                                            ('writes', 4, 1), ('write_bytes', 6, 512)]}
    time_text = (directory / 'time.txt').read_text()
    summary['os_time'] = {key: float(re.search(re.escape(label) + r':\s+([\d.]+)', time_text).group(1))
                           for key, label in [('user_seconds', 'User time (seconds)'),
                                              ('system_seconds', 'System time (seconds)'),
                                              ('max_rss_kib', 'Maximum resident set size (kbytes)'),
                                              ('file_inputs', 'File system inputs'), ('file_outputs', 'File system outputs')]}
    rows = []
    for line in (directory / 'iostat.txt').read_text().splitlines():
        fields = line.split()
        if fields and fields[0] == 'Device':
            columns = fields[1:]
        elif fields and fields[0] == (directory / 'device-name.txt').read_text().strip():
            rows.append(dict(zip(columns, map(float, fields[1:]))))
    summary['device_iostat'] = {name: sum(row[name] for row in rows) / len(rows)
                                 for name in ('aqu-sz', '%util')}
    for operation in ('r', 'w'):
        count = sum(row[operation + '/s'] for row in rows)
        summary['device_iostat'][operation + '_await_ms_request_weighted'] = (
            sum(row[operation + '/s'] * row[operation + '_await'] for row in rows) / count if count else 0)
    summary['validation'] = {'passed': not errors, 'errors': errors}
    return summary


if __name__ == '__main__':
    result = report(Path(sys.argv[1]), *sys.argv[2:])
    json.dump(result, sys.stdout, indent=2, allow_nan=False)
    print()
    sys.exit(0 if result['validation']['passed'] else 1)
