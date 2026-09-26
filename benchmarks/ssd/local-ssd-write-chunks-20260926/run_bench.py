#!/usr/bin/env python3
"""Compare write chunking at equal outstanding bytes with mixed-size reads."""
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import subprocess

OUT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    'previous', OUT.parent / 'local-ssd-random-read-write-20260925' / 'run_bench.py')
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
bench.OUT = OUT
bench.DATA = Path('/mnt/local-ssd') / OUT.name
bench.RAMP = 3


def main():
    if not __debug__:
        raise RuntimeError('do not disable safety checks with Python -O')
    assert subprocess.check_output(['hostname'], text=True).strip() == 'm8g-32cpu-local-ssd'
    assert not (OUT / 'metadata.json').exists()
    assert os.path.ismount('/mnt/local-ssd')
    source = subprocess.check_output(
        ['findmnt', '-n', '-o', 'SOURCE', '/mnt/local-ssd'], text=True).strip()
    model = Path('/sys/block/nvme1n1/device/model').read_text().strip()
    assert source == '/dev/nvme1n1' and 'Instance Storage' in model, (source, model)
    assert shutil.disk_usage('/mnt/local-ssd').free > 20 * 1024**3
    bench.DATA.mkdir(exist_ok=False)
    metadata = dict(start=bench.stamp(), runtime_seconds=15, ramp_seconds=3,
                    reader_qd=32, write_rate_cap=None,
                    bssplit='4k/50:1m/50', read_file_bytes=8*1024**3,
                    write_file_bytes=4*1024**3, shuffle_seed=20260926,
                    fio=subprocess.check_output(['fio', '--version'], text=True).strip())
    (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    cases = []
    rng = random.Random(20260926)
    for repeat in [1, 2]:
        settings = [('read-only', 0, 0), ('1MiB-qd1', 1048576, 1),
                    ('64KiB-qd16-batch', 65536, 16),
                    ('64KiB-qd16-rolling', 65536, 16), ('64KiB-qd1', 65536, 1)]
        rng.shuffle(settings)
        for setting, block, qd in settings:
            name = f'{setting}-repeat{repeat}'
            reader = bench.options('randread', 1048576)
            reader.update(size='8G', bssplit='4k/50:1m/50', blockalign=4096,
                          write_lat_log=str(OUT / name), log_avg_msec=0,
                          log_entries=262144)
            jobs = [('reader', reader)]
            if qd:
                writer = bench.options('write', block, qd)
                writer['size'] = '4G'
                if setting == '64KiB-qd16-batch':
                    writer.update(iodepth_batch_submit=16,
                                  iodepth_batch_complete_min=16,
                                  iodepth_batch_complete_max=16)
                jobs.append(('writer', writer))
            cases.append(dict(name=name, repeat=repeat, setting=setting,
                              write_block_bytes=block, write_qd=qd, jobs=jobs))
    (OUT / 'cases.json').write_text(json.dumps(cases, indent=2, default=str))
    hardware = subprocess.check_output(
        'hostname; uname -a; findmnt /mnt/local-ssd; df -B1 /mnt/local-ssd; '
        'grep . /sys/block/nvme1n1/queue/{scheduler,max_sectors_kb,nr_requests,write_cache}',
        shell=True, executable='/bin/bash', text=True)
    (OUT / 'hardware.txt').write_text(hardware)
    with (OUT / 'iostat.txt').open('w') as monitor:
        iostat = subprocess.Popen(['iostat', '-dxm', '-y', '1'], stdout=monitor)
        try:
            for kind, size in [('read', '8G'), ('write', '4G')]:
                opts = bench.options('read' if kind == 'read' else 'write', 1048576)
                opts.update(size=size, rw='write', end_fsync=1)
                bench.run_case(f'prepare-{kind}', [('prepare', opts)], prepare=True)
            for case in cases:
                bench.run_case(case['name'], case['jobs'], runtime=15)
            metadata['end'] = bench.stamp()
            (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
        finally:
            iostat.terminate()
            iostat.wait()
    for name in ('read.bin', 'write.bin'):
        (bench.DATA / name).unlink()
    bench.DATA.rmdir()
    (OUT / 'DONE').write_text(bench.stamp() + '\n')
    print(f'{bench.stamp()} DONE; benchmark data files removed', flush=True)


if __name__ == '__main__':
    main()
