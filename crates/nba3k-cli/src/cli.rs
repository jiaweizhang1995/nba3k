use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "nba3k", version, about = "NBA 2K-style GM mode CLI")]
pub struct Cli {
    /// Path to save file (SQLite). Required for most commands.
    #[arg(long, global = true, env = "NBA3K_SAVE")]
    pub save: Option<PathBuf>,

    /// Run a script file of REPL commands, then exit. Ignored if a subcommand is given.
    #[arg(long, global = true)]
    pub script: Option<PathBuf>,

    /// Sim engine choice. (M2 implements `statistical`; `possession` is v2.)
    #[arg(long, global = true, default_value = "statistical")]
    pub engine: String,

    /// Force God mode (overrides whatever is on the save).
    #[arg(long, global = true)]
    pub god: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A single REPL line is parsed using the same `Command` enum.
/// We use a wrapper Parser when reading from stdin so `--save` is honored
/// per-line if user wants to override.
#[derive(Parser, Debug, Clone)]
#[command(name = "nba3k", no_binary_name = true)]
pub struct ReplLine {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Create a new save file. Writes to the path in --save.
    New(NewArgs),
    /// Load an existing save (no-op if --save points at one).
    Load { path: PathBuf },
    /// Print save status.
    Status(JsonFlag),
    /// Save (flush). SQLite auto-flushes; this is a no-op placeholder.
    Save,
    /// Exit the REPL.
    #[command(visible_alias = "exit")]
    Quit,
    /// (M2) Sim a number of days.
    SimDay { count: Option<u32> },
    /// (M2) Sim until a phase.
    SimTo { phase: String },
    /// (M2) League standings.
    Standings(JsonFlag),
    /// (M2) Show a team roster.
    Roster {
        team: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// (M2) Show a player by name (case-insensitive substring match).
    Player {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// (M3) Trade subcommands.
    Trade(TradeArgs),
    /// (M5) Draft subcommands.
    Draft(DraftArgs),
    /// Developer / calibration tooling.
    Dev(DevArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct NewArgs {
    /// Team abbreviation, e.g. BOS, LAL.
    #[arg(long, value_name = "ABBREV")]
    pub team: String,
    /// Game mode: standard, god, hardcore, sandbox.
    #[arg(long, default_value = "standard")]
    pub mode: String,
    /// Starting season identifier (year of finals, e.g. 2026 = 2025-26 season).
    #[arg(long, default_value_t = 2026)]
    pub season: u16,
    /// Deterministic RNG seed.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
}

#[derive(Parser, Debug, Clone, Copy, Default)]
pub struct JsonFlag {
    /// Emit JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct TradeArgs {
    #[command(subcommand)]
    pub action: TradeAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TradeAction {
    /// (M3) Propose a trade.
    Propose {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, value_delimiter = ',')]
        send: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        receive: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// (M3) List active negotiations.
    List(JsonFlag),
    /// (M3) Respond to a counter-offer.
    Respond {
        id: u64,
        /// One of: accept, reject, counter
        action: String,
        #[arg(long)]
        json: bool,
    },
    /// (M3) Show full negotiation chain.
    Chain {
        id: u64,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct DevArgs {
    #[command(subcommand)]
    pub action: DevAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevAction {
    /// Run randomized trade evaluations across GM pairs.
    CalibrateTrade {
        #[arg(long, default_value_t = 200)]
        runs: u32,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct DraftArgs {
    #[command(subcommand)]
    pub action: DraftAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DraftAction {
    /// (M5) Show prospect board.
    Board(JsonFlag),
    /// (M5) Make a draft pick.
    Pick { player: String },
}
