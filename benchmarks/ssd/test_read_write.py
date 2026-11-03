import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import read_write as bench


class ReadWriteTests(unittest.TestCase):
    def test_histogram_and_jobs(self):
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary)
            histogram = out / 'sizes.csv'
            histogram.write_text('size_bytes,count\n2,2\n4096,3\n4097,4\n1048576,1\n')
            counts = bench.read_histogram(histogram)
            self.assertEqual(counts, {4096: 5, 8192: 4, 1048576: 1})
            path = out / 'reads.iolog'
            sampled = bench.write_iolog(path, out / 'read.data', counts, 100)
            original = path.read_text()
            self.assertEqual(sum(sampled.values()), 100)
            self.assertEqual(bench.write_iolog(path, out / 'read.data', counts, 100), sampled)
            self.assertEqual(path.read_text(), original)
            for line in original.splitlines()[3:-1]:
                _, operation, offset, size = line.split()
                self.assertEqual(operation, 'read')
                self.assertIn(int(size), counts)
                self.assertEqual(int(offset) % 4096, 0)
                self.assertLessEqual(int(offset) + int(size), bench.READ_BYTES)
            for case, rate in bench.CASES:
                config = bench.job_config(out, out, case, rate, 60, 10)
                self.assertIn('iodepth=32\n', config)
                self.assertEqual('[writes]' in config, rate is not None)
                if rate is not None:
                    self.assertIn('bs=1M\niodepth=8\n', config)
                self.assertEqual('rate=' in config, bool(rate))
            histogram.write_text('size_bytes,count\n4096,0\n')
            with self.assertRaises(ValueError):
                bench.read_histogram(histogram)

    def test_latency_accounting_and_size_shares(self):
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary)
            for case, _ in bench.CASES:
                # Throughput and latency accounting deliberately have different counts.
                job = {'jobname': 'reads', 'usr_cpu': 2, 'sys_cpu': 3,
                       'read': {'bw_bytes': 1000000, 'iops': 2, 'io_bytes': 2097152, 'total_ios': 2,
                                'lat_ns': {'N': 3, 'mean': 13000000 / 3}}}
                (out / f'{case}.json').write_text(json.dumps({'jobs': [job]}))
                (out / f'{case}-reads_lat.1.log').write_text(
                    '0, 1000000, 0, 4096\n1, 9000000, 0, 2097152\n2, 3000000, 0, 4096\n')
            with contextlib.redirect_stdout(io.StringIO()):
                bench.summarize(out)
            result = json.loads((out / 'summary.json').read_text())[0]
            self.assertEqual(result['read_ios'], 2)
            self.assertEqual(result['read_latency_samples'], 3)
            self.assertEqual(result['p99_ms'], 9)
            self.assertEqual(result['cpu_percent'], 5)
            small, large = result['sizes']
            self.assertEqual(small['count'], 2)
            self.assertEqual(small['p99_ms'], 3)
            self.assertEqual(large['count'], 1)
            self.assertAlmostEqual(small['byte_percent'] + large['byte_percent'], 100)
            self.assertIn('read-only', (out / 'RESULT.md').read_text())
