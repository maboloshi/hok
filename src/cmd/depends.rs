//! List dependencies of a package.

use clap::Parser;
use libscoop::{QueryOption, Session, package};

use crate::{Result, output};

/// Show dependencies of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package
    package: String,

    /// Use the specified architecture (32bit/64bit/arm64), overriding the
    /// runtime-detected and configured default (Scoop's -a/--arch)
    #[arg(short = 'a', long = "arch")]
    arch: Option<String>,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `-a/--arch` overrides the effective architecture (Scoop's
    // `Format-ArchitectureString` + `Get-DefaultArchitecture` override).
    // Parsed after the session was created, so it beats the
    // `default_architecture` config — architecture-specific `depends`
    // entries then resolve for the requested architecture.
    if let Some(arch) = args.arch.as_deref() {
        session.set_default_architecture(arch)?;
    }

    let query = args.package;
    let queries = vec![query.as_str()];
    let options = vec![QueryOption::Explicit];
    let mut result = package::query::query(session, queries, options, false)?;

    if result.is_empty() {
        return Err(anyhow::anyhow!(rust_i18n::t!(
            "cmd.depends_not_found",
            query = query
        )));
    }

    // Pick the first match (or let user choose if multiple)
    let pkg = if result.len() == 1 {
        result.remove(0)
    } else {
        result.sort_by_key(|p| p.ident());
        output::info(format!("Found multiple packages named '{query}':\n"));
        for (idx, pkg) in result.iter().enumerate() {
            output::named(
                format!("{idx}."),
                format!("{}/{}", pkg.bucket(), pkg.name()),
            );
        }
        let idx = output::prompt_select(result.len())
            .ok_or_else(|| anyhow::anyhow!(rust_i18n::t!("cmd.depends_invalid")))?;
        result.remove(idx)
    };

    // Display the dependency tree
    let tree = package::depends::dependencies_tree(session, pkg.name(), pkg.bucket())?;
    for node in &tree {
        if node.depth == 0 {
            output::named(format!("{}/{}", node.bucket, node.name), "");
        } else if node.already_listed {
            println!(
                "{:indent$} {} {}",
                "",
                node.name,
                rust_i18n::t!("cmd.depends_already"),
                indent = node.depth * 2
            );
        } else {
            println!(
                "{:indent$} {} [{}]",
                "",
                node.name,
                node.bucket,
                indent = node.depth * 2
            );
        }
    }

    Ok(())
}
