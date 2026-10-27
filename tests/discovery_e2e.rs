//! End-to-end tests over a live-shaped SerpApi fixture.
//!
//! The dummy directory export is gone: the session now inhales SerpApi
//! `google_patents` pages itself. These tests drive the same path the server
//! does — `ingest_document` → `session::apply` → discovery walk — against two
//! inline fixture responses, so the answers are validated on data the client
//! actually shipped us rather than a toy corpus.

use patent_graveyard::data::{apply, snapshot};
use patent_graveyard::discovery::{explore, DiscoveryConfig, RelationKind};
use patent_graveyard::ingest::ingest_document;

/// A SerpApi-shaped page: two strong queries' worth of organic results over a
/// couple of overlapping families (US lapsed / US in force / no US filing).
const QUERY_A: &str = r#"{
  "search_parameters": { "q": "solid state battery" },
  "search_information": { "total_results": 4 },
  "organic_results": [
    {
      "patent_link": "https://patents.google.com/patent/US10111111B2/en",
      "title": "Solid state electrolyte assembly",
      "snippet": "A bolted solid state electrolyte cell housing.",
      "filing_date": "2011-02-01",
      "publication_date": "2018-12-04",
      "grant_date": "2018-12-04",
      "inventor": "Ada Lovelace",
      "assignee": "Coil Batteries Ltd",
      "publication_number": "US10111111B2",
      "country_status": { "US": "NOT_ACTIVE", "EP": "ACTIVE", "KR": "NOT_ACTIVE" }
    },
    {
      "patent_link": "https://patents.google.com/patent/EP2666601A2/en",
      "title": "Solid state electrolyte assembly",
      "snippet": "A bolted solid state electrolyte cell housing.",
      "filing_date": "2011-02-01",
      "publication_date": "2013-12-04",
      "inventor": "Ada Lovelace",
      "assignee": "Coil Batteries Ltd",
      "publication_number": "EP2666601A2",
      "country_status": { "EP": "NOT_ACTIVE" }
    },
    {
      "patent_link": "https://patents.google.com/patent/US10777777B2/en",
      "title": "Ultrasonic welder for battery cells",
      "snippet": "Ultrasonic interconnect welder for foil cells.",
      "filing_date": "2012-03-02",
      "publication_date": "2020-01-02",
      "grant_date": "2020-01-02",
      "inventor": "Grace Hopper",
      "assignee": "Coil Batteries Ltd",
      "publication_number": "US10777777B2",
      "country_status": { "US": "ACTIVE" }
    }
  ]
}"#;

const QUERY_B: &str = r#"{
  "search_parameters": { "q": "perovskite solar cell" },
  "search_information": { "total_results": 3 },
  "organic_results": [
    {
      "patent_link": "https://patents.google.com/patent/US10222222B2/en",
      "title": "Perovskite module lamination line",
      "snippet": "Roll-to-roll perovskite lamination.",
      "filing_date": "2013-05-05",
      "publication_date": "2019-05-28",
      "grant_date": "2019-05-28",
      "inventor": "Alan Turing",
      "assignee": "Halide Solar Institute",
      "publication_number": "US10222222B2",
      "country_status": { "US": "NOT_ACTIVE", "WO": "ACTIVE" }
    },
    {
      "patent_link": "https://patents.google.com/patent/JP5050505A/en",
      "title": "Perovskite module lamination line",
      "snippet": "Roll-to-roll perovskite lamination.",
      "filing_date": "2013-05-05",
      "publication_date": "2014-03-18",
      "inventor": "Alan Turing",
      "assignee": "Halide Solar Institute",
      "publication_number": "JP5050505A",
      "country_status": { "JP": "NOT_ACTIVE" }
    },
    {
      "patent_link": "https://patents.google.com/patent/US10888888B2/en",
      "title": "Stabilised perovskite ink",
      "snippet": "Passivation additive stabilises a perovskite precursor ink.",
      "filing_date": "2019-01-09",
      "publication_date": "2021-03-11",
      "grant_date": "2021-03-11",
      "inventor": "Alan Turing",
      "assignee": "Halide Solar Institute",
      "publication_number": "US10888888B2",
      "country_status": { "US": "ACTIVE" }
    }
  ]
}"#;

/// Reset the global session to exactly the fixture pages and hand it back.
fn session_with_fixture() -> () {
    use patent_graveyard::data::session;
    *session().write().unwrap() = Default::default();
    for raw in [QUERY_A, QUERY_B] {
        let (patents, info) = ingest_document(raw).unwrap();
        apply(patents, info);
    }
}

#[test]
fn session_ingests_every_publication() {
    session_with_fixture();
    let c = snapshot();
    assert_eq!(
        c.len(),
        6,
        "expected all 6 fixture publications after merge"
    );
    assert_eq!(c.sources.len(), 2, "two source queries expected");
    // Every patent should have a non-empty title and a jurisdiction.
    for p in &c.patents {
        assert!(!p.title.is_empty(), "{} has no title", p.id);
        assert!(!p.jurisdiction.is_empty());
    }
}

#[test]
fn repeat_fetches_are_de_duplicated() {
    session_with_fixture();
    let (patents, info) = ingest_document(QUERY_A).unwrap();
    let added = apply(patents, info);
    assert_eq!(added, 0, "re-ingesting the same page must add nothing new");
    assert_eq!(snapshot().len(), 6);

    // Provenance replaces in place rather than stacking duplicates.
    assert_eq!(
        snapshot()
            .sources
            .iter()
            .filter(|s| s.query == "solid state battery")
            .count(),
        1
    );
}

#[test]
fn us_verdicts_partition_the_session() {
    session_with_fixture();
    let c = snapshot();
    let graveyard = c.patents.iter().filter(|p| p.is_us_graveyard()).count();
    let active = c
        .patents
        .iter()
        .filter(|p| {
            matches!(
                p.us_status(),
                Some(patent_graveyard::model::CountryStatus::Active)
            )
        })
        .count();
    let absent = c.patents.iter().filter(|p| p.is_us_absent()).count();

    // Every patent falls into exactly one bucket.
    assert_eq!(graveyard + active + absent, c.len());
    assert!(graveyard > 0, "the fixture must contain graveyard patents");
}

#[test]
fn a_seed_produces_explained_suggestions() {
    session_with_fixture();
    let c = snapshot();
    let seed = "US10111111B2".to_string();
    assert!(c.get(&seed).is_some(), "seed must exist in the session");

    let map = explore(&c, &[seed], DiscoveryConfig::default());
    assert!(
        !map.suggestions.is_empty(),
        "a family-linked seed must yield suggestions"
    );

    // Every suggestion must carry at least one reason.
    for s in &map.suggestions {
        assert!(
            !s.evidence.is_empty(),
            "suggestion {} has no explanation",
            s.id
        );
    }
}

#[test]
fn family_members_are_linked_across_jurisdictions() {
    session_with_fixture();
    let c = snapshot();
    // US10111111B2 and EP2666601A2 describe the same invention for the same
    // assignee — a family split that is the whole point of the product.
    let seed = "US10111111B2".to_string();
    let map = explore(&c, &[seed], DiscoveryConfig::default());
    let family = map.suggestions.iter().any(|s| s.id == "EP2666601A2")
        || map
            .suggestions
            .iter()
            .flat_map(|s| &s.evidence)
            .any(|e| e.kind == RelationKind::Family);
    assert!(family, "expected the EP sibling in the walked suggestions");
}

#[test]
fn suggestions_are_ranked_by_score() {
    session_with_fixture();
    let c = snapshot();
    let seeds = vec!["US10111111B2".to_string(), "US10222222B2".to_string()];
    let map = explore(&c, &seeds, DiscoveryConfig::default());
    let scores: Vec<f64> = map.suggestions.iter().map(|s| s.score).collect();
    for pair in scores.windows(2) {
        assert!(pair[0] >= pair[1], "suggestions must be sorted descending");
    }
}

#[test]
fn discovery_is_deterministic() {
    // The island renders on the server and re-renders during hydration. If the
    // engine's output were not reproducible (e.g. leaked `HashMap` ordering),
    // the two renders could disagree. This locks that down.
    session_with_fixture();
    let c = snapshot();
    let seeds = vec!["US10111111B2".to_string(), "US10222222B2".to_string()];
    let a = serde_json::to_string(&explore(&c, &seeds, DiscoveryConfig::default())).unwrap();
    let b = serde_json::to_string(&explore(&c, &seeds, DiscoveryConfig::default())).unwrap();
    assert_eq!(
        a, b,
        "the discovery walk must be reproducible for SSR hydration"
    );
}
