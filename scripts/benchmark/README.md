# Benchmark

This directory contains scripts for benchmarking the performance between Scoop,
Hok, and other implementations. Various commands that are frequently used by
users are benchmarked, including `search`, `list` and `bucket list`.

## Usage

To run the benchmark, you will need to have the **Windows Sandbox** feature enabled.
Then, run the following command to generate a runnable sandbox configuration from
the template:

```powershell
$ ./generate.ps1 # or ./generate.ps1 -Proxy <proxy> to set up a proxy for the benchmark environment
```

After that, a `sandbox.wsb` file will be generated. Double click it to launch
the sandbox, and wait for the benchmark to finish. The benchmark results will
be printed to the console.

The benchmark script `bench.ps1` is generated with the `bench.ps1.template`
template. You can view the template to see how the benchmark is performed.

## Host-direct hok-vs-Scoop guardrail benchmark

[`scripts/benchmark_vs_scoop.ps1`](../benchmark_vs_scoop.ps1) is a lighter,
host-direct alternative that does **not** require Windows Sandbox. It runs the
same read-only commands (`list`, `search`, `info`, `which`, `prefix`) on both
hok and official Scoop, captures each one's exit code, output and wall time,
reports behavioral equivalence (exit code + normalized text) and a timing ratio,
and saves per-case outputs + `summary.csv` to `scripts/benchmark-output/`.

```powershell
# build hok first, then:
./scripts/benchmark_vs_scoop.ps1
# optional: also run network/stateful commands (status/checkver/download/install)
./scripts/benchmark_vs_scoop.ps1 -IncludeNetwork
```

Unavailable environments are skipped gracefully: a missing hok binary or Scoop
installation, an offline network, or a per-case 20s timeout are all reported as
`SKIP`/`TIMEOUT` rather than aborting the run.
