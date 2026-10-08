mod docs;
mod fixtures;
mod owned;
mod runner;
mod suites;
mod support;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
#[derive(Parser)]
#[command(about = "Rust quality tools for log_print; synthetic real-process fixtures")]
struct Cli {
    #[command(subcommand)]
    command: Option<Action>,
    #[arg(long)]
    report_dir: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = 1800,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    timeout: u64,
    #[arg(long)]
    skip_build: bool,
    #[arg(long)]
    list: bool,
}
#[derive(Subcommand)]
enum Action {
    /// Run one real-process suite against already built workspace binaries.
    Suite {
        #[arg(
            value_parser = ["protocol-v2",
            "supervisor-v2",
            "supervisor-failures-v2",
            "cli-v2",
            "inputs-v2",
            "outputs-v2",
            "webui-v2",
            "tui-v2"]
        )]
        name: String,
        #[arg(long)]
        case: Option<String>,
    },
    /// Install and build the locked offline WebUI frontend.
    Frontend,
    /// Run Playwright against an isolated application and SQLite archive.
    Browser,
    /// Verify local links and anchors in a built documentation site.
    Docs {
        directory: PathBuf,
        #[arg(long, default_value = "/")]
        base: String,
    },
    #[command(hide = true)]
    Fixture {
        mode: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}
fn main() -> anyhow::Result<std::process::ExitCode> {
    let args = Cli::parse();
    if let Some(Action::Fixture { mode, args }) = &args.command {
        fixtures::run(mode, args)?;
        return Ok(std::process::ExitCode::SUCCESS);
    }
    ctrlc::set_handler(|| support::INTERRUPTED.store(true, std::sync::atomic::Ordering::Relaxed))?;
    let success = match args.command {
        Some(Action::Suite { name, case }) => {
            let mut suite = suites::Suite::new(case);
            match name.as_str() {
                "protocol-v2" => suites::protocol::run(&mut suite),
                "supervisor-v2" => suites::supervisor::run(&mut suite),
                "supervisor-failures-v2" => suites::supervisor::failures(&mut suite),
                "cli-v2" => suites::cli::run(&mut suite),
                "inputs-v2" => suites::inputs::run(&mut suite),
                "outputs-v2" => suites::outputs::run(&mut suite),
                "webui-v2" => suites::webui::run(&mut suite, "webui"),
                "tui-v2" => {
                    suites::webui::run(&mut suite, "tui");
                    suites::tui::run(&mut suite)
                }
                _ => unreachable!(),
            }
            println!(
                "{}",
                serde_json::json!({ "suite" : name, "passed" : suite.passed,
                "failed" : suite.failed, "skipped" : suite.skipped })
            );
            suite.passed > 0 && suite.failed == 0
        }
        Some(Action::Frontend) => {
            runner::frontend()?;
            true
        }
        Some(Action::Browser) => {
            suites::webui::browser();
            true
        }
        Some(Action::Docs { directory, base }) => docs::verify(&directory, &base)?,
        Some(Action::Fixture { .. }) => unreachable!(),
        None => runner::run(args.report_dir, args.timeout, args.skip_build, args.list)?,
    };
    Ok(
        if success && !support::CLEANUP_FAILED.load(std::sync::atomic::Ordering::Relaxed) {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        },
    )
}
