//! GraveTender's mobile API.
//!
//! The desktop desk is an SSR terminal. The phone is a small, authenticated
//! assistant that talks to this endpoint instead of rendering the whole corpus.
//! Keys never leave the server, and the mobile client never spends SerpApi
//! credits: it works from the current session and its saved reports.
//!
//! Storage is deliberately modest: one SQLite file for reports. The live
//! patent session is still the same in-memory store the desktop uses.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::agent::{self, ReportKind};
use crate::data;
use crate::model::Patent;

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

static DB: OnceLock<Result<Mutex<Connection>, String>> = OnceLock::new();

fn open_db() -> Result<Mutex<Connection>, String> {
    let path = std::env::var("GRAVE_TENDER_DB").unwrap_or_else(|_| "gravetender.db".to_string());
    if let Some(parent) = Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create report directory: {e}"))?;
        }
    }
    let conn = Connection::open(&path)
        .map_err(|e| format!("could not open report database {path}: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS reports (
             id          TEXT PRIMARY KEY,
             kind        TEXT NOT NULL,
             title       TEXT NOT NULL,
             body_md     TEXT NOT NULL,
             citations   TEXT NOT NULL,
             model       TEXT NOT NULL,
             created_at  INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS reports_created_at
             ON reports(created_at DESC);",
    )
    .map_err(|e| format!("could not initialise report database: {e}"))?;
    Ok(Mutex::new(conn))
}

fn db() -> Result<&'static Mutex<Connection>, String> {
    DB.get_or_init(open_db).as_ref().map_err(|e| e.clone())
}

/// Called at boot so a broken database fails loudly in the server log rather
/// than on the first phone request.
pub fn init_storage() -> Result<(), String> {
    db().map(|_| ())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub citations: Vec<String>,
    pub model: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDetail {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body_md: String,
    pub citations: Vec<String>,
    pub model: String,
    pub created_at: i64,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn citations_from_json(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn list_reports_db(limit: usize) -> Result<Vec<ReportSummary>, String> {
    let conn = db()?;
    let conn = conn
        .lock()
        .map_err(|_| "report database lock poisoned".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, citations, model, created_at
             FROM reports ORDER BY created_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            let citations: String = row.get(3)?;
            Ok(ReportSummary {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                citations: citations_from_json(&citations),
                model: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn get_report_db(id: &str) -> Result<Option<ReportDetail>, String> {
    let conn = db()?;
    let conn = conn
        .lock()
        .map_err(|_| "report database lock poisoned".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, body_md, citations, model, created_at
             FROM reports WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([id]).map_err(|e| e.to_string())?;
    let Some(row) = rows.next().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let citations: String = row.get(4).map_err(|e| e.to_string())?;
    Ok(Some(ReportDetail {
        id: row.get(0).map_err(|e| e.to_string())?,
        kind: row.get(1).map_err(|e| e.to_string())?,
        title: row.get(2).map_err(|e| e.to_string())?,
        body_md: row.get(3).map_err(|e| e.to_string())?,
        citations: citations_from_json(&citations),
        model: row.get(5).map_err(|e| e.to_string())?,
        created_at: row.get(6).map_err(|e| e.to_string())?,
    }))
}

fn insert_report_db(
    kind: &str,
    title: &str,
    body_md: &str,
    citations: &[String],
    model: &str,
) -> Result<ReportDetail, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = now_millis();
    let citations_json = serde_json::to_string(citations).map_err(|e| e.to_string())?;
    let conn = db()?;
    let conn = conn
        .lock()
        .map_err(|_| "report database lock poisoned".to_string())?;
    conn.execute(
        "INSERT INTO reports (id, kind, title, body_md, citations, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, kind, title, body_md, citations_json, model, created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(ReportDetail {
        id,
        kind: kind.to_string(),
        title: title.to_string(),
        body_md: body_md.to_string(),
        citations: citations.to_vec(),
        model: model.to_string(),
        created_at,
    })
}

fn delete_report_db(id: &str) -> Result<bool, String> {
    let conn = db()?;
    let conn = conn
        .lock()
        .map_err(|_| "report database lock poisoned".to_string())?;
    let changed = conn
        .execute("DELETE FROM reports WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(changed > 0)
}

// ---------------------------------------------------------------------------
// Auth + response helpers
// ---------------------------------------------------------------------------

fn response_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": message.into() })),
    )
        .into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn check_auth(headers: &HeaderMap) -> Result<(), Response> {
    let expected = std::env::var("MOBILE_API_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            response_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Mobile access is not configured: set MOBILE_API_TOKEN on the server.",
            )
        })?;

    let got = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim();

    if constant_time_eq(got.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        let mut response = response_error(StatusCode::UNAUTHORIZED, "Invalid access token.");
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        );
        Err(response)
    }
}

// ---------------------------------------------------------------------------
// Payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PatentBrief {
    id: String,
    publication_number: String,
    title: String,
    jurisdiction: String,
    year: Option<i32>,
    link: String,
}

#[derive(Debug, Serialize)]
struct SessionSummary {
    total: usize,
    lapsed: usize,
    active: usize,
    absent: usize,
    generated_at: String,
    sources: Vec<String>,
    recent: Vec<PatentBrief>,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<agent::ClientTurn>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportRequest {
    pub kind: ReportKind,
    pub patent_id: Option<String>,
    pub topic: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/mobile/health — deliberately unauthenticated so load balancers and
/// the app's connection test can distinguish "down" from "bad token".
pub async fn health() -> Response {
    let storage = init_storage().is_ok();
    Json(serde_json::json!({
        "ok": storage,
        "app": "GraveTender",
        "storage": storage,
        "agent": std::env::var("GROQ_API_KEY").map(|k| !k.trim().is_empty()).unwrap_or(false),
    }))
    .into_response()
}

/// GET /api/mobile/bootstrap — a small, mobile-shaped snapshot.
pub async fn bootstrap(headers: HeaderMap) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }

    let corpus = data::snapshot();
    let lapsed = corpus
        .patents
        .iter()
        .filter(|p| p.is_us_graveyard())
        .count();
    let active = corpus
        .patents
        .iter()
        .filter(|p| matches!(p.us_status(), Some(crate::model::CountryStatus::Active)))
        .count();
    let absent = corpus.patents.iter().filter(|p| p.is_us_absent()).count();
    let recent = corpus
        .patents
        .iter()
        .filter(|p| p.is_us_graveyard())
        .take(12)
        .map(brief)
        .collect();
    let sources = corpus
        .sources
        .iter()
        .map(|s| s.query.clone())
        .filter(|q| !q.is_empty())
        .take(6)
        .collect();

    let reports = match list_reports_db(50) {
        Ok(reports) => reports,
        Err(err) => return response_error(StatusCode::INTERNAL_SERVER_ERROR, err),
    };

    Json(serde_json::json!({
        "ok": true,
        "app": "GraveTender",
        "session": SessionSummary {
            total: corpus.len(),
            lapsed,
            active,
            absent,
            generated_at: corpus.generated_at,
            sources,
            recent,
        },
        "reports": reports,
        "model": agent::active_model(),
    }))
    .into_response()
}

/// POST /api/mobile/chat — the same bounded agent as the desktop, with the
/// mobile transcript validation intact.
pub async fn chat(headers: HeaderMap, Json(request): Json<ChatRequest>) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }
    if request.messages.is_empty() {
        return response_error(StatusCode::BAD_REQUEST, "empty transcript");
    }

    match agent::converse(request.messages).await {
        Ok(reply) => Json(serde_json::json!({
            "ok": true,
            "reply": reply.reply,
            "citations": reply.citations,
            "model": reply.model,
            "steps": reply.steps,
        }))
        .into_response(),
        Err(err) => {
            let status = match &err {
                agent::AgentError::Refused(_) | agent::AgentError::RateLimited => StatusCode::OK,
                agent::AgentError::MissingKey => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_GATEWAY,
            };
            if status != StatusCode::OK {
                eprintln!("mobile agent error: {err}");
            }
            response_error(status, err.to_string())
        }
    }
}

/// POST /api/mobile/reports — generate and persist one report.
pub async fn create_report(
    headers: HeaderMap,
    Json(request): Json<CreateReportRequest>,
) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }

    match agent::generate_report(request.kind, request.patent_id, request.topic).await {
        Ok(draft) => match insert_report_db(
            request.kind.as_str(),
            &draft.title,
            &draft.body_md,
            &draft.citations,
            &draft.model,
        ) {
            Ok(report) => Json(serde_json::json!({ "ok": true, "report": report })).into_response(),
            Err(err) => response_error(StatusCode::INTERNAL_SERVER_ERROR, err),
        },
        Err(err) => {
            let status = match &err {
                agent::AgentError::Refused(_) | agent::AgentError::RateLimited => StatusCode::OK,
                agent::AgentError::MissingKey => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_GATEWAY,
            };
            if status != StatusCode::OK {
                eprintln!("mobile report error: {err}");
            }
            response_error(status, err.to_string())
        }
    }
}

/// GET /api/mobile/reports
pub async fn list_reports(headers: HeaderMap) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }
    match list_reports_db(100) {
        Ok(reports) => Json(serde_json::json!({ "ok": true, "reports": reports })).into_response(),
        Err(err) => response_error(StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

/// GET /api/mobile/reports/:id
pub async fn get_report(
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }
    match get_report_db(&id) {
        Ok(Some(report)) => {
            Json(serde_json::json!({ "ok": true, "report": report })).into_response()
        }
        Ok(None) => response_error(StatusCode::NOT_FOUND, "Report not found."),
        Err(err) => response_error(StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

/// DELETE /api/mobile/reports/:id
pub async fn delete_report(
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if let Err(response) = check_auth(&headers) {
        return response;
    }
    match delete_report_db(&id) {
        Ok(true) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(false) => response_error(StatusCode::NOT_FOUND, "Report not found."),
        Err(err) => response_error(StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

fn brief(p: &Patent) -> PatentBrief {
    PatentBrief {
        id: p.id.clone(),
        publication_number: p.publication_number.clone(),
        title: p.title.clone(),
        jurisdiction: p.jurisdiction.clone(),
        year: p.year(),
        link: p.link.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_rejects_prefixes_and_wrong_lengths() {
        assert!(constant_time_eq(b"grave-tender-dev", b"grave-tender-dev"));
        assert!(!constant_time_eq(b"grave-tender-de", b"grave-tender-dev"));
        assert!(!constant_time_eq(b"grave-tender-dev!", b"grave-tender-dev"));
        assert!(!constant_time_eq(b"", b"grave-tender-dev"));
    }

    #[test]
    fn report_json_round_trips_citations() {
        let citations = vec!["US10111111B2".to_string(), "US10777777B2".to_string()];
        let encoded = serde_json::to_string(&citations).unwrap();
        assert_eq!(citations_from_json(&encoded), citations);
        assert!(citations_from_json("not json").is_empty());
    }
}
