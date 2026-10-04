import csv
import os
import subprocess
import sys
from pathlib import Path

# Run from the isolated Linux checkout. Each invocation owns and removes its data file.
out = Path(sys.argv[1])
out.mkdir()
cases = [
    (4096, 1, 0, False),
    (4096, 32, 0, False),
    (4096, 128, 0, False),
    (1048576, 32, 0, False),
    (4096, 32, 4, False),
    (1048576, 32, 0, True),
    (4096, 32, 0, True),
]
with (out / "results.csv").open("w") as output:
    writer = csv.writer(output)
    writer.writerow(["repeat", "binary", "unpooled", "size", "readers", "writers", "seconds", "read_MB_s", "write_MB_s", "iops", "p50_us", "p99_us", "cpu_percent"])
    for repeat in range(int(sys.argv[2])):
        for index, (size, readers, writers, unpooled) in enumerate(cases):
            for binary in (["baseline", "candidate"] if (repeat + index) % 2 == 0 else ["candidate", "baseline"]):
                env = dict(os.environ, READ_SIZE=str(size), READERS=str(readers))
                env.pop("UNPOOLED", None)
                if unpooled:
                    env["UNPOOLED"] = "1"
                name = f"{repeat}-{index}-{binary}"
                with (out / f"{name}.log").open("w") as log:
                    run = subprocess.run([f"./{binary}", "/mnt/local-ssd", "5", str(writers)], env=env, text=True, stdout=subprocess.PIPE, stderr=log, timeout=90, check=True)
                (out / f"{name}.csv").write_text(run.stdout)
                row = run.stdout.strip().splitlines()[-1].split(",")
                writer.writerow([repeat, binary, unpooled] + row)
                output.flush()
                print(name, unpooled, ",".join(row), flush=True)
