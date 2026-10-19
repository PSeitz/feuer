"""Compare downloaded pod4 profiles wholly inside each measured QPS10 window."""
import collections
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent / 'raw'
PROFILES = ROOT / 'host-profiles'
manifest = json.loads((PROFILES / 'download-manifest.json').read_text())
stages = {}
for version in [20, 21]:
    rows = json.loads((ROOT / f'stage-source-{version}.json').read_text())
    stages[version] = next(row for row in rows if row['stage'] == 'stage-6-qps-10')

profiles = []
identities = collections.defaultdict(set)
for item in sorted(manifest, key=lambda item: (item['version'], item['start_ms'])):
    stage = stages[item['version']]
    inside = (item['start_ms'] >= float(stage['metrics_start_epoch'])*1000 and
              item['end_ms'] <= float(stage['metrics_end_epoch'])*1000)
    assert inside == item['inside_stage']
    if not inside:
        continue
    folder = PROFILES / f"feuer{item['version']}" / item['profile_id']
    metadata = json.loads((folder / 'metadata.json').read_text())
    tags = dict(tag.split(':', 1) for tag in metadata['tags'])
    assert metadata['start'] == item['start_ms'] and metadata['end'] == item['end_ms']
    assert metadata['family'] == 'ebpf' and metadata['service'] == 'quickwit'
    assert tags['pod_name'] == 'cloudprem-ws-pascal-feuer-searcher-4'
    assert tags['host'] == 'i-00c7b764c5b7e456c'
    assert tags['image_tag'] == f"cp-rc-0.1.35-feuer-{item['version']}"
    identities[item['version']].add((tags['container_id'], tags['image_id']))
    inputs = list(folder.rglob('cpu.symbolicated.pprof'))
    assert len(inputs) == 1
    source = inputs[0]
    raw = folder / 'cpu-raw.txt'
    if not raw.exists():
        with raw.open('x') as output:
            subprocess.run(['go','tool','pprof','-symbolize=none','-sample_index=cpu-time','-raw',str(source)], stdout=output, check=True)
    header = raw.read_text()[:300]
    assert 'Period: 50000000\n' in header
    assert 'cpu-time/nanoseconds cpu-samples/count\n' in header
    subprocess.run([sys.executable, str(ROOT.parent / 'analyze-pprof.py'), str(raw)], stdout=subprocess.DEVNULL, check=True)
    result = json.loads(raw.with_name(raw.stem + '-attribution.json').read_text())
    assert abs(result['cpu_seconds'] - result['sample_count']*.05) < 1e-6
    profiles.append(dict(version=item['version'], profile_id=item['profile_id'], start_ms=item['start_ms'], end_ms=item['end_ms'],
        wall_seconds=(item['end_ms']-item['start_ms'])/1000, sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
        sample_count=result['sample_count'], cpu_seconds=result['cpu_seconds'],
        unknown_leaf_cpu_share=result['unknown_leaf_cpu_share'], categories=result['categories'], threads=result['threads'],
        rule_order=result['rule_order'], attribution_file=str(raw.with_name(raw.stem + '-attribution.json').relative_to(ROOT))))

summaries = []
for version in [20, 21]:
    selected = [p for p in profiles if p['version'] == version]
    assert len(selected) == 4 and len(identities[version]) == 1
    assert all(p['rule_order'] == profiles[0]['rule_order'] for p in selected)
    total_cpu = sum(p['cpu_seconds'] for p in selected)
    category_names = sorted({c['category'] for p in selected for c in p['categories']})
    categories = []
    for name in category_names:
        values = [next((c for c in p['categories'] if c['category'] == name), {'cpu_seconds':0,'percent':0}) for p in selected]
        cpu = sum(v['cpu_seconds'] for v in values)
        categories.append(dict(category=name, cpu_seconds=cpu, percent=100*cpu/total_cpu,
            min_profile_percent=min(v['percent'] for v in values), max_profile_percent=max(v['percent'] for v in values)))
    assert abs(sum(c['cpu_seconds'] for c in categories)-total_cpu) < 1e-6
    summaries.append(dict(version=version, profile_count=4, sample_count=sum(p['sample_count'] for p in selected),
        cpu_seconds=total_cpu, sum_profile_wall_seconds=sum(p['wall_seconds'] for p in selected),
        start_ms=selected[0]['start_ms'], end_ms=selected[-1]['end_ms'],
        metadata_overlap_ms=sum(max(0,a['end_ms']-b['start_ms']) for a,b in zip(selected, selected[1:])),
        unknown_leaf_cpu_share=sum(p['unknown_leaf_cpu_share']*p['cpu_seconds'] for p in selected)/total_cpu,
        categories=categories))

output = dict(method='Same ordered disjoint stack rules for every sample. Version shares use summed CPU seconds, not a mean of profile percentages. Metadata overlaps are reported, not de-duplicated without sample timestamps.',
    excluded_profiles=[p for p in manifest if not p['inside_stage']], identities={str(k):list(v) for k,v in identities.items()},
    profiles=profiles, versions=summaries)
(PROFILES / 'pod4-version-attribution.json').write_text(json.dumps(output, indent=2))
for summary in summaries:
    print('VERSION', summary['version'], 'CPU_SECONDS',summary['cpu_seconds'],'SAMPLES',summary['sample_count'],
          'WALL_SECONDS_SUM',summary['sum_profile_wall_seconds'],'METADATA_OVERLAP_MS',summary['metadata_overlap_ms'],
          'UNKNOWN_LEAF_SHARE',summary['unknown_leaf_cpu_share'])
    for c in summary['categories']:print(c)
print('\nPER PROFILE')
for p in profiles:
    values = {c['category']:c['percent'] for c in p['categories']}
    utc = datetime.datetime.fromtimestamp(p['start_ms']/1000,datetime.timezone.utc).isoformat()
    print(p['version'],utc,'CPU',p['cpu_seconds'],'memory_insert',round(values['Memory insertion / eviction / compaction'],2),
          'tantivy',round(values['Tantivy paths outside cache paths'],2),
          'storage',round(values['Storage paths outside memory-cache paths'],2),
          'unresolved',round(values['Unresolved leaf without recognized subsystem'],2))
