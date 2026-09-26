#!/usr/bin/env python3
"""Summarize a completed write-QD sweep; raw fio JSON remains the source."""
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def load(case):
    result = json.loads((ROOT / (case['name'] + '.json')).read_text())
    jobs = {job['jobname']: job for job in result['jobs']}
    expected = ({'reader'} if case['read_bytes'] else set()) | ({'writer'} if case['write_qd'] else set())
    assert set(jobs) == expected and all(job['error'] == 0 for job in jobs.values())
    row = dict(case=case['name'], kind=case['kind'], read_block_bytes=case['read_bytes'],
               write_qd=case['write_qd'], write_cap_MB_s=case['write_cap_MB_s'],
               runtime_seconds=case['runtime'])
    for name, direction in [('reader', 'read'), ('writer', 'write')]:
        stats = jobs.get(name, {}).get(direction)
        if stats:
            assert stats['runtime'] >= case['runtime'] * 1000 and stats['io_bytes'] > 0
        row[f'{direction}_MB_s'] = stats['bw_bytes'] / 1e6 if stats else 0
        row[f'{direction}_IOPS'] = stats['iops'] if stats else 0
        row[f'{direction}_p99_ms'] = stats['clat_ns']['percentile']['99.000000'] / 1e6 if stats else 0
    row['cpu_pct'] = sum(job['usr_cpu'] + job['sys_cpu'] for job in jobs.values())
    return row


def main():
    assert (ROOT / 'DONE').exists(), 'wait for a successful, complete sweep'
    metadata = json.loads((ROOT / 'metadata.json').read_text())
    assert metadata['ioengine'] == 'libaio'
    cases = json.loads((ROOT / 'cases.json').read_text())
    assert len(cases) == 48 and len({case['name'] for case in cases}) == 48
    rows = [load(case) for case in cases]
    with (ROOT / 'summary.csv').open('w', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    blocks, depths = metadata['blocks'], [0] + metadata['writer_depths']
    labels = {bs: f'{bs // 1024} KiB' if bs < 1048576 else '1 MiB' for bs in blocks}
    cells = {(row['read_block_bytes'], row['write_qd']): row for row in rows if row['kind'] == 'mixed'}
    report = [
        '# Random reads: sequential writer queue-depth sweep', '',
        f"Measured **{metadata['start'][:10]}** on `m8g-32cpu-local-ssd` (EC2 `m8gd.8xlarge`).", '',
        'One random reader at **QD32**, with a separate sequential **1 MiB writer**.',
        '**No write-rate cap** in the QD sweep. QD is the per-stream maximum outstanding fio requests,',
        'not a global shared limit or a measured device queue depth.',
        'fio libaio, O_DIRECT, same workload settings as the prior random-read sweep.',
        'Main cases: **5 seconds warmup + 30 seconds measurement**, shuffled execution order.',
        'Throughput is decimal MB/s; block sizes are binary KiB/MiB.', '',
    ]
    headers = ['No writes'] + [f'Write QD{qd}' for qd in depths[1:]]
    for title, field, precision in [('Read throughput (MB/s)', 'read_MB_s', 1),
                                    ('Achieved write throughput (MB/s)', 'write_MB_s', 1),
                                    ('Read p99 completion latency (ms)', 'read_p99_ms', 3)]:
        report += [f'## {title}', '', '| Read block | ' + ' | '.join(headers) + ' |',
                   '| --- | ' + ' | '.join(['---:'] * len(depths)) + ' |']
        for bs in blocks:
            values = [f'{cells[bs, qd][field]:,.{precision}f}' for qd in depths]
            report.append(f'| {labels[bs]} | ' + ' | '.join(values) + ' |')
        report.append('')
    report += ['## Write-only controls', '',
               '| Writer QD | Write MB/s | Write p99 ms |', '| ---: | ---: | ---: |']
    for row in sorted((r for r in rows if r['kind'] == 'write-only'), key=lambda r: r['write_qd']):
        report.append(f"| {row['write_qd']} | {row['write_MB_s']:,.1f} | {row['write_p99_ms']:.3f} |")
    report += ['', '## Rate-capped controls', '',
               'Writer QD32; same initialized files and timing as the QD sweep.', '',
               '| Read block | Write cap MB/s | Read MB/s | Actual write MB/s | Read p99 ms |',
               '| --- | ---: | ---: | ---: | ---: |']
    for row in sorted((r for r in rows if r['kind'] == 'rate'), key=lambda r: (r['read_block_bytes'], r['write_cap_MB_s'])):
        report.append(f"| {labels[row['read_block_bytes']]} | {row['write_cap_MB_s']} | {row['read_MB_s']:,.1f} | {row['write_MB_s']:,.1f} | {row['read_p99_ms']:.3f} |")
    report += ['', '## Longer repeats', '',
               '5 seconds warmup + 60 seconds measurement. Not averaged into main results.', '',
               '| Read block | Write QD | Main read MB/s | Repeat read MB/s | Change | Repeat write MB/s | Repeat read p99 ms |',
               '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in rows:
        if row['kind'] != 'repeat':
            continue
        bs, qd = row['read_block_bytes'], row['write_qd']
        original = cells[bs, qd]['read_MB_s']
        change = 100 * (row['read_MB_s'] / original - 1)
        report.append(f"| {labels[bs]} | {qd} | {original:,.1f} | {row['read_MB_s']:,.1f} | {change:+.1f}% | {row['write_MB_s']:,.1f} | {row['read_p99_ms']:.3f} |")
    report += ['', '## Scope', '',
               '- Single main runs plus selected repeats, not confidence intervals.',
               '- Fully initialized 128 GiB read and 64 GiB write files; low-occupancy SSD.',
               '- No aged/full-device preconditioning or per-write fsync.',
               '- p99 is fio completion latency, not Feuer end-to-end latency.',
               '- Only 1 MiB sequential writes; no small writes, RMW, or mixed read sizes.',
               '- Separate fio streams, not Feuer scheduling or its shared 64-slot ring.',
               '- This measures static concurrency limits, not adaptive-policy transitions.', '',
               'See [methodology and interpretation](METHODOLOGY.md), [CSV](summary.csv), and',
               '[the preceding rate-cap sweep](../local-ssd-random-read-write-20260925/REPORT.md).', '']
    (ROOT / 'REPORT.md').write_text('\n'.join(report))


if __name__ == '__main__':
    main()
