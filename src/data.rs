//! Single-session patent store.
//!
//! One process, one session: everything ingested from SerpApi lives here, in
//! memory, for the life of the server. No disk writes — a restart starts a
//! fresh session. When this moves to a small AWS host later, this is the exact
//! seam where a durable store (SQLite, Redis, …) plugs in: keep the API of
//! `snapshot` / `apply` and back it with whatever storage that deployment uses.
//!
//! Corrections to earlier design: the corpus used to be a static, lazily
//! loaded directory export. It is now mutable session state, because the
//! analyst grows it live from the command line.

use std::sync::{OnceLock, RwLock};

use crate::model::Corpus;
use crate::model::SourceInfo;

static SESSION: OnceLock<RwLock<Corpus>> = OnceLock::new();

/// The session corpus, behind a lock (the server mutates it on every fetch).
pub fn session() -> &'static RwLock<Corpus> {
    SESSION.get_or_init(|| RwLock::new(Corpus::default()))
}

/// Read-only snapshot for server rendering. A clone is cheap for the data
/// sizes involved (hundreds of patents at most).
pub fn snapshot() -> Corpus {
    session().read().expect("session lock poisoned").clone()
}

/// Merge one fetched page into the session. De-duplication and the
/// provenance trail are handled by `ingest::merge_into`.
///
/// Returns how many publications were NEW (not already in the session).
pub fn apply(patents: Vec<crate::model::Patent>, info: SourceInfo) -> usize {
    let mut session = session().write().expect("session lock poisoned");
    crate::ingest::merge_into(&mut session, patents, info)
}

/// Session statistics for the dashboard.
pub fn corpus_stats() -> CorpusStats {
    let c = snapshot();
    let us_active = c
        .patents
        .iter()
        .filter(|p| matches!(p.us_status(), Some(crate::model::CountryStatus::Active)))
        .count();
    let us_graveyard = c.patents.iter().filter(|p| p.is_us_graveyard()).count();
    let us_absent = c.patents.iter().filter(|p| p.is_us_absent()).count();
    let jurisdictions: std::collections::BTreeSet<String> =
        c.patents.iter().map(|p| p.jurisdiction.clone()).collect();
    CorpusStats {
        total: c.len(),
        us_active,
        us_graveyard,
        us_absent,
        jurisdictions: jurisdictions.into_iter().collect(),
        sources: c.sources.clone(),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusStats {
    pub total: usize,
    pub us_active: usize,
    pub us_graveyard: usize,
    pub us_absent: usize,
    pub jurisdictions: Vec<String>,
    pub sources: Vec<crate::model::SourceInfo>,
}
