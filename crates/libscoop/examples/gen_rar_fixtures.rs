//! Generate the `rars`-written part of the RAR extraction fixture corpus.
//!
//! WinRAR 7.23 removed RAR4 creation (`Rar.exe a -ma4` -> `ERROR: Unknown
//! option: ma4`, exit code 7), so the RAR 1.5-4.x fixtures cannot come from
//! WinRAR. They are written here instead, with the `rars` writer, from content
//! generated deterministically in this file. That keeps every committed
//! archive either produced by us or by WinRAR from content we generate, so
//! nothing carries an unclear redistribution licence.
//!
//! The generated archives are committed; this generator is not run by the test
//! suite (CI has no WinRAR and no network) and exists only so the corpus can be
//! rebuilt. `tests/fixtures/rar/README.md` records the provenance.
//!
//! Run from the crate root:
//!
//! ```text
//! cargo run -p libscoop --example gen_rar_fixtures
//! ```

use rars::Builder;
use rars::version::ArchiveVersion;
use std::path::{Path, PathBuf};

/// Deterministic byte source, so the corpus needs no checked-in input blobs.
fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
            ((state >> 8) & 0xff) as u8
        })
        .collect()
}

struct Entry {
    name: &'static str,
    data: Vec<u8>,
}

/// The shared content tree: compressible text, an empty file, incompressible
/// bytes and nested directories.
///
/// Deliberately ASCII-only. RAR 1.5-4.x has no reliable way to mark a member
/// name as UTF-8 — a non-ASCII name is decoded against the OEM code page unless
/// the archive sets the legacy Unicode flag. Measured: the two decoders
/// disagree on such a name in a RAR4 archive (the `unrar` backend decodes the
/// stored UTF-8 bytes as CP437 and yields bytes that are not a legal Windows
/// filename), so a non-ASCII RAR4 member cannot serve as a parity baseline.
/// Non-ASCII names are covered by the RAR5 fixtures, where the name is
/// unambiguously UTF-8 and both decoders agree.
fn base_entries() -> Vec<Entry> {
    let readme: String = (0..200)
        .map(|i| format!("line {i:04}: the quick brown fox jumps over the lazy dog\n"))
        .collect();
    vec![
        Entry {
            name: "readme.txt",
            data: readme.into_bytes(),
        },
        Entry {
            name: "empty.bin",
            data: Vec::new(),
        },
        Entry {
            name: "binary.bin",
            data: pseudo_random(4096, 12345),
        },
        Entry {
            name: "sub/nested.txt",
            data: "nested payload\n".repeat(64).into_bytes(),
        },
        Entry {
            name: "sub/deep/leaf.bin",
            data: pseudo_random(512, 777),
        },
    ]
}

/// A plausible PE with dense E8/E9 rel32 opcodes: the shape the x86 filter is
/// selected for.
fn x86_payload() -> Vec<u8> {
    let mut data = vec![0u8; 16 * 1024];
    data[0] = b'M';
    data[1] = b'Z';
    for offset in (0x200..data.len() - 5).step_by(5) {
        data[offset] = if offset % 10 == 0 { 0xe8 } else { 0xe9 };
        let delta = ((offset * 7) % 65536) as i32 - 32768;
        data[offset + 1..offset + 5].copy_from_slice(&delta.to_le_bytes());
    }
    data
}

/// Interleaved multi-channel ramp: the shape the delta filter is selected for.
fn delta_payload() -> Vec<u8> {
    let (rows, channels) = (400u32, 4u32);
    (0..rows)
        .flat_map(|row| (0..channels).map(move |ch| ((ch * 7 + row * 3) & 0xff) as u8))
        .collect()
}

/// Write one archive, or one volume set when `volume_size` is set.
fn build(
    out: &Path,
    name: &str,
    format: ArchiveVersion,
    store: bool,
    solid: bool,
    volume_size: Option<usize>,
    entries: &[&Entry],
) {
    let mut builder = Builder::new(format).store(store).solid(solid);
    if !store {
        builder = builder.compression_level(Some(5));
    }
    if let Some(size) = volume_size {
        builder = builder.volume_size(Some(size));
    }
    for entry in entries {
        // `mtime = None` keeps a clock reading out of the container, so the
        // corpus does not change from run to run for no reason.
        builder
            .add_bytes(
                entry.name.as_bytes().to_vec(),
                entry.data.clone(),
                None,
                None,
            )
            .unwrap_or_else(|e| panic!("add {}: {e}", entry.name));
    }

    let path = out.join(name);
    if volume_size.is_none() {
        builder
            .write_to_path(&path, None)
            .unwrap_or_else(|e| panic!("write {name}: {e}"));
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!("{name}: {len} bytes");
        return;
    }

    let volumes = builder.build_volumes(None).expect("build volumes");
    assert!(volumes.len() > 1, "{name}: expected a split volume set");
    // RAR 1.5-4.x volume sets use the old naming (`x.rar`, `x.r00`, `x.r01`).
    // WinRAR 7.23 refuses a `.partN.rar`-named RAR4 set outright, looking for
    // `...part2.r00` instead, so this is the only valid shape for RAR4.
    let stem = name.strip_suffix(".rar").expect("name ends in .rar");
    for (index, bytes) in volumes.iter().enumerate() {
        let target = if index == 0 {
            out.join(format!("{stem}.rar"))
        } else {
            out.join(format!("{stem}.r{:02}", index - 1))
        };
        std::fs::write(&target, bytes).expect("write volume");
    }
    println!("{name}: {} volumes", volumes.len());
}

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rar");
    std::fs::create_dir_all(&out).expect("create fixture directory");

    let base = base_entries();
    let base_refs: Vec<&Entry> = base.iter().collect();
    let payloads = [
        Entry {
            name: "x86_stream.bin",
            data: x86_payload(),
        },
        Entry {
            name: "delta_ramp.bin",
            data: delta_payload(),
        },
    ];
    let payload_refs: Vec<&Entry> = payloads.iter().collect();
    let volume = [Entry {
        name: "volpayload.bin",
        data: pseudo_random(40 * 1024, 424_242),
    }];
    let volume_refs: Vec<&Entry> = volume.iter().collect();

    // RAR 1.5-4.x: WinRAR 7.23 cannot produce these.
    build(
        &out,
        "rar4-normal.rar",
        ArchiveVersion::Rar40,
        false,
        false,
        None,
        &base_refs,
    );
    build(
        &out,
        "rar4-solid.rar",
        ArchiveVersion::Rar40,
        false,
        true,
        None,
        &base_refs,
    );
    build(
        &out,
        "rar4-store.rar",
        ArchiveVersion::Rar40,
        true,
        false,
        None,
        &base_refs,
    );
    build(
        &out,
        "rar4-largepayloads.rar",
        ArchiveVersion::Rar40,
        false,
        false,
        None,
        &payload_refs,
    );
    build(
        &out,
        "rar4-vol.rar",
        ArchiveVersion::Rar40,
        true,
        false,
        Some(16 * 1024),
        &volume_refs,
    );
}
