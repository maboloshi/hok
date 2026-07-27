use criterion::{black_box, criterion_group, criterion_main, Criterion};
use scoop_hash::ChecksumBuilder;

macro_rules! hash_bench {
    ($a:ident, $m:ident) => {
        fn $a(size: usize) {
            let data = &vec![0xffu8; size][..];
            let mut hasher = ChecksumBuilder::new().$m().build();
            hasher.consume(data);
            hasher.finalize();
        }
    };
}

macro_rules! bench_group {
    ($g:ident, $a:ident, $label:expr) => {
        fn $g(c: &mut Criterion) {
            let mut group = c.benchmark_group($label);
            group.bench_function("scoop_hash", |b| b.iter(|| $a(black_box(100))));
            group.finish();
        }
    };
}

// Generate hash functions
hash_bench!(md5, md5);
hash_bench!(sha1, sha1);
hash_bench!(sha256, sha256);
hash_bench!(sha512, sha512);

// 100-byte benchmarks
bench_group!(bench_md5_100, md5, "md5_100");
bench_group!(bench_sha1_100, sha1, "sha1_100");
bench_group!(bench_sha256_100, sha256, "sha256_100");
bench_group!(bench_sha512_100, sha512, "sha512_100");

// 1000-byte benchmarks
bench_group!(bench_md5_1000, md5, "md5_1000");
bench_group!(bench_sha1_1000, sha1, "sha1_1000");
bench_group!(bench_sha256_1000, sha256, "sha256_1000");
bench_group!(bench_sha512_1000, sha512, "sha512_1000");

// 10000-byte benchmarks
bench_group!(bench_md5_10000, md5, "md5_10000");
bench_group!(bench_sha1_10000, sha1, "sha1_10000");
bench_group!(bench_sha256_10000, sha256, "sha256_10000");
bench_group!(bench_sha512_10000, sha512, "sha512_10000");

// 100000-byte benchmarks
bench_group!(bench_md5_100000, md5, "md5_100000");
bench_group!(bench_sha1_100000, sha1, "sha1_100000");
bench_group!(bench_sha256_100000, sha256, "sha256_100000");
bench_group!(bench_sha512_100000, sha512, "sha512_100000");

// 1000000-byte benchmarks
bench_group!(bench_md5_1000000, md5, "md5_1000000");
bench_group!(bench_sha1_1000000, sha1, "sha1_1000000");
bench_group!(bench_sha256_1000000, sha256, "sha256_1000000");
bench_group!(bench_sha512_1000000, sha512, "sha512_1000000");

criterion_group!(
    benches,
    bench_md5_100,
    bench_sha1_100,
    bench_sha256_100,
    bench_sha512_100,
    bench_md5_1000,
    bench_sha1_1000,
    bench_sha256_1000,
    bench_sha512_1000,
    bench_md5_10000,
    bench_sha1_10000,
    bench_sha256_10000,
    bench_sha512_10000,
    bench_md5_100000,
    bench_sha1_100000,
    bench_sha256_100000,
    bench_sha512_100000,
    bench_md5_1000000,
    bench_sha1_1000000,
    bench_sha256_1000000,
    bench_sha512_1000000,
);
criterion_main!(benches);
