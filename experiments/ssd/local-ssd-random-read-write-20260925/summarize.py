#!/usr/bin/env python3
"""Generate tables only from a successfully completed random-read sweep."""
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def load(name, bs, rate, phase):
    result = json.loads((ROOT / f'{name}.json').read_text())
    jobs = {job['jobname']: job for job in result['jobs']}
    expected = {'reader'} if rate == 0 else {'reader', 'writer'}
    assert set(jobs) == expected and all(job['error'] == 0 for job in jobs.values())
    read = jobs['reader']['read']
    write = jobs.get('writer', {}).get('write', {})
    return dict(phase=phase, read_block_bytes=bs, write_cap_MB_s=rate,
                read_MB_s=read['bw_bytes'] / 1e6,
                write_MB_s=write.get('bw_bytes', 0) / 1e6,
                read_IOPS=read['iops'],
                read_p99_ms=read['clat_ns']['percentile']['99.000000'] / 1e6)


def main():
    assert (ROOT / 'DONE').exists(), 'wait for a successful, complete sweep'
    metadata = json.loads((ROOT / 'metadata.json').read_text())
    assert metadata['read_pattern'] == 'randread' and metadata['write_pattern'] == 'write'
    blocks = metadata['blocks']
    rates = metadata['mixed_write_targets_MB_s']
    rows = [load(f'random-mixed-bs{bs}-rate{rate}', bs, rate, 'main')
            for bs in blocks for rate in rates]
    repeats = [load(f'repeat-bs{bs}-rate{rate}', bs, rate, 'repeat')
               for bs, rate in [(16384, 0), (16384, 450), (16384, 910), (65536, 910)]]
    with (ROOT / 'summary.csv').open('w', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows + repeats)
    cells = {(row['read_block_bytes'], row['write_cap_MB_s']): row for row in rows}
    labels = {bs: f'{bs // 1024} KiB' if bs < 1048576 else f'{bs // 1048576} MiB'
              for bs in blocks}
    report = [
        '# Random reads with sequential writes', '',
        f"Measured **{metadata['start'][:10]}** on `m8g-32cpu-local-ssd` (EC2 `m8gd.8xlarge`).", '',
        '**Workload:** one uniform random reader at QD32, plus a separate sequential writer',
        'using **1 MiB blocks at up to QD32**, on the same local NVMe SSD.',
        'fio libaio with O_DIRECT; fully initialized 128 GiB read and 64 GiB write files.',
        'Each main cell: 5 seconds warmup + 30 seconds measurement; shuffled execution order.', '',
        'Throughput is decimal MB/s. Columns are requested write caps, not achieved rates.',
        'No writer is started for the zero-cap baseline. Uncapped means no write-rate limit.',
        f"Standalone 1 MiB write control: **{metadata['calibration_write_MB_s']:,.1f} MB/s**.", '',
    ]
    headers = ['No writes'] + [f'{rate:,} MB/s cap' for rate in rates[1:-1]] + ['Uncapped']
    for title, field, precision in [('Random read throughput (MB/s)', 'read_MB_s', 1),
                                    ('Achieved sequential write throughput (MB/s)', 'write_MB_s', 1),
                                    ('Random read p99 completion latency (ms)', 'read_p99_ms', 3)]:
        report += [f'## {title}', '', '| Read block | ' + ' | '.join(headers) + ' |',
                   '| --- | ' + ' | '.join(['---:'] * len(rates)) + ' |']
        for bs in blocks:
            values = [f'{cells[bs, rate][field]:,.{precision}f}' for rate in rates]
            report.append(f'| {labels[bs]} | ' + ' | '.join(values) + ' |')
        report.append('')
    report += ['## Longer validation runs', '',
               '5 seconds warmup + 60 seconds measurement, using the same initialized files.',
               'Separate observations, not averaged into the main tables.', '',
               '| Read block | Write cap MB/s | Main read MB/s | Repeat read MB/s | Change | Repeat write MB/s |',
               '| --- | ---: | ---: | ---: | ---: | ---: |']
    for row in repeats:
        bs, rate = row['read_block_bytes'], row['write_cap_MB_s']
        original = cells[bs, rate]['read_MB_s']
        change = 100 * (row['read_MB_s'] / original - 1)
        report.append(f"| {labels[bs]} | {rate} | {original:,.1f} | {row['read_MB_s']:,.1f} | {change:+.1f}% | {row['write_MB_s']:,.1f} |")
    report += ['', '## Limits and sources', '',
               '- These are random-read measurements, not relabeled sequential-read results.',
               '- Single main runs and selected repeats are not confidence intervals.',
               '- Low-occupancy SSD, no full-drive/aged steady-state preconditioning.',
               '- O_DIRECT bypasses host page cache, not device caches; no per-write fsync.',
               '- p99 is fio completion latency, not application end-to-end latency.',
               '- Same fio settings as the original sweep except the read access pattern and matrix scope.',
               '  The runs are on different dates, so comparisons also include device-state/time drift.',
               '- This is a fio device microbenchmark, not a Feuer API or Rust-backend benchmark.', '',
               'See [methodology and reproduction](METHODOLOGY.md), [CSV](summary.csv), and',
               '[the original sequential-read results](../ssd-concurrent-read-write.md).', '']
    (ROOT / 'REPORT.md').write_text('\n'.join(report))


if __name__ == '__main__':
    main()
