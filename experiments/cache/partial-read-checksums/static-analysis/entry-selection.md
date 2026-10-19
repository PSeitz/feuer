# Geometry for choosing a cheaper covering entry

Historical range-aware analysis. Full report:

`experiments/cache/partial-read-checksums/raw/static-analysis/entry-selection/REPORT.md`

**Finding: this trace offers no cheaper covering-entry selection.**

- Validated **1,606,645 requests and 2,800,015,397,526 requested bytes** in the first 600 seconds.
- **Zero requests** have two non-containing historical covering ranges. An independent check allowing ranges from anywhere in the prefix also found zero.
- Only **45 distinct partial-overlap pairs** exist. Every later-start range is **smaller**, by **326,008–413,600 bytes**. Neither member exceeds 8 MiB, and no request fits either pair’s intersection.
- Ignoring containment creates **2,477 misleading comparisons**, totaling **141.547 MB** of logical size difference. Those same-start entries cannot coexist in the disk index.
- A separate immediate-publication model, including contained-entry removal and learned checksums, likewise finds **zero cheaper selections**. Its modeled reads are explicitly not treated as actual replay I/O.
- Saved telemetry confirms **3,647 actual candidate reads above 8 MiB**, but the necessary cross-entry geometry is absent.

Scripts, exact results, anonymized examples, alignment calculations, and validation tests are local in the same directory.