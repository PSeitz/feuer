import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent / 'raw'
rows = json.loads((ROOT / 'stage-cache-metrics.json').read_text())
output = []
for version in [20, 21]:
    series = {name: json.loads((ROOT / f'cold-{name}-{version}.json').read_text())['series'] for name in ['runtime-busy', 'runtime-queue', 'search-queue']}
    for items in series.values():
        assert len(items) == 14
        assert len({dict(t.split(':', 1) for t in s['tag_set'])['cloudprem_node_id'] for s in items}) == 14
    for row in [r for r in rows if r['version'] == version and r['cache'] == 'split_range']:
        start, end = row['start'], row['end']
        result = dict(version=version, stage=row['stage'], duration=end-start)
        for name, items in series.items():
            integrals, coverages, peaks = [], [], []
            for s in items:
                integral = coverage = peak = 0
                for timestamp, value in s['pointlist']:
                    timestamp /= 1000
                    overlap = max(0, min(end, timestamp + s['interval']) - max(start, timestamp))
                    if overlap and value is not None:
                        integral += value * overlap / s['interval'] if name == 'runtime-busy' else value * overlap
                        coverage += overlap
                        peak = max(peak, value)
                integrals.append(integral)
                coverages.append(coverage)
                peaks.append(peak)
            result[name + '_min_coverage_fraction'] = min(coverages) / (end-start)
            if name == 'runtime-busy':
                result['runtime_busy_seconds'] = sum(integrals) / 1000
                result['mean_busy_workers_per_pod'] = sum(integrals) / 1000 / (end-start) / 14
            else:
                result[name + '_mean_per_pod'] = sum(integrals) / sum(coverages)
                result[name + '_max_sample'] = max(peaks)
        output.append(result)
(ROOT / 'stage-runtime-metrics.json').write_text(json.dumps(output, indent=2))
for stage in sorted({r['stage'] for r in output}):
    print(stage)
    for r in [r for r in output if r['stage'] == stage]:
        print(r['version'], 'mean main-runtime busy workers/pod', round(r['mean_busy_workers_per_pod'], 2),
              'runtime-queue mean/max', round(r['runtime-queue_mean_per_pod'], 2), round(r['runtime-queue_max_sample'], 2),
              'search-queue mean/max', round(r['search-queue_mean_per_pod'], 2), round(r['search-queue_max_sample'], 2),
              'min coverage', round(min(v for k, v in r.items() if 'coverage' in k), 3))
