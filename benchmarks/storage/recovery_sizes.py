"""Aggregate recorded source-download sizes without retaining identifiers or event order.

Usage: python3 benchmarks/storage/recovery_sizes.py PATH/source-sizes.bin > benchmarks/storage/recovery-sizes.csv
Input: little-endian u64 payload lengths from replay-feuer's buffer-sizes/analyze.py.
"""

from collections import defaultdict
from pathlib import Path
import struct
import sys

CHUNK_BYTES = 1024 * 1024
PAGE_BYTES = 4096
sizes = defaultdict(lambda: [0, 0])
for (length,) in struct.iter_unpack("<Q", Path(sys.argv[1]).read_bytes()):
    aligned = (max(1, length) + PAGE_BYTES - 1) // PAGE_BYTES * PAGE_BYTES
    # Keep subchunk alignment sizes distinct. Larger downloads have the same whole-chunk count.
    key = (0, aligned) if aligned < CHUNK_BYTES else (1, (aligned + CHUNK_BYTES - 1) // CHUNK_BYTES)
    sizes[key][0] += length
    sizes[key][1] += 1

print("size_bytes,count")
for total, count in (sizes[key] for key in sorted(sizes)):
    print(f"{total // count},{count}")
