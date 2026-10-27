//! PatentGraveyard — server entrypoint & binary target.
#![recursion_limit = "1024"]
//!
//! In islands mode the server binary renders the "sea" of HTML and the tiny
//! islands are hydrated independently in the browser. All data loading happens
//! here, on the server, so the WASM bundle never grows with the dataset.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use patent_graveyard::app::shell;

    // Passing `Some("Cargo.toml")` makes Leptos read `[package.metadata.leptos]`
    // directly. Without it (i.e. `None`) Leptos expects the environment
    // variables that `cargo-leptos` would have injected, and falls back to
    // defaults — which would serve the islands from the wrong path.
    let conf = get_configuration(Some("Cargo.toml")).unwrap();
    let leptos_options = conf.leptos_options;
    // Caddy proxies to loopback on the server. For LAN testing on a physical
    // phone, `GRAVE_TENDER_ADDR=0.0.0.0:3000` opens the same server to the LAN.
    let addr = std::env::var("GRAVE_TENDER_ADDR")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(leptos_options.site_addr);
    let routes = {
        let opts = leptos_options.clone();
        generate_route_list(move || shell(opts.clone()))
    };

    // GraveTender's mobile API. Permissive CORS is safe here because every
    // route except /health requires the shared bearer token; Tauri's mobile
    // WebView uses a custom scheme origin that is not practical to enumerate.
    let mobile_api = Router::new()
        .route(
            "/health",
            axum::routing::get(patent_graveyard::mobile::health),
        )
        .route(
            "/bootstrap",
            axum::routing::get(patent_graveyard::mobile::bootstrap),
        )
        .route("/chat", axum::routing::post(patent_graveyard::mobile::chat))
        .route(
            "/reports",
            axum::routing::get(patent_graveyard::mobile::list_reports)
                .post(patent_graveyard::mobile::create_report),
        )
        .route(
            "/reports/{id}",
            axum::routing::get(patent_graveyard::mobile::get_report)
                .delete(patent_graveyard::mobile::delete_report),
        )
        .layer(tower_http::cors::CorsLayer::permissive());

    if let Err(err) = patent_graveyard::mobile::init_storage() {
        eprintln!("⚠️  GraveTender report storage unavailable: {err}");
    }

    let app = Router::new()
        .nest("/api/mobile", mobile_api)
        // Served before Leptos' default file handler, with `no-store`: the
        // wasm/js islands keep constant file names across rebuilds, and a
        // browser module cache would otherwise resurrect a STALE island after
        // every new build — the page looks alive but hydrates old code.
        .route(
            "/pkg/{*path}",
            axum::routing::get(patent_graveyard::app::serve_pkg_no_store),
        )
        // Live ingest: the command bar's <GO> calls this; the server holds the
        // SerpApi key and merges the page into the session before replying.
        .route(
            "/api/search",
            axum::routing::get(patent_graveyard::app::api_search),
        )
        // The patent agent. The browser posts the transcript; the server owns
        // the Groq key, the session context and the tool loop.
        .route(
            "/api/agent",
            axum::routing::post(patent_graveyard::app::api_agent),
        )
        // Phase 2 Strategic diligence pipeline: Groq 5 aspects + SerpApi searches + synthesis
        .route(
            "/api/strategy",
            axum::routing::post(patent_graveyard::app::api_strategy),
        )
        // Organisation research campaign: PPUBS grants + applications, with
        // assignee-variant expansion and de-duplication. This does not touch
        // the dead-only session corpus.
        .route(
            "/api/research",
            axum::routing::post(patent_graveyard::app::api_research),
        )
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    println!("🚀 PatentGraveyard listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
fn main() {
    // The binary is only used in SSR mode; the browser uses the wasm blob.
}
