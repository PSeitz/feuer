# Geometry for sharing active partial reads

Historical range-aware analysis. Full report and local reproduction scripts:

`experiments/cache/partial-read-checksums/raw/static-analysis/read-sharing/REPORT.md`

**Finding: sharing a larger active partial read has no opportunity in this prefix under the verified exact-range-source contract.**

- Validated **1,606,645 requests / 2,800,015,397,526 bytes**.
- Across **123,885 distinct object/ranges**, found **6,468 strict-containment relationships**, but **zero three-level chains**: entry E containing partial checksum L containing smaller request R. That chain is necessary for the proposed selection improvement.
- The largest strictly contained observed range is **293,197 bytes**. Therefore **>8-MiB reads cannot be learned partial reads here**. They use whole-entry checksums, whose active reads are already preferred.
- **Zero strict nested-request pairs within 0.1, 1, 10, or 100 ms**. At 1 second: **104 later requests / 1,292,975 requested bytes**, none beneath >8-MiB covering requests.
- Exact repeats are much more common, but are not this proposal’s opportunity. At 100 ms: **8,899 requests / 1.698 GB**, potentially memory hits or sharing already supported.
- Saved telemetry confirms **89.907% memory hits** and **3,647 reads >8 MiB accounting for at least 78.871% of read bytes**. Neither converts request proximity into new partial-read savings.

The report includes timing sensitivity, history prerequisites, anonymized examples, telemetry constraints, and the structural exclusion proof—including cases where the active read exceeds its initiating request.
