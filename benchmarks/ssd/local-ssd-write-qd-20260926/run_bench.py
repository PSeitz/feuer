#!/usr/bin/env python3
"""Sweep writer QD, retaining the previous random-read contention workload."""
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import subprocess

OUT = Path(__file__).resolve().parent
# Reuse fio generation, execution checks, and file sizing from the prior sweep.
spec = importlib.util.spec_from_file_location(
    'previous', OUT.parent / 'local-ssd-random-read-write-20260925' / 'run_bench.py')
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
bench.OUT = OUT
bench.DATA = Path('/mnt/local-ssd') / OUT.name
BLOCKS = [4096, 16384, 32768, 65536, 1048576]
DEPTHS = [1, 2, 4, 8, 16, 32]


def mixed(bs, qd, rate=None):
    jobs = [('reader', bench.options('randread', bs))]
    if qd:
        writer = bench.options('write', 1048576, qd)
        if rate is not None:
            writer['rate'] = f'0,{rate * 1000000}'
        jobs.append(('writer', writer))
    return jobs


def main():
    if not __debug__:
        raise RuntimeError('do not disable safety checks with Python -O')
    assert subprocess.check_output(['hostname'], text=True).strip() == 'm8g-32cpu-local-ssd'
    assert not (OUT / 'metadata.json').exists(), 'fresh output directory required'
    assert os.path.ismount('/mnt/local-ssd')
    source = subprocess.check_output(
        ['findmnt', '-n', '-o', 'SOURCE', '/mnt/local-ssd'], text=True).strip()
    model = Path('/sys/block/nvme1n1/device/model').read_text().strip()
    assert source == '/dev/nvme1n1' and 'Instance Storage' in model, (source, model)
    assert shutil.disk_usage('/mnt/local-ssd').free > 250 * 1024**3
    bench.DATA.mkdir(exist_ok=False)
    cases = []
    for bs in BLOCKS:
        for qd in [0] + DEPTHS:
            cases.append(dict(name=f'mixed-bs{bs}-qd{qd}', kind='mixed',
                              read_bytes=bs, write_qd=qd, write_cap_MB_s=None,
                              runtime=30, jobs=mixed(bs, qd)))
    for qd in DEPTHS:
        cases.append(dict(name=f'write-only-qd{qd}', kind='write-only',
                          read_bytes=0, write_qd=qd, write_cap_MB_s=None,
                          runtime=30,
                          jobs=[('writer', bench.options('write', 1048576, qd))]))
    for bs, rate in [(16384, 180), (16384, 450), (65536, 450)]:
        cases.append(dict(name=f'rate-bs{bs}-cap{rate}', kind='rate',
                          read_bytes=bs, write_qd=32, write_cap_MB_s=rate,
                          runtime=30, jobs=mixed(bs, 32, rate)))
    random.Random(20260926).shuffle(cases)
    for bs, qd in [(16384, 0), (16384, 1), (16384, 4), (65536, 4)]:
        cases.append(dict(name=f'repeat-bs{bs}-qd{qd}', kind='repeat',
                          read_bytes=bs, write_qd=qd, write_cap_MB_s=None,
                          runtime=60, jobs=mixed(bs, qd)))
    metadata = dict(start=bench.stamp(), data_dir=str(bench.DATA), blocks=BLOCKS,
                    writer_depths=DEPTHS, reader_qd=32, writer_block_bytes=1048576,
                    ioengine='libaio', ramp_seconds=5, shuffle_seed=20260926,
                    read_file_bytes=128*1024**3, write_file_bytes=64*1024**3,
                    fio=subprocess.check_output(['fio', '--version'], text=True).strip())
    (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    (OUT / 'cases.json').write_text(json.dumps(cases, indent=2, default=str))
    hardware = subprocess.check_output(
        'hostname; uname -a; lscpu; free -b; '
        'lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS,MODEL; '
        'findmnt /mnt/local-ssd; df -B1 /mnt/local-ssd; '
        'grep . /sys/block/nvme1n1/queue/{scheduler,max_sectors_kb,nr_requests,write_cache}',
        shell=True, executable='/bin/bash', text=True)
    (OUT / 'hardware.txt').write_text(hardware)
    with (OUT / 'iostat.txt').open('w') as monitor:
        iostat = subprocess.Popen(['iostat', '-dxm', '-y', '5'], stdout=monitor)
        try:
            for kind in ('read', 'write'):
                opts = bench.options('read' if kind == 'read' else 'write', 1048576)
                opts.update(rw='write', end_fsync=1)
                bench.run_case(f'prepare-{kind}', [('prepare', opts)], prepare=True)
            for index, case in enumerate(cases, 1):
                print(f'CASE {index}/{len(cases)}', flush=True)
                bench.run_case(case['name'], case['jobs'], runtime=case['runtime'])
            metadata['end'] = bench.stamp()
            (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
        finally:
            iostat.terminate()
            iostat.wait()
    # Remove only files created by this run; retain data and logs on failure.
    for name in ('read.bin', 'write.bin'):
        (bench.DATA / name).unlink()
    bench.DATA.rmdir()
    (OUT / 'DONE').write_text(bench.stamp() + '\n')
    print(f'{bench.stamp()} DONE; benchmark data files removed', flush=True)


if __name__ == '__main__':
    main()
