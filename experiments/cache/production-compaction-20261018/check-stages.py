import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent / 'raw'
spec = importlib.util.spec_from_file_location('analysis', ROOT.parent / 'analyze.py')
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)
rows = json.loads((ROOT / 'stage-cache-metrics.json').read_text())
result = []
for version in [20, 21]:
    metrics = {name: json.loads((ROOT / f'cold-{name}-{version}.json').read_text())['series'] for name in analysis.METRICS}
    for stage in ['stage-4-qps-4', 'stage-5-qps-5', 'stage-6-qps-10']:
        row = next(r for r in rows if r['version'] == version and r['stage'] == stage and r['cache'] == 'split_range')
        for trim in [0, 20, 40]:
            for pod in sorted(analysis.PODS):
                def value(name, **labels):
                    return analysis.total(metrics[name], row['start'] + trim, row['end'] - trim, cache='split_range', cloudprem_node_id=pod, **labels)
                queued, rejected = value('write-admissions', outcome='queued'), value('write-admissions', outcome='queue_full')
                memory, disk, callback = [value('lookup-bytes', source=source) for source in ['memory', 'disk', 'callback']]
                result.append(dict(version=version, stage=stage, trim_seconds=trim, pod=pod,
                    compactions=value('compactions', operation='compact'), reclaimed_charge=value('compacted-charge'),
                    queued=queued, queue_full=rejected, queue_full_share=rejected / (queued + rejected),
                    wait_sum=value('write-wait-sum-base'), wait_count=value('write-wait-count-base'),
                    memory=memory, disk=disk, callback=callback))
(ROOT / 'pod-stage-metrics.json').write_text(json.dumps(result, indent=2))
print('\nBoundary sensitivity, split_range')
for stage in ['stage-4-qps-4', 'stage-5-qps-5', 'stage-6-qps-10']:
    for trim in [0, 20, 40]:
        values = []
        for version in [20, 21]:
            subset = [r for r in result if r['version'] == version and r['stage'] == stage and r['trim_seconds'] == trim]
            def summed(key): return sum(r[key] for r in subset)
            values.append(dict(version=version, queue_pct=100 * summed('queue_full') / (summed('queue_full') + summed('queued')),
                wait_ms=1000 * summed('wait_sum') / summed('wait_count'),
                memory_pct=100 * summed('memory') / (summed('memory') + summed('disk') + summed('callback'))))
        print(stage, 'trim', trim, values)
print('\nQPS10 pod concentration')
for version in [20, 21]:
    subset = [r for r in result if r['version'] == version and r['stage'] == 'stage-6-qps-10' and r['trim_seconds'] == 0]
    total_rejected = sum(r['queue_full'] for r in subset)
    print(version, 'rejection-share range', min(r['queue_full_share'] for r in subset), max(r['queue_full_share'] for r in subset),
          'largest pod share of rejections', max(r['queue_full'] for r in subset) / total_rejected,
          'mean-wait range ms', min(r['wait_sum'] / r['wait_count'] * 1000 for r in subset), max(r['wait_sum'] / r['wait_count'] * 1000 for r in subset))

print('\nQPS10 CPU from previously validated report evidence')
for version in [20, 21]:
    responses = {}
    for metric in ['cpu_seconds-container_name', 'cpu_requests-container_name']:
        response = json.loads((ROOT / f'qps10-{metric}-{version}.json').read_text())
        series = response['response']['series']
        assert len(series) == 14
        assert {dict(tag.split(':', 1) for tag in s['tag_set'])['pod_name'] for s in series} == analysis.PODS
        assert all('container_name:cloudprem' in s['tag_set'] for s in series)
        responses[metric] = {dict(tag.split(':', 1) for tag in s['tag_set'])['pod_name']: s for s in series}
    print('version', version)
    for pod in sorted(analysis.PODS):
        cpu = [v * 1e-9 for _, v in responses['cpu_seconds-container_name'][pod]['pointlist'] if v is not None]
        requests = {v for _, v in responses['cpu_requests-container_name'][pod]['pointlist'] if v is not None}
        assert len(requests) == 1
        request = requests.pop()
        print(pod.rsplit('-', 1)[-1], 'observed mean/peak % requests', round(sum(cpu)/len(cpu)/request*100, 1), round(max(cpu)/request*100, 1))
