"""
Check which Scoop helper functions are embedded in hok's asset_scripts.

Parses asset_scripts/core.ps1 and decompress.ps1 for function definitions,
then compares against the known helper list (scripts/known-helpers.txt).
Reports coverage and any missing functions.

known-helpers.txt is the reference list of Scoop PS helper functions
encountered in the wild — its initial content was harvested from
analyze-bucket.ps1 output (scanning ~20k manifest files).
"""

import re, pathlib

helpers = set()
for fn in ['asset_scripts/core.ps1', 'asset_scripts/decompress.ps1']:
    for line in pathlib.Path(fn).read_bytes().decode('utf-8-sig').split('\n'):
        m = re.match(r'function\s+(\S+)', line.strip())
        if m:
            helpers.add(m.group(1))

known = set()
for line in pathlib.Path('scripts/known-helpers.txt').read_bytes().decode('utf-8-sig').split('\n'):
    if line.strip():
        known.add(line.strip())

native = {'Stop-Service', 'Start-Service'}
missing = known - helpers - native
covered = helpers & known

print(f"Embedded: {len(helpers)} functions")
print(f"Known:    {len(known)} functions")
print(f"Covered:  {len(covered)} functions")
print(f"Missing:  {len(missing)} functions")
if missing:
    print(f"Missing list: {sorted(missing)}")
else:
    print("All covered!")
