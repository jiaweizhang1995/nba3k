use anyhow::Result;
use std::io::{self, IsTerminal};

mod cli;
mod commands;
mod repl;
mod state;

use cli::Cli;
use clap::Parser;

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let mut app = state::AppState::new(cli.save.clone(), cli.god);

    // Single-shot subcommand: execute and exit.
    if let Some(cmd) = cli.command.clone() {
        return commands::dispatch(&mut app, cmd);
    }

    // Script file: replay each line as a parsed command, then exit.
    if let Some(script) = cli.script.as_ref() {
        return repl::run_script(&mut app, script);
    }

    // Pipe-driven: stdin not a tty → read each line as a command, exit on EOF.
    if !io::stdin().is_terminal() {
        return repl::run_pipe(&mut app);
    }

    // Interactive REPL.
    repl::run_interactive(&mut app)
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("NBA3K_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt().with_env_filter(filter).with_writer(io::stderr).init();
}
