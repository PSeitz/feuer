import collections
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent / 'raw'
METRICS = ['compacted-charge', 'compactions', 'evictions', 'lookup-bytes', 'write-admissions', 'write-wait-sum-base', 'write-wait-count-base']
PODS = {f'cloudprem-ws-pascal-feuer-searcher-{i}' for i in range(14)}


def total(series, start, end, **labels):
    values = []
    for s in series:
        tags = dict(t.split(':', 1) for t in s['tag_set'])
        if any(tags.get(k) != v for k, v in labels.items()):
            continue
        for timestamp, value in s['pointlist']:
            timestamp /= 1000
            overlap = max(0, min(end, timestamp + s['interval']) - max(start, timestamp))
            if overlap and value is not None:
                values.append(value * overlap / s['interval'])
    return sum(values) if values else None


def divide(a, b):
    return a / b if a is not None and b else None


rows = []
for version in [20, 21]:
    metrics = {}
    for name in METRICS:
        response = json.loads((ROOT / f'cold-{name}-{version}.json').read_text())
        series = response['series']
        logical = collections.Counter()
        pods = set()
        for s in series:
            tags = dict(t.split(':', 1) for t in s['tag_set'])
            pods.add(tags['cloudprem_node_id'])
            logical[tuple((k, v) for k, v in sorted(tags.items()) if k not in ('host', 'version'))] += 1
            assert s['interval'] == 20
        assert pods == PODS
        assert all(n == 1 for n in logical.values())
        metrics[name] = series
    source = json.loads((ROOT / f'stage-source-{version}.json').read_text())
    for stage in source:
        start, end = float(stage['metrics_start_epoch']), float(stage['metrics_end_epoch'])
        for cache in ['split_range', 'predicate', 'partial_request']:
            def value(name, **labels):
                return total(metrics[name], start, end, cache=cache, **labels)
            memory = value('lookup-bytes', source='memory')
            disk = value('lookup-bytes', source='disk')
            callback = value('lookup-bytes', source='callback')
            queued = value('write-admissions', outcome='queued')
            rejected = value('write-admissions', outcome='queue_full')
            row = dict(version=version, stage=stage['stage'], cache=cache, start=start, end=end,
                       query_count=int(stage['total']), p50=float(stage['p50']), p95=float(stage['p95']),
                       cpu_seconds=float(stage['cpu_seconds']), s3_bytes=float(stage['s3_downloaded_bytes']),
                       disk_read_bytes=float(stage['disk_read_bytes']),
                       reclaimed_charge=value('compacted-charge'), compactions=value('compactions', operation='compact'),
                       evictions=value('evictions'), queued=queued, queue_full=rejected,
                       queue_full_share=divide(rejected, queued + rejected) if queued is not None and rejected is not None else None,
                       mean_write_wait_seconds=divide(value('write-wait-sum-base'), value('write-wait-count-base')),
                       memory_bytes=memory, disk_bytes=disk, callback_bytes=callback,
                       memory_byte_share=divide(memory, memory + disk + callback) if all(v is not None for v in [memory, disk, callback]) else None)
            rows.append(row)
(ROOT / 'stage-cache-metrics.json').write_text(json.dumps(rows, indent=2))
with (ROOT.parent / 'results' / 'stage-cache-metrics.csv').open('w') as f:
    writer = csv.DictWriter(f, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(row for row in rows if row['cache'] == 'split_range')
print('split_range, values 20 -> 21')
for stage in sorted({r['stage'] for r in rows}):
    a, b = [r for r in rows if r['cache'] == 'split_range' and r['stage'] == stage]
    def pair(key, scale=1):
        return ' -> '.join('N/A' if r[key] is None else f'{r[key] * scale:,.2f}' for r in [a, b])
    print(stage, 'compactions', pair('compactions'), 'reclaimed GB', pair('reclaimed_charge', 1e-9),
          'evictions', pair('evictions'), 'queue full %', pair('queue_full_share', 100),
          'write wait ms', pair('mean_write_wait_seconds', 1000), 'memory byte %', pair('memory_byte_share', 100))
    print('  per request:', ', '.join(f'{key} {(b[key] / b["query_count"]) / (a[key] / a["query_count"]) - 1:+.2%}' for key in ['cpu_seconds', 's3_bytes', 'disk_read_bytes']))
