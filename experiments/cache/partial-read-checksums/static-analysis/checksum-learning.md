# Geometry for learning observed-range checksums

Historical range-aware analysis. Full report and local reproduction scripts:

`experiments/cache/partial-read-checksums/raw/static-analysis/checksum-learning/REPORT.md`

**Findings**
- Validated the exact 600-second prefix: **1,606,645 requests / 2,800,015,397,526 bytes**.
- For large covering candidates, source-start history offers **843 writer seeds across 189 entries**, requiring **2.531 MB hashing / approximately 27 kB metadata**. Of these, **825 are subsequently requested**.
- First-later-lookup verified-read learning with an assumed 100 ms completion offers only **78 seeds across 15 large entries**.
- Delaying the assumed read opportunity by 60 seconds produces 1,454 seeds, but **none are new if preceding requests already learned their checksums**.
- Every modeled improvement concerns previously observed ranges. Benefits occur at the **first subsequent disk use**, not on every repeated request.
- Read-sharing sensitivity materially reduces conditional byte savings and can increase read-call counts. These figures are not realized replay savings.

**Assessment:** evidence favors writer learning, especially when large entries replace smaller ranges. Actual savings remain unresolved because publication timing, checksum state, memory hits, and read membership are unavailable.
