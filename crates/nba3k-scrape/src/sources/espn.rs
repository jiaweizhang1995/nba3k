//! ESPN public JSON client. M31 — replaces the failed nba_api shellout
//! attempt with stable, no-Python endpoints.
//!
//! All endpoints are documented (semi-officially) at espn.com / api.espn.com
//! and have been stable across multiple seasons. None require auth.
//!
//! Politeness: 100 ms per request, separate gate from BBRef's 3 s. Retry on
//! 5xx and transport errors with backoffs [300ms, 800ms, 2s]. 404 is a real
//! "no data" answer (e.g. an off-day with no scoreboard) — caller handles.
//!
//! Caching: file-based via `cache::Cache`, JSON ext, custom TTLs:
//!  - teams / standings / player_stats: 12 h
//!  - scoreboard / roster: 6 h
//!  - news: 1 h

use crate::cache::Cache;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------

const URL_TEAMS: &str =
    "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/teams";
const URL_STANDINGS: &str =
    "https://site.web.api.espn.com/apis/v2/sports/basketball/nba/standings?season=";
const URL_SCOREBOARD: &str =
    "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/scoreboard?dates=";
const URL_ROSTER_PREFIX: &str =
    "https://site.web.api.espn.com/apis/site/v2/sports/basketball/nba/teams/";
const URL_PLAYER_STATS: &str = "https://site.web.api.espn.com/apis/common/v3/sports/basketball/nba/statistics/byathlete?seasontype=2&limit=600&season=";
const URL_NEWS_TRADES: &str =
    "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/news?type=Trade&limit=";

const USER_AGENT: &str =
    "nba3k-claude/0.1.0 (personal use; +https://github.com/CarfagnoArcino/nba3k-claude)";
const MIN_INTERVAL: Duration = Duration::from_millis(100);
static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

fn gate() {
    let mut last = LAST_REQUEST.lock().unwrap();
    if let Some(prev) = *last {
        let elapsed = prev.elapsed();
        if elapsed < MIN_INTERVAL {
            drop(last);
            sleep(MIN_INTERVAL - elapsed);
            last = LAST_REQUEST.lock().unwrap();
        }
    }
    *last = Some(Instant::now());
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .context("build reqwest client")
}

/// GET with retry + politeness gate. `Ok(Some(bytes))` on success, `Ok(None)`
/// on 404 (real "no data"), `Err` on transport / 5xx after retries.
fn fetch_url(url: &str) -> Result<Option<Vec<u8>>> {
    let cli = client()?;
    let backoffs = [
        Duration::from_millis(300),
        Duration::from_millis(800),
        Duration::from_millis(2000),
    ];
    for attempt in 0..backoffs.len() {
        gate();
        match cli.get(url).send() {
            Ok(r) => {
                let status = r.status();
                if status.is_success() {
                    return Ok(Some(r.bytes().context("read body")?.to_vec()));
                }
                if status.as_u16() == 404 {
                    return Ok(None);
                }
                if status.is_server_error() || status.as_u16() == 429 {
                    tracing::warn!(?status, attempt, url, "ESPN transient error, retrying");
                    sleep(backoffs[attempt]);
                    continue;
                }
                bail!("ESPN HTTP {} for {}", status, url);
            }
            Err(e) => {
                tracing::warn!(error=?e, attempt, url, "ESPN transport error, retrying");
                sleep(backoffs[attempt]);
            }
        }
    }
    Err(anyhow!("ESPN GET failed after retries: {url}"))
}

fn ttl_long() -> Duration {
    Duration::from_secs(60 * 60 * 12) // 12 h
}
fn ttl_med() -> Duration {
    Duration::from_secs(60 * 60 * 6) // 6 h
}
fn ttl_short() -> Duration {
    Duration::from_secs(60 * 60) // 1 h
}

fn cached_or_fetch(
    cache: &Cache,
    key: &str,
    ttl: Duration,
    url: &str,
) -> Result<Option<Vec<u8>>> {
    if let Some(b) = cache.get("espn", key, "json", ttl) {
        return Ok(Some(b));
    }
    let bytes = fetch_url(url)?;
    if let Some(ref b) = bytes {
        cache.put("espn", key, "json", b)?;
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspnTeam {
    pub id: u32,
    pub abbrev: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnStandingRow {
    pub abbrev: String,
    pub conf: String,
    pub w: u16,
    pub l: u16,
    pub conf_rank: u16,
    pub div: String,
    pub div_rank: u16,
    pub streak: i32,
    pub last10: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnGameRow {
    pub date: NaiveDate,
    pub home_abbrev: String,
    pub away_abbrev: String,
    pub home_pts: Option<u16>,
    pub away_pts: Option<u16>,
    pub completed: bool,
    pub home_record: Option<String>,
    pub away_record: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnRosterEntry {
    pub espn_id: u64,
    pub display_name: String,
    pub jersey: Option<String>,
    pub position: Option<String>,
    pub age: Option<u8>,
    pub height_in: Option<u16>,
    pub weight_lb: Option<u16>,
    pub injury_status: Option<String>,
    pub injury_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnPlayerSeasonStat {
    pub espn_id: u64,
    pub display_name: String,
    pub team_abbrev: String,
    pub gp: u16,
    pub mpg: f32,
    pub ppg: f32,
    pub rpg: f32,
    pub apg: f32,
    pub spg: f32,
    pub bpg: f32,
    pub fg_pct: f32,
    pub three_pct: f32,
    pub ft_pct: f32,
    pub ts_pct: f32,
    pub usage: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnNewsItem {
    pub published: DateTime<Utc>,
    pub headline: String,
    pub link: String,
    pub categories: Vec<String>,
}

// ---------------------------------------------------------------------------
// Fetch + parse helpers
// ---------------------------------------------------------------------------

pub fn fetch_teams(cache: &Cache) -> Result<Option<Vec<u8>>> {
    cached_or_fetch(cache, "teams", ttl_long(), URL_TEAMS)
}

pub fn parse_teams(bytes: &[u8]) -> Result<Vec<EspnTeam>> {
    let v: Value = serde_json::from_slice(bytes).context("teams json")?;
    let teams = v
        .pointer("/sports/0/leagues/0/teams")
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("ESPN teams payload missing teams array"))?;
    let mut out = Vec::with_capacity(30);
    for entry in teams {
        let t = entry.get("team").unwrap_or(entry);
        let id = t.get("id").and_then(|x| x.as_str()).and_then(|s| s.parse::<u32>().ok());
        let abbrev = t.get("abbreviation").and_then(|x| x.as_str()).map(|s| s.to_string());
        let name = t.get("displayName").and_then(|x| x.as_str()).map(|s| s.to_string());
        if let (Some(id), Some(abbrev), Some(name)) = (id, abbrev, name) {
            out.push(EspnTeam {
                id,
                abbrev,
                display_name: name,
            });
        }
    }
    if out.len() < 30 {
        bail!(
            "ESPN teams payload yielded {} teams (expected 30) — schema may have changed",
            out.len()
        );
    }
    Ok(out)
}

pub fn fetch_standings(cache: &Cache, season_year: u16) -> Result<Option<Vec<u8>>> {
    let url = format!("{URL_STANDINGS}{season_year}");
    cached_or_fetch(cache, &format!("standings_{season_year}"), ttl_long(), &url)
}

pub fn parse_standings(bytes: &[u8]) -> Result<Vec<EspnStandingRow>> {
    let v: Value = serde_json::from_slice(bytes).context("standings json")?;
    let conferences = v
        .get("children")
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("ESPN standings payload missing children[]"))?;
    let mut out = Vec::new();
    for conf in conferences {
        let conf_name = conf
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let entries = conf
            .pointer("/standings/entries")
            .and_then(|x| x.as_array())
            .ok_or_else(|| anyhow!("ESPN standings missing entries[]"))?;
        for e in entries {
            let abbrev = e
                .pointer("/team/abbreviation")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if abbrev.is_empty() {
                continue;
            }
            let stats = e.get("stats").and_then(|x| x.as_array());
            let mut row = EspnStandingRow {
                abbrev,
                conf: conf_name.clone(),
                w: 0,
                l: 0,
                conf_rank: 0,
                div: String::new(),
                div_rank: 0,
                streak: 0,
                last10: String::new(),
            };
            if let Some(stats) = stats {
                for s in stats {
                    let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    let val = s.get("value").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let display = s
                        .get("displayValue")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    match name {
                        "wins" => row.w = val as u16,
                        "losses" => row.l = val as u16,
                        "playoffSeed" | "conferenceRank" => {
                            row.conf_rank = val as u16;
                        }
                        "divisionRank" => row.div_rank = val as u16,
                        "streak" => row.streak = val as i32,
                        "last10" | "lasttengames" => row.last10 = display,
                        _ => {}
                    }
                }
            }
            out.push(row);
        }
    }
    if out.is_empty() {
        bail!("ESPN standings parse yielded zero rows — schema may have changed");
    }
    Ok(out)
}

pub fn fetch_scoreboard(cache: &Cache, date: NaiveDate) -> Result<Option<Vec<u8>>> {
    let key = format!("scoreboard_{}", date.format("%Y%m%d"));
    let url = format!("{URL_SCOREBOARD}{}", date.format("%Y%m%d"));
    cached_or_fetch(cache, &key, ttl_med(), &url)
}

pub fn parse_scoreboard(bytes: &[u8]) -> Result<Vec<EspnGameRow>> {
    let v: Value = serde_json::from_slice(bytes).context("scoreboard json")?;
    let events = match v.get("events").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Ok(vec![]), // off-day or empty payload
    };
    let mut out = Vec::with_capacity(events.len());
    for ev in events {
        let date_str = ev.get("date").and_then(|x| x.as_str()).unwrap_or("");
        // ESPN dates look like `2026-01-29T00:00Z` — RFC3339 minus seconds.
        // Take the first 10 chars (`YYYY-MM-DD`) and parse as a NaiveDate.
        if date_str.len() < 10 {
            continue;
        }
        let date = match NaiveDate::parse_from_str(&date_str[..10], "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };
        let comp = match ev
            .get("competitions")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
        {
            Some(c) => c,
            None => continue,
        };
        let competitors = match comp.get("competitors").and_then(|x| x.as_array()) {
            Some(c) => c,
            None => continue,
        };
        let mut home: Option<&Value> = None;
        let mut away: Option<&Value> = None;
        for c in competitors {
            match c.get("homeAway").and_then(|x| x.as_str()) {
                Some("home") => home = Some(c),
                Some("away") => away = Some(c),
                _ => {}
            }
        }
        let (Some(home), Some(away)) = (home, away) else {
            continue;
        };
        let completed = ev
            .pointer("/status/type/completed")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let home_abbrev = home
            .pointer("/team/abbreviation")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let away_abbrev = away
            .pointer("/team/abbreviation")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if home_abbrev.is_empty() || away_abbrev.is_empty() {
            continue;
        }
        let home_pts = home
            .get("score")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<u16>().ok());
        let away_pts = away
            .get("score")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<u16>().ok());
        let home_record = home
            .pointer("/records/0/summary")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let away_record = away
            .pointer("/records/0/summary")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        out.push(EspnGameRow {
            date,
            home_abbrev,
            away_abbrev,
            home_pts,
            away_pts,
            completed,
            home_record,
            away_record,
        });
    }
    Ok(out)
}

pub fn fetch_roster(cache: &Cache, espn_team_id: u32) -> Result<Option<Vec<u8>>> {
    let url = format!("{URL_ROSTER_PREFIX}{espn_team_id}/roster");
    cached_or_fetch(cache, &format!("roster_{espn_team_id}"), ttl_med(), &url)
}

pub fn parse_roster(bytes: &[u8]) -> Result<(String, Vec<EspnRosterEntry>)> {
    let v: Value = serde_json::from_slice(bytes).context("roster json")?;
    let abbrev = v
        .pointer("/team/abbreviation")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let athletes = v
        .get("athletes")
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("ESPN roster payload missing athletes[]"))?;
    let mut out = Vec::with_capacity(athletes.len());
    for a in athletes {
        let espn_id = a
            .get("id")
            .and_then(|x| x.as_str().or_else(|| x.as_u64().map(|_| "")))
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| a.get("id").and_then(|x| x.as_u64()))
            .unwrap_or(0);
        let display_name = a
            .get("displayName")
            .or_else(|| a.get("fullName"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if display_name.is_empty() {
            continue;
        }
        let jersey = a
            .get("jersey")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let position = a
            .pointer("/position/abbreviation")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let age = a.get("age").and_then(|x| x.as_u64()).map(|n| n as u8);
        let height_in = a.get("height").and_then(|x| x.as_u64()).map(|n| n as u16);
        let weight_lb = a.get("weight").and_then(|x| x.as_u64()).map(|n| n as u16);
        let mut injury_status = None;
        let mut injury_detail = None;
        if let Some(injuries) = a.get("injuries").and_then(|x| x.as_array()) {
            if let Some(first) = injuries.first() {
                injury_status = first
                    .get("status")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                injury_detail = first
                    .get("details")
                    .and_then(|x| x.get("detail"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
            }
        }
        out.push(EspnRosterEntry {
            espn_id,
            display_name,
            jersey,
            position,
            age,
            height_in,
            weight_lb,
            injury_status,
            injury_detail,
        });
    }
    Ok((abbrev, out))
}

pub fn fetch_player_stats(cache: &Cache, season_year: u16) -> Result<Option<Vec<u8>>> {
    let url = format!("{URL_PLAYER_STATS}{season_year}");
    cached_or_fetch(
        cache,
        &format!("player_stats_{season_year}"),
        ttl_long(),
        &url,
    )
}

pub fn parse_player_stats(bytes: &[u8]) -> Result<Vec<EspnPlayerSeasonStat>> {
    // Schema: top-level `categories[].names` enumerates stat names; each
    // per-athlete `categories[].values` is a parallel array. Match by
    // category name and array index.
    use std::collections::HashMap;

    let v: Value = serde_json::from_slice(bytes).context("player_stats json")?;
    let top_cats = v
        .get("categories")
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("ESPN player_stats payload missing categories[]"))?;
    // Build cat_name → Vec<stat_name>.
    let mut schema: HashMap<String, Vec<String>> = HashMap::new();
    for c in top_cats {
        let cname = c
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let names: Vec<String> = c
            .get("names")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|n| n.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        schema.insert(cname, names);
    }

    let athletes = v
        .get("athletes")
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("ESPN player_stats payload missing athletes[]"))?;
    let mut out = Vec::with_capacity(athletes.len());
    for a in athletes {
        let ath = a.get("athlete").unwrap_or(a);
        let espn_id = ath
            .get("id")
            .and_then(|x| x.as_str().and_then(|s| s.parse::<u64>().ok()).or(x.as_u64()))
            .unwrap_or(0);
        let display_name = ath
            .get("displayName")
            .or_else(|| ath.get("fullName"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if display_name.is_empty() {
            continue;
        }
        let team_abbrev = ath
            .get("teamShortName")
            .and_then(|x| x.as_str())
            .or_else(|| ath.pointer("/team/abbreviation").and_then(|x| x.as_str()))
            .or_else(|| a.pointer("/team/abbreviation").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        let mut row = EspnPlayerSeasonStat {
            espn_id,
            display_name,
            team_abbrev,
            gp: 0,
            mpg: 0.0,
            ppg: 0.0,
            rpg: 0.0,
            apg: 0.0,
            spg: 0.0,
            bpg: 0.0,
            fg_pct: 0.0,
            three_pct: 0.0,
            ft_pct: 0.0,
            ts_pct: 0.0,
            usage: 0.0,
        };
        if let Some(cats) = a.get("categories").and_then(|x| x.as_array()) {
            for c in cats {
                let cname = c.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let names = match schema.get(cname) {
                    Some(n) => n,
                    None => continue,
                };
                let values = match c.get("values").and_then(|x| x.as_array()) {
                    Some(v) => v,
                    None => continue,
                };
                for (i, name) in names.iter().enumerate() {
                    let val = values
                        .get(i)
                        .and_then(|x| x.as_f64())
                        .unwrap_or(0.0) as f32;
                    let key = format!("{cname}.{name}");
                    match key.as_str() {
                        "general.gamesPlayed" => row.gp = val as u16,
                        "general.avgMinutes" => row.mpg = val,
                        "general.avgRebounds" => row.rpg = val,
                        "offensive.avgPoints" => row.ppg = val,
                        "offensive.avgAssists" => row.apg = val,
                        "offensive.fieldGoalPct" => row.fg_pct = val / 100.0,
                        "offensive.threePointFieldGoalPct" => row.three_pct = val / 100.0,
                        "offensive.freeThrowPct" => row.ft_pct = val / 100.0,
                        "defensive.avgSteals" => row.spg = val,
                        "defensive.avgBlocks" => row.bpg = val,
                        _ => {}
                    }
                }
            }
        }
        // ts_pct = PTS / (2 * (FGA + 0.44 * FTA)) — but ESPN does not expose
        // per-game FGA/FTA in this view. Leave 0 and let the importer/UI
        // tolerate. usage = 0 likewise.
        out.push(row);
    }
    Ok(out)
}

pub fn fetch_news_trades(cache: &Cache, limit: u32) -> Result<Option<Vec<u8>>> {
    let url = format!("{URL_NEWS_TRADES}{limit}");
    cached_or_fetch(cache, "news_trades", ttl_short(), &url)
}

#[derive(Debug, Deserialize)]
struct NewsArticle {
    headline: String,
    published: String,
    #[serde(default)]
    categories: Vec<NewsCategory>,
    #[serde(default)]
    links: Option<NewsLinks>,
}

#[derive(Debug, Deserialize)]
struct NewsCategory {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NewsLinks {
    #[serde(default)]
    web: Option<NewsHref>,
}

#[derive(Debug, Deserialize)]
struct NewsHref {
    href: String,
}

#[derive(Debug, Deserialize)]
struct NewsRoot {
    #[serde(default)]
    articles: Vec<NewsArticle>,
}

pub fn parse_news_trades(bytes: &[u8]) -> Result<Vec<EspnNewsItem>> {
    let root: NewsRoot = serde_json::from_slice(bytes).context("news json")?;
    let mut out = Vec::with_capacity(root.articles.len());
    for a in root.articles {
        let published = match DateTime::parse_from_rfc3339(&a.published) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => continue,
        };
        let link = a
            .links
            .as_ref()
            .and_then(|l| l.web.as_ref())
            .map(|w| w.href.clone())
            .unwrap_or_default();
        let cats: Vec<String> = a
            .categories
            .into_iter()
            .filter_map(|c| c.description.or(c.kind))
            .collect();
        out.push(EspnNewsItem {
            published,
            headline: a.headline,
            link,
            categories: cats,
        });
    }
    Ok(out)
}
