#!/usr/bin/env python3
"""Run only on the idle local-SSD benchmark host. Outputs are never overwritten."""
import json
import random
import shutil
import subprocess
from pathlib import Path

BASE = Path(__file__).resolve().parent
OUT = BASE / 'results'
DATA = Path('/mnt/local-ssd/feuer-uring-20260928')
BINARY = BASE / 'target/release/feuer-uring-bench'

def run(args, output):
    with output.open('x') as f:
        subprocess.run([str(a) for a in args], stdout=f, stderr=subprocess.STDOUT, check=True)

def fio(name, extra):
    output = OUT / (name + '.json')
    run(['fio', '--name=' + name, '--filename=' + str(DATA / 'data'),
         '--ioengine=io_uring', '--direct=1', '--thread=1', '--output-format=json',
         '--fallocate=none', '--group_reporting=1', '--exitall_on_error=1'] + extra, output)
    result = json.loads(output.read_text())
    assert all(job['error'] == 0 for job in result['jobs']), result
    return result

def main():
    assert subprocess.check_output(['hostname'], text=True).strip() == 'm8g-32cpu-local-ssd'
    assert subprocess.check_output(['findmnt', '-n', '-o', 'SOURCE', '/mnt/local-ssd'], text=True).strip() == '/dev/nvme1n1'
    assert 'Instance Storage' in subprocess.check_output(['lsblk', '-dn', '-o', 'MODEL', '/dev/nvme1n1'], text=True)
    assert shutil.disk_usage('/mnt/local-ssd').free > 30 * 1024**3
    processes = subprocess.check_output(['ps', '-eo', 'comm,args'], text=True)
    assert not any(('benchmark.sh' in p or p.startswith('fio ') or p.startswith('quickwit ')) for p in processes.splitlines())
    OUT.mkdir()
    DATA.mkdir()
    run(['bash', '-c', 'date -u; uname -a; lscpu; findmnt /mnt/local-ssd; df -h /mnt/local-ssd; ~/.cargo/bin/rustc -Vv; fio --version'], OUT / 'host.txt')
    with (OUT / 'iostat.txt').open('x') as log:
        monitor = subprocess.Popen(['iostat', '-dxm', '-y', '2', 'nvme1n1'], stdout=log)
        try:
            init = fio('initialize', ['--rw=write', '--bs=1m', '--iodepth=32', '--size=24g', '--end_fsync=1', '--refill_buffers=1'])
            assert init['jobs'][0]['write']['io_bytes'] == 24 * 1024**3
            run([BINARY, 'smoke', DATA / 'data'], OUT / 'smoke.txt')
            cases = [(s, q, 0) for s in (4096, 65536, 1048576) for q in (1, 8, 32, 64)]
            cases += [(1048576, 0, q) for q in (1, 8, 32, 64)]
            cases += [(s, 32, q) for s in (4096, 1048576) for q in (1, 8, 64)]
            cases += [(4096, 128, 0)]
            rng = random.Random(20260928)
            for repeat in range(3):
                rng.shuffle(cases)
                for size, rq, wq in cases:
                    name = f'r{repeat}-bs{size}-rq{rq}-wq{wq}'
                    print(name, flush=True)
                    # Adjacent controls each repeat; fio measures completion latency, not API latency.
                    if (size, rq, wq) in [(4096, 1, 0), (4096, 64, 0), (1048576, 32, 0), (1048576, 0, 32)]:
                        writing = wq > 0
                        fio('fio-' + name, ['--rw=' + ('write' if writing else 'randread'),
                            '--bs=' + str(size), '--iodepth=' + str(wq or rq),
                            '--offset=' + str(16 * 1024**3 if writing else 0),
                            '--size=' + str((8 if writing else 16) * 1024**3),
                            '--allow_file_create=0', '--time_based=1', '--runtime=20', '--ramp_time=5',
                            '--norandommap=1', '--randrepeat=1', '--nonvectored=1', '--end_fsync=1'])
                    run([BINARY, 'run', DATA / 'data', size, rq, wq], OUT / (name + '.json'))
                    result = json.loads((OUT / (name + '.json')).read_text())['result']
                    assert not rq or result['read']['completed'] > 0
                    assert not wq or result['write']['completed'] > 0
            (OUT / 'DONE').write_text('All 69 Rust cases and 12 fio controls passed.\n')
        finally:
            monitor.terminate()
            monitor.wait()
    (DATA / 'data').unlink()
    DATA.rmdir()

if __name__ == '__main__':
    main()
