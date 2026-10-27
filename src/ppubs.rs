//! USPTO Patent Public Search (PPUBS) research campaigns.
//!
//! This is deliberately separate from the dead-only SerpApi session. It is a
//! **research** connector: it can return grants and published applications for
//! any assignee and date range, with no register-status claims attached. The
//! analyst uses it to build a complete organisation list; the graveyard map
//! still only ingests US-lapsed patents through the SerpApi path.
//!
//! PPUBS has no documented public API. The web application opens a short-lived
//! server session and sends the returned `x-access-token` on every search. We
//! mirror that flow here, server-side, so the browser never needs a PPUBS
//! session and no data-source credentials leak into the WASM bundle.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{Corpus, Patent, SourceInfo};

const PPUBS_BASE: &str = "https://ppubs.uspto.gov/api";
const PPUBS_REFERER: &str = "https://ppubs.uspto.gov/pubwebapp/";
const USER_AGENT: &str = "GraveTender/0.1 (patent research; contact: local analyst)";
const PAGE_SIZE: usize = 1000;
const MAX_RESULTS: usize = 20_000;

#[derive(Debug, thiserror::Error)]
pub enum PpsError {
    #[error("assignee is required")]
    EmptyAssignee,
    #[error("date range is invalid: {0}")]
    BadRange(String),
    #[error("USPTO PPUBS session did not return an access token")]
    NoToken,
    #[error("USPTO PPUBS returned an error: {0}")]
    Api(String),
    #[error("USPTO PPUBS request failed: {0}")]
    Http(String),
    #[error("could not parse USPTO PPUBS response: {0}")]
    Parse(String),
}

/// The browser sends this to `/api/research`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRequest {
    /// Canonical assignee name, e.g. `University of Rochester`.
    pub assignee: String,
    /// Optional extra name variants. The server also generates safe variants.
    #[serde(default)]
    pub variants: Vec<String>,
    /// Four-digit start year. Defaults to 1850.
    #[serde(default = "default_from")]
    pub from: String,
    /// Four-digit end year. Defaults to 2026.
    #[serde(default = "default_to")]
    pub to: String,
    /// Include US published applications (US-PGPUB) as well as grants.
    #[serde(default = "default_true")]
    pub include_applications: bool,
    /// Add a broad token query (for example `(University SAME Rochester)`) so
    /// misspellings and unlisted variants are found. Defaults to true.
    #[serde(default = "default_true")]
    pub auto_variants: bool,
}

fn default_from() -> String {
    "1850".to_string()
}
fn default_to() -> String {
    "2026".to_string()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VariantCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResearchStats {
    pub total: usize,
    pub grants: usize,
    pub applications: usize,
    pub from: String,
    pub to: String,
    pub source_queries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResponse {
    pub ok: bool,
    pub query: String,
    pub query_variants: Vec<String>,
    pub corpus: Corpus,
    pub variants: Vec<VariantCount>,
    pub stats: ResearchStats,
    pub warnings: Vec<String>,
}

/// One PPUBS session. It is intentionally short-lived; a new campaign opens a
/// new session so a stale token can never linger in the server process.
struct Session {
    client: reqwest::Client,
    token: String,
    case_id: u64,
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    #[serde(rename = "userCase")]
    user_case: UserCase,
}

#[derive(Debug, Deserialize)]
struct UserCase {
    #[serde(rename = "caseId")]
    case_id: u64,
}

#[derive(Debug, Deserialize)]
struct CountsResponse {
    #[serde(default)]
    id: u64,
    #[serde(default, rename = "numResults")]
    num_results: usize,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    patents: Vec<RawPatent>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPatent {
    #[serde(default)]
    guid: String,
    #[serde(default, rename = "documentId")]
    document_id: String,
    #[serde(default, rename = "publicationReferenceDocumentNumber")]
    publication_reference: String,
    #[serde(default, rename = "inventionTitle")]
    title: String,
    #[serde(default, rename = "assigneeName")]
    assignee_name: Option<Vec<String>>,
    #[serde(default, rename = "inventorsShort")]
    inventors_short: Option<String>,
    #[serde(default, rename = "datePublished")]
    date_published: Option<String>,
    #[serde(default, rename = "applicationFilingDate")]
    application_filing_date: Option<DateField>,
    #[serde(default, rename = "priorityClaimsDate")]
    priority_claims_date: Option<DateField>,
    #[serde(default, rename = "type")]
    document_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum DateField {
    One(String),
    Many(Vec<String>),
}

impl DateField {
    fn first(&self) -> Option<&str> {
        match self {
            Self::One(value) => non_empty(value),
            Self::Many(values) => values.iter().find_map(|value| non_empty(value)),
        }
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

impl Session {
    async fn open() -> Result<Self, PpsError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|err| PpsError::Http(err.to_string()))?;

        let response = client
            .post(format!("{PPUBS_BASE}/users/me/session"))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::REFERER, PPUBS_REFERER)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("12345")
            .send()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;

        let status = response.status();
        let token = response
            .headers()
            .get("x-access-token")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or(PpsError::NoToken)?;
        let body = response
            .text()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;
        if !status.is_success() {
            return Err(PpsError::Http(format!("{status}: {body}")));
        }
        let session: SessionResponse =
            serde_json::from_str(&body).map_err(|err| PpsError::Parse(err.to_string()))?;

        Ok(Self {
            client,
            token,
            case_id: session.user_case.case_id,
        })
    }

    async fn counts(&self, query: &str, databases: &[&str]) -> Result<CountsResponse, PpsError> {
        let body = query_body(self.case_id, query, databases);
        let response = self
            .client
            .post(format!("{PPUBS_BASE}/searches/counts"))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::REFERER, PPUBS_REFERER)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-access-token", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;
        if !status.is_success() {
            return Err(PpsError::Http(format!("{status}: {text}")));
        }
        let counts: CountsResponse =
            serde_json::from_str(&text).map_err(|err| PpsError::Parse(err.to_string()))?;
        if let Some(error) = counts.error {
            return Err(PpsError::Api(error.to_string()));
        }
        Ok(counts)
    }

    async fn search_page(
        &self,
        query: &str,
        databases: &[&str],
        query_id: u64,
        start: usize,
    ) -> Result<Vec<RawPatent>, PpsError> {
        let body = serde_json::json!({
            "start": start,
            "pageCount": PAGE_SIZE,
            "sort": "date_publ desc",
            "docFamilyFiltering": "noFiltering",
            "searchType": 1,
            "familyIdEnglishOnly": true,
            "familyIdFirstPreferred": "US-PGPUB",
            "familyIdSecondPreferred": "USPAT",
            "familyIdThirdPreferred": "FPRS",
            "showDocPerFamilyPref": "showEnglish",
            "queryId": query_id.to_string(),
            "tagDocSearch": false,
            "query": {
                "caseId": self.case_id.to_string(),
                "hl_snippets": "2",
                "op": "OR",
                "q": query,
                "queryName": query,
                "highlights": "1",
                "qt": "brs",
                "spellCheck": false,
                "viewName": "tile",
                "plurals": true,
                "britishEquivalents": true,
                "databaseFilters": databases
                    .iter()
                    .map(|name| serde_json::json!({ "databaseName": name, "countryCodes": [] }))
                    .collect::<Vec<_>>(),
                "searchType": 1,
                "ignorePersist": true,
                "userEnteredQuery": query,
            },
        });

        let response = self
            .client
            .post(format!("{PPUBS_BASE}/searches/searchWithBeFamily"))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::REFERER, PPUBS_REFERER)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-access-token", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| PpsError::Http(err.to_string()))?;
        if !status.is_success() {
            return Err(PpsError::Http(format!("{status}: {text}")));
        }
        let page: SearchResponse =
            serde_json::from_str(&text).map_err(|err| PpsError::Parse(err.to_string()))?;
        if let Some(error) = page.error {
            return Err(PpsError::Api(error.to_string()));
        }
        Ok(page.patents)
    }
}

fn query_body(case_id: u64, query: &str, databases: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "caseId": case_id.to_string(),
        "hl_snippets": "2",
        "op": "OR",
        "q": query,
        "queryName": query,
        "highlights": "1",
        "qt": "brs",
        "spellCheck": false,
        "viewName": "tile",
        "plurals": true,
        "britishEquivalents": true,
        "databaseFilters": databases
            .iter()
            .map(|name| serde_json::json!({ "databaseName": name, "countryCodes": [] }))
            .collect::<Vec<_>>(),
        "searchType": 1,
        "ignorePersist": false,
        "userEnteredQuery": query,
    })
}

/// Run one research campaign against PPUBS.
pub async fn campaign(request: &ResearchRequest) -> Result<ResearchResponse, PpsError> {
    let base = request.assignee.trim();
    if base.is_empty() {
        return Err(PpsError::EmptyAssignee);
    }

    let from = normalize_year(&request.from, 1850)?;
    let to = normalize_year(&request.to, 2026)?;
    if from > to {
        return Err(PpsError::BadRange(format!("{from} is after {to}")));
    }

    let query_variants = build_variants(base, &request.variants, request.auto_variants);
    let query = build_query(&query_variants, from, to);
    let databases = database_filters(request.include_applications);

    let session = Session::open().await?;
    let counts = session.counts(&query, &databases).await?;
    let total = counts.num_results.min(MAX_RESULTS);

    let mut raw_patents: Vec<RawPatent> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut start = 0usize;
    while start < total {
        let page = session
            .search_page(&query, &databases, counts.id, start)
            .await?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for patent in page {
            let id = patent_id(&patent);
            if id.is_empty() || !seen.insert(id) {
                continue;
            }
            raw_patents.push(patent);
        }
        start += page_len;
        if page_len < PAGE_SIZE && raw_patents.len() >= total {
            break;
        }
    }

    let variants = discover_variants(&raw_patents, base);
    let mut patents: Vec<Patent> = raw_patents.iter().filter_map(to_patent).collect();
    patents.sort_by(|a, b| {
        b.publication_date
            .cmp(&a.publication_date)
            .then_with(|| b.publication_number.cmp(&a.publication_number))
    });

    let grants = patents.iter().filter(|p| p.kind == "Grant").count();
    let applications = patents
        .iter()
        .filter(|p| p.kind == "Application")
        .count();
    let warnings = if request.include_applications {
        Vec::new()
    } else {
        vec!["Published applications are excluded; only granted patents are shown.".to_string()]
    };

    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();

    let corpus = Corpus {
        patents,
        generated_at,
        sources: vec![SourceInfo {
            query: query.clone(),
            total_results: counts.num_results as u64,
            retrieved: raw_patents.len(),
        }],
    };

    Ok(ResearchResponse {
        ok: true,
        query,
        query_variants,
        corpus,
        variants,
        stats: ResearchStats {
            total: raw_patents.len(),
            grants,
            applications,
            from: from.to_string(),
            to: to.to_string(),
            source_queries: 1,
        },
        warnings,
    })
}

fn normalize_year(value: &str, fallback: i32) -> Result<i32, PpsError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(fallback);
    }
    let year = value
        .get(0..4)
        .and_then(|part| part.parse::<i32>().ok())
        .filter(|year| (1800..=2200).contains(year))
        .ok_or_else(|| PpsError::BadRange(format!("{value:?} is not a four-digit year")))?;
    Ok(year)
}

fn database_filters(include_applications: bool) -> Vec<&'static str> {
    let mut databases = vec!["USPAT", "USOCR"];
    if include_applications {
        databases.push("US-PGPUB");
    }
    databases
}

/// Names the query will try. Exact phrases cover case and punctuation
/// differences; the broad `SAME` clause catches misspellings such as
/// "University o Rochester".
fn build_variants(base: &str, supplied: &[String], auto: bool) -> Vec<String> {
    let mut variants: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut add = |name: &str| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let key = name.to_ascii_lowercase();
        if seen.insert(key) {
            variants.push(name.to_string());
        }
    };

    add(base);
    for name in supplied {
        add(name);
    }

    let lower = base.to_ascii_lowercase();
    if lower.starts_with("the ") {
        if let Some(rest) = base.get(4..) {
            add(rest);
        }
    } else {
        add(&format!("The {base}"));
    }

    if lower.contains("university") {
        if !lower.contains("medical center") {
            add(&format!("{base} Medical Center"));
        }
        if !lower.contains("office of technology transfer") {
            add(&format!("{base}, Office of Technology Transfer"));
        }
    }

    if auto {
        if let Some(broad) = broad_variant(base) {
            add(&broad);
        }
    }

    variants
}

/// Use the two most distinctive tokens for a `SAME` query. This preserves the
/// organisation name while tolerating missing words and typos.
fn broad_variant(base: &str) -> Option<String> {
    let tokens = significant_tokens(base);
    if tokens.len() < 2 {
        return None;
    }
    Some(format!("({} SAME {})", tokens[0], tokens[1]))
}

fn significant_tokens(value: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "of", "and", "for", "at", "in", "on", "inc", "llc", "ltd", "corp",
        "corporation", "company", "co", "medical", "center", "centre", "school",
        "institute", "college", "office", "technology", "transfer", "research",
        "university",
    ];
    let mut tokens: Vec<String> = value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() > 1)
        .filter(|token| !STOP.contains(&token.as_str()))
        .collect();
    tokens.dedup();
    // `university` is a stop word, so put it back when it is the only useful
    // anchor. This keeps broad search useful for names like "University of X".
    if tokens.len() < 2 && value.to_ascii_lowercase().contains("university") {
        tokens.insert(0, "university".to_string());
    }
    tokens
}

fn build_query(variants: &[String], from: i32, to: i32) -> String {
    let clauses: Vec<String> = variants
        .iter()
        .map(|variant| {
            if variant.starts_with('(') && variant.ends_with(')') {
                format!("{variant}.ASNM.")
            } else {
                format!("\"{}\".ASNM.", escape_query(variant))
            }
        })
        .collect();
    format!(
        "({}) AND @PD>={}0101 AND @PD<={}1231",
        clauses.join(" OR "),
        from,
        to
    )
}

fn escape_query(value: &str) -> String {
    value
        .replace('\\', " ")
        .replace('"', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn patent_id(raw: &RawPatent) -> String {
    let source = if !raw.document_id.trim().is_empty() {
        &raw.document_id
    } else if !raw.guid.trim().is_empty() {
        &raw.guid
    } else {
        &raw.publication_reference
    };
    clean_id(source)
}

fn clean_id(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn to_patent(raw: &RawPatent) -> Option<Patent> {
    let id = patent_id(raw);
    if id.is_empty() {
        return None;
    }
    let publication_date = raw.date_published.as_deref().and_then(normalize_date);
    let filing_date = raw
        .application_filing_date
        .as_ref()
        .and_then(DateField::first)
        .and_then(normalize_date);
    let priority_date = raw
        .priority_claims_date
        .as_ref()
        .and_then(DateField::first)
        .and_then(normalize_date)
        .or_else(|| filing_date.clone());
    let document_type = raw.document_type.as_deref().unwrap_or_default();
    let kind = match document_type {
        "USPAT" | "USOCR" => "Grant",
        "US-PGPUB" => "Application",
        _ => "",
    };
    let assignee = raw
        .assignee_name
        .as_ref()
        .map(|names| names.join(", "))
        .unwrap_or_default();
    let inventor = raw.inventors_short.clone().unwrap_or_default();

    Some(Patent {
        id: id.clone(),
        publication_number: id.clone(),
        link: format!("https://patents.google.com/patent/{id}/en"),
        title: raw.title.trim().to_string(),
        abstract_text: String::new(),
        priority_date,
        filing_date,
        publication_date: publication_date.clone(),
        grant_date: (kind == "Grant").then(|| publication_date.clone()).flatten(),
        inventor,
        assignee,
        language: "en".to_string(),
        thumbnail: None,
        pdf: None,
        figures: Vec::new(),
        jurisdiction: "US".to_string(),
        country_status: Vec::new(),
        source_queries: vec!["USPTO Patent Public Search".to_string()],
        source: "USPTO Patent Public Search".to_string(),
        kind: kind.to_string(),
    })
}

fn normalize_date(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() >= 10 && value.as_bytes().get(4) == Some(&b'-') {
        Some(value[0..10].to_string())
    } else if value.len() >= 8 && value[0..8].chars().all(|c| c.is_ascii_digit()) {
        Some(format!(
            "{}-{}-{}",
            &value[0..4],
            &value[4..6],
            &value[6..8]
        ))
    } else {
        non_empty(value).map(str::to_string)
    }
}

fn discover_variants(patents: &[RawPatent], base: &str) -> Vec<VariantCount> {
    let tokens = significant_tokens(base);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for patent in patents {
        for name in patent.assignee_name.iter().flatten() {
            let normalized = name.to_ascii_lowercase();
            if tokens
                .iter()
                .all(|token| normalized.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == token))
            {
                *counts.entry(name.trim().to_string()).or_default() += 1;
            }
        }
    }
    let mut variants: Vec<VariantCount> = counts
        .into_iter()
        .map(|(name, count)| VariantCount { name, count })
        .collect();
    variants.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_three_required_variants_and_a_broad_query() {
        let variants = build_variants("University of Rochester", &[], true);
        let lower: Vec<String> = variants.iter().map(|v| v.to_ascii_lowercase()).collect();
        assert!(lower.contains(&"university of rochester".to_string()));
        assert!(lower.contains(&"the university of rochester".to_string()));
        assert!(lower.contains(&"university of rochester medical center".to_string()));
        assert!(lower.contains(&"(university same rochester)".to_string()));
    }

    #[test]
    fn date_range_is_applied() {
        let query = build_query(&["University of Rochester".to_string()], 1976, 2026);
        assert!(query.contains("@PD>=19760101"));
        assert!(query.contains("@PD<=20261231"));
    }

    #[test]
    fn misspelled_assignee_is_discovered() {
        let raw = vec![RawPatent {
            guid: "US-20130188409-A1".into(),
            document_id: "US 20130188409 A1".into(),
            publication_reference: "20130188409".into(),
            title: "Test".into(),
            assignee_name: Some(vec!["University o Rochester, Office of Technology Transfer".into()]),
            inventors_short: None,
            date_published: Some("2013-07-25T00:00:00Z".into()),
            application_filing_date: None,
            priority_claims_date: None,
            document_type: Some("US-PGPUB".into()),
        }];
        let variants = discover_variants(&raw, "University of Rochester");
        assert_eq!(variants.len(), 1);
        assert!(variants[0].name.contains("University o Rochester"));
    }
}
