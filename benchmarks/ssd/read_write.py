#!/usr/bin/env python3
"""Measure native SSD read capacity for a size histogram alongside 1-MiB writes."""

import argparse
from bisect import bisect_left
from collections import Counter
import csv
import hashlib
from itertools import accumulate
import json
import math
from pathlib import Path
import random
import shutil
import subprocess
import tempfile
import time

MIB = 1024**2
READ_BYTES = 64 * 1024**3
WRITE_BYTES = 8 * 1024**3
CASES = [('read-only', None), ('writes-capped', 450_000_000), ('writes-uncapped', 0)]
SIZE_LIMITS = [4096, 65536, MIB, 8 * MIB]
SIZE_LABELS = ['4 KiB', '>4–64 KiB', '>64 KiB–1 MiB', '>1–8 MiB', '>8 MiB']


def read_histogram(path):
    counts = Counter()
    with path.open() as f:
        for row in csv.DictReader(f):
            size, count = int(row['size_bytes']), int(row['count'])
            if size <= 0 or size > READ_BYTES or count <= 0:
                raise ValueError('Sizes must be in 1..64 GiB and counts must be positive')
            counts[((size + 4095) // 4096) * 4096] += count
    if not counts:
        raise ValueError('Histogram must contain at least one size')
    return dict(sorted(counts.items()))


def write_iolog(path, filename, histogram, requests):
    rng = random.Random(20261102)
    sizes = rng.choices(list(histogram), cum_weights=list(accumulate(histogram.values())), k=requests)
    with path.open('w') as f:
        f.write(f'fio version 2 iolog\n{filename} add\n{filename} open\n')
        for size in sizes:
            offset = rng.randrange((READ_BYTES - size) // 4096 + 1) * 4096
            f.write(f'{filename} read {offset} {size}\n')
        f.write(f'{filename} close\n')
    return Counter(sizes)


def job_config(out, data, case, rate, seconds, warmup):
    config = f'''[global]
ioengine=io_uring
direct=1
thread=1
time_based=1
runtime={seconds}
ramp_time={warmup}
iodepth_batch_submit=1
iodepth_batch_complete_min=1
iodepth_batch_complete_max=1
lat_percentiles=1
percentile_list=50:95:99

[reads]
read_iolog={out}/reads.iolog
replay_no_stall=1
iodepth=32
write_lat_log={out}/{case}-reads
log_avg_msec=0
log_entries=1048576
'''
    if rate is not None:
        config += f'''
[writes]
filename={data}/write.data
size={WRITE_BYTES}
rw=write
bs=1M
iodepth=8
'''
        if rate:
            config += f'rate={rate}\n'
    return config


def latency(values):
    values = sorted(values)
    return {'mean_ms': sum(values) / len(values) / 1e6,
            **{f'p{p}_ms': values[math.ceil((len(values) - 1) * p / 100)] / 1e6 for p in [50, 95, 99]}}


def summarize(out):
    results = []
    for case, _ in CASES:
        jobs = json.loads((out / f'{case}.json').read_text())['jobs']
        read = next(j['read'] for j in jobs if j['jobname'] == 'reads')
        groups = [[] for _ in SIZE_LABELS]
        byte_counts = [0] * len(groups)
        log, = out.glob(f'{case}-reads_lat.*.log')
        with log.open() as f:
            for row in csv.reader(f):
                _, ns, direction, size = map(int, row[:4])
                assert direction == 0
                group = bisect_left(SIZE_LIMITS, size)
                groups[group].append(ns)
                byte_counts[group] += size
        values = [ns for group in groups for ns in group]
        assert len(values) == read['lat_ns']['N']
        assert math.isclose(sum(values) / len(values), read['lat_ns']['mean'])
        writer = next((j['write'] for j in jobs if j['jobname'] == 'writes'), None)
        result = dict(case=case, read_MB_s=read['bw_bytes'] / 1e6, read_iops=read['iops'],
                      write_MB_s=writer['bw_bytes'] / 1e6 if writer else 0,
                      write_iops=writer['iops'] if writer else 0,
                      cpu_percent=sum(j['usr_cpu'] + j['sys_cpu'] for j in jobs),
                      read_latency_samples=len(values), read_ios=read['total_ios'],
                      read_bytes=read['io_bytes'], read_logged_bytes=sum(byte_counts),
                      **latency(values))
        result['sizes'] = [dict(size=label, count=len(group),
                                operation_percent=100 * len(group) / len(values),
                                byte_percent=100 * size_bytes / sum(byte_counts), **latency(group))
                           for label, group, size_bytes in zip(SIZE_LABELS, groups, byte_counts) if group]
        results.append(result)
    (out / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
    lines = ['# SSD size-mix results', '',
             '| Case | Read MB/s | Read IOPS | Write MB/s | Read mean ms | Read p99 ms | CPU % |',
             '|---|---:|---:|---:|---:|---:|---:|']
    for r in results:
        lines.append(f"| {r['case']} | {r['read_MB_s']:.1f} | {r['read_iops']:.0f} | {r['write_MB_s']:.1f} | "
                     f"{r['mean_ms']:.3f} | {r['p99_ms']:.3f} | {r['cpu_percent']:.1f} |")
    for r in results:
        lines += ['', f"## {r['case']}: aligned read sizes", '',
                  '| Size | Samples | Operations % | Bytes % | Mean ms | p50 ms | p99 ms |',
                  '|---|---:|---:|---:|---:|---:|---:|---:|']
        for s in r['sizes']:
            lines.append(f"| {s['size']} | {s['count']} | {s['operation_percent']:.2f} | {s['byte_percent']:.2f} | "
                         f"{s['mean_ms']:.3f} | {s['p50_ms']:.3f} | {s['p99_ms']:.3f} |")
    (out / 'RESULT.md').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('histogram', type=Path, help='CSV columns: size_bytes,count (operation counts)')
    parser.add_argument('--directory', type=Path, required=True, help='Existing SSD directory for temporary data')
    parser.add_argument('--output', type=Path, required=True, help='Fresh experiments/ directory on another device')
    parser.add_argument('--requests', type=int, default=1_000_000, help='Generated reads, must cover warmup and measurement')
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--warmup', type=int, default=10)
    args = parser.parse_args()
    if args.requests <= 0 or args.seconds <= 0 or not 0 <= args.warmup < args.seconds:
        parser.error('Requests and seconds must be positive, warmup must be in 0..seconds-1')
    out, directory = args.output.resolve(), args.directory.resolve()
    if any(c.isspace() or c == ':' for c in str(out) + str(directory)):
        parser.error('fio paths must not contain whitespace or colons')
    for tool in ['fio', 'iostat', 'pidstat', '/usr/bin/time']:
        if not shutil.which(tool):
            parser.error(f'Missing prerequisite: {tool}')
    histogram = read_histogram(args.histogram)
    if shutil.disk_usage(directory).free < READ_BYTES + WRITE_BYTES + 8 * 1024**3:
        parser.error('SSD needs at least 80 GiB free')
    out.mkdir(parents=True)
    shutil.copyfile(args.histogram, out / 'histogram.csv')
    (out / 'settings.json').write_text(json.dumps({**vars(args), 'seed': 20261102}, default=str, indent=2) + '\n')
    shutil.copyfile(__file__, out / 'read_write.py')
    commands = []

    def run(command, name):
        commands.append(command)
        (out / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        with (out / f'{name}.stdout').open('w') as stdout, (out / f'{name}.stderr').open('w') as stderr:
            subprocess.run(command, check=True, stdout=stdout, stderr=stderr)

    run(['bash', '-c', 'date -u; uname -a; python3 --version; fio --version; lscpu; lsblk -o NAME,MODEL,SIZE,MOUNTPOINTS; '
         'df -h; free -h; ps -eo pid,comm,args'], 'environment')
    run(['iostat', '-dx', '-y', '1', '5'], 'idle')
    with tempfile.TemporaryDirectory(prefix='feuer-read-write-', dir=directory) as temporary:
        data = Path(temporary)
        sampled = write_iolog(out / 'reads.iolog', data / 'read.data', histogram, args.requests)
        with (out / 'sampled-sizes.csv').open('w') as f:
            writer = csv.writer(f)
            writer.writerow(['size_bytes', 'count'])
            writer.writerows(sorted(sampled.items()))
        digest = hashlib.sha256()
        with (out / 'reads.iolog').open('rb') as f:
            for chunk in iter(lambda: f.read(1024 * 1024), b''):
                digest.update(chunk)
        (out / 'reads.sha256').write_text(digest.hexdigest() + '\n')
        for case, rate in CASES:
            job = out / f'{case}.fio'
            job.write_text(job_config(out, data, case, rate, args.seconds, args.warmup))
            run(['fio', '--parse-only', str(job)], f'{case}-parse')
        for name, size in [('read', READ_BYTES), ('write', WRITE_BYTES)]:
            print(f'Initializing {name} file', flush=True)
            run(['fio', '--name=initialize', f'--filename={data}/{name}.data', f'--size={size}',
                 '--rw=write', '--bs=1M', '--ioengine=io_uring', '--direct=1', '--iodepth=8',
                 '--end_fsync=1', '--buffer_pattern=0x5a', '--output-format=json',
                 f'--output={out}/initialize-{name}.json'], f'initialize-{name}')
            assert all(j['error'] == 0 for j in json.loads((out / f'initialize-{name}.json').read_text())['jobs'])
        for case, _ in CASES:
            print(f'Running {case}', flush=True)
            time.sleep(5)
            monitors = []
            handles = []
            try:
                for label, command in [('iostat', ['iostat', '-dx', '-y', '1']),
                                       ('pidstat', ['pidstat', '-u', '-r', '-d', '-h', '-C', 'fio', '1'])]:
                    handle = (out / f'{case}.{label}').open('w')
                    handles.append(handle)
                    monitors.append(subprocess.Popen(command, stdout=handle, stderr=subprocess.STDOUT))
                run(['/usr/bin/time', '-v', '-o', str(out / f'{case}.time'), 'fio', str(out / f'{case}.fio'),
                     '--output-format=json', f'--output={out}/{case}.json'], case)
            finally:
                for monitor in monitors:
                    monitor.terminate()
                    monitor.wait()
                for handle in handles:
                    handle.close()
            jobs = json.loads((out / f'{case}.json').read_text())['jobs']
            assert all(job['error'] == 0 for job in jobs)
            read = next(job['read'] for job in jobs if job['jobname'] == 'reads')
            if read['runtime'] < args.seconds * 1000 or read['total_ios'] == 0:
                raise RuntimeError('Read input exhausted before measurement finished. Increase --requests.')
        summarize(out)
    (out / 'DONE').write_text('Three cases completed once each. Temporary data removed.\n')


if __name__ == '__main__':
    main()
