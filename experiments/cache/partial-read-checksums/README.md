# Partial-read checksums

Question: does learning requested-range checksums after verified disk reads reduce
workload-level I/O compared with whole-entry verification?

The [matched 10-minute replay](REPLAY.md) measured 1.53% fewer DataFile read bytes
in one baseline/candidate pair, with slightly more CPU and mixed latency results.
It does not establish a small performance improvement or isolate checksum reuse
from scheduling-dependent admission and tier selection.

- [Replay settings and artifact hashes](replay-settings.json)
- [Reproduction procedure](reproduce/README.md) and the exact tested patch

The [offline trace analyses](static-analysis/README.md) evaluate the range geometry
needed by five proposals. Their conditional opportunities and negative findings
are not measured savings from the replayed patch.

Routine [implementation validation](raw/IMPLEMENTATION-VALIDATION.md) is archived
under ignored `raw/`, along with raw captures, full source snapshots, and validation
logs. Those links require the retained local archive.
