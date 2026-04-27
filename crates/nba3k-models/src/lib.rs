//! Realism Engine — explainable scoring models for player/team/trade
//! decisions. Workers fill the per-model bodies; orchestrator pre-locks
//! the public surface so each worker can `cargo build` in isolation.
//!
//! Layout:
//! - `Score` + `Reason` — uniform output type. Every model returns one.
//! - `player_value`     — Worker A
//! - `contract_value`   — Worker A
//! - `team_context`     — Worker B
//! - `star_protection`  — Worker B
//! - `asset_fit`        — Worker D (depends on A's signatures)
//! - `trade_acceptance` — Worker D (composite over 1-5)
//! - `stat_projection`  — Worker C
//! - `weights`          — TOML loader for tuning, with hardcoded defaults.

pub mod player_value;
pub mod contract_value;
pub mod contract_gen;
pub mod contract_extension;
pub mod team_context;
pub mod star_protection;
pub mod asset_fit;
pub mod trade_acceptance;
pub mod stat_projection;
pub mod progression;
pub mod retirement;
pub mod team_chemistry;
pub mod training;
pub mod weights;

pub use progression::{
    apply_progression_step, progress_player, regress_player, update_dynamic_potential,
    AttributeDelta, PlayerDevelopment,
};
pub use retirement::should_retire;
pub use training::{apply_training_focus, TrainingDelta, TrainingFocus};

// ---------------------------------------------------------------------------
// Common output types — every model uses these.
// ---------------------------------------------------------------------------

/// A scoring-model result: the numeric value (units depend on model — cents,
/// 0..1 probability, raw stat counts) plus an ordered list of contributing
/// reasons. Reasons are sorted by `|delta|` desc so callers can render the
/// top-K explanations cleanly.
#[derive(Debug, Clone)]
pub struct Score {
    pub value: f64,
    pub reasons: Vec<Reason>,
}

/// One named contribution to a Score. `delta` is signed: positive raised the
/// score, negative lowered it.
#[derive(Debug, Clone, Copy)]
pub struct Reason {
    pub label: &'static str,
    pub delta: f64,
}

impl Score {
    pub fn new(value: f64) -> Self {
        Self { value, reasons: Vec::new() }
    }

    pub fn with_reason(mut self, label: &'static str, delta: f64) -> Self {
        self.reasons.push(Reason { label, delta });
        self
    }

    /// Add a reason in-place.
    pub fn add(&mut self, label: &'static str, delta: f64) {
        self.value += delta;
        self.reasons.push(Reason { label, delta });
    }

    /// Sort reasons by |delta| descending. Call before rendering top-K.
    pub fn sort_reasons(&mut self) {
        self.reasons
            .sort_by(|a, b| b.delta.abs().partial_cmp(&a.delta.abs()).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Truncate to the top-K reasons by |delta|. Mutates in place.
    pub fn top_k(&mut self, k: usize) {
        self.sort_reasons();
        if self.reasons.len() > k {
            self.reasons.truncate(k);
        }
    }

    /// Borrow reasons.
    pub fn reasons(&self) -> &[Reason] {
        &self.reasons
    }

    /// Combine two scores: values add, reasons concatenate.
    pub fn merge(&mut self, other: Score) {
        self.value += other.value;
        self.reasons.extend(other.reasons);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("missing data: {0}")]
    MissingData(String),
}

pub type ModelResult<T> = Result<T, ModelError>;
