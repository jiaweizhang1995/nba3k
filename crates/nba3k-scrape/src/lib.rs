//! nba3k-scrape: produce a fresh `seed_<season>.sqlite` from public NBA
//! sources (BBRef bootstrap, stats.nba.com via Python `nba_api`, HoopsHype
//! contracts, and a 2026 mock draft). See `crates/nba3k-scrape/README.md`
//! for usage and the Python install requirement.

pub mod assertions;
pub mod cache;
pub mod ids;
pub mod overrides;
pub mod politeness;
pub mod ratings;
pub mod seed;
pub mod sources;
pub mod teams;
