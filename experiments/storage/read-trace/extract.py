#!/usr/bin/env python3
"""Extract an ordered read-only window from the read-volume-duplicates event log."""

import csv
import sys

source, start_seconds, duration_seconds = sys.argv[1:]
start = int(start_seconds) * 1_000_000_000
end = start + int(duration_seconds) * 1_000_000_000
with open(source) as events:
    reads = sorted(
        (int(row["time_ns"]), int(row["address"]), int(row["end"]) - int(row["start"]))
        for row in csv.DictReader(events)
        if row["kind"] == "read_start" and start <= int(row["time_ns"]) < end
    )
writer = csv.writer(sys.stdout)
writer.writerow(("time_ns", "offset", "length"))
for time, offset, length in reads:
    writer.writerow((time - start, offset, length))
