use std::path::Path;

use crate::{error::Fallible, package::manifest_walker, Manifest};

#[derive(Debug, Clone, Default)]
pub struct MissingCheckverReport {
    pub total: u32,
    pub missing_checkver: u32,
    pub missing_autoupdate: u32,
    pub supported_items: Vec<String>,
    pub missing_items: Vec<MissingItem>,
}

#[derive(Debug, Clone)]
pub struct MissingItem {
    pub name: String,
    pub issues: Vec<String>,
}

pub fn scan(dir: &Path, supported: bool) -> Fallible<MissingCheckverReport> {
    let mut report = MissingCheckverReport::default();

    for path in manifest_walker::discover(dir)? {
        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let has_checkver = manifest.checkver().is_some();
        let has_autoupdate = manifest.autoupdate().is_some();
        report.total += 1;

        if supported {
            if has_checkver || has_autoupdate {
                report.supported_items.push(name);
            }
        } else {
            let mut issues = Vec::new();
            if !has_checkver {
                issues.push("checkver".to_string());
                report.missing_checkver += 1;
            }
            if !has_autoupdate {
                issues.push("autoupdate".to_string());
                report.missing_autoupdate += 1;
            }
            if !issues.is_empty() {
                report.missing_items.push(MissingItem { name, issues });
            }
        }
    }

    Ok(report)
}
