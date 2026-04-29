use crate::cache::Cache;
use anyhow::{anyhow, bail, Context, Result};
use scraper::{ElementRef, Html, Selector};
use std::path::PathBuf;
use std::time::Duration;

const FUTURE_PICKS_URL: &str = "https://www.spotrac.com/nba/draft/future";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct SpotracClient {
    http: reqwest::blocking::Client,
    cache: Cache,
    ttl: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPick {
    pub year: u16,
    pub round: u8,
    pub original_team_abbrev: String,
    pub current_owner_abbrev: String,
    pub is_swap: bool,
    pub protection_text: Option<String>,
}

impl SpotracClient {
    pub fn new(cache_dir: impl Into<PathBuf>, ttl: Duration) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()
            .context("build Spotrac HTTP client")?;
        Ok(Self {
            http,
            cache: Cache::new(cache_dir)?,
            ttl,
        })
    }

    pub fn fetch_future_picks(&self) -> Result<Vec<RawPick>> {
        let bytes = self.fetch_html()?;
        let html = std::str::from_utf8(&bytes).context("Spotrac future picks HTML is not UTF-8")?;
        parse_future_picks_html(html)
    }

    fn fetch_html(&self) -> Result<Vec<u8>> {
        let key = "future_picks";
        if std::env::var_os("NBA3K_SPOTRAC_NO_CACHE").is_none() {
            if let Some(bytes) = self.cache.get("spotrac", key, "html", self.ttl) {
                return Ok(bytes);
            }
        }
        let response = self
            .http
            .get(FUTURE_PICKS_URL)
            .send()
            .context("GET Spotrac NBA future draft picks")?;
        if !response.status().is_success() {
            bail!("Spotrac returned HTTP {}", response.status());
        }
        let bytes = response
            .bytes()
            .context("read Spotrac future picks")?
            .to_vec();
        self.cache.put("spotrac", key, "html", &bytes)?;
        Ok(bytes)
    }
}

pub fn parse_future_picks_html(html: &str) -> Result<Vec<RawPick>> {
    let doc = Html::parse_document(html);
    let pane_sel = Selector::parse("div.tab-pane").unwrap();
    let row_sel = Selector::parse("tr").unwrap();
    let h2_sel = Selector::parse("h2").unwrap();
    let img_sel = Selector::parse("img").unwrap();
    let cell_sel = Selector::parse("td.center[colspan]").unwrap();
    let div_sel = Selector::parse("div").unwrap();

    let mut picks = Vec::new();
    for pane in doc.select(&pane_sel) {
        let Some(round) = round_from_pane(&pane) else {
            continue;
        };
        let mut year: Option<u16> = None;
        for row in pane.select(&row_sel) {
            if let Some(h2) = row.select(&h2_sel).next() {
                let text = clean_text(h2.text());
                year = text.parse::<u16>().ok();
                continue;
            }
            let Some(year) = year else { continue };
            let Some(img) = row.select(&img_sel).next() else {
                continue;
            };
            let Some(src) = img.value().attr("src") else {
                continue;
            };
            let Some(original) = abbrev_from_logo(src) else {
                continue;
            };
            let Some(cell) = row.select(&cell_sel).next() else {
                continue;
            };
            let is_swap = row.html().contains("fa-refresh");
            let owner = owner_from_cell(&cell).ok_or_else(|| {
                anyhow!("Spotrac row missing owner for {year} R{round} {original}")
            })?;
            let protection_text = protection_from_cell(&cell, &div_sel);
            picks.push(RawPick {
                year,
                round,
                original_team_abbrev: original,
                current_owner_abbrev: owner,
                is_swap,
                protection_text,
            });
        }
    }
    if picks.is_empty() {
        bail!("Spotrac parser found zero future picks");
    }
    Ok(picks)
}

fn round_from_pane(pane: &ElementRef<'_>) -> Option<u8> {
    let id = pane.value().attr("id")?;
    if id.starts_with("round1_") {
        Some(1)
    } else if id.starts_with("round2_") {
        Some(2)
    } else {
        None
    }
}

fn protection_from_cell(cell: &ElementRef<'_>, div_sel: &Selector) -> Option<String> {
    cell.select(div_sel)
        .skip(1)
        .map(|d| clean_text(d.text()))
        .find(|s| !s.is_empty())
}

fn owner_from_cell(cell: &ElementRef<'_>) -> Option<String> {
    let text = clean_text(cell.text());
    text.split_whitespace()
        .filter_map(|token| {
            let raw = token
                .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_uppercase();
            if (2..=4).contains(&raw.len()) && raw.chars().all(|c| c.is_ascii_uppercase()) {
                normalize_spotrac_abbrev(&raw).map(str::to_string)
            } else {
                None
            }
        })
        .next()
}

fn abbrev_from_logo(src: &str) -> Option<String> {
    let marker = "nba_";
    let start = src.find(marker)? + marker.len();
    let tail = &src[start..];
    let raw = tail
        .split('.')
        .next()?
        .trim_end_matches(|c: char| c.is_ascii_digit());
    normalize_spotrac_abbrev(raw).map(str::to_string)
}

fn normalize_spotrac_abbrev(raw: &str) -> Option<&'static str> {
    Some(match raw.to_ascii_uppercase().as_str() {
        "ATL" => "ATL",
        "BOS" => "BOS",
        "BKN" | "BRK" => "BRK",
        "CHA" | "CHO" => "CHO",
        "CHI" => "CHI",
        "CLE" => "CLE",
        "DAL" => "DAL",
        "DEN" => "DEN",
        "DET" => "DET",
        "GS" | "GSW" => "GSW",
        "HOU" => "HOU",
        "IND" => "IND",
        "LAC" => "LAC",
        "LAL" => "LAL",
        "MEM" => "MEM",
        "MIA" => "MIA",
        "MIL" => "MIL",
        "MIN" => "MIN",
        "NO" | "NOP" => "NOP",
        "NY" | "NYK" => "NYK",
        "OKC" => "OKC",
        "ORL" => "ORL",
        "PHI" => "PHI",
        "PHX" | "PHO" => "PHO",
        "POR" => "POR",
        "SAC" => "SAC",
        "SA" | "SAS" => "SAS",
        "TOR" => "TOR",
        "UTA" | "UTAH" => "UTA",
        "WAS" | "WSH" => "WAS",
        _ => return None,
    })
}

fn clean_text<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    parts
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
