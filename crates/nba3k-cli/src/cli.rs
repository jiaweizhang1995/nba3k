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

    /// Sim engine choice. Options: `statistical` (default), `possession` (v2, planned).
    #[arg(long, global = true, default_value = "statistical")]
    pub engine: String,

    /// Force God mode (overrides whatever is on the save).
    #[arg(long, global = true)]
    pub god: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "nba3k",
    no_binary_name = true,
    about = "NBA 2K-style GM mode — type a command, `help` for the list, `quit` to exit"
)]
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
    /// Sim a number of days.
    SimDay {
        /// Days to simulate (default 1).
        count: Option<u32>,
    },
    /// Sim until a phase or named day marker.
    /// Phases: regular | regular-end | playoffs | trade-deadline | offseason.
    /// Markers: all-star (day 41) | cup-final (day 55) | season-end (playoffs done).
    SimTo {
        phase: String,
    },
    /// Sim 7 days, pausing on incoming trade offers / user-team injuries unless
    /// --no-pause is set.
    SimWeek {
        /// Skip pause-on-event check.
        #[arg(long)]
        no_pause: bool,
    },
    /// Sim 30 days with the same pause semantics as `sim-week`.
    SimMonth {
        /// Skip pause-on-event check.
        #[arg(long)]
        no_pause: bool,
    },
    /// League standings (current or historical).
    Standings {
        /// Season year (default: current).
        #[arg(long)]
        season: Option<u16>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show a team roster.
    Roster {
        /// Team abbreviation (e.g. BOS, LAL). Defaults to your save's team.
        team: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Assign a role tag to a player.
    /// Roles: star, starter, sixth, role, bench, prospect.
    RosterSetRole {
        /// Player name (case-insensitive substring match; must be unambiguous).
        player: String,
        /// Role: star | starter | sixth | role | bench | prospect.
        role: String,
    },
    /// Show a player by name (case-insensitive substring match).
    Player {
        /// Player name (case-insensitive substring match).
        name: String,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Trade subcommands.
    Trade(TradeArgs),
    /// Draft subcommands.
    Draft(DraftArgs),
    /// Show team chemistry breakdown.
    Chemistry {
        /// Team abbreviation (e.g. BOS, LAL).
        team: String,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show end-of-season awards.
    Awards {
        /// Season year (default: current). e.g. 2026 = 2025-26 season.
        #[arg(long)]
        season: Option<u16>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Playoff bracket / sim.
    Playoffs(PlayoffsArgs),
    /// Champion + finals MVP + awards bundle.
    SeasonSummary(JsonFlag),
    /// Advance to the next season — runs progression pass, drafts, FA stub.
    SeasonAdvance(JsonFlag),
    /// Show GM inbox: trade demands from unhappy stars + roster alerts.
    Messages(JsonFlag),
    /// (M10) Show a player's career stats across seasons.
    Career {
        /// Player name (case-insensitive substring match).
        name: String,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M10) Free-agency subcommands.
    Fa(FaArgs),
    /// (M10) Training camp — allocate dev points to a player.
    Training {
        /// Player name (case-insensitive substring match; must be on user team).
        player: String,
        /// Attribute to train: shoot | inside | def | reb | ath | handle.
        focus: String,
    },
    /// (M11) Show a team's salary cap / luxury tax status.
    Cap {
        /// Team abbreviation (e.g. BOS, LAL). Defaults to your save's team.
        team: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M11) Manually retire a player (e.g. force a 41yo veteran into HOF list).
    Retire {
        /// Player name (case-insensitive substring match).
        player: String,
    },
    /// (M12) Hall of Fame — retired players ranked by career production.
    Hof {
        /// Limit display to top N (default 30).
        #[arg(long, default_value_t = 30)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M13) Mid-season award race — top-5 leaders for MVP/DPOY/ROY/6MOY/MIP.
    AwardsRace(JsonFlag),
    /// (M13) Recent league news — trades, signings, retirements, injuries, awards.
    News {
        /// Limit display to most recent N entries (default 30).
        #[arg(long, default_value_t = 30)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M14) Coach subcommands — hire / fire / inspect.
    Coach(CoachArgs),
    /// (M14) Scout a draft prospect to reveal their ratings.
    Scout {
        /// Prospect name (case-insensitive substring match).
        player: String,
    },
    /// (M16) In-season NBA Cup tournament — bracket + result.
    Cup {
        /// Season year (default: current).
        #[arg(long)]
        season: Option<u16>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M16) Show trade rumors — AI-team interest signals on players league-wide.
    Rumors {
        /// Limit to N strongest rumors (default 20).
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M17) Show incoming trade offers from AI teams targeting your players.
    Offers {
        /// Limit display (default 10).
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M17) Negotiate a contract extension for one of your roster players.
    Extend {
        /// Player name (must be on user team).
        player: String,
        /// Annual salary in millions of dollars.
        #[arg(long)]
        salary_m: f64,
        /// Length of extension in years.
        #[arg(long, default_value_t = 3)]
        years: u8,
    },
    /// (M17) Track favorite players: add / remove / list.
    Notes(NotesArgs),
    /// (M18) Recap of recent games (last N days).
    Recap {
        /// Last N days to recap (default 1).
        #[arg(long, default_value_t = 1)]
        days: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M16) Side-by-side roster + cap comparison of two teams.
    Compare {
        /// First team abbreviation.
        team_a: String,
        /// Second team abbreviation.
        team_b: String,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M15) All-Star Game roster + result for a given season.
    AllStar {
        /// Season year (default: current).
        #[arg(long)]
        season: Option<u16>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M15) Manage save files.
    Saves(SavesArgs),
    /// (M14) League leaderboards for the current season or career.
    Records {
        /// Scope: `season` (current regular season) or `career`.
        #[arg(long, default_value = "season")]
        scope: String,
        /// Stat: ppg | rpg | apg | spg | bpg | three_made | fg_pct.
        #[arg(long, default_value = "ppg")]
        stat: String,
        /// Top-N entries (default 10).
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Developer / calibration tooling.
    Dev(DevArgs),
    /// (M20) Launch the GM-mode TUI shell. Default path is the 8-menu shell;
    /// `--legacy` falls back to the M19 5-tab read-only dashboard.
    Tui {
        /// Use the high-contrast TV palette + extra padding.
        #[arg(long)]
        tv: bool,
        /// Open the legacy M19 5-tab dashboard instead of the M20 shell.
        #[arg(long)]
        legacy: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct PlayoffsArgs {
    #[command(subcommand)]
    pub action: PlayoffsAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PlayoffsAction {
    /// Show R1 bracket.
    Bracket(JsonFlag),
    /// Sim the full bracket.
    Sim(JsonFlag),
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
    /// Propose a trade.
    Propose {
        /// Sender team abbreviation (e.g. BOS).
        #[arg(long)]
        from: String,
        /// Receiver team abbreviation (e.g. LAL).
        #[arg(long)]
        to: String,
        /// Comma-separated player names sent BY --from.
        #[arg(long, value_delimiter = ',')]
        send: Vec<String>,
        /// Comma-separated player names received FROM --to.
        #[arg(long, value_delimiter = ',')]
        receive: Vec<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Force acceptance through god-mode override.
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// List active negotiations.
    List(JsonFlag),
    /// Respond to a counter-offer.
    Respond {
        /// Trade chain id (from `trade list`).
        id: u64,
        /// One of: accept, reject, counter.
        action: String,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show full negotiation chain.
    Chain {
        /// Trade chain id (from `trade list`).
        id: u64,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// (M10) Propose a 3-team trade. Each `--leg` is `TM:player1,player2`.
    Propose3 {
        /// Three legs, each `ABBR:player1,player2,...`. Pass --leg three times.
        #[arg(long, value_name = "ABBR:player_csv", action = clap::ArgAction::Append)]
        leg: Vec<String>,
        /// Emit JSON instead of text.
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
    /// Print team's 9-feature quality vector + derived ORtg/DRtg.
    TeamStrength {
        team: String,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct NotesArgs {
    #[command(subcommand)]
    pub action: NotesAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum NotesAction {
    /// Mark a player as favorited (with optional comment).
    Add {
        /// Player name.
        player: String,
        /// Optional comment text.
        #[arg(long)]
        text: Option<String>,
    },
    /// Remove a player from favorites.
    Remove {
        /// Player name.
        player: String,
    },
    /// List all favorited players.
    List {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct SavesArgs {
    #[command(subcommand)]
    pub action: SavesAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SavesAction {
    /// List save files in a directory (default: current dir + /tmp).
    List {
        /// Directory to scan.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show metadata about a save file.
    Show {
        /// Save path to inspect.
        path: PathBuf,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Delete a save file (requires confirmation flag).
    Delete {
        /// Save path to delete.
        path: PathBuf,
        /// Required to actually delete (safety).
        #[arg(long)]
        yes: bool,
    },
    /// (M18) Export save data to JSON for backup or sharing.
    Export {
        /// Source save path.
        path: PathBuf,
        /// Destination JSON path.
        #[arg(long)]
        to: PathBuf,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct CoachArgs {
    #[command(subcommand)]
    pub action: CoachAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CoachAction {
    /// Show current team coach + traits.
    Show {
        /// Team abbreviation (defaults to user team).
        team: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Fire current coach and immediately hire a generated replacement.
    Fire {
        /// Team abbreviation (defaults to user team).
        team: Option<String>,
    },
    /// List coach pool (currently a generated set scaled to league size).
    Pool {
        /// Top-N entries (default 10).
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct FaArgs {
    #[command(subcommand)]
    pub action: FaAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FaAction {
    /// List available free agents (top by OVR).
    List(JsonFlag),
    /// Sign a free agent to the user's team.
    Sign {
        /// Player name (case-insensitive substring; must be a free agent).
        player: String,
    },
    /// Cut a player from the user's team to the FA pool.
    Cut {
        /// Player name (case-insensitive substring; must be on user team).
        player: String,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct DraftArgs {
    #[command(subcommand)]
    pub action: DraftAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DraftAction {
    /// Show prospect board (top-60).
    Board(JsonFlag),
    /// Show the draft order for the current season.
    Order(JsonFlag),
    /// AI auto-pick the entire draft.
    Sim(JsonFlag),
    /// User picks one prospect for their team.
    Pick {
        /// Prospect name (case-insensitive substring match).
        player: String,
    },
}
