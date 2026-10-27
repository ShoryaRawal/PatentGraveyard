//! # PatentGraveyard
//!
//! A Rust + Leptos application that lets a VC fund mine the "patent graveyard"
//! — patents whose US rights have lapsed while the underlying invention may
//! still be protected elsewhere — and explore similar patents as an interactive
//! graph, in the spirit of ResearchRabbit for research papers.
//!
//! ## Architecture (Leptos islands)
//!
//! * [`app`] renders the "sea" of server-rendered HTML. Everything in a plain
//!   `#[component]` runs **only on the server** in islands mode, so it can read
//!   the corpus directly with no serialization cost and no WASM weight.
//! * [`islands`] contains the small, interactive `#[island]` components. Only
//!   these are compiled to `wasm32` and hydrated in the browser: the explorer
//!   controls, the similarity graph (custom canvas), the detail panel, and the
//!   portfolio filter.
//! * [`data`] is the single in-memory session store: everything the analysts
//!   fetch from SerpApi in one session lives here, until the process restarts.
//! * [`discovery`] is the recommendation engine — citation-network-style,
//!   explainable, no LLM. See that module for how it maps ResearchRabbit's model
//!   onto patents.
//!
//! This split is the whole point of the exercise: a partner opening the app gets
//! a fully-rendered page instantly (SSR), and pays for interactivity only where
//! they actually interact.

// Deeply nested `view!` trees (the entity profile editor) overflow the default
// monomorphisation depth in release builds.
#![recursion_limit = "1024"]

#[cfg(feature = "ssr")]
pub mod agent;
pub mod app;
pub mod canvas;
pub mod data;
pub mod discovery;
pub mod ingest;
pub mod islands;
#[cfg(feature = "ssr")]
pub mod mobile;
pub mod model;
#[cfg(feature = "ssr")]
pub mod ppubs;
#[cfg(feature = "ssr")]
pub mod serpapi;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    // Islands mode: hydrate each island independently instead of the whole body.
    leptos::mount::hydrate_islands();
    let _ = app::App;
}
