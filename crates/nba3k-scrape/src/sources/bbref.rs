//! Basketball-Reference team page scraper.
//!
//! ToS note: BBRef forbids scraped-data tools. Treat this as a one-time
//! bootstrap — cache aggressively (30d TTL), 1 req/3s rate, identifying UA.
//! Never redistribute scraped HTML.
//!
//! URL pattern: `https://www.basketball-reference.com/teams/{ABBR}/{end_year}.html`
//! e.g. `/teams/BOS/2026.html` for the 2025-26 Celtics.
//!
//! We pull two tables that live on every team page:
//!   - `roster` — id, position, age, height, weight, nationality, exp, college
//!   - `per_game` — per-game stats; many cells live in `<tr data-row=...>`
//!     and `<td data-stat="..."></td>`, which is stable.
//!
//! Where a stat column is missing or unparseable we substitute 0.0 so the
//! ratings stage can degrade gracefully rather than crash.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use scraper::{Html, Selector};

use super::{parse_position, RawPlayerStats};
use crate::cache::{html_ttl, Cache};
use crate::politeness::Fetcher;

pub struct BbrefSource<'a> {
    pub fetcher: &'a Fetcher,
    pub cache: &'a Cache,
    /// e.g. 2026 for the 2025-26 season.
    pub end_year: u16,
}

impl<'a> BbrefSource<'a> {
    pub fn fetch_team(&self, abbrev: &str) -> Result<Vec<RawPlayerStats>> {
        let key = format!("{}_{}", abbrev, self.end_year);
        let html_bytes = if let Some(b) = self.cache.get("bbref", &key, "html", html_ttl()) {
            tracing::debug!(abbrev, "bbref cache hit");
            b
        } else {
            let url = format!(
                "https://www.basketball-reference.com/teams/{}/{}.html",
                abbrev, self.end_year
            );
            let bytes = self.fetcher.get(&url).context("fetch bbref team page")?;
            self.cache.put("bbref", &key, "html", &bytes)?;
            // Be extra polite even on cache miss before next call.
            std::thread::sleep(Duration::from_millis(50));
            bytes
        };
        let html = std::str::from_utf8(&html_bytes).context("bbref body utf-8")?;
        parse_team_page(html).with_context(|| format!("parse bbref page {abbrev}"))
    }
}

fn parse_team_page(html: &str) -> Result<Vec<RawPlayerStats>> {
    // BBRef wraps secondary tables in HTML comments to avoid certain ad
    // blockers. Strip the `<!-- ... -->` markers around tables before parsing
    // so `select()` finds them.
    let html = strip_table_comments(html);
    let doc = Html::parse_document(&html);

    let roster = parse_roster(&doc)?;
    let per_game = parse_per_game(&doc).unwrap_or_default();

    if roster.is_empty() {
        return Err(anyhow!("no roster rows parsed"));
    }

    let mut out = Vec::with_capacity(roster.len());
    for r in roster {
        let stats = per_game.iter().find(|p| names_match(&p.name, &r.name)).cloned();
        let (primary, secondary) = parse_position(&r.position);
        let age = r.age.unwrap_or(stats.as_ref().and_then(|s| s.age).unwrap_or(25));
        out.push(RawPlayerStats {
            name: r.name,
            primary_position: primary,
            secondary_position: secondary,
            age,
            games: stats.as_ref().map(|s| s.games).unwrap_or(0.0),
            minutes_per_game: stats.as_ref().map(|s| s.mpg).unwrap_or(0.0),
            pts: stats.as_ref().map(|s| s.pts).unwrap_or(0.0),
            trb: stats.as_ref().map(|s| s.trb).unwrap_or(0.0),
            ast: stats.as_ref().map(|s| s.ast).unwrap_or(0.0),
            stl: stats.as_ref().map(|s| s.stl).unwrap_or(0.0),
            blk: stats.as_ref().map(|s| s.blk).unwrap_or(0.0),
            tov: stats.as_ref().map(|s| s.tov).unwrap_or(0.0),
            fg_pct: stats.as_ref().map(|s| s.fg_pct).unwrap_or(0.0),
            three_pct: stats.as_ref().map(|s| s.three_pct).unwrap_or(0.0),
            ft_pct: stats.as_ref().map(|s| s.ft_pct).unwrap_or(0.0),
            usage: None,
        });
    }
    Ok(out)
}

fn names_match(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphabetic())
            .flat_map(|c| c.to_lowercase())
            .collect()
    }
    norm(a) == norm(b)
}

fn strip_table_comments(html: &str) -> String {
    // BBRef wraps `<table>...</table>` blocks in `<!-- ... -->` to evade
    // simple ad blockers. We only need to unwrap the comment around tables.
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        if let Some(end) = after.find("-->") {
            out.push_str(&after[..end]);
            rest = &after[end + 3..];
        } else {
            // unmatched comment — bail out and keep remaining text
            out.push_str("<!--");
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone)]
struct RosterRow {
    name: String,
    position: String,
    age: Option<u8>,
}

fn parse_roster(doc: &Html) -> Result<Vec<RosterRow>> {
    let table_sel = Selector::parse("table#roster").map_err(|e| anyhow!("selector: {e:?}"))?;
    let row_sel = Selector::parse("tbody tr").map_err(|e| anyhow!("selector: {e:?}"))?;

    let mut rows = Vec::new();
    let Some(table) = doc.select(&table_sel).next() else {
        return Ok(rows);
    };
    for tr in table.select(&row_sel) {
        let name = cell_text(&tr, "player").unwrap_or_default();
        let position = cell_text(&tr, "pos").unwrap_or_default();
        // Roster table no longer carries a direct `age` column — derive it
        // from `birth_date` (csk encoded as YYYYMMDD on BBRef) against the
        // current season's Feb 1 reference date (BBRef's NBA convention).
        let age = roster_age_from_tr(&tr);
        if name.trim().is_empty() {
            continue;
        }
        rows.push(RosterRow { name, position, age });
    }
    Ok(rows)
}

fn roster_age_from_tr(tr: &scraper::ElementRef<'_>) -> Option<u8> {
    // Try direct `age` first (very old caches), then derive from birth_date.
    if let Some(s) = cell_text(tr, "age") {
        if let Ok(n) = s.trim().parse::<u8>() {
            return Some(n);
        }
    }
    let csk_sel = Selector::parse(r#"[data-stat="birth_date"]"#).ok()?;
    let cell = tr.select(&csk_sel).next()?;
    // Prefer the `csk="19980902"` numeric form — easier to parse.
    if let Some(csk) = cell.value().attr("csk") {
        if csk.len() >= 4 {
            if let Ok(year) = csk[..4].parse::<i32>() {
                let now_year: i32 = chrono::Utc::now().date_naive().format("%Y").to_string().parse().unwrap_or(2026);
                let age = (now_year - year).clamp(17, 50);
                return Some(age as u8);
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
struct PerGameRow {
    name: String,
    age: Option<u8>,
    games: f32,
    mpg: f32,
    pts: f32,
    trb: f32,
    ast: f32,
    stl: f32,
    blk: f32,
    tov: f32,
    fg_pct: f32,
    three_pct: f32,
    ft_pct: f32,
}

fn parse_per_game(doc: &Html) -> Result<Vec<PerGameRow>> {
    // BBRef uses `per_game_stats`, `per_game`, or `per_game-team` ids
    // depending on era; try the modern one first.
    let candidates = ["table#per_game_stats", "table#per_game", "table#per_game-team"];
    let row_sel = Selector::parse("tbody tr").map_err(|e| anyhow!("selector: {e:?}"))?;

    for sel_str in candidates {
        let table_sel = Selector::parse(sel_str).map_err(|e| anyhow!("selector: {e:?}"))?;
        if let Some(table) = doc.select(&table_sel).next() {
            let mut out = Vec::new();
            for tr in table.select(&row_sel) {
                // BBRef switched the player-column data-stat to `name_display`
                // in 2024+ tables; fall back to legacy `player` for older
                // cached HTML.
                let name = cell_text(&tr, "name_display")
                    .or_else(|| cell_text(&tr, "player"))
                    .unwrap_or_default();
                if name.trim().is_empty() {
                    continue;
                }
                out.push(PerGameRow {
                    name,
                    age: cell_text(&tr, "age").and_then(|s| s.trim().parse::<u8>().ok()),
                    // BBRef per_game uses `games` (full word) not `g`.
                    games: f_any(&tr, &["games", "g"]),
                    mpg: f(&tr, "mp_per_g"),
                    pts: f(&tr, "pts_per_g"),
                    trb: f(&tr, "trb_per_g"),
                    ast: f(&tr, "ast_per_g"),
                    stl: f(&tr, "stl_per_g"),
                    blk: f(&tr, "blk_per_g"),
                    tov: f(&tr, "tov_per_g"),
                    fg_pct: f(&tr, "fg_pct"),
                    three_pct: f(&tr, "fg3_pct"),
                    ft_pct: f(&tr, "ft_pct"),
                });
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
    }
    Ok(vec![])
}

fn f_any(tr: &scraper::ElementRef<'_>, stats: &[&str]) -> f32 {
    for s in stats {
        if let Some(v) = cell_text(tr, s).and_then(|s| s.trim().parse::<f32>().ok()) {
            return v;
        }
    }
    0.0
}

fn cell_text(tr: &scraper::ElementRef<'_>, stat: &str) -> Option<String> {
    let s = format!("[data-stat=\"{stat}\"]");
    let sel = Selector::parse(&s).ok()?;
    let cell = tr.select(&sel).next()?;
    let txt: String = cell.text().collect::<String>().trim().to_string();
    if txt.is_empty() {
        None
    } else {
        Some(txt)
    }
}

fn f(tr: &scraper::ElementRef<'_>, stat: &str) -> f32 {
    cell_text(tr, stat)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_basic_comments() {
        let s = "before<!-- <table>X</table> -->after";
        assert_eq!(strip_table_comments(s), "before <table>X</table> after");
    }

    #[test]
    fn name_match_handles_punct() {
        assert!(names_match("De'Aaron Fox", "DeAaron Fox"));
        assert!(names_match("Luka Dončić", "Luka Dončić"));
        assert!(!names_match("Foo Bar", "Bar Foo"));
    }
}
