#!/usr/bin/env python3
import csv
import json
import statistics
from pathlib import Path

base = Path(__file__).resolve().parent
assert (base / 'results/DONE').exists(), 'suite is incomplete'
groups = {}
rows = []
for path in sorted((base / 'results').glob('r*-bs*.json')):
    data = json.loads(path.read_text())
    key = (data['read_bytes'], data['read_qd'], data['write_qd'])
    result = data['result']
    row = dict(case=path.stem, read_bytes=key[0], read_qd=key[1], write_qd=key[2], cpu_pct=result['cpu_pct'])
    for direction in ('read', 'write'):
        row.update({direction + '_' + k: v for k, v in result[direction].items()})
    rows.append(row)
    groups.setdefault(key, []).append(row)
assert len(rows) == 69 and len(groups) == 23
with (base / 'summary.csv').open('w') as f:
    writer = csv.DictWriter(f, fieldnames=rows[0].keys())
    writer.writeheader()
    writer.writerows(rows)
lines = ['# Feuer io_uring benchmark', '',
    'Host: `m8g-32cpu-local-ssd`, local NVMe, O_DIRECT. See [methodology](README.md).', '',
    'Each cell is the median of three runs; p99 is the median of per-run p99s, not a pooled percentile.',
    'MB/s is decimal; CPU 100% = one core. QD is application outstanding requests per direction.', '',
    '| Read KiB | Read QD | Write QD | Read MB/s | Write MB/s | Read p99 µs | Write p99 µs | CPU % |',
    '|---:|---:|---:|---:|---:|---:|---:|---:|']
for key, values in sorted(groups.items()):
    assert len(values) == 3
    metrics = [statistics.median(v[k] for v in values) for k in
               ('read_MB_s', 'write_MB_s', 'read_p99_us', 'write_p99_us', 'cpu_pct')]
    lines.append('| ' + ' | '.join(map(str, [key[0] // 1024, key[1], key[2]])) + ' | ' +
                 ' | '.join(f'{v:.1f}' for v in metrics) + ' |')
lines += ['', '## Adjacent fio controls', '',
          'These use native io_uring and omit Feuer allocation, admission, eventfd, and reply overhead.',
          'fio latency is completion latency and must not be compared as identical to API latency.', '',
          '| Case | MB/s | Completion p99 µs |', '|---|---:|---:|']
for path in sorted((base / 'results').glob('fio-*.json')):
    job = json.loads(path.read_text())['jobs'][0]
    direction = job['write'] if job['write']['io_bytes'] else job['read']
    lines.append(f"| {path.stem} | {direction['bw_bytes']/1e6:.1f} | {direction['clat_ns']['percentile']['99.000000']/1000:.1f} |")
(base / 'REPORT.md').write_text('\n'.join(lines) + '\n')
