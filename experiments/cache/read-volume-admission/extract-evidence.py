"""Extract opening counters and replay-phase I/O from retained phase snapshots."""
import json
from pathlib import Path

experiment = Path(__file__).resolve().parent
source = experiment.parent / "partial-read-checksums/raw/replay/results"
keys = [
    'feuer_disk_io_bytes_total{operation="read"}',
    'feuer_disk_io_total{operation="read",outcome="success"}',
    'feuer_disk_io_bytes_total{operation="write"}',
    'feuer_disk_io_total{operation="write",outcome="success"}',
]


def counters(path):
    return {parts[0]: int(parts[1]) for line in path.read_text().splitlines()
            if len(parts := line.split()) >= 2 and parts[0] in keys}


result = {}
for case in ["baseline", "candidate"]:
    before = counters(source / case / "before.prom")
    settled = counters(source / case / "settled.prom")
    result[case] = {"before": before,
                    "phase_delta": {key: settled[key] - before[key] for key in keys}}
output = experiment / "raw"
output.mkdir(exist_ok=True)
(output / "phase-io.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
