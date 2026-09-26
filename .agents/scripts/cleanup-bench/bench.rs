//! Throwaway benchmark: serial vs parallel deletion of cleanup-shaped trees.
//!
//! Uses `std::fs::remove_dir_all`, the exact implementation behind hok's
//! `internal::fs::remove_dir` (crates/libscoop/src/internal/fs.rs:44-45), so the
//! measured per-unit cost is the real cost hok pays per old-version directory.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Scenario {
    name: &'static str,
    apps: usize,
    vers: usize,
    files: usize,
}

fn gen_tree(root: &Path, sc: &Scenario) {
    let _ = fs::remove_dir_all(root);
    let payload = [b'x'; 32];
    for a in 0..sc.apps {
        for v in 0..sc.vers {
            let d = root.join(format!("app{a}")).join(format!("{v}.0.0"));
            fs::create_dir_all(&d).unwrap();
            for f in 0..sc.files {
                let mut fh = File::create(d.join(format!("f{f}.bin"))).unwrap();
                fh.write_all(&payload).unwrap();
            }
        }
    }
}

fn version_units(root: &Path, sc: &Scenario) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for a in 0..sc.apps {
        for ver in 0..sc.vers {
            v.push(root.join(format!("app{a}")).join(format!("{ver}.0.0")));
        }
    }
    v
}

fn app_units(root: &Path, sc: &Scenario) -> Vec<PathBuf> {
    (0..sc.apps).map(|a| root.join(format!("app{a}"))).collect()
}

fn del_serial(units: &[PathBuf]) -> u128 {
    let t = Instant::now();
    for u in units {
        fs::remove_dir_all(u).unwrap();
    }
    t.elapsed().as_millis()
}

fn del_parallel(units: &[PathBuf], threads: usize) -> u128 {
    let idx = AtomicUsize::new(0);
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= units.len() {
                        break;
                    }
                    fs::remove_dir_all(&units[i]).unwrap();
                }
            });
        }
    });
    t.elapsed().as_millis()
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| {
        panic!("usage: bench <scratch-dir>")
    }));
    let scenarios = [
        Scenario {
            name: "wide-12apps-1old-3k",
            apps: 12,
            vers: 1,
            files: 3000,
        },
        Scenario {
            name: "skew-2apps-6old-3k",
            apps: 2,
            vers: 6,
            files: 3000,
        },
        Scenario {
            name: "big-8apps-1old-10k",
            apps: 8,
            vers: 1,
            files: 10000,
        },
    ];

    println!("scenario,total_files,serial_ver_ms,par2_ver_ms,par4_ver_ms,par4_app_ms");
    for sc in &scenarios {
        let total = sc.apps * sc.vers * sc.files;

        gen_tree(&root, sc);
        let serial = del_serial(&version_units(&root, sc));

        gen_tree(&root, sc);
        let par2 = del_parallel(&version_units(&root, sc), 2);

        gen_tree(&root, sc);
        let par4 = del_parallel(&version_units(&root, sc), 4);

        gen_tree(&root, sc);
        let par4_app = del_parallel(&app_units(&root, sc), 4);

        println!("{},{},{},{},{},{}", sc.name, total, serial, par2, par4, par4_app);
    }

    let _ = fs::remove_dir_all(&root);
}
