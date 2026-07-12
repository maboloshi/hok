use clap::{crate_description, crate_name, crate_version, Parser, Subcommand};
use clap_verbosity_flag::Verbosity;
use libscoop::Session;
use tracing_subscriber::{
    filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

mod alias;
mod bucket;
mod cache;
mod cat;
mod checkhashes;
mod checkup;
mod checkurls;
mod checkver;
mod cleanup;
mod completions;
mod config;
mod create;
mod depends;
mod export;
mod formatjson;
mod hold;
mod home;
mod import;
mod info;
mod install;
mod list;
mod missing_checkver;
mod prefix;
mod reset;
mod search;
mod shim;
mod status;
mod unhold;
mod uninstall;
mod update;
mod upgrade;
mod virustotal;
mod which;

use crate::Result;

#[derive(Parser)]
#[command(
    name = crate_name!(),
    version = crate_version!(),
    about = crate_description!(),
    long_about = format!("{}

If you find any bugs or have a feature request, please open an issue on
GitHub: https://github.com/chawyehsu/hok/issues", crate_description!()),
    subcommand_required = true,
    arg_required_else_help = true,
    max_term_width = 100,
    after_help = format!(
        "Type '{} help <command>' to get help for a specific command.",
        crate_name!()
    )
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// The verbosity level
    #[command(flatten)]
    verbose: Verbosity,

    /// Show detailed operation information for debugging
    #[arg(global = true, long)]
    pub detail: bool,
}

#[derive(Subcommand)]
pub enum Command {
    Alias(alias::Args),
    Bucket(bucket::Args),
    Cache(cache::Args),
    Cat(cat::Args),
    Checkhashes(checkhashes::Args),
    Checkup(checkup::Args),
    Checkurls(checkurls::Args),
    Checkver(checkver::Args),
    Cleanup(cleanup::Args),
    Completions(completions::Args),
    Config(config::Args),
    Create(create::Args),
    Depends(depends::Args),
    Export(export::Args),
    FormatJson(formatjson::Args),
    Hold(hold::Args),
    Home(home::Args),
    Import(import::Args),
    Info(info::Args),
    Install(install::Args),
    List(list::Args),
    MissingCheckver(missing_checkver::Args),
    Prefix(prefix::Args),
    #[clap(alias = "s")]
    Search(search::Args),
    Shim(shim::Args),
    Status(status::Args),
    Reset(reset::Args),
    Unhold(unhold::Args),
    #[clap(alias = "rm", alias = "remove")]
    Uninstall(uninstall::Args),
    #[clap(alias = "u")]
    Update(update::Args),
    Upgrade(upgrade::Args),
    Virustotal(virustotal::Args),
    Which(which::Args),
}

/// CLI entry point
pub fn start() -> Result<()> {
    let args = Cli::parse();
    setup_logger(args.verbose.tracing_level_filter(), args.detail)?;
    crate::set_detail(args.detail);

    let session = Session::default();
    let user_agent = format!("Scoop/1.0 (+https://scoop.sh/) Hok/{}", crate_version!());
    let _ = session.set_user_agent(&user_agent);

    match args.command {
        Command::Alias(args) => alias::execute(args, &session),
        Command::Bucket(args) => bucket::execute(args, &session),
        Command::Cache(args) => cache::execute(args, &session),
        Command::Cat(args) => cat::execute(args, &session),
        Command::Checkhashes(args) => checkhashes::execute(args, &session),
        Command::Checkup(args) => checkup::execute(args, &session),
        Command::Checkurls(args) => checkurls::execute(args, &session),
        Command::Checkver(args) => checkver::execute(args, &session),
        Command::Cleanup(args) => cleanup::execute(args, &session),
        Command::Completions(args) => completions::execute(args),
        Command::Config(args) => config::execute(args, &session),
        Command::Create(args) => create::execute(args, &session),
        Command::Depends(args) => depends::execute(args, &session),
        Command::Export(args) => export::execute(args, &session),
        Command::FormatJson(args) => formatjson::execute(args),
        Command::Hold(args) => hold::execute(args, &session),
        Command::Home(args) => home::execute(args, &session),
        Command::Import(args) => import::execute(args, &session),
        Command::Info(args) => info::execute(args, &session),
        Command::Install(args) => install::execute(args, &session),
        Command::List(args) => list::execute(args, &session),
        Command::MissingCheckver(args) => missing_checkver::execute(args),
        Command::Prefix(args) => prefix::execute(args, &session),
        Command::Search(args) => search::execute(args, &session),
        Command::Shim(args) => shim::execute(args, &session),
        Command::Reset(args) => reset::execute(args, &session),
        Command::Status(args) => status::execute(args, &session),
        Command::Unhold(args) => unhold::execute(args, &session),
        Command::Uninstall(args) => uninstall::execute(args, &session),
        Command::Update(args) => update::execute(args, &session),
        Command::Upgrade(args) => upgrade::execute(args, &session),
        Command::Virustotal(args) => virustotal::execute(args, &session),
        Command::Which(args) => which::execute(args, &session),
    }
}

fn setup_logger(level_filter: LevelFilter, detail: bool) -> Result<()> {
    // When --detail is active, ensure at least DEBUG level (unless user set higher via -vv)
    let effective_level = if detail { level_filter.max(LevelFilter::DEBUG) } else { level_filter };

    // filter for low-level/depedency logs
    let low_level_filter = match effective_level {
        LevelFilter::OFF => LevelFilter::OFF,
        LevelFilter::ERROR => LevelFilter::ERROR,
        LevelFilter::WARN => LevelFilter::WARN,
        LevelFilter::INFO => LevelFilter::WARN,
        LevelFilter::DEBUG => LevelFilter::INFO,
        LevelFilter::TRACE => LevelFilter::TRACE,
    };

    let mut layer_env_filter = EnvFilter::builder()
        .with_default_directive(effective_level.into())
        .from_env()?;

    // The custom `HOK_LOG_LEVEL` environment variable was introduced to set the
    // log level for hok since the first version.
    if let Ok(level) = std::env::var("HOK_LOG_LEVEL") {
        layer_env_filter = layer_env_filter.add_directive(format!("libscoop={level}").parse()?);
    }

    layer_env_filter = layer_env_filter
        // add low-level filter for git2
        .add_directive(format!("git2={}", low_level_filter).parse()?)
        // shortcuts-rs uses log crate internally; suppress its verbose debug output
        .add_directive("shortcuts_rs=warn".parse()?);

    let layer_fmt = tracing_subscriber::fmt::layer().without_time();

    tracing_subscriber::registry()
        .with(layer_env_filter)
        .with(layer_fmt)
        .init();

    Ok(())
}
