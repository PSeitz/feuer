#!/usr/bin/env python3
"""Validate and summarize mixed-size reads, splitting latency by actual I/O size."""
import csv
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def main():
    assert (ROOT / 'DONE').exists(), 'wait for successful completion'
    cases = json.loads((ROOT / 'cases.json').read_text())
    assert len(cases) == 6
    rows = []
    for case in cases:
        data = json.loads((ROOT / (case['name'] + '.json')).read_text())
        jobs = {job['jobname']: job for job in data['jobs']}
        expected = {'reader'} if case['write_cap_MB_s'] == 0 else {'reader', 'writer'}
        assert set(jobs) == expected and all(job['error'] == 0 for job in jobs.values())
        read = jobs['reader']['read']
        write = jobs.get('writer', {}).get('write', {})
        assert read['runtime'] >= 15000 and read['io_bytes'] > 0
        if write:
            assert write['runtime'] >= 15000 and write['io_bytes'] > 0
        samples = {4096: [], 1048576: []}
        logs = list(ROOT.glob(case['name'] + '_clat.*.log'))
        assert len(logs) == 1
        with logs[0].open() as source:
            for record in csv.reader(source):
                _, latency, direction, size = map(int, record[:4])
                assert direction == 0 and size in samples
                samples[size].append(latency)
        count = sum(map(len, samples.values()))
        assert count == read['clat_ns']['N']
        mean = sum(map(sum, samples.values())) / count
        assert math.isclose(mean, read['clat_ns']['mean'], rel_tol=0, abs_tol=0.001)
        # fio latency and bandwidth counters differ at ramp/drain boundaries.
        assert abs(count - read['total_ios']) <= 32
        logged_bytes = sum(size * len(values) for size, values in samples.items())
        assert abs(logged_bytes - read['io_bytes']) <= 32 * 1048576
        small_pct = 100 * len(samples[4096]) / count
        assert 49 < small_pct < 51, small_pct
        row = dict(case=case['name'], repeat=case['repeat'], write_cap_MB_s=case['write_cap_MB_s'],
                   read_MB_s=read['bw_bytes'] / 1e6, write_MB_s=write.get('bw_bytes', 0) / 1e6,
                   small_read_pct=small_pct)
        for size, label in [(4096, '4KiB'), (1048576, '1MiB')]:
            values = sorted(samples[size])
            row[f'{label}_count'] = len(values)
            row[f'{label}_mean_ms'] = sum(values) / len(values) / 1e6
            row[f'{label}_p99_ms'] = values[math.ceil(0.99 * len(values)) - 1] / 1e6
        rows.append(row)
    with (ROOT / 'summary.csv').open('w', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    report = ['# Mixed 4 KiB / 1 MiB random reads', '',
              'Measured **2026-09-26** on `m8g-32cpu-local-ssd` (`m8gd.8xlarge`).', '',
              '**50/50 by request count**, selected within one QD32 random reader',
              'using `bssplit=4k/50:1m/50`. This is approximately **99.61% large-read bytes**.',
              'A separate sequential writer uses 1 MiB requests at QD32.',
              'fio libaio, O_DIRECT, initialized 8 GiB read and 4 GiB write files.',
              'Two shuffled rounds; each case has **3 seconds warmup + 15 seconds measurement**.',
              'These are short screening runs, not sustained-performance estimates.', '',
              '| Round | Write cap MB/s | Read MB/s | Actual write MB/s | 4 KiB requests % | 4 KiB p99 ms | 1 MiB p99 ms |',
              '| ---: | --- | ---: | ---: | ---: | ---: | ---: |']
    for row in sorted(rows, key=lambda r: (r['repeat'], str(r['write_cap_MB_s']))):
        report.append(f"| {row['repeat']} | {row['write_cap_MB_s']} | {row['read_MB_s']:,.1f} | {row['write_MB_s']:,.1f} | {row['small_read_pct']:.2f} | {row['4KiB_p99_ms']:.3f} | {row['1MiB_p99_ms']:.3f} |")
    report += ['', 'Latency percentiles are nearest-rank p99 from individual fio completion-latency',
               'records, separated by actual request size. Sample counts and combined mean',
               'are checked against fio JSON completion-latency statistics. Latency and',
               'bandwidth counters differ slightly at ramp/drain boundaries (bounded by QD32).',
               'Throughput uses fio JSON bandwidth. All cases use the same per-I/O logging.', '',
               'See [methodology and interpretation](METHODOLOGY.md) and [CSV](summary.csv).', '']
    (ROOT / 'REPORT.md').write_text('\n'.join(report))


if __name__ == '__main__':
    main()
