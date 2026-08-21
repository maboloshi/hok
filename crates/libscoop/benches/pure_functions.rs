//! Criterion guardrail benchmarks for libscoop's key *pure* functions (phase 5).
//!
//! Covers the version comparator, glob → regex conversion, hash computation,
//! manifest (JSON) parsing, and URL string helpers. All of these are pure
//! (no I/O / no async), so they benchmark directly without a tokio runtime.
//! Async operations (checkver / download / bucket update) are exercised by the
//! vs-Scoop script benchmark (`scripts/benchmark_vs_scoop.ps1`) instead.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use libscoop::compare_versions;
use libscoop::hash::ChecksumBuilder;
use libscoop::package::Manifest;
use libscoop::string::{glob_to_regex, matches_any_glob};
use libscoop::url::{decoded, github_owner_repo, remote_filename, url_filename};

// ── version ───────────────────────────────────────────────────────────────

fn bench_compare_versions(c: &mut Criterion) {
    c.bench_function("version/compare_equal", |b| {
        b.iter(|| black_box(compare_versions(black_box("1.2.3"), black_box("1.2.0"))))
    });
    c.bench_function("version/compare_prerelease", |b| {
        b.iter(|| black_box(compare_versions(black_box("1.2.0-rc1"), black_box("1.2.0"))))
    });
    c.bench_function("version/compare_nightly", |b| {
        b.iter(|| {
            black_box(compare_versions(
                black_box("2025.1.2-3"),
                black_box("2024.12.31-1"),
            ))
        })
    });
}

// ── string (glob) ─────────────────────────────────────────────────────────

fn bench_glob_to_regex(c: &mut Criterion) {
    c.bench_function("string/glob_to_regex", |b| {
        b.iter(|| black_box(glob_to_regex(black_box("*.dll"))))
    });
    c.bench_function("string/matches_any_glob", |b| {
        let patterns = vec![String::from("*.dll"), String::from("*.exe")];
        b.iter(|| black_box(matches_any_glob(black_box("foo.dll"), &patterns)))
    });
}

// ── hash ──────────────────────────────────────────────────────────────────

fn bench_hash(c: &mut Criterion) {
    // A typical scoop package payload chunk (64 KiB read buffer).
    let data = vec![0xABu8; 65536];
    c.bench_function("hash/sha256-64k", |b| {
        b.iter(|| {
            let mut h = ChecksumBuilder::new().sha256().build();
            h.consume(black_box(&data));
            black_box(h.finalize())
        })
    });
    c.bench_function("hash/md5-64k", |b| {
        b.iter(|| {
            let mut h = ChecksumBuilder::new().md5().build();
            h.consume(black_box(&data));
            black_box(h.finalize())
        })
    });
    c.bench_function("hash/sha512-64k", |b| {
        b.iter(|| {
            let mut h = ChecksumBuilder::new().sha512().build();
            h.consume(black_box(&data));
            black_box(h.finalize())
        })
    });
}

// ── manifest parse ────────────────────────────────────────────────────────

fn bench_manifest_parse(c: &mut Criterion) {
    // A representative Scoop manifest JSON (sub-key-free, mid-size).
    let json = r#"{
        "version": "1.2.3",
        "description": "A test package",
        "homepage": "https://example.com",
        "license": "MIT",
        "url": "https://example.com/foo.zip",
        "hash": "0b3e2b1c...",
        "bin": "foo.exe",
        "depends": ["dep1", "dep2"],
        "checkver": { "github": "owner/repo" }
    }"#;
    c.bench_function("manifest/parse_json", |b| {
        b.iter(|| black_box(Manifest::from_json(black_box("test"), black_box(json))))
    });
}

// ── url ───────────────────────────────────────────────────────────────────

fn bench_url(c: &mut Criterion) {
    let url = "https://github.com/owner/repo/releases/download/1.0/app-1.0.zip?token=abc";
    c.bench_function("url/remote_filename", |b| {
        b.iter(|| black_box(remote_filename(black_box(url))))
    });
    c.bench_function("url/url_filename", |b| {
        b.iter(|| black_box(url_filename(black_box(url))))
    });
    c.bench_function("url/decoded", |b| {
        b.iter(|| black_box(decoded(black_box("hello%20world%21"))))
    });
    c.bench_function("url/github_owner_repo", |b| {
        b.iter(|| black_box(github_owner_repo(black_box(url))))
    });
}

criterion_group!(
    benches,
    bench_compare_versions,
    bench_glob_to_regex,
    bench_hash,
    bench_manifest_parse,
    bench_url,
);
criterion_main!(benches);
