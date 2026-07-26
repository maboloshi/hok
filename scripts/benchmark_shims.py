"""Benchmark shim implementations.

Measures execution time of whoami.exe (console app) launched through
various shim implementations. Results include mean, min, and overhead
vs direct execution.

Usage:
    python scripts/benchmark_shims.py

Preparation:
    - Build hok-shim: cargo build -p hok-shim --release
    - Download upstream shims (optional):
      Place in shim_test/{name}/shim.exe
      Sources: https://github.com/ScoopInstaller/Shim/releases
"""
import subprocess, time, os, sys, tempfile, shutil

WHOAMI = r"C:\Windows\System32\whoami.exe"
HOK_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SHIMS = {
    "Direct":  None,
    "hok-shim": os.path.join(HOK_ROOT, "target", "release", "hok-shim.exe"),
    "Rust":    os.path.join(HOK_ROOT, "shim_test", "Rust_Shim_v0.1.1", "shim.exe"),
    "Zig":     os.path.join(HOK_ROOT, "shim_test", "Zig_Shim_v0.1.2", "shim.exe"),
    "C++":     os.path.join(HOK_ROOT, "shim_test", "C++_Shim_v0.1.1", "shim.exe"),
    "C#":      os.path.join(HOK_ROOT, "shim_test", "C#_Shim_v0.1.1", "shim.exe"),
}
RUNS = 30
WARMUP = 10


def run(cmd):
    start = time.perf_counter()
    r = subprocess.run(cmd, capture_output=True, timeout=10)
    return (time.perf_counter() - start) * 1000


results = {}
for name, path in SHIMS.items():
    if not path and name != "Direct":
        continue
    if name == "Direct":
        exe = WHOAMI
    else:
        if not os.path.exists(path):
            print(f"{name:>10}: SKIPPED (not found)")
            continue
        tmp = tempfile.mkdtemp(prefix="shim_")
        shutil.copy(path, os.path.join(tmp, "test.exe"))
        with open(os.path.join(tmp, "test.shim"), "w", encoding="utf-8") as f:
            f.write(f"path = {WHOAMI}\n")
        exe = os.path.join(tmp, "test.exe")

    for _ in range(WARMUP):
        try:
            run(exe)
        except:
            pass

    times = []
    for _ in range(RUNS):
        try:
            times.append(run(exe))
        except Exception as e:
            sys.stderr.write(f"ERROR {name}: {e}\n")

    avg = sum(times) / len(times)
    results[name] = (avg, min(times))
    print(f"{name:>10}: avg={avg:7.1f} ms  min={min(times):6.1f} ms")

    if name != "Direct":
        shutil.rmtree(tmp, ignore_errors=True)

base = results["Direct"][0]
print()
print(f"{'Name':>10} | {'avg ms':>7} | {'min ms':>7} | {'overhead':>8} | {'size':>7}")
print("-" * 55)
for name in ("hok-shim", "Rust", "Zig", "C++", "C#"):
    if name not in results:
        continue
    avg, mn = results[name]
    sz = os.path.getsize(SHIMS[name]) // 1024
    print(f"{name:>10} | {avg:7.1f} | {mn:7.1f} | +{avg - base:6.1f} ms | {sz:>4} KB")
