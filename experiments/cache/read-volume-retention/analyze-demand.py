#!/usr/bin/env python3
"""Summarize demand geometry, not cache residency or measured SSD savings."""
import json
from pathlib import Path

import numpy as np

root = Path(__file__).resolve().parent
source = root.parent / 'partial-read-checksums/raw/static-analysis/promotion/requests.npy'
a = np.load(source)
assert len(a) == 1606645
sizes = a['end'] - a['start']
assert int(sizes.sum()) == 2800015397526
buckets = np.array([32768, 262144, 524288, 1048576, 2097152, 3145728, 4194304,
                    8388608, 12582912, 16777216, 25165824, 33554432, 67108864], dtype='u8')
charges = np.maximum(sizes, buckets[np.minimum(np.searchsorted(buckets, sizes), len(buckets)-1)])
keys = a[['lo', 'hi', 'start', 'end']]
_, first, counts = np.unique(keys, return_index=True, return_counts=True)
unique_sizes, unique_charges = sizes[first], charges[first]
rows = []
for lower, upper in [(0, 32768), (32768, 262144), (262144, 1048576),
                     (1048576, 8388608), (8388608, 67108864), (67108864, 2**63)]:
    demand = (sizes > lower) & (sizes <= upper)
    unique = (unique_sizes > lower) & (unique_sizes <= upper)
    rows.append(dict(lower_exclusive=lower, upper_inclusive=upper,
                     requests=int(demand.sum()), returned_bytes=int(sizes[demand].sum()),
                     unique_exact_ranges=int(unique.sum()),
                     unique_payload_bytes=int(unique_sizes[unique].sum()),
                     unique_fresh_capacity=int(unique_charges[unique].sum()),
                     repeated_ranges=int((unique & (counts > 1)).sum())))
result = dict(requests=len(a), returned_bytes=int(sizes.sum()), unique_exact_ranges=len(first),
              unique_payload_bytes=int(unique_sizes.sum()),
              unique_fresh_capacity=int(unique_charges.sum()), bins=rows,
              scope='Every distinct exact range counted once, not simultaneous resident entries. Overlaps are separate. Fresh pool capacity excludes alignment and large-bucket reuse.')
(root / 'raw/demand-geometry.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
