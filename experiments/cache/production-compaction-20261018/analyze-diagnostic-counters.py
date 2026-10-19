import json
from pathlib import Path
import re
import sys

ROOT = Path(sys.argv[1])


def prometheus(path):
    result = {}
    for line in path.read_text().splitlines():
        match = re.fullmatch(r'(\w+)(?:\{(.*)\})?\s+(\S+)', line)
        if not match:
            continue
        tags = tuple(sorted((key, json.loads('"' + value + '"')) for key, value in re.findall(r'(\w+)="((?:\\.|[^"\\])*)"', match[2] or '')))
        result[(match[1], tags)] = float(match[3])
    return result


def snapshot(label):
    cpu = dict(line.split() for line in (ROOT / f'{label}-cpu.txt').read_text().splitlines())
    process = (ROOT / f'{label}-process.txt').read_text().rsplit(') ', 1)[1].split()
    return dict(metrics=prometheus(ROOT / f'{label}.prom'), cpu_seconds=int(cpu['usage_usec']) / 1e6,
                throttled_seconds=int(cpu['throttled_usec']) / 1e6,
                process_ticks=int(process[11]) + int(process[12]), process_start=process[19],
                uptime_seconds=float((ROOT / f'{label}-uptime.txt').read_text().split()[0]))


results = []
for first, last in [('before-load', 'before-profile'), ('before-profile', 'after-profile'), ('before-profile', 'after-failure'), ('before-load', 'after-load')]:
    if not (ROOT / f'{last}.prom').exists():
        continue
    before, after = snapshot(first), snapshot(last)
    assert before['process_start'] == after['process_start'], 'Process restarted'
    wall = after['uptime_seconds'] - before['uptime_seconds']
    item = dict(first=first, last=last, wall_seconds=wall,
                cpu_seconds=after['cpu_seconds'] - before['cpu_seconds'],
                process_cpu_ticks=after['process_ticks'] - before['process_ticks'],
                throttled_seconds=after['throttled_seconds'] - before['throttled_seconds'], caches={})
    item['average_cores'] = item['cpu_seconds'] / wall
    def delta(name, **tags):
        key = (name, tuple(sorted(tags.items())))
        if key not in before['metrics'] or key not in after['metrics']:
            return None
        value = after['metrics'][key] - before['metrics'][key]
        assert value >= 0, (name, tags, value)
        return value
    item['s3_bytes'] = delta('cloudprem_object_storage_get_requests_bytes_count')
    item['s3_requests'] = delta('cloudprem_object_storage_get_requests_count')
    for cache in ['split_range', 'predicate', 'partial_request']:
        def value(name, **tags): return delta(name, cache=cache, **tags)
        values = dict(compactions=value('feuer_memory_operations_total', operation='compact'),
                      reclaimed_charge=value('feuer_memory_compacted_payload_bytes_total'),
                      evictions=value('feuer_memory_eviction_triggering_insertions_total'),
                      queued=value('feuer_disk_write_queue_total', outcome='queued'),
                      queue_full=value('feuer_disk_write_queue_total', outcome='queue_full'),
                      wait_sum=value('feuer_disk_write_queue_duration_seconds_sum'),
                      wait_count=value('feuer_disk_write_queue_duration_seconds_count'),
                      disk_read_bytes=value('feuer_disk_io_bytes_total', operation='read'),
                      disk_write_bytes=value('feuer_disk_io_bytes_total', operation='write'),
                      lookup_bytes={source:value('feuer_lookup_bytes_total', source=source) for source in ['memory', 'disk', 'callback']},
                      read_errors=value('feuer_disk_io_total', operation='read', outcome='error'),
                      write_errors=value('feuer_disk_io_total', operation='write', outcome='error'),
                      callback_errors=value('feuer_lookup_total', outcome='callback_error'))
        values['queue_full_share'] = values['queue_full'] / (values['queue_full'] + values['queued']) if values['queue_full'] is not None and values['queued'] else None
        values['mean_write_wait_seconds'] = values['wait_sum'] / values['wait_count'] if values['wait_count'] else None
        values['memory_byte_share'] = values['lookup_bytes']['memory'] / sum(values['lookup_bytes'].values()) if all(v is not None for v in values['lookup_bytes'].values()) and sum(values['lookup_bytes'].values()) else None
        item['caches'][cache] = values
    results.append(item)
(ROOT / 'counter-analysis.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results, indent=2))
