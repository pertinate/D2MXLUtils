//! Backend for the in-game Median XL item database search overlay.
//! This module calls the public item API, normalizes responses for the Svelte
//! UI, caches successful lookups, and rate-limits uncached requests.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const API_URL: &str = "https://tsw.vn.cz/stats/api_item.php";
const RATE_LIMIT_COUNT: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(10);
const TOO_MANY_MATCHES_MESSAGE: &str = "Too many matches. Keep typing to narrow results.";
const RATE_LIMIT_MESSAGE: &str = "Search is cooling down. Try again in a few seconds.";
const SEARCH_FAILED_MESSAGE: &str = "Search failed. Check connection and try again.";
const ITEM_NOT_FOUND_MESSAGE: &str = "Item not found";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MxlItemDetail {
    pub name: String,
    pub name_display: String,
    pub quality: String,
    pub class: String,
    pub type_name: String,
    pub runeword: String,
    pub runeword_level: String,
    pub stats: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MxlItemEntry {
    pub name: String,
    pub quality: String,
    pub class: String,
    pub type_name: String,
    pub detail: Option<MxlItemDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MxlItemSearchResult {
    Results {
        query: String,
        entries: Vec<MxlItemEntry>,
        message: Option<String>,
    },
    NotFound {
        query: String,
        message: String,
    },
    RateLimited {
        message: String,
        retry_after_ms: u64,
    },
    Error {
        query: String,
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct RawApiResponse {
    ok: Option<bool>,
    query: Option<String>,
    count: Option<usize>,
    truncated: Option<bool>,
    items: Option<Vec<RawItem>>,
    matches: Option<Vec<RawMatch>>,
    message: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawItem {
    name: String,
    #[serde(default)]
    name_display: String,
    quality: String,
    class: String,
    #[serde(rename = "type", default)]
    type_name: String,
    #[serde(default)]
    runeword: String,
    #[serde(default)]
    runeword_level: String,
    #[serde(default)]
    stats: String,
}

#[derive(Debug, Deserialize)]
struct RawMatch {
    name: String,
    quality: String,
    class: String,
    #[serde(rename = "type", default)]
    type_name: String,
}

pub struct MxlItemApiState {
    cache: Mutex<HashMap<String, MxlItemSearchResult>>,
    limiter: Mutex<RequestWindow>,
    agent: ureq::Agent,
}

impl Default for MxlItemApiState {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            limiter: Mutex::new(RequestWindow::default()),
            agent: default_agent(),
        }
    }
}

#[derive(Default)]
struct RequestWindow {
    requests: VecDeque<Instant>,
}

impl RequestWindow {
    fn check(&mut self, now: Instant) -> Result<(), u64> {
        while let Some(front) = self.requests.front().copied() {
            if now.duration_since(front) >= RATE_LIMIT_WINDOW {
                self.requests.pop_front();
            } else {
                break;
            }
        }

        if self.requests.len() < RATE_LIMIT_COUNT {
            self.requests.push_back(now);
            return Ok(());
        }

        let oldest = self.requests.front().copied().unwrap_or(now);
        let retry_after = RATE_LIMIT_WINDOW.saturating_sub(now.duration_since(oldest));
        Err(retry_after.as_millis() as u64)
    }
}

fn normalize_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn percent_encode_query(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn default_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .timeout_connect(Some(Duration::from_secs(3)))
        .timeout_recv_body(Some(Duration::from_secs(5)))
        .build()
        .into()
}

fn result_is_cacheable(result: &MxlItemSearchResult) -> bool {
    !matches!(
        result,
        MxlItemSearchResult::RateLimited { .. } | MxlItemSearchResult::Error { .. }
    )
}

fn fetch_from_api(agent: &ureq::Agent, trimmed: &str) -> MxlItemSearchResult {
    let url = format!("{}?q={}", API_URL, percent_encode_query(trimmed));
    let body = match agent.get(&url).call() {
        Ok(response) => match response.into_body().read_to_string() {
            Ok(body) => body,
            Err(_) => {
                return MxlItemSearchResult::Error {
                    query: trimmed.to_string(),
                    message: SEARCH_FAILED_MESSAGE.to_string(),
                }
            }
        },
        Err(err) => {
            let text = err.to_string();
            if text.contains("404") {
                return MxlItemSearchResult::NotFound {
                    query: trimmed.to_string(),
                    message: ITEM_NOT_FOUND_MESSAGE.to_string(),
                };
            }
            return MxlItemSearchResult::Error {
                query: trimmed.to_string(),
                message: SEARCH_FAILED_MESSAGE.to_string(),
            };
        }
    };

    parse_api_response(trimmed, &body)
}

impl MxlItemApiState {
    fn cached_or_fetch(&self, query: &str, now: Instant) -> MxlItemSearchResult {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return MxlItemSearchResult::Error {
                query: String::new(),
                message: String::new(),
            };
        }

        let key = normalize_query(trimmed);
        match self.cache.lock() {
            Ok(cache) => {
                if let Some(cached) = cache.get(&key) {
                    return cached.clone();
                }
            }
            Err(_) => {
                return MxlItemSearchResult::Error {
                    query: trimmed.to_string(),
                    message: SEARCH_FAILED_MESSAGE.to_string(),
                }
            }
        }

        match self.limiter.lock() {
            Ok(mut limiter) => {
                if let Err(retry_after_ms) = limiter.check(now) {
                    return MxlItemSearchResult::RateLimited {
                        message: RATE_LIMIT_MESSAGE.to_string(),
                        retry_after_ms,
                    };
                }
            }
            Err(_) => {
                return MxlItemSearchResult::Error {
                    query: trimmed.to_string(),
                    message: SEARCH_FAILED_MESSAGE.to_string(),
                }
            }
        }

        let result = fetch_from_api(&self.agent, trimmed);
        if result_is_cacheable(&result) {
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(key, result.clone());
            }
        }
        result
    }
}

#[tauri::command]
pub fn search_mxl_items(
    query: String,
    state: tauri::State<MxlItemApiState>,
) -> MxlItemSearchResult {
    state.cached_or_fetch(&query, Instant::now())
}

fn parse_api_response(query: &str, body: &str) -> MxlItemSearchResult {
    let parsed: RawApiResponse = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return MxlItemSearchResult::Error {
                query: query.to_string(),
                message: SEARCH_FAILED_MESSAGE.to_string(),
            }
        }
    };

    if parsed.ok == Some(false)
        || parsed
            .error
            .as_deref()
            .map(|e| e.eq_ignore_ascii_case("Item not found."))
            .unwrap_or(false)
    {
        return MxlItemSearchResult::NotFound {
            query: query.to_string(),
            message: ITEM_NOT_FOUND_MESSAGE.to_string(),
        };
    }

    if let Some(items) = parsed.items {
        let entries = items
            .into_iter()
            .map(|item| {
                let name_display = if item.name_display.is_empty() {
                    item.name.clone()
                } else {
                    item.name_display.clone()
                };
                let detail = MxlItemDetail {
                    name: item.name.clone(),
                    name_display,
                    quality: item.quality.clone(),
                    class: item.class.clone(),
                    type_name: item.type_name.clone(),
                    runeword: item.runeword,
                    runeword_level: item.runeword_level,
                    stats: item.stats,
                };
                MxlItemEntry {
                    name: item.name,
                    quality: item.quality,
                    class: item.class,
                    type_name: item.type_name,
                    detail: Some(detail),
                }
            })
            .collect();
        return MxlItemSearchResult::Results {
            query: parsed.query.unwrap_or_else(|| query.to_string()),
            entries,
            message: None,
        };
    }

    if let Some(matches) = parsed.matches {
        let entries = matches
            .into_iter()
            .map(|m| MxlItemEntry {
                name: m.name,
                quality: m.quality,
                class: m.class,
                type_name: m.type_name,
                detail: None,
            })
            .collect();
        return MxlItemSearchResult::Results {
            query: parsed.query.unwrap_or_else(|| query.to_string()),
            entries,
            message: parsed
                .truncated
                .unwrap_or(false)
                .then(|| TOO_MANY_MATCHES_MESSAGE.to_string()),
        };
    }

    MxlItemSearchResult::Error {
        query: query.to_string(),
        message: SEARCH_FAILED_MESSAGE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_items_response() {
        let body = r#"{
          "ok": true,
          "query": "Azurewrath",
          "count": 1,
          "truncated": false,
          "items": [{
            "index": 1,
            "name": "Azurewrath",
            "name_display": "Azurewrath",
            "quality": "SU",
            "class": "Crystal Swords",
            "type": "Crystal Sword",
            "runeword": "",
            "runeword_level": "",
            "stats": "Required Level: 100\nSocketed (6)"
          }]
        }"#;

        let result = parse_api_response("Azurewrath", body);

        match result {
            MxlItemSearchResult::Results {
                entries, message, ..
            } => {
                assert_eq!(message, None);
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "Azurewrath");
                assert_eq!(entries[0].quality, "SU");
                assert_eq!(entries[0].class, "Crystal Swords");
                assert_eq!(entries[0].type_name, "Crystal Sword");
                assert_eq!(
                    entries[0].detail.as_ref().unwrap().stats,
                    "Required Level: 100\nSocketed (6)"
                );
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn parses_matches_response_with_friendly_truncated_message() {
        let body = r#"{
          "ok": true,
          "query": "sacred",
          "count": 3,
          "truncated": true,
          "matches": [
            { "index": 1, "name": "Sacred Charge", "quality": "Sacred Set", "class": "Barbarian One-Handed Axes", "type": "Hammerhead Axe" }
          ],
          "message": "More than 3 matches found. Refine q or request one result explicitly."
        }"#;

        let result = parse_api_response("sacred", body);

        match result {
            MxlItemSearchResult::Results {
                entries, message, ..
            } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "Sacred Charge");
                assert_eq!(entries[0].detail, None);
                assert_eq!(message, Some(TOO_MANY_MATCHES_MESSAGE.to_string()));
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn parses_item_not_found_response() {
        let body = r#"{ "ok": false, "error": "Item not found." }"#;

        let result = parse_api_response("missing", body);

        assert_eq!(
            result,
            MxlItemSearchResult::NotFound {
                query: "missing".to_string(),
                message: ITEM_NOT_FOUND_MESSAGE.to_string(),
            }
        );
    }

    #[test]
    fn invalid_json_returns_controlled_error() {
        let result = parse_api_response("bad", "not json");

        assert_eq!(
            result,
            MxlItemSearchResult::Error {
                query: "bad".to_string(),
                message: SEARCH_FAILED_MESSAGE.to_string(),
            }
        );
    }

    #[test]
    fn limiter_allows_ten_requests_per_window() {
        let start = Instant::now();
        let mut limiter = RequestWindow::default();

        for _ in 0..RATE_LIMIT_COUNT {
            assert_eq!(limiter.check(start), Ok(()));
        }

        assert!(limiter.check(start).is_err());
    }

    #[test]
    fn limiter_recovers_after_window() {
        let start = Instant::now();
        let mut limiter = RequestWindow::default();

        for _ in 0..RATE_LIMIT_COUNT {
            assert_eq!(limiter.check(start), Ok(()));
        }

        assert_eq!(limiter.check(start + RATE_LIMIT_WINDOW), Ok(()));
    }

    #[test]
    fn percent_encodes_spaces_and_apostrophes() {
        assert_eq!(
            percent_encode_query("Red Vex' Curse"),
            "Red%20Vex%27%20Curse"
        );
    }

    #[test]
    fn normalize_query_trims_and_lowercases() {
        assert_eq!(normalize_query("  Azurewrath  "), "azurewrath");
    }

    #[test]
    fn rate_limited_result_serializes_retry_after_as_camel_case() {
        let json = serde_json::to_value(MxlItemSearchResult::RateLimited {
            message: RATE_LIMIT_MESSAGE.to_string(),
            retry_after_ms: 1234,
        })
        .unwrap();

        assert_eq!(json["kind"], "rateLimited");
        assert_eq!(json["retryAfterMs"], 1234);
        assert!(json.get("retry_after_ms").is_none());
    }
}
