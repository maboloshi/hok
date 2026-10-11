# 7z fixtures

`empty-entries.7z` (206 B) — the stream-less-entry corpus for the 7z extraction
path (`internal/archive.rs`, `test_extract_7z_empty_entries*`).

Contents (from `7z l -slt`):

| Entry | Type | Size |
| --- | --- | --- |
| `emptydir` | directory | 0 |
| `sub` | directory | — |
| `sub/emptydir2` | directory (empty) | 0 |
| `empty.txt` | file, no stream | 0 |
| `sub/empty2.txt` | file, no stream | 0 |
| `plain.txt` | file | 5 (`hello`) |

None of the empty files and none of the directory entries carry a data stream,
so they live in no 7z block. The per-block decoder therefore never visits them,
which is why they must be materialized by an explicit pass.

## Regenerating

Created with 7-Zip 26.03 (x64) from this source tree:

```
src/plain.txt          = "hello" (no trailing newline)
src/empty.txt          = 0 bytes
src/sub/empty2.txt     = 0 bytes
src/emptydir/          = empty directory
src/sub/emptydir2/     = empty directory
```

```powershell
7z a -t7z tests/fixtures/7z/empty-entries.7z src\* -mx0
```

`-mx0` keeps the fixture byte-tiny and store-only, so the archive is stable and
needs no C++ toolchain to read back. Extraction is asserted against the exact
tree above; see the two tests next to `test_detect_format` in
`crates/libscoop/src/internal/archive.rs`.
