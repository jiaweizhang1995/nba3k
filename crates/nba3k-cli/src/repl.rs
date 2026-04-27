use crate::cli::ReplLine;
use crate::commands;
use crate::state::AppState;
use anyhow::{Context, Result};
use clap::Parser;
use std::io::{self, BufRead};
use std::path::Path;

const PROMPT: &str = "nba3k> ";

pub fn run_interactive(app: &mut AppState) -> Result<()> {
    use rustyline::{error::ReadlineError, DefaultEditor};
    let mut rl = DefaultEditor::new()?;
    eprintln!("nba3k REPL — type `help` for commands, `quit` to exit.");
    loop {
        match rl.readline(PROMPT) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(line);
                // Treat `help` as a synonym for the global `--help` so users
                // don't get clap's "unrecognized subcommand" path.
                if matches!(line, "help" | "?") {
                    print_help();
                    continue;
                }
                if let Err(e) = exec_line(app, line) {
                    eprintln!("{}", format_repl_error(&e));
                }
                if app.should_quit {
                    break;
                }
            }
            Err(ReadlineError::Eof | ReadlineError::Interrupted) => break,
            Err(e) => return Err(e.into()),
        }
    }
    eprintln!("bye.");
    Ok(())
}

fn print_help() {
    use clap::CommandFactory;
    let mut cmd = ReplLine::command();
    let _ = cmd.print_help();
    eprintln!();
}

/// Strip the leading "error: " (clap adds one) so the REPL doesn't print
/// "error: error: ..." when surfacing parse errors.
fn format_repl_error(e: &anyhow::Error) -> String {
    let raw = format!("{:#}", e);
    let stripped = raw.strip_prefix("error: ").unwrap_or(&raw);
    format!("error: {}", stripped)
}

pub fn run_pipe(app: &mut AppState) -> Result<()> {
    let stdin = io::stdin();
    let mut had_error = false;
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Err(e) = exec_line(app, line) {
            eprintln!("{}", format_repl_error(&e));
            had_error = true;
            // Don't bail — match interactive REPL behavior so a single bad
            // line doesn't kill the whole pipe.
        }
        if app.should_quit {
            break;
        }
    }
    if had_error {
        anyhow::bail!("one or more piped commands failed");
    }
    Ok(())
}

pub fn run_script(app: &mut AppState, path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading script {}", path.display()))?;
    for (i, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Err(e) = exec_line(app, line) {
            return Err(e.context(format!("{}:{}: `{}`", path.display(), i + 1, line)));
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn exec_line(app: &mut AppState, line: &str) -> Result<()> {
    let tokens = shlex::split(line)
        .ok_or_else(|| anyhow::anyhow!("could not tokenize line (mismatched quotes?)"))?;
    let parsed = ReplLine::try_parse_from(tokens.iter()).map_err(|e| {
        // Print clap's nice formatted help/error then bubble.
        anyhow::anyhow!("{}", e)
    })?;
    commands::dispatch(app, parsed.command)
}
