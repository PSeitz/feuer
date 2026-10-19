#!/usr/bin/env bash
set -euo pipefail
root=/home/ubuntu/feuer-partial-checksums-replay
cd "$root"
[[ $(<build-exit.txt) == 0 ]]
sha256sum -c binary-sha256.txt
for revision in baseline candidate; do
    status=0
    bash run-case.sh "$root" "$revision" range 256000 256000 262144 || status=$?
    printf '%s %s\n' "$revision" "$status" >> pair-status.txt
    # A report can reject unfinished writes without invalidating successful request completion.
    # Preserve that result and continue to the other revision, without flushing or rerunning.
    [[ $(<"$root/results/$revision/exit-status.txt") == 0 ]]
done
printf '0\n' > pair-exit.txt
