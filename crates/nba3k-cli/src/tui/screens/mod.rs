//! Per-screen renderers + key handlers. Wave-0 wires home/calendar/saves/
//! new_game placeholders + a single stub renderer used by Roster, Rotation,
//! Trades, Draft, Finance until M21/M22 ship those screens.

pub mod calendar;
pub mod home;
pub mod legacy;
pub mod new_game;
pub mod saves;
pub mod stub;

pub use stub::render_stub;
