//! HoopsHype salary table scraper.
//!
//! HoopsHype is the v1 primary contracts source (server-rendered HTML, no
//! API, scraper-tolerated historically). We parse the season summary at
//! `https://hoopshype.com/salaries/players/` which has columns:
//!   PLAYER | 2025/26 | 2026/27 | 2027/28 | ... | TOTAL
//!
//! Salary cells look like `$45,640,084`. Empty cells are unsigned years.
//!
//! We deliberately keep this best-effort: every year we successfully parse
//! becomes a `ContractYear` (guaranteed=true unless we know otherwise);
//! exotic clauses (kickers, NTCs, options) come from `rating_overrides.toml`.

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use scraper::{Html, Selector};

use crate::cache::{html_ttl, Cache};
use crate::politeness::Fetcher;

#[derive(Debug, Clone)]
pub struct ContractRow {
    pub player_name: String,
    /// Per-year salary in cents, indexed from `start_year` (e.g. 2025-26 = year 0).
    pub salaries: Vec<i64>,
}

pub struct HoopsHypeSource<'a> {
    pub fetcher: &'a Fetcher,
    pub cache: &'a Cache,
}

impl<'a> HoopsHypeSource<'a> {
    pub fn fetch_all(&self) -> Result<Vec<ContractRow>> {
        let html_bytes = if let Some(b) = self.cache.get("hoopshype", "players", "html", html_ttl())
        {
            b
        } else {
            // The server-rendered table lives at this URL.
            let url = "https://hoopshype.com/salaries/players/";
            let bytes = self.fetcher.get(url).context("fetch hoopshype")?;
            self.cache.put("hoopshype", "players", "html", &bytes)?;
            bytes
        };
        let html = std::str::from_utf8(&html_bytes).context("hoopshype utf-8")?;
        parse_table(html)
    }
}

fn parse_table(html: &str) -> Result<Vec<ContractRow>> {
    let doc = Html::parse_document(html);
    // HoopsHype uses a `.hh-salaries-ranking-table` table; fall back to any
    // `<table>` if class names changed.
    let candidates = [
        "table.hh-salaries-ranking-table",
        "table.hh-salaries-table",
        "table",
    ];
    let row_sel = Selector::parse("tbody tr").map_err(|e| anyhow!("selector: {e:?}"))?;
    let cell_sel = Selector::parse("td").map_err(|e| anyhow!("selector: {e:?}"))?;
    let dollars = Regex::new(r"\$\s*([\d,]+)").unwrap();

    for sel_str in candidates {
        let table_sel = Selector::parse(sel_str).map_err(|e| anyhow!("selector: {e:?}"))?;
        if let Some(table) = doc.select(&table_sel).next() {
            let mut out = Vec::new();
            for tr in table.select(&row_sel) {
                let cells: Vec<String> = tr
                    .select(&cell_sel)
                    .map(|c| c.text().collect::<String>())
                    .collect();
                if cells.len() < 3 {
                    continue;
                }
                // Convention: first non-numeric cell is the rank, second is the
                // player name (or anchor); subsequent cells are dollar
                // amounts. We pick the first cell that is not pure-numeric
                // and not a dollar amount as the player name.
                let mut name: Option<String> = None;
                let mut salaries: Vec<i64> = Vec::new();
                for cell in &cells {
                    let trimmed = cell.trim();
                    if let Some(caps) = dollars.captures(trimmed) {
                        let digits: String =
                            caps[1].chars().filter(|c| c.is_ascii_digit()).collect();
                        if let Ok(d) = digits.parse::<i64>() {
                            // Cents (×100). HoopsHype displays whole dollars.
                            salaries.push(d.saturating_mul(100));
                            continue;
                        }
                    }
                    if name.is_none() {
                        // Skip pure-numeric rank cells.
                        if !trimmed
                            .chars()
                            .all(|c| c.is_ascii_digit() || c.is_whitespace())
                            && !trimmed.is_empty()
                        {
                            name = Some(trimmed.to_string());
                        }
                    }
                }
                if let Some(n) = name {
                    if !salaries.is_empty() {
                        // The last column is the TOTAL — drop it.
                        if salaries.len() > 1 {
                            salaries.pop();
                        }
                        out.push(ContractRow {
                            player_name: n,
                            salaries,
                        });
                    }
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
    }
    // No rows parsed; return empty so caller falls back to a hand-curated
    // table or skips contracts entirely.
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_table() {
        let html = r#"
            <table class="hh-salaries-ranking-table">
              <tbody>
                <tr><td>1</td><td>Stephen Curry</td><td>$59,606,817</td><td>$62,587,158</td><td>$122,193,975</td></tr>
                <tr><td>2</td><td>Joel Embiid</td><td>$55,224,526</td><td>$59,365,366</td><td>$114,589,892</td></tr>
              </tbody>
            </table>
        "#;
        let rows = parse_table(html).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].player_name, "Stephen Curry");
        assert_eq!(rows[0].salaries.len(), 2); // total dropped
        assert_eq!(rows[0].salaries[0], 59_606_817 * 100);
    }
}
