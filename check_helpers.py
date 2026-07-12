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
