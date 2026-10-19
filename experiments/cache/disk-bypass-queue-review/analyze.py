#!/usr/bin/env python3
"""Read the saved Yahoo benchmark, without replaying or changing production."""
import csv
import datetime
import hashlib
import json
import math
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
RAW = HERE / 'raw'
RAW.mkdir(exist_ok=True)
BENCHMARK = Path('/Users/pascal.seitz/go/src/github.com/DataDog/Development/yahoo/byoc-yahoo-sim/user-sim/reports/sweep_20261018T160235Z_cold')
WORKSPACE = 'pascal-feuer'
data = json.loads((BENCHMARK / 'comparison-data.json').read_text())
rows = [r for r in data['rows'] if r['workspace'] == WORKSPACE]
assert len(rows) == 10

def iso(t):
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).isoformat()

def quantiles(values):
    a = sorted(values)
    return {name: a[min(len(a)-1, max(0, math.ceil(len(a)*q)-1))] for name, q in
            [('min', 0), ('p50', .5), ('p95', .95), ('max', 1)]}

results = []
manifest = {}
for r in rows:
    name = f"{r['stage']}-{WORKSPACE}-disk_read_bytes-operation.json"
    source = BENCHMARK / 'metric-evidence' / name
    shutil.copyfile(source, RAW / name)
    manifest[name] = hashlib.sha256(source.read_bytes()).hexdigest()
    x = json.loads(source.read_text())
    series = x['response']['series']
    assert len(series) == 14
    nodes = set()
    rates = []
    fleet = {}
    steps = set()
    nulls = 0
    metadata_intervals = set()
    for s in series:
        tags = dict(t.split(':', 1) for t in s['tag_set'])
        assert tags['kube_cluster_name'] == 'yahoo-sim'
        assert tags['operation'] == 'read'
        node = tags['cloudprem_node_id']
        assert node.startswith('cloudprem-ws-pascal-feuer-searcher-')
        assert node not in nodes
        nodes.add(node)
        points = s['pointlist']
        step = min(b[0]-a[0] for a,b in zip(points, points[1:]))/1000
        assert step == 10
        steps.add(step)
        metadata_intervals.add(s['interval'])
        for t, v in points:
            if v is None:
                nulls += 1
                continue
            rate = v/step/1e9
            rates.append(rate)
            fleet.setdefault(t, []).append(rate)
    full_fleet = {t:sum(v)/14 for t,v in fleet.items() if len(v)==14}
    with (RAW / f"{r['stage']}-disk-gbps-per-node.csv").open('w') as f:
        w = csv.writer(f)
        w.writerow(['utc', 'fleet_mean_GB_per_second_per_node'])
        w.writerows((iso(t/1000), v) for t,v in sorted(full_fleet.items()))
    pending = [p['peak'] for p in r['metrics']['resources']['peaks'] if p.get('pool') == 'search' and p['unit'] == 'tasks']
    results.append({
        'qps': r['qps'], 'start_utc': iso(r['metrics_start_epoch']),
        'end_utc': iso(r['metrics_end_epoch']), 'nodes': len(nodes),
        'disk_read_TB': r['disk_read_bytes']/1e12,
        'disk_GB_per_second_per_node_over_run': r['disk_read_bytes']/r['duration_seconds']/14/1e9,
        'search_thread_pool_max_pending': max(pending) if pending else None,
        'query_p95_seconds': r['p95'],
        'point_spacing_seconds': sorted(steps), 'api_interval_metadata_seconds': sorted(metadata_intervals),
        'null_points': nulls, 'node_samples': len(rates),
        'node_sample_GB_per_second': quantiles(rates),
        'fleet_mean_GB_per_second_per_node': quantiles(list(full_fleet.values())),
        'node_samples_below_1_GB_per_second_fraction': sum(v<1 for v in rates)/len(rates),
        'node_samples_at_least_3_GB_per_second_fraction': sum(v>=3 for v in rates)/len(rates),
        'query': x['query'],
    })
def latency_series(name):
    response = json.loads((RAW / name).read_text())
    output = {}
    for series in response['series']:
        tags = dict(t.split(':', 1) for t in series['tag_set'])
        assert tags['operation'] == 'read'
        assert 'kube_cluster_name:yahoo-sim' in series['scope'].split(',')
        assert tags['cloudprem_node_id'].startswith('cloudprem-ws-pascal-feuer-searcher-')
        key = (tags['cloudprem_node_id'], tags['cache'])
        assert key not in output
        output[key] = dict(series['pointlist'])
    return output

sums = latency_series('latency-sum-full.json')
counts = latency_series('latency-count-full.json')
assert sums.keys() == counts.keys()
for row in results:
    start = datetime.datetime.fromisoformat(row['start_utc']).timestamp()*1000
    end = datetime.datetime.fromisoformat(row['end_utc']).timestamp()*1000
    row['read_latency_ms'] = {}
    for cache in ['all', 'split_range', 'predicate']:
        total = count = 0
        for key, points in sums.items():
            if cache != 'all' and key[1] != cache:
                continue
            for t, value in points.items():
                n = counts[key].get(t)
                if start <= t < end and value is not None and n:
                    total += value
                    count += n
        row['read_latency_ms'][cache] = {
            'mean': total/count*1000 if count else None, 'observations': count,
        }

full_disk = json.loads((RAW / 'disk-read-full.json').read_text())
fleet = {}
assert len(full_disk['series']) == 14
for series in full_disk['series']:
    for t, value in series['pointlist']:
        if value is not None:
            fleet.setdefault(t/1000, []).append(value)
gaps = []
for previous, following in zip(results, results[1:]):
    end = datetime.datetime.fromisoformat(previous['end_utc']).timestamp()
    start = datetime.datetime.fromisoformat(following['start_utc']).timestamp()
    interior = [(t,v) for t,v in sorted(fleet.items()) if end+10 <= t <= start-10]
    gaps.append({
        'after_qps': previous['qps'], 'gap_seconds': start-end,
        'interior_samples': [{'utc': iso(t), 'nodes': len(v), 'read_bytes': sum(v)} for t,v in interior],
    })
summary = {
    'benchmark': str(BENCHMARK), 'workspace': WORKSPACE,
    'interstage_recovery': gaps,
    'latency_method': 'Sum of distribution duration sums divided by distribution counts, using non-null matched ten-second bucket timestamps inside each stage. Successful DataFile reads, not pure device latency or payload-admission wait.',
    'deployed_source': {'image_digest': 'sha256:1d5d24320d66412dfd4ce9e064ad4078119e4b697409da4aaa0f8f8dc3255c49', 'pomsky': '85fe717a88aee29ee699270f84a818c75415a6ab', 'feuer': '4e7a7d12478c645d4d534ca6ef0fc33d87d279ca', 'payload_read_queue_present': False},
    'org': 'byoc-sim.datadoghq.com',
    'method': 'Reuse saved independently validated operation:read queries. Divide as_count buckets by observed 10-second timestamp spacing. Report API interval metadata separately. Queue peaks refer only to Quickwit search thread-pool tasks, not PayloadReadQueue.',
    'results': results,
}
(HERE / 'results.json').write_text(json.dumps(summary, indent=2)+'\n')
(HERE / 'settings.json').write_text(json.dumps({
    'benchmark': str(BENCHMARK), 'workspace': WORKSPACE,
    'new_replay': False, 'new_benchmark': False,
    'source_sha256': manifest,
    'schema_probe': 'kubectl exec searcher-0 curl http://127.0.0.1:7280/metrics, after benchmark, used for metric names only',
}, indent=2)+'\n')
print(json.dumps(summary, indent=2))
