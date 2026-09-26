#!/usr/bin/env python3
"""Random reads during sequential writes; derived from the 2026-09-19 fio runner."""
import datetime
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import time

OUT = Path(__file__).resolve().parent
DATA = Path('/mnt/local-ssd') / OUT.name
BLOCKS = [4096, 16384, 65536, 262144, 1048576, 4194304]
RUNTIME = 30
RAMP = 5


def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def run_case(name, jobs, *, prepare=False, runtime=RUNTIME):
    lines = ['[global]', 'ioengine=libaio', 'direct=1', 'thread=1',
             'iodepth=32', 'numjobs=1', 'invalidate=1', 'fallocate=none',
             'refill_buffers=1', 'randrepeat=0', 'norandommap=1',
             'exitall_on_error=1', 'percentile_list=50:95:99:99.9']
    if not prepare:
        lines += ['allow_file_create=0', 'time_based=1', f'runtime={runtime}',
                  f'ramp_time={RAMP}', 'log_avg_msec=1000',
                  f'write_bw_log={OUT / name}', 'per_job_logs=1']
    for jobname, options in jobs:
        lines += ['', f'[{jobname}]']
        lines += [f'{key}={value}' for key, value in options.items()]
    config = OUT / f'{name}.fio'
    config.write_text('\n'.join(lines) + '\n')
    print(f'{stamp()} START {name}', flush=True)
    start = time.monotonic()
    with (OUT / f'{name}.stderr').open('w') as err:
        subprocess.run(['fio', str(config), '--output-format=json',
                        f'--output={OUT / (name + ".json")}'],
                       stderr=err, check=True)
    result = json.loads((OUT / f'{name}.json').read_text())
    if len(result['jobs']) != len(jobs) or any(job['error'] for job in result['jobs']):
        raise RuntimeError(f'fio job error: {name}')
    for job, (_, opts) in zip(result['jobs'], jobs):
        direction = 'read' if opts['rw'] == 'randread' else 'write'
        stats = job[direction]
        if prepare:
            expected = int(str(opts['size']).removesuffix('G')) * 1024**3
            if stats['io_bytes'] != expected:
                raise RuntimeError(f'incomplete initialization: {name}')
        elif stats['runtime'] < runtime * 1000 or stats['io_bytes'] <= 0:
            raise RuntimeError(f'incomplete measurement: {name}')
    rates = [(job['jobname'], round(job['read']['bw_bytes'] / 1e6, 2),
              round(job['write']['bw_bytes'] / 1e6, 2)) for job in result['jobs']]
    print(f'{stamp()} END {name} {time.monotonic()-start:.1f}s (job, read MB/s, write MB/s)={rates}', flush=True)
    return result


def options(rw, bs, qd=32):
    read = rw in ('read', 'randread')
    return dict(filename=DATA / ('read.bin' if read else 'write.bin'),
                size='128G' if read else '64G', rw=rw, bs=bs, iodepth=qd)


def main():
    if not __debug__:
        raise RuntimeError('do not disable safety checks with Python -O')
    assert subprocess.check_output(['hostname'], text=True).strip() == 'm8g-32cpu-local-ssd'
    assert not (OUT / 'metadata.json').exists(), 'fresh output directory required'
    assert os.path.ismount('/mnt/local-ssd'), 'local SSD must be mounted'
    model = Path('/sys/block/nvme1n1/device/model').read_text().strip()
    source = subprocess.check_output(['findmnt', '-n', '-o', 'SOURCE', '/mnt/local-ssd'], text=True).strip()
    assert source == '/dev/nvme1n1' and 'Instance Storage' in model, (source, model)
    assert shutil.disk_usage('/mnt/local-ssd').free > 250 * 1024**3
    DATA.mkdir(exist_ok=False)
    metadata = dict(start=stamp(), data_dir=str(DATA), blocks=BLOCKS,
                    runtime_seconds=RUNTIME, ramp_seconds=RAMP,
                    read_pattern='randread', write_pattern='write',
                    reader_qd=32, writer_qd=32, writer_block_bytes=1048576,
                    read_file_bytes=128*1024**3, write_file_bytes=64*1024**3,
                    fio=subprocess.check_output(['fio', '--version'], text=True).strip())
    (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    hardware = subprocess.check_output('hostname; uname -a; lscpu; free -b; lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS,MODEL; findmnt /mnt/local-ssd; df -B1 /mnt/local-ssd; grep . /sys/block/nvme1n1/queue/{scheduler,max_sectors_kb,nr_requests,write_cache}', shell=True, executable='/bin/bash', text=True)
    (OUT / 'hardware.txt').write_text(hardware)
    with (OUT / 'iostat.txt').open('w') as monitor:
        iostat = subprocess.Popen(['iostat', '-dxm', '-y', '5'], stdout=monitor)
        try:
            for kind in ('read', 'write'):
                opts = options('read' if kind == 'read' else 'write', 1048576)
                opts.update(rw='write', end_fsync=1)
                run_case(f'prepare-{kind}', [('prepare', opts)], prepare=True)
            # Retain a standalone control, but pin caps to the original experiment.
            calibration = run_case('calibration-write', [('writer', options('write', 1048576))])
            capacity = calibration['jobs'][0]['write']['bw_bytes'] / 1e6
            rates = [0, 180, 450, 910, 1370, 1640, 'unlimited']
            metadata.update(calibration_write_MB_s=capacity, mixed_write_targets_MB_s=rates)
            (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
            cases = []
            for rate in rates:
                for bs in BLOCKS:
                    jobs = [('reader', options('randread', bs))]
                    if rate != 0:
                        writer = options('write', 1048576)
                        if rate != 'unlimited':
                            writer['rate'] = f'0,{rate * 1000000}'
                        jobs.append(('writer', writer))
                    cases.append((f'random-mixed-bs{bs}-rate{rate}', jobs))
            random.Random(20260919).shuffle(cases)
            (OUT / 'order.json').write_text(json.dumps([name for name, _ in cases], indent=2))
            for index, (name, jobs) in enumerate(cases, 1):
                print(f'CASE {index}/{len(cases)}', flush=True)
                run_case(name, jobs)
            for bs, rate in [(16384, 0), (16384, 450), (16384, 910), (65536, 910)]:
                name = f'random-mixed-bs{bs}-rate{rate}'
                jobs = next(jobs for case_name, jobs in cases if case_name == name)
                run_case(f'repeat-bs{bs}-rate{rate}', jobs, runtime=60)
            metadata['end'] = stamp()
            (OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2))
        finally:
            iostat.terminate()
            iostat.wait()
    # Delete only the two files created by this run. Keep all measurements on EBS.
    for name in ('read.bin', 'write.bin'):
        (DATA / name).unlink()
    DATA.rmdir()
    (OUT / 'DONE').write_text(stamp() + '\n')
    print(f'{stamp()} DONE; benchmark data files removed', flush=True)


if __name__ == '__main__':
    main()
