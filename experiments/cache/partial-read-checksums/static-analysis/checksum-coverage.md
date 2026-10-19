# Geometry for two-checksum verification

Historical range-aware analysis. Full report:

`experiments/cache/partial-read-checksums/raw/static-analysis/checksum-coverage/SUMMARY.md`

**Finding: no implementation opportunity in this prefix.**

- Validated **1,606,645 requests / 2,800,015,397,526 requested bytes**.
- **Zero** straddling requests have two earlier contributing ranges plus a strictly larger original containing entry.
- Only **three** requests have two-range coverage: one has no prior covering entry, and two repeat exact whole entries already checksummed.
- For those two repeats, splitting verification increases aligned reads from **8 KiB to 12 KiB each**.
- **Zero opportunities above 8 MiB.** Ignoring chronology entirely still reveals no larger original parent.

Scripts explicitly model contained-entry replacement and separate observed geometry from conditional same-entry learning. Actual replay checksum availability remains unknown, but cannot supply the absent geometry.

Scripts, results, and validation logs are local in the same directory.