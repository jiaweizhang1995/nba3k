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
    eprintln!("nba3k REPL — `help` for commands, `quit` to exit.");
    loop {
        match rl.readline(PROMPT) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(line);
                if let Err(e) = exec_line(app, line) {
                    eprintln!("error: {:#}", e);
                }
                if app.should_quit {
                    break;
                }
            }
            Err(ReadlineError::Eof | ReadlineError::Interrupted) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn run_pipe(app: &mut AppState) -> Result<()> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Err(e) = exec_line(app, line) {
            eprintln!("error: {:#}", e);
            return Err(e);
        }
        if app.should_quit {
            break;
        }
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
