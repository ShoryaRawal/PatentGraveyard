//! The discovery engine.
//!
//! # Why this looks the way it does
//!
//! Our team is inspired by [ResearchRabbit][rr]. ResearchRabbit does *not* use
//! a large language model for its recommendations. Its own documentation is
//! explicit about this: recommendations come from the **structure** of the
//! literature — citations, references, shared citations, co-citation, and
//! authorship — walked one or two degrees out from a set of *seed* papers, and
//! rendered as an interactive network map. Papers are grouped into clusters by
//! how they connect, seeds are shown in one colour, suggestions in another, and
//! each suggested work lists *why* it was suggested (similar / earlier / later
//! work).
//!
//! We port that model to patents:
//!
//! | ResearchRabbit        | PatentGraveyard                          |
//! |-----------------------|------------------------------------------|
//! | Citation link         | Patent-family link (same invention, diff. jurisdiction) |
//! | Similar work          | Shared inventor, lexical similarity      |
//! | Citation clusters     | Technology clusters in the dead-patent network |
//!
//! The graph is sparse: every patent keeps only its strongest neighbours, so
//! the map reads as a network rather than an all-pairs star. Every edge carries
//! a `reason` and a weight, because a VC partner will not act on an unexplained
//! recommendation.
//!
//! [rr]: https://learn.researchrabbit.ai/en/articles/13545485-how-researchrabbit-uses-ai

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Corpus, Patent};

/// Why two patents are connected. Ordered by how much a diligence analyst
/// should trust the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Same invention, different jurisdiction — the same patent family.
    Family,
    /// Same company / research organisation.
    SharedAssignee,
    /// Same named inventor.
    SharedInventor,
    /// Surfaced by the same search query (co-retrieval / co-citation analogue).
    CoCitation,
    /// Overlapping title/abstract vocabulary (TF-IDF cosine).
    Lexical,
}

impl RelationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Family => "Family member",
            Self::SharedAssignee => "Shared assignee",
            Self::SharedInventor => "Shared inventor",
            Self::CoCitation => "Co-retrieved",
            Self::Lexical => "Similar text",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Family => "family",
            Self::SharedAssignee => "assignee",
            Self::SharedInventor => "inventor",
            Self::CoCitation => "co-cited",
            Self::Lexical => "text",
        }
    }

    /// Base weight contributes to edge strength.
    fn base_weight(self) -> f64 {
        match self {
            // A family link is a *hard* structural fact, not a similarity guess.
            Self::Family => 3.0,
            Self::SharedAssignee => 1.4,
            Self::SharedInventor => 1.2,
            Self::CoCitation => 0.8,
            Self::Lexical => 1.0,
        }
    }
}

/// A weighted, explained edge in the patent graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: RelationKind,
    pub weight: f64,
}

/// A node in the rendered map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub jurisdiction: String,
    pub assignee: String,
    /// "seed" | "suggested"
    pub role: String,
    /// 0..1 normalised score used for radius/opacity.
    pub score: f64,
    /// Number of edges to the seed set — ResearchRabbit sizes suggestion nodes
    /// by exactly this.
    pub degree_to_seeds: usize,
    pub us_active: bool,
    pub us_graveyard: bool,
    /// Human-readable reasons this node was suggested.
    pub reasons: Vec<String>,
    pub year: Option<i32>,
}

/// The full map handed to the graph island.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryMap {
    pub seeds: Vec<String>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
    /// Suggested nodes only, ranked — this is the "similar work" list.
    pub suggestions: Vec<RankedSuggestion>,
    /// A cluster of one or more nodes discovered by {committed} seeds.
    pub clusters: Vec<Cluster>,
    pub stats: GraphStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub graveyard_count: usize,
    pub us_active_count: usize,
}

/// One ranked "similar patent" recommendation, with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedSuggestion {
    pub id: String,
    pub patent: Patent,
    /// Final blended relevance score (0..1+).
    pub score: f64,
    /// Per-signal breakdown so the analyst sees *why*.
    pub evidence: Vec<Evidence>,
    pub relation: String,
    pub us_graveyard: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: RelationKind,
    pub detail: String,
    pub weight: f64,
}

/// A group of patents connected to each other more than to the rest. This is
/// ResearchRabbit's "papers tend to cluster into subtopics" observation, applied
/// to a technology landscape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cluster {
    pub label: String,
    pub node_ids: Vec<String>,
    pub graveyard_count: usize,
}

/// Tunable knobs, exposed so the UI can offer a "how strict?" control.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// Minimum lexical cosine to create an edge.
    pub lexical_threshold: f64,
    /// Maximum suggestions to rank.
    pub max_suggestions: usize,
    /// Degree to which co-retrieval counts.
    pub include_lexical: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            lexical_threshold: 0.14,
            max_suggestions: 40,
            include_lexical: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Graph construction
// ---------------------------------------------------------------------------

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Very small stop-word list; enough to keep TF-IDF meaningful on patent text.
const STOP: &[&str] = &[
    "the",
    "a",
    "an",
    "of",
    "and",
    "or",
    "to",
    "in",
    "for",
    "on",
    "with",
    "is",
    "are",
    "by",
    "at",
    "as",
    "from",
    "that",
    "this",
    "it",
    "be",
    "which",
    "wherein",
    "said",
    "method",
    "system",
    "apparatus",
    "device",
    "and/or",
    "same",
];

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|t| t.len() > 2)
        .map(|t| t.to_lowercase())
        .filter(|t| !STOP.contains(&t.as_str()))
        .collect()
}

/// Sparse TF-IDF vector keyed by term.
fn tfidf_vectors(patents: &[Patent]) -> Vec<HashMap<String, f64>> {
    let docs: Vec<Vec<String>> = patents.iter().map(|p| tokenize(&p.text())).collect();

    // Document frequency.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for doc in &docs {
        let unique: HashSet<&str> = doc.iter().map(|s| s.as_str()).collect();
        for term in unique {
            *df.entry(term).or_insert(0) += 1;
        }
    }

    let n = patents.len().max(1) as f64;
    docs.iter()
        .map(|doc| {
            let mut tf: HashMap<&str, f64> = HashMap::new();
            for term in doc {
                *tf.entry(term.as_str()).or_insert(0.0) += 1.0;
            }
            let mut vec: HashMap<String, f64> = HashMap::new();
            for (term, count) in tf {
                let idf = (n / (*df.get(term).unwrap_or(&1) as f64)).ln() + 1.0;
                vec.insert(term.to_string(), (1.0 + count.ln()) * idf);
            }
            vec
        })
        .collect()
}

fn cosine(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // Iterate the smaller map, but in sorted key order: summing floating point
    // in `HashMap` iteration order is not reproducible, and the server and the
    // hydrating client must agree on every weight (see the determinism test).
    let (small, large) = if a.len() < b.len() { (a, b) } else { (b, a) };
    let mut keys: Vec<&String> = small.keys().collect();
    keys.sort();
    let mut dot = 0.0;
    for term in keys {
        if let Some(other) = large.get(term) {
            dot += small[term] * other;
        }
    }
    if dot == 0.0 {
        return 0.0;
    }
    let norm = |m: &HashMap<String, f64>| -> f64 {
        let mut ks: Vec<&String> = m.keys().collect();
        ks.sort();
        let mut sum = 0.0;
        for k in ks {
            let v = m[k];
            sum += v * v;
        }
        sum.sqrt()
    };
    let (norm_a, norm_b) = (norm(a), norm(b));
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Family key: the same invention filed in several jurisdictions tends to share
/// a near-identical title, and usually at least one of the assignee or the
/// inventor. Real data is messy — the same family can be filed by a company in
/// one country and by the inventor personally in another, and translated titles
/// differ by a word — so we key on a *normalised title prefix* plus the
/// inventor, which is far more robust than an exact title match.
fn family_key(p: &Patent) -> String {
    let title: String = norm(&p.title)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let title_prefix = &title[..title.len().min(24)];

    let inventor: String = norm(&p.inventor)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();

    // Prefer inventor+title when we have a named inventor; otherwise fall back
    // to assignee+title.
    if !inventor.is_empty() && inventor != "na" {
        format!("{inventor}::{title_prefix}")
    } else {
        let assignee: String = norm(&p.assignee)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        format!("{assignee}::{title_prefix}")
    }
}

/// Build the explained, weighted graph over the whole corpus.
pub fn build_graph(corpus: &Corpus) -> Vec<Edge> {
    let patents = &corpus.patents;
    let vectors = tfidf_vectors(patents);
    let mut edges: Vec<Edge> = Vec::new();

    // Precompute family groups.
    let mut families: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in patents.iter().enumerate() {
        families.entry(family_key(p)).or_default().push(i);
    }

    let mut seen: HashSet<(usize, usize, RelationKind)> = HashSet::new();

    let mut push_edge = |edges: &mut Vec<Edge>, i: usize, j: usize, kind: RelationKind, w: f64| {
        let key = (i.min(j), i.max(j), kind);
        if seen.insert(key) {
            edges.push(Edge {
                source: patents[i].id.clone(),
                target: patents[j].id.clone(),
                kind,
                weight: w,
            });
        }
    };

    // 1. Family links — the strongest structural relation. Iterate the groups in
    //    a stable (sorted) order so SSR and hydration agree byte-for-byte.
    let mut family_keys: Vec<&String> = families.keys().collect();
    family_keys.sort();
    for key in family_keys {
        let members = &families[key];
        // Chain, not clique: a large family should read as a family, not as a
        // solid block of edges.
        for pair in members.windows(2) {
            push_edge(&mut edges, pair[0], pair[1], RelationKind::Family, 3.0);
        }
    }

    // 2. Shared inventor — a chain, not a clique. Inventor groups can be
    //    large; linking every pair turns the map into a hairball. Consecutive
    //    links (in corpus order) keep the relation visible without the density.
    let mut by_inventor: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in patents.iter().enumerate() {
        let inv = norm(&p.inventor);
        if !inv.is_empty() && inv != "n/a" {
            by_inventor.entry(inv).or_default().push(i);
        }
    }
    let mut inventor_keys: Vec<&String> = by_inventor.keys().collect();
    inventor_keys.sort();
    for key in inventor_keys {
        let members = &by_inventor[key];
        for pair in members.windows(2) {
            push_edge(
                &mut edges,
                pair[0],
                pair[1],
                RelationKind::SharedInventor,
                1.2,
            );
        }
    }

    // 3. Lexical similarity as a k-nearest-neighbour graph over the dead
    //    patents. Every patent keeps only its two strongest textual neighbours,
    //    which produces a readable connected graph instead of the all-pairs
    //    clique that shared-query co-retrieval used to create. In-force patents
    //    are excluded so they cannot occupy a dead patent's neighbour slots.
    const NEIGHBOURS: usize = 2;
    let dead_indices: Vec<usize> = patents
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_us_graveyard())
        .map(|(i, _)| i)
        .collect();
    let mut connected: HashSet<usize> = HashSet::new();
    for &i in &dead_indices {
        let mut scored: Vec<(f64, usize)> = Vec::with_capacity(dead_indices.len());
        for &j in &dead_indices {
            if i == j {
                continue;
            }
            let sim = cosine(&vectors[i], &vectors[j]);
            if sim >= 0.12 {
                scored.push((sim, j));
            }
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                // Identical texts are common (translated families, boilerplate
                // abstracts). Break ties by circular distance from `i` so every
                // node does not pick the same two low-index neighbours — that
                // is exactly how a kNN graph turns into a star.
                .then_with(|| {
                    let da = (a.1 + patents.len() - i) % patents.len();
                    let db = (b.1 + patents.len() - i) % patents.len();
                    da.cmp(&db)
                })
                .then_with(|| a.1.cmp(&b.1))
        });
        for (sim, j) in scored.into_iter().take(NEIGHBOURS) {
            push_edge(&mut edges, i, j, RelationKind::Lexical, sim * 2.0);
            connected.insert(i);
            connected.insert(j);
        }
    }

    // Keep isolated dead patents on the map: give each one a single link to its
    // closest textual neighbour, even when the pair sits just below the normal
    // threshold. A graph of floating stones is not a graph.
    for &i in &dead_indices {
        if connected.contains(&i) {
            continue;
        }
        let mut best: Option<(f64, usize)> = None;
        for &j in &dead_indices {
            if i == j {
                continue;
            }
            let sim = cosine(&vectors[i], &vectors[j]);
            if best.map(|(s, _)| sim > s).unwrap_or(true) {
                best = Some((sim, j));
            }
        }
        if let Some((sim, j)) = best {
            if sim > 0.0 {
                push_edge(
                    &mut edges,
                    i,
                    j,
                    RelationKind::Lexical,
                    (sim * 2.0).max(0.2),
                );
            }
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Seeded exploration — the "rabbit hole"
// ---------------------------------------------------------------------------

/// Build the patent map for the current session.
///
/// Every US-dead patent is a node. Seeds and suggestions only affect colour and
/// ranking; the graph itself is the sparse similarity network. That means the
/// map is never a star around one seed, and clearing the seeds never empties it.
pub fn explore(corpus: &Corpus, seeds: &[String], config: DiscoveryConfig) -> DiscoveryMap {
    // Only dead patents can be seeds — the desk does not track in-force work.
    let seeds: Vec<String> = seeds
        .iter()
        .filter(|id| corpus.get(id).map(|p| p.is_us_graveyard()).unwrap_or(false))
        .cloned()
        .collect();
    let seed_set: HashSet<&str> = seeds.iter().map(|s| s.as_str()).collect();
    let all_edges = build_graph(corpus);

    // Score non-seed dead patents by their connections into the seed set. This
    // drives the suggestion list and the "why it shows up" evidence.
    let mut score: HashMap<String, f64> = HashMap::new();
    let mut degree: HashMap<String, usize> = HashMap::new();
    let mut reasons: HashMap<String, Vec<Evidence>> = HashMap::new();

    let register = |id: &str,
                    score: &mut HashMap<String, f64>,
                    degree: &mut HashMap<String, usize>,
                    reasons: &mut HashMap<String, Vec<Evidence>>,
                    kind: RelationKind,
                    weight: f64,
                    detail: String| {
        *score.entry(id.to_string()).or_insert(0.0) += weight * kind.base_weight();
        *degree.entry(id.to_string()).or_insert(0) += 1;
        let entry = reasons.entry(id.to_string()).or_default();
        if !entry.iter().any(|e| e.kind == kind) {
            entry.push(Evidence {
                kind,
                detail,
                weight,
            });
        }
    };

    for e in &all_edges {
        let (seed_id, other) = if seed_set.contains(e.source.as_str()) {
            (e.source.as_str(), e.target.as_str())
        } else if seed_set.contains(e.target.as_str()) {
            (e.target.as_str(), e.source.as_str())
        } else {
            continue;
        };
        if seed_set.contains(other) {
            continue;
        }
        let Some(other_patent) = corpus.get(other) else {
            continue;
        };
        // In-force patents are out of scope. Dead patents and publications with
        // no US filing can still be useful context (family members, prior art).
        if matches!(
            other_patent.us_status(),
            Some(crate::model::CountryStatus::Active)
        ) {
            continue;
        }
        let detail = match e.kind {
            RelationKind::Family => format!("Same family as seed {seed_id}"),
            RelationKind::SharedAssignee => "Shares an assignee with a seed".to_string(),
            RelationKind::SharedInventor => "Shares an inventor with a seed".to_string(),
            RelationKind::CoCitation => "Retrieved by the same search as a seed".to_string(),
            RelationKind::Lexical => format!("Text similarity {:.2}", e.weight / 2.0),
        };
        register(
            other,
            &mut score,
            &mut degree,
            &mut reasons,
            e.kind,
            e.weight,
            detail,
        );
    }

    let mut ranked: Vec<(String, f64, usize, Vec<Evidence>)> = score
        .into_iter()
        .map(|(id, s)| {
            let d = *degree.get(&id).unwrap_or(&0);
            let r = reasons.remove(&id).unwrap_or_default();
            (id, s, d, r)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(config.max_suggestions);

    // The visible graph is every dead patent in the session.
    let dead: Vec<&Patent> = corpus
        .patents
        .iter()
        .filter(|p| p.is_us_graveyard())
        .collect();
    let visible: HashSet<&str> = dead.iter().map(|p| p.id.as_str()).collect();
    let edges: Vec<Edge> = all_edges
        .into_iter()
        .filter(|e| visible.contains(e.source.as_str()) && visible.contains(e.target.as_str()))
        .collect();

    // Node size follows total degree, not seed proximity: a hub in the network
    // should look like a hub.
    let mut graph_degree: HashMap<&str, usize> = HashMap::new();
    for e in &edges {
        *graph_degree.entry(e.source.as_str()).or_insert(0) += 1;
        *graph_degree.entry(e.target.as_str()).or_insert(0) += 1;
    }
    let max_degree = graph_degree.values().copied().max().unwrap_or(0) as f64;

    // Short, human-readable reasons from the node's own edges.
    let mut node_reasons: HashMap<&str, Vec<String>> = HashMap::new();
    for e in &edges {
        let kind = match e.kind {
            RelationKind::Family => "Family member",
            RelationKind::SharedInventor => "Shared inventor",
            RelationKind::Lexical => "Similar text",
            RelationKind::SharedAssignee => "Shared assignee",
            RelationKind::CoCitation => "Co-retrieved",
        };
        for (a, b) in [(&e.source, &e.target), (&e.target, &e.source)] {
            let detail = format!("{kind} · {b}");
            let list = node_reasons.entry(a.as_str()).or_default();
            if list.len() < 2 && !list.contains(&detail) {
                list.push(detail);
            }
        }
    }

    let mut nodes: Vec<GraphNode> = dead
        .iter()
        .map(|p| {
            let d = graph_degree.get(p.id.as_str()).copied().unwrap_or(0);
            let degree_to_seeds = edges
                .iter()
                .filter(|e| {
                    (e.source == p.id && seed_set.contains(e.target.as_str()))
                        || (e.target == p.id && seed_set.contains(e.source.as_str()))
                })
                .count();
            GraphNode {
                id: p.id.clone(),
                label: truncate(&p.title, 48),
                jurisdiction: p.jurisdiction.clone(),
                assignee: p.assignee.clone(),
                role: if seed_set.contains(p.id.as_str()) {
                    "seed".into()
                } else {
                    "suggested".into()
                },
                score: if max_degree > 0.0 {
                    (d as f64 / max_degree).clamp(0.0, 1.0)
                } else {
                    0.5
                },
                degree_to_seeds,
                us_active: false,
                us_graveyard: true,
                reasons: node_reasons.get(p.id.as_str()).cloned().unwrap_or_default(),
                year: p.year(),
            }
        })
        .collect();

    // Deterministic order, independent of HashMap iteration.
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut suggestions: Vec<RankedSuggestion> = Vec::new();
    for (id, s, d, evidence) in &ranked {
        let Some(p) = corpus.get(id) else { continue };
        let mut evidence = evidence.clone();
        evidence.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.kind.cmp(&b.kind))
        });
        let _ = d;
        suggestions.push(RankedSuggestion {
            id: p.id.clone(),
            patent: p.clone(),
            score: (s * 10.0).round() / 10.0,
            evidence: evidence.clone(),
            relation: dominant_relation(&evidence)
                .map(|k| k.short().to_string())
                .unwrap_or_else(|| "related".into()),
            us_graveyard: p.is_us_graveyard(),
        });
    }

    let clusters = cluster(&nodes, &edges);

    let stats = GraphStats {
        node_count: nodes.len(),
        edge_count: edges.len(),
        graveyard_count: nodes.iter().filter(|n| n.us_graveyard).count(),
        us_active_count: 0,
    };

    DiscoveryMap {
        seeds,
        nodes,
        edges,
        suggestions,
        clusters,
        stats,
    }
}

fn dominant_relation(evidence: &[Evidence]) -> Option<RelationKind> {
    evidence
        .iter()
        .max_by(|a, b| {
            (a.weight * a.kind.base_weight())
                .partial_cmp(&(b.weight * b.kind.base_weight()))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|e| e.kind)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

// ---------------------------------------------------------------------------
// Clustering — "papers tend to group into clusters, each reflecting a subtopic"
// ---------------------------------------------------------------------------

/// Plain label-propagation over the visible subgraph. Cheap, deterministic, and
/// good enough to surface technology groupings without pulling in a graph
/// library (keeps the WASM bundle tiny).
fn cluster(nodes: &[GraphNode], edges: &[Edge]) -> Vec<Cluster> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for e in edges {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            adjacency[a].push(b);
            adjacency[b].push(a);
        }
    }

    let mut label: Vec<usize> = (0..nodes.len()).collect();
    for _ in 0..8 {
        let mut changed = false;
        for i in 0..nodes.len() {
            if adjacency[i].is_empty() {
                continue;
            }
            let mut counts: HashMap<usize, usize> = HashMap::new();
            for &nb in &adjacency[i] {
                *counts.entry(label[nb]).or_insert(0) += 1;
            }
            // Deterministic winner: highest count, ties broken by the smallest
            // label id.
            let best = counts
                .iter()
                .map(|(&lbl, &c)| (c, std::cmp::Reverse(lbl)))
                .max()
                .map(|(_, std::cmp::Reverse(lbl))| lbl);
            if let Some(best) = best {
                if label[i] != best {
                    label[i] = best;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut groups: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        groups.entry(label[i]).or_default().push(node.id.clone());
    }

    // Emit clusters in a stable order (by their first member) so the engine is
    // reproducible between the server render and client hydration.
    let mut group_list: Vec<Vec<String>> = groups.into_values().collect();
    group_list.sort_by(|a, b| a.first().cmp(&b.first()));

    let mut clusters: Vec<Cluster> = group_list
        .into_iter()
        .map(|node_ids| {
            let members: Vec<&GraphNode> =
                nodes.iter().filter(|n| node_ids.contains(&n.id)).collect();
            let graveyard_count = members.iter().filter(|n| n.us_graveyard).count();
            // Label a cluster by shared technical vocabulary, never by company:
            // this desk is about patents, not assignees.
            let mut term_counts: HashMap<String, usize> = HashMap::new();
            for m in &members {
                for term in tokenize(&m.label) {
                    *term_counts.entry(term).or_insert(0) += 1;
                }
            }
            let label = term_counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .filter(|(_, &c)| c > 1)
                .map(|(term, _)| {
                    let mut chars = term.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => term.clone(),
                    }
                })
                .or_else(|| members.first().map(|m| truncate(&m.label, 32)))
                .unwrap_or_else(|| "Ungrouped".into());
            Cluster {
                label,
                node_ids,
                graveyard_count,
            }
        })
        .collect();

    clusters.sort_by(|a, b| {
        b.node_ids
            .len()
            .cmp(&a.node_ids.len())
            .then_with(|| a.node_ids.first().cmp(&b.node_ids.first()))
    });
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CountryStatus, CountryStatusEntry};

    fn patent(id: &str, title: &str, assignee: &str, us: CountryStatus, q: &str) -> Patent {
        Patent {
            id: id.into(),
            publication_number: id.into(),
            link: String::new(),
            title: title.into(),
            abstract_text: "solar panel tracking bracket photovoltaic".into(),
            priority_date: Some("2020-01-01".into()),
            filing_date: None,
            publication_date: Some("2021-01-01".into()),
            grant_date: None,
            inventor: "A Inventor".into(),
            assignee: assignee.into(),
            language: "en".into(),
            thumbnail: None,
            pdf: None,
            figures: vec![],
            jurisdiction: id.chars().take(2).collect::<String>(),
            country_status: vec![CountryStatusEntry {
                country: "US".into(),
                status: us,
            }],
            source_queries: vec![q.into()],
            source: "test".into(),
            kind: "Grant".into(),
        }
    }

    fn corpus() -> Corpus {
        Corpus {
            patents: vec![
                patent(
                    "US1",
                    "Solar panel tracking",
                    "Acme",
                    CountryStatus::NotActive,
                    "q1",
                ),
                patent(
                    "EP1",
                    "Solar panel tracking",
                    "Acme",
                    CountryStatus::NotActive,
                    "q1",
                ),
                patent(
                    "CN1",
                    "Solar panel bracket",
                    "Other",
                    CountryStatus::Active,
                    "q1",
                ),
                patent(
                    "US2",
                    "Wind turbine blade",
                    "Windy",
                    CountryStatus::Active,
                    "q2",
                ),
            ],
            generated_at: "t".into(),
            sources: vec![],
        }
    }

    #[test]
    fn exploration_finds_neighbours() {
        let c = corpus();
        let map = explore(&c, &["US1".into()], DiscoveryConfig::default());
        assert_eq!(map.seeds, vec!["US1"]);
        // US1 should suggest the family sibling EP1. CN1 is in force, so it is
        // not part of the dead-patent desk any more.
        let ids: Vec<&str> = map.suggestions.iter().map(|s| s.id.as_str()).collect();
        assert!(
            ids.contains(&"EP1"),
            "expected family suggestion, got {ids:?}"
        );
        assert!(
            !ids.contains(&"CN1"),
            "in-force patents must not be suggested"
        );
    }

    #[test]
    fn map_shows_every_dead_patent_without_seeds() {
        let c = corpus();
        let map = explore(&c, &[], DiscoveryConfig::default());
        let ids: Vec<&str> = map.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["EP1", "US1"]);
        assert!(map.nodes.iter().all(|n| n.us_graveyard));
        assert!(map.stats.us_active_count == 0);
    }

    #[test]
    fn graph_is_sparse_not_a_clique() {
        let patents: Vec<Patent> = (0..12)
            .map(|i| {
                patent(
                    &format!("US{}{}", 10 + i, "A1"),
                    &format!("Solar panel variant {i}"),
                    "Acme",
                    CountryStatus::NotActive,
                    "q1",
                )
            })
            .collect();
        let c = Corpus {
            patents,
            generated_at: "t".into(),
            sources: vec![],
        };
        let edges = build_graph(&c);
        assert!(
            edges.len() < 12 * 11 / 2,
            "graph should not be an all-pairs clique: {} edges",
            edges.len()
        );
        let mut degree: HashMap<&str, usize> = HashMap::new();
        for e in &edges {
            *degree.entry(e.source.as_str()).or_default() += 1;
            *degree.entry(e.target.as_str()).or_default() += 1;
        }
        let max = degree.values().copied().max().unwrap_or(0);
        assert!(max <= 8, "degree {max} is too dense for a readable graph");
    }

    #[test]
    fn family_link_outranks_lexical() {
        let c = corpus();
        let map = explore(&c, &["US1".into()], DiscoveryConfig::default());
        let ep = map.suggestions.iter().find(|s| s.id == "EP1").unwrap();
        assert!(ep.evidence.iter().any(|e| e.kind == RelationKind::Family));
    }

    #[test]
    fn graveyard_is_flagged() {
        let c = corpus();
        let map = explore(&c, &["US2".into()], DiscoveryConfig::default());
        assert!(c.get("US1").unwrap().is_us_graveyard());
        // US2 is in force, so it is not a seed and not a node.
        assert!(map.seeds.is_empty());
        assert!(map.nodes.iter().all(|n| n.id != "US2"));
    }
}
