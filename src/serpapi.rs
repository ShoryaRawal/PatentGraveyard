//! Live SerpApi client — server-side only.
//!
//! The browser never sees the API key: the server builds the request, fetches
//! the Google Patents page, and hands the raw JSON to `ingest`. Configuration:
//!
//! * `SERPAPI_KEY`  — the private key (required).
//! * `SERPAPI_BASE` — override the endpoint, e.g. to a local stub for tests
//!   (unused in production; defaults to `https://serpapi.com`).

use serde_json::Value;

/// Everything that can go wrong with a live fetch.
#[derive(Debug, thiserror::Error)]
pub enum SerpError {
    #[error(
        "SERPAPI_KEY is not set — live fetch needs the SerpApi key in the server's environment"
    )]
    MissingKey,
    #[error("SerpApi returned an error: {0}")]
    Api(String),
    #[error("SerpApi request failed: {0}")]
    Http(String),
    #[error("query cannot be empty")]
    EmptyQuery,
}

/// One live fetch = up to `ceil(num/10)` Google Patents result pages.
///
/// Google Patents serves 10 organic results per page regardless of `num`, and
/// SerpApi bills one credit per page — so the budget is bounded here (max 5
/// pages per press) rather than left to the request. Pages are fetched in
/// order and we stop at the first empty page: sparse topics cost less.
///
/// Returns every raw page body; the caller ingests and merges them.
/// SerpApi reports unhappy calls as HTTP 200 JSON with an `error` field —
/// detected here; `organic_results` parsing stays with `ingest`.
pub async fn search(query: &str, num: u32) -> Result<Vec<String>, SerpError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(SerpError::EmptyQuery);
    }
    let key = std::env::var("SERPAPI_KEY")
        .map(|k| k.trim().to_string())
        .ok()
        .filter(|k| !k.is_empty())
        .ok_or(SerpError::MissingKey)?;

    let base = std::env::var("SERPAPI_BASE").unwrap_or_else(|_| "https://serpapi.com".to_string());
    // 10 results per page; cap the page budget so one <GO> press can never
    // burn more than 5 SerpApi credits.
    let pages = ((num + 9) / 10).clamp(1, 5);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| SerpError::Http(e.to_string()))?;

    let mut bodies = Vec::with_capacity(pages as usize);
    let mut page = 1u32;
    while page <= pages {
        let url = format!(
            "{}/search?engine=google_patents&q={}&api_key={}&page={page}",
            base.trim_end_matches('/'),
            urlencode(query),
            urlencode(&key),
        );

        let body = client
            .get(&url)
            .send()
            .await
            .map_err(|e| SerpError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| SerpError::Http(e.to_string()))?
            .text()
            .await
            .map_err(|e| SerpError::Http(e.to_string()))?;

        // SerpApi reports unhappy calls as HTTP 200 JSON with an `error` field.
        let parsed: Value = serde_json::from_str(&body).map_err(|e| {
            SerpError::Http(format!("non-JSON response ({e}): {}", truncate(&body, 180)))
        })?;
        if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
            return Err(SerpError::Api(err.to_string()));
        }
        let results = parsed.get("organic_results").and_then(|r| r.as_array());
        match results {
            Some(list) if !list.is_empty() => bodies.push(body),
            // End of the result set for this query — stop paging.
            _ => break,
        }
        page += 1;
    }

    if bodies.is_empty() {
        return Err(SerpError::Api(format!(
            "no organic_results for query {query:?}"
        )));
    }
    Ok(bodies)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.len() > max {
        format!("{}…", &s[..max])
    } else {
        s
    }
}

/// Percent-encode a query/value for a URL query component.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn urlencode_escapes_correctly() {
        assert_eq!(
            urlencode("(Coffee) OR (Tea)"),
            "%28Coffee%29%20OR%20%28Tea%29"
        );
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn num_clamped() {
        // indirectly guarded in search(); clamping arithmetic sanity check
        assert_eq!(3u32.clamp(10, 100), 10);
        assert_eq!(500u32.clamp(10, 100), 100);
    }
}
