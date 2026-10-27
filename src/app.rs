//! The application shell and the server-rendered "sea" of HTML.
//!
//! Everything in this module that is **not** inside an `#[island]` runs only on
//! the server. That lets us read the corpus synchronously while rendering:
//! no loading spinners, no serialized-then-rehydrated data blobs, and no growth
//! in the WASM bundle as the dataset grows.
//!
//! Layout is a fixed, full-viewport terminal: a black brand bar, and a single
//! interactive *workspace* island carrying the command line, the tape, the
//! work face and the function keys. The page never scrolls — only the panels
//! inside the workspace do.

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};

use crate::islands::Workspace;

/// The HTML document shell. `islands=true` makes Leptos hydrate each island
/// independently rather than the entire body.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options islands=true/>
                <link rel="stylesheet" href="/pkg/main.css?v=24"/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

/// Serves `/pkg/*` with `Cache-Control: no-store`.
///
/// The island bundle's file names do not change between builds, so anything a
/// browser keeps from a previous build is wrong by definition — especially
/// during development, where a stale module makes the page *look* fine while
/// hydrating old code (i.e. a blank map over fresh CSS). Browsers never cache
/// `no-store` responses, so every load revalidates.
///
/// Server-only: axum lives behind the `ssr` feature.
#[cfg(feature = "ssr")]
pub async fn serve_pkg_no_store(
    axum::extract::Path(path): axum::extract::Path<String>,
    axum::extract::State(leptos::prelude::LeptosOptions {
        site_root,
        site_pkg_dir,
        ..
    }): axum::extract::State<leptos::prelude::LeptosOptions>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let file = std::path::Path::new(site_root.as_ref())
        .join(site_pkg_dir.as_ref())
        .join(&path);
    let Ok(bytes) = std::fs::read(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Only three kinds of files live in /pkg.
    let mime = if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".css") {
        "text/css"
    } else {
        "text/javascript"
    };
    axum::http::Response::builder()
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes))
        .unwrap()
}

/// GET /api/search?q=…&num=… — the live ingest endpoint.
///
/// The browser asks; the server (which alone holds `SERPAPI_KEY`) calls the
/// SerpApi Google Patents engine, merges the page into the session, and returns
/// the refreshed session snapshot: corpus + stats.
///
/// Errors surface as HTTP 502 with `{ ok: false, error }` so the command bar
/// can show the cause and fall back to its local behaviour.
#[cfg(feature = "ssr")]
pub async fn api_search(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Json};

    let query = params.get("q").map(|s| s.trim()).unwrap_or("");
    let num: u32 = params.get("num").and_then(|n| n.parse().ok()).unwrap_or(10);

    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "empty query" })),
        )
            .into_response();
    }

    let fetched = crate::serpapi::search(query, num).await;
    let pages = match fetched {
        Ok(pages) => pages,
        Err(err) => {
            eprintln!("serpapi fetch failed for {query:?}: {err}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "ok": false, "error": err.to_string() })),
            )
                .into_response();
        }
    };

    // The ingest layer owns the external schema; here we only own the session.
    // Every page of this press shares one provenance entry.
    let mut patents: Vec<crate::model::Patent> = Vec::new();
    let mut total_results: u64 = 0;
    let mut retrieved: usize = 0;
    for raw in &pages {
        let (mut page_patents, info) = match crate::ingest::ingest_document(raw) {
            Ok(doc) => doc,
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "ok": false, "error": format!("unparseable SerpApi page: {err}") })),
                )
                    .into_response();
            }
        };
        total_results += info.total_results;
        retrieved += info.retrieved;
        patents.append(&mut page_patents);
    }

    let info = crate::model::SourceInfo {
        query: query.to_string(),
        total_results,
        retrieved,
    };
    let added = crate::data::apply(patents, info);

    let snapshot = crate::data::snapshot();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "query": query,
            "added": added,
            "total": snapshot.len(),
            "lapsed": snapshot.patents.iter().filter(|p| p.is_us_graveyard()).count(),
            "corpus": snapshot,
            "stats": crate::data::corpus_stats(),
        })),
    )
        .into_response()
}

/// POST /api/research — an organisation-wide PPUBS campaign.
///
/// The browser sends `{ assignee, from, to, include_applications }`. The
/// server opens a short-lived PPUBS session, expands the assignee name,
/// paginates every result, de-duplicates publications, and returns a separate
/// research corpus. It never mutates the dead-only SerpApi session.
#[cfg(feature = "ssr")]
pub async fn api_research(
    axum::Json(request): axum::Json<crate::ppubs::ResearchRequest>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Json};

    if request.assignee.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "assignee is required" })),
        )
            .into_response();
    }

    match crate::ppubs::campaign(&request).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => {
            eprintln!("PPUBS research failed: {err}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "ok": false, "error": err.to_string() })),
            )
                .into_response()
        }
    }
}

/// POST /api/agent — the patent analyst chat.
///
/// The browser posts the visible transcript (`{ messages: [{role, content}] }`);
/// the server trims it, retrieves the relevant slice of the session, runs the
/// bounded tool loop against Groq, and replies with `{ ok, reply, citations }`.
/// Guardrail refusals are 200s with `ok: false` so the panel can show them as
/// normal analyst messages rather than transport errors.
#[cfg(feature = "ssr")]
pub async fn api_agent(
    axum::Json(request): axum::Json<crate::agent::AgentRequest>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Json};

    if request.messages.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "empty transcript" })),
        )
            .into_response();
    }

    match crate::agent::converse(request.messages).await {
        Ok(reply) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "reply": reply.reply,
                "citations": reply.citations,
                "model": reply.model,
                "steps": reply.steps,
                "research": reply.research,
            })),
        )
            .into_response(),
        Err(err) => {
            let status = match &err {
                crate::agent::AgentError::Refused(_) | crate::agent::AgentError::RateLimited => {
                    StatusCode::OK
                }
                crate::agent::AgentError::MissingKey => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_GATEWAY,
            };
            if status != StatusCode::OK {
                eprintln!("agent error: {err}");
            }
            (
                status,
                Json(serde_json::json!({ "ok": false, "error": err.to_string() })),
            )
                .into_response()
        }
    }
}

/// POST /api/strategy — 5-aspect decomposition, SerpApi harvest, and Groq synthesis.
#[cfg(feature = "ssr")]
pub async fn api_strategy(
    axum::Json(request): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Json};

    let prompt = request
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .trim();

    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "prompt is required" })),
        )
            .into_response();
    }

    match crate::agent::run_strategy(prompt).await {
        Ok(session) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "session": session })),
        )
            .into_response(),
        Err(err) => {
            eprintln!("strategy pipeline failed: {err}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "ok": false, "error": err })),
            )
                .into_response()
        }
    }
}

/// The root component. In islands mode this renders once, on the server.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
            // The version query busts any CSS the browser may have cached under the
    // constant /pkg file name — bump it when the stylesheet changes meaningfully.
    <Stylesheet id="leptos" href="/pkg/main.css?v=24"/>
            <Title text="PatentGraveyard — Patent Discovery for Venture Diligence"/>
            <div class="app">
                <TopBar/>
                <WorkspaceScreen/>
            </div>
        }
}

// ---------------------------------------------------------------------------
// Top bar — brand, at-a-glance KPIs, market badge, method disclosure.
// All server-rendered; none of this ships as WASM.
// ---------------------------------------------------------------------------

#[component]
fn TopBar() -> impl IntoView {
    // Shown so a hand-configured debug server (SERPAPI_BASE override) can
    // never masquerade as the live feed. The badge is computed once, at boot.
    let sandbox = std::env::var("SERPAPI_BASE")
        .map(|v| !v.trim().is_empty() && !v.contains("serpapi.com"))
        .unwrap_or(false);
    view! {
        <header class="topbar">
            <div class="brand">
                <span class="brand-mark">"PGY"</span>
                <div>
                    <div class="brand-name">"PATENTGRAVEYARD"</div>
                    <div class="brand-tag">"VENTURE DILIGENCE TERMINAL"</div>
                </div>
            </div>

            <div class="topbar-right">
                {if sandbox { view! {
                    <span class="market-badge sandbox-badge" title="SERPAPI_BASE is overridden — fetches hit a local stub, not SerpApi.">
                        "SANDBOX STUB — NOT LIVE"
                    </span>
                }.into_any() } else { view! { <span></span> }.into_any() }}
                <MethodDisclosure/>
                <span class="market-badge">
                    <span class="flag">"🇺🇸"</span>
                    "US market · country register"
                </span>
            </div>
        </header>
    }
}

/// The method explanation, in a native `<details>` element so it needs no
/// JavaScript and therefore no WASM.
#[component]
fn MethodDisclosure() -> impl IntoView {
    view! {
        <details class="method-pop">
            <summary>"ⓘ Strategic Diligence"</summary>
            <div class="method-panel">
                <h4>"How the Venture Diligence Engine Works"</h4>
                <p>
                    "Our multi-phase diligence pipeline turns founder questions into actionable IP commercialization plans:"
                </p>
                <div class="mm">
                    <div><b>"1. Deconstruct"</b>" — Groq model decomposes venture thesis into 5 strategic commercial outcomes."</div>
                    <div><b>"2. Harvest"</b>" — SerpApi executes precision Google Patents queries for each outcome."</div>
                    <div><b>"3. Classify"</b>" — US status verification flags expired graveyard patents free to build upon."</div>
                    <div><b>"4. Model & Synthesize"</b>" — Groq maps which patent helps where and constructs a phased commercial timeline."</div>
                </div>
            </div>
        </details>
    }
}

// ---------------------------------------------------------------------------
// Workspace — the single interactive island.
// ---------------------------------------------------------------------------

#[component]
fn WorkspaceScreen() -> impl IntoView {
    view! {
        <Workspace
            initial_session=crate::model::default_solar_strategy()
        />
    }
}
