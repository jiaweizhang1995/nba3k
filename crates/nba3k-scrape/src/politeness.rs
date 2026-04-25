//! Global rate limiter + polite HTTP fetcher.
//!
//! - 1 request per 3 seconds (matches BBRef `Crawl-delay`).
//! - Up to 3 retries on 429/5xx with exponential backoff (3s, 9s, 27s).
//! - User-Agent identifies the project per BBRef community norms.
//!
//! The per-host gate is process-global; running multiple scrapers
//! concurrently is unsupported (and would violate ToS).

use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

const USER_AGENT: &str =
    "nba3k-claude/0.1.0 (personal use; +https://github.com/CarfagnoArcino/nba3k-claude)";

const MIN_INTERVAL: Duration = Duration::from_millis(3000);

static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

fn gate() {
    let mut last = LAST_REQUEST.lock().unwrap();
    if let Some(prev) = *last {
        let elapsed = prev.elapsed();
        if elapsed < MIN_INTERVAL {
            drop(last); // release before sleeping
            sleep(MIN_INTERVAL - elapsed);
            last = LAST_REQUEST.lock().unwrap();
        }
    }
    *last = Some(Instant::now());
}

pub struct Fetcher {
    client: reqwest::blocking::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .context("build reqwest client")?;
        Ok(Self { client })
    }

    /// GET a URL with rate limiting + retries. Returns the body bytes.
    pub fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut backoff = Duration::from_secs(3);
        for attempt in 0..3 {
            gate();
            tracing::info!(url, attempt, "GET");
            let resp = self.client.get(url).send();
            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return Ok(r.bytes().context("read body")?.to_vec());
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        tracing::warn!(?status, attempt, "transient HTTP error, retrying");
                        sleep(backoff);
                        backoff *= 3;
                        continue;
                    }
                    bail!("HTTP {} for {}", status, url);
                }
                Err(e) => {
                    tracing::warn!(?e, attempt, "transport error, retrying");
                    sleep(backoff);
                    backoff *= 3;
                }
            }
        }
        Err(anyhow!("3 retries exhausted for {url}"))
    }

    pub fn user_agent(&self) -> &'static str {
        USER_AGENT
    }
}
