# Reproduce the checksum comparison

These scripts and `candidate.patch` are unchanged copies of the captured experiment
artifacts. They target the historical source, not the current implementation.
The patch predates the subsequent naming and test cleanup in the local review archive.

## Inputs and source

Use `m8g-32cpu-local-ssd-2`, Linux aarch64, Rust 1.99.0, and
`libjemalloc2` 5.3.0-2build1. The runner also requires `sysstat`, Python 3,
GNU time, `flock`, and the mounted local SSD. See
[settings](../replay-settings.json) for input, archive, patch, and binary hashes.

The trace is not committed or present in this experiment directory. Supply the
original `trace.tar.zst` with SHA-256
`a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7`.
A substitute trace does not reproduce this comparison.

In a fresh workspace at `/home/ubuntu/feuer-partial-checksums-replay`:

1. Create `baseline.tar` using
   `git archive 033284d264062e438010784cb6a2118568130786 > baseline.tar`
   from a checkout containing that commit. Its hash matches the retained archive.
2. Extract it separately into `baseline/` and `candidate/`. Copy all scripts and
   `candidate.patch` from this directory into the workspace root. Create `bin/`
   and `input/`, and put the trace at `input/trace.tar.zst`.
3. From `candidate/`, run `git apply --check ../candidate.patch` then
   `git apply ../candidate.patch`. This patch includes the tested source, tests,
   and historical documentation changes. Do not substitute a later cleanup patch.
4. From the workspace root run `bash build.sh`. Separate target directories are
   used for the two revisions. Check hashes against the saved settings when using
   the original build environment. Other environments may produce different binaries.
5. Follow the [timed SSD replay procedure](../../../storage/timed-replay.md).
   When a replay is explicitly requested, run `bash run-pair.sh` once. It executes
   baseline then candidate, each with a fresh cache and the first 600 seconds.
6. Run `python3 compare.py .` after both cases. It writes generated comparison and
   metric reports, not the curated interpretation in `../REPLAY.md`.

The scripts use the workspace path above and Linux-specific tools. They are not a
portable benchmark harness. Keep the workspace and its raw outputs local and ignored.
Do not run extra repetitions without an explicit request.

## Recalculate from retained captures without replay

From the experiment directory:

```sh
python3 reproduce/report.py raw/replay/results/baseline baseline range 256000 256000 262144
python3 reproduce/report.py raw/replay/results/candidate candidate range 256000 256000 262144
```

These commands read captured counters, summaries, and OS/device observations and
print validated JSON without rewriting the evidence. Curation reproduced both saved
reports, apart from insignificant floating-point summation differences in iostat
averages. The histogram and window scripts can regenerate their derived artifacts
in a copy of the captures. No new replay or Linux build was performed during curation.
