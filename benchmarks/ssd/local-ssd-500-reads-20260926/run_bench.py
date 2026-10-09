#!/usr/bin/env python3
"""Measure actual completion time of 500-read batches, not a throughput estimate."""
import csv
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import statistics
import subprocess
import time

OUT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    'previous', OUT.parent / 'local-ssd-random-read-write-20260925' / 'run_bench.py')
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
bench.OUT = OUT
bench.DATA = Path('/mnt/local-ssd') / OUT.name


def main():
    if not __debug__:
        raise RuntimeError('do not disable safety checks')
    assert subprocess.check_output(['hostname'], text=True).strip() == 'm8g-32cpu-local-ssd'
    assert not (OUT / 'metadata.json').exists()
    assert os.path.ismount('/mnt/local-ssd')
    source = subprocess.check_output(['findmnt', '-n', '-o', 'SOURCE', '/mnt/local-ssd'], text=True).strip()
    model = Path('/sys/block/nvme1n1/device/model').read_text().strip()
    assert source == '/dev/nvme1n1' and 'Instance Storage' in model
    assert shutil.disk_usage('/mnt/local-ssd').free > 20 * 1024**3
    bench.DATA.mkdir(exist_ok=False)
    metadata = dict(start=bench.stamp(), reads_per_batch=500, read_bytes=4096,
                    measured_batches_per_case=1000, warmup_batches_per_case=100,
                    read_file_bytes=8*1024**3, write_file_bytes=4*1024**3,
                    kernel=subprocess.check_output(['uname', '-a'], text=True).strip(),
                    fio=subprocess.check_output(['fio', '--version'], text=True).strip())
    (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', '-o', str(OUT / 'batch_reads'),
                    str(OUT / 'batch_reads.c')], check=True)
    for kind, size in [('read', '8G'), ('write', '4G')]:
        opts = bench.options('read' if kind == 'read' else 'write', 1048576)
        opts.update(size=size, rw='write', end_fsync=1)
        bench.run_case(f'prepare-{kind}', [('prepare', opts)], prepare=True)
    writer_config = OUT / 'writer.fio'
    writer_config.write_text(f'''[writer]
ioengine=libaio
direct=1
thread=1
filename={bench.DATA / 'write.bin'}
size=4G
allow_file_create=0
rw=write
bs=1M
iodepth=32
refill_buffers=1
time_based=1
runtime=120
write_bw_log={OUT / 'writer'}
log_avg_msec=500
''')
    summaries = []
    # Reverse QD order in round 2. Compare both states in each round.
    for repeat in [1, 2]:
        for background in [False, True]:
            writer = None
            if background:
                writer = subprocess.Popen(['fio', str(writer_config), '--output-format=json',
                                           f'--output={OUT / f"writer-{repeat}.json"}'],
                                          stdout=subprocess.DEVNULL)
                time.sleep(3)
                assert writer.poll() is None
            try:
                for qd in ([32, 64] if repeat == 1 else [64, 32]):
                    name = f'batch-qd{qd}-writes{int(background)}-repeat{repeat}'
                    print(f'{bench.stamp()} START {name}', flush=True)
                    start = bench.stamp()
                    with (OUT / f'{name}.csv').open('w') as output:
                        subprocess.run([str(OUT / 'batch_reads'), str(bench.DATA / 'read.bin'), str(qd)],
                                       stdout=output, check=True, timeout=30)
                    if writer:
                        assert writer.poll() is None, 'writer stopped during measurement'
                    with (OUT / f'{name}.csv').open() as source:
                        records = list(csv.DictReader(source))
                    assert [int(r['batch']) for r in records] == list(range(1000))
                    samples = sorted(float(r['latency_ms']) for r in records)
                    assert samples[0] > 0
                    row = dict(case=name, repeat=repeat, qd=qd, background_writes=background,
                               start=start, end=bench.stamp(), batches=len(samples),
                               mean_ms=statistics.mean(samples),
                               p50_ms=samples[math.ceil(.5*len(samples))-1],
                               p95_ms=samples[math.ceil(.95*len(samples))-1],
                               p99_ms=samples[math.ceil(.99*len(samples))-1], max_ms=samples[-1])
                    summaries.append(row)
                    print(json.dumps(row), flush=True)
            finally:
                if writer:
                    writer.send_signal(signal.SIGINT)
                    writer.wait(timeout=10)
            if background:
                text = (OUT / f'writer-{repeat}.json').read_text()
                # fio prefixes its JSON with a notice when stopped with SIGINT.
                prefix, brace, body = text.partition('{')
                assert prefix.strip() in ('', 'fio: terminating on signal 2')
                result = json.loads(brace + body)
                assert len(result['jobs']) == 1 and result['jobs'][0]['error'] == 0
                assert result['jobs'][0]['write']['io_bytes'] > 0
                for log in OUT.glob('writer_bw.*.log'):
                    log.rename(log.with_name(log.name.replace('writer_', f'writer-{repeat}_', 1)))
    with (OUT / 'summary.csv').open('w', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(summaries[0]))
        writer.writeheader()
        writer.writerows(summaries)
    metadata['end'] = bench.stamp()
    (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    for name in ('read.bin', 'write.bin'):
        (bench.DATA / name).unlink()
    bench.DATA.rmdir()
    (OUT / 'DONE').write_text(bench.stamp() + '\n')


if __name__ == '__main__':
    main()
