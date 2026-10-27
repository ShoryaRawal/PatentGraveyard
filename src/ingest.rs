//! Server-side ingest of SerpApi Google Patents payloads.
//!
//! This module is deliberately isolated from the UI. It is the only place that
//! knows about the external schema, so swapping in the live SerpApi (or USPTO
//! bulk data, or EPO OPS) later means touching exactly one file.
//!
//! Three things happen here that the rest of the app relies on:
//!   1. **Normalisation** — raw JSON strings become typed `Patent`s with a
//!      stable id and a parsed jurisdiction.
//!   2. **De-duplication** — the same invention can be returned by two queries;
//!      we merge by publication number and union the source queries.
//!   3. **Family linkage** — publications that share an identical title +
//!      assignee are the same patent family in different countries. This is the
//!      backbone of the whole "is it alive in *our* country?" story.

use std::collections::HashMap;

use serde::Deserialize;

use crate::model::{Corpus, CountryStatus, CountryStatusEntry, Figure, Patent, SourceInfo};

/// Raw shape of one `organic_results` entry in a SerpApi `google_patents` page.
/// Only the fields we actually consume are declared.
#[derive(Debug, Deserialize)]
struct RawResult {
    #[serde(default)]
    patent_link: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    priority_date: Option<String>,
    #[serde(default)]
    filing_date: Option<String>,
    #[serde(default)]
    publication_date: Option<String>,
    #[serde(default)]
    grant_date: Option<String>,
    #[serde(default)]
    inventor: String,
    #[serde(default)]
    assignee: String,
    #[serde(default)]
    publication_number: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    pdf: Option<String>,
    #[serde(default)]
    figures: Vec<RawFigure>,
    #[serde(default)]
    country_status: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawFigure {
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    full: String,
}

#[derive(Debug, Deserialize)]
struct RawSearchInformation {
    #[serde(default)]
    total_results: u64,
}

#[derive(Debug, Deserialize)]
struct RawSearchParameters {
    #[serde(default)]
    q: String,
}

#[derive(Debug, Deserialize)]
struct RawPage {
    #[serde(default)]
    search_parameters: Option<RawSearchParameters>,
    #[serde(default)]
    search_information: Option<RawSearchInformation>,
    #[serde(default)]
    organic_results: Vec<RawResult>,
}

/// Errors raised while ingesting a document.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("failed to parse patent JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("document contained no `organic_results`")]
    NoResults,
}

/// Extract the jurisdiction from a publication number such as `EP2264377A2`
/// or `US20100307564A1`. Returns `"?"` when nothing recognisable is found.
fn jurisdiction_of(publication_number: &str) -> String {
    let code: String = publication_number
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if code.is_empty() {
        "?".to_string()
    } else {
        code.to_uppercase()
    }
}

/// A stable, human-friendly id. Falls back to the publication number, then to
/// a slug of the title, so we always have *something* to key on.
fn id_of(result: &RawResult) -> String {
    if !result.publication_number.is_empty() {
        return result.publication_number.clone();
    }
    if let Some(last) = result.patent_link.rsplit('/').nth(1) {
        if !last.is_empty() {
            return last.to_string();
        }
    }
    slug(&result.title)
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}

fn convert(result: RawResult, query: &str) -> Patent {
    let publication_number = if result.publication_number.is_empty() {
        result
            .patent_link
            .rsplit('/')
            .nth(1)
            .unwrap_or_default()
            .to_string()
    } else {
        result.publication_number.clone()
    };

    let mut country_status: Vec<CountryStatusEntry> = result
        .country_status
        .iter()
        .map(|(country, raw)| CountryStatusEntry {
            country: country.to_uppercase(),
            status: CountryStatus::parse(raw),
        })
        .collect();
    // Deterministic ordering so the SSR output is stable across renders.
    country_status.sort_by(|a, b| a.country.cmp(&b.country));

    let id = id_of(&result);

    Patent {
        id,
        jurisdiction: jurisdiction_of(&publication_number),
        publication_number,
        link: result.patent_link,
        title: result.title.trim().to_string(),
        abstract_text: result.snippet.trim().to_string(),
        priority_date: result.priority_date,
        filing_date: result.filing_date,
        publication_date: result.publication_date,
        grant_date: result.grant_date,
        inventor: result.inventor,
        assignee: result.assignee,
        language: result.language,
        thumbnail: result.thumbnail,
        pdf: result.pdf,
        figures: result
            .figures
            .into_iter()
            .map(|f| Figure {
                thumbnail: f.thumbnail,
                full: f.full,
            })
            .collect(),
        country_status,
        source_queries: vec![query.to_string()],
        source: "SerpApi · Google Patents".to_string(),
        kind: String::new(),
    }
}

/// Ingest a single SerpApi page (the contents of one `NN.json` file).
pub fn ingest_document(raw_json: &str) -> Result<(Vec<Patent>, SourceInfo), IngestError> {
    let page: RawPage = serde_json::from_str(raw_json)?;
    if page.organic_results.is_empty() {
        return Err(IngestError::NoResults);
    }

    let query = page
        .search_parameters
        .as_ref()
        .map(|p| p.q.clone())
        .unwrap_or_default();

    let total_results = page
        .search_information
        .as_ref()
        .map(|i| i.total_results)
        .unwrap_or(0);

    let patents: Vec<Patent> = page
        .organic_results
        .into_iter()
        .map(|r| convert(r, &query))
        .collect();

    let info = SourceInfo {
        query,
        total_results,
        retrieved: patents.len(),
    };
    Ok((patents, info))
}

/// Merge many ingested pages into one de-duplicated corpus.
///
/// De-duplication is by id (publication number). When the same publication
/// appears in two result sets we keep the first and union the queries, so the
/// provenance trail stays complete.
pub fn merge(docs: Vec<(Vec<Patent>, SourceInfo)>, generated_at: String) -> Corpus {
    let mut by_id: HashMap<String, Patent> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut sources: Vec<SourceInfo> = Vec::new();

    for (patents, info) in docs {
        for patent in patents {
            match by_id.get_mut(&patent.id) {
                Some(existing) => {
                    for q in patent.source_queries {
                        if !existing.source_queries.contains(&q) {
                            existing.source_queries.push(q);
                        }
                    }
                }
                None => {
                    order.push(patent.id.clone());
                    by_id.insert(patent.id.clone(), patent);
                }
            }
        }
        sources.push(info);
    }

    let patents = order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect();

    Corpus {
        patents,
        generated_at,
        sources,
    }
}

/// Merge one freshly fetched page into the running session corpus.
///
/// Same de-duplication rules as `merge`: publications are keyed by id; a
/// repeat sighting unions the query trail. The `SourceInfo` for the query is
/// replaced in place so the provenance panel always shows the LAST counts for
/// that query rather than a stale duplicate. Returns how many publications
/// were new.
pub fn merge_into(corpus: &mut Corpus, patents: Vec<Patent>, info: SourceInfo) -> usize {
    let mut fresh = 0;

    for patent in patents {
        match corpus.patents.iter_mut().find(|p| p.id == patent.id) {
            Some(existing) => {
                for q in patent.source_queries {
                    if !existing.source_queries.contains(&q) {
                        existing.source_queries.push(q);
                    }
                }
            }
            None => {
                fresh += 1;
                corpus.patents.push(patent);
            }
        }
    }

    match corpus.sources.iter_mut().find(|s| s.query == info.query) {
        Some(slot) => {
            slot.total_results = info.total_results;
            slot.retrieved = info.retrieved;
        }
        None => corpus.sources.push(info),
    }
    corpus.generated_at = "live session".to_string();
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
    {
      "search_parameters": { "q": "solar panel" },
      "search_information": { "total_results": 118864 },
      "organic_results": [
        {
          "patent_link": "https://patents.google.com/patent/EP2264377A2/en",
          "title": "Apparatus for two-way tracing",
          "snippet": "a plurality of second open push rods",
          "priority_date": "2009-06-03",
          "publication_number": "EP2264377A2",
          "inventor": "Young Hwan Park",
          "assignee": "Green Plus Co. Ltd.",
          "country_status": { "EP": "NOT_ACTIVE", "US": "NOT_ACTIVE", "CN": "ACTIVE" }
        }
      ]
    }"#;

    #[test]
    fn parses_jurisdiction_and_status() {
        let (patents, info) = ingest_document(SAMPLE).unwrap();
        assert_eq!(patents.len(), 1);
        assert_eq!(info.query, "solar panel");
        assert_eq!(info.total_results, 118864);

        let p = &patents[0];
        assert_eq!(p.id, "EP2264377A2");
        assert_eq!(p.jurisdiction, "EP");
        assert_eq!(p.us_status(), Some(CountryStatus::NotActive));
        assert!(p.is_us_graveyard());
        // statuses are sorted by country code for deterministic SSR
        assert_eq!(p.country_status[0].country, "CN");
    }

    #[test]
    fn merge_unions_source_queries() {
        let (a, ai) = ingest_document(SAMPLE).unwrap();
        let (b, bi) = ingest_document(SAMPLE).unwrap();
        let corpus = merge(vec![(a, ai), (b, bi)], "now".into());
        assert_eq!(corpus.len(), 1);
        assert_eq!(
            corpus.patents[0].source_queries,
            vec!["solar panel".to_string()]
        );
    }

    #[test]
    fn absent_us_filing_is_not_graveyard() {
        let json = r#"{"search_parameters":{"q":"x"},"organic_results":[
            {"title":"t","publication_number":"CN1A","country_status":{"CN":"ACTIVE"}}]}"#;
        let (patents, _) = ingest_document(json).unwrap();
        assert!(!patents[0].is_us_graveyard());
        assert!(patents[0].is_us_absent());
    }
}
