# PatentGraveyard

A Rust + [Leptos](https://leptos.dev) application that lets a venture-capital
fund "reap the patent graveyard": find patents whose **US rights have lapsed**
while the invention may still be protected elsewhere, discover **similar
patents**, understand **what they are and who holds them**, and see whether the
underlying technology is **active in the United States**.

Our team modelled the discovery experience on
[ResearchRabbit](https://www.researchrabbit.ai) — the literature-mapping tool —
and ported its ideas from research papers to patents. Where ResearchRabbit walks
a **citation network** and shows you an interactive map of *similar / earlier /
later* work, PatentGraveyard walks a **sparse patent graph** (families, shared
inventors, and nearest-neighbour text similarity) and shows you an interactive
map of the patents you should look at next — with every recommendation explained.

> **Market:** United States (US). All status verdicts are read from the
> `country_status` data reported by Google Patents.

---

## What it does

Given a live session of **US-lapsed patents**, the app:

1. Places every dead patent in the session on the graph and builds **sparse
   structural edges** — family members, shared inventors, and two nearest
   neighbours by TF-IDF text similarity.
2. Ranks neighbours and attaches the **evidence** that put each edge there
   (`family member`, `shared inventor`, `similar text`). Seeds are optional
   highlights, not a filter.
3. Runs **label propagation** over the network to group patents into technology
   **clusters**.
4. Computes a per-patent **US verdict** — `lapsed`, `in force`, or `never filed`
   — because that is the decision a fund actually makes.

The dashboard answers the client's four questions directly:

| Question | Where it's answered |
|---|---|
| **What are these patents?** | The Graveyard table: title, assignee, jurisdiction, priority date, plain-English signal. |
| **How can they help us?** | The US verdict + "free to build in the US" signal; the network map shows the surrounding landscape. |
| **Are they active in our country?** | `US status` column and the detail panel verdict, derived from `country_status`. |
| **Similar patents?** | The Similarity map: ranked suggestions with per-edge evidence (clusters are computed by the engine and surfaced as evidence). |

---

## Architecture — Leptos islands + WASM

The app uses Leptos **islands architecture**, which is a perfect fit for a
data-heavy diligence tool: the page is a "sea" of server-rendered HTML, and only
small, genuinely interactive regions are compiled to WebAssembly and hydrated.

The whole page is a **fixed, full-viewport dashboard** — it never scrolls. The
information a partner needs is arranged so it is all visible at once, and only
individual panels scroll internally:

```
┌──────────────────────────────────────────────────────────────────────┐
│ top bar: brand · KPIs · market badge · "How it works"   (server HTML) │
├─────────────────────────────┬────────────────────────────────────────┤
│ THE GRAVEYARD               │ SIMILARITY MAP   (nested island)        │
│ (nested island)             │  seed chips · force-directed canvas     │
│  filter bar                 ├────────────────────────────────────────┤
│  scrollable patent table    │ SIMILAR PATENTS / DETAIL (scrollable)   │
├─────────────────────────────┴────────────────────────────────────────┤
│ status bar: seed searches · provenance notice            (server HTML)│
└──────────────────────────────────────────────────────────────────────┘
```

```
Browser                                        Server (Axum + SSR)
┌─────────────────────────────────────┐        ┌───────────────────────────────┐
│ top bar + status bar (no WASM)      │◀───────│  app.rs  (#[component] only)  │
│                                     │        │  reads corpus synchronously   │
│ <leptos-island Workspace>           │        │  data.rs  loads + caches      │
│   <island> GraveyardPanel </island> │◀─ WASM │  ingest.rs  parses SerpApi    │
│   <island> MapPanel       </island> │        │  discovery.rs  graph engine   │
│ </leptos-island>                    │        │  canvas.rs  force-directed map │
└─────────────────────────────────────┘        └───────────────────────────────┘
```

* **`src/app.rs`** — the shell and every server-only component (top bar, KPIs,
  status bar, the "How it works" disclosure). In islands mode these are *not*
  compiled into the WASM bundle, so they read the corpus directly with no
  serialization and no client cost. Adding static content here does not grow the
  bundle at all.
* **`src/islands.rs`** — the interactive islands, built as **nested islands**:
  * `Workspace` — the island root. Owns the shared state (map seeds + the
    selected patent) and provides it to its children through Leptos **context**,
    so the table and the map stay in sync with no server round-trip.
  * `GraveyardPanel` — client-side filter / sort of the scrollable patent table,
    with a one-click "seed this patent" action.
  * `MapPanel` — the ResearchRabbit-style similarity map, seed chips and the
    ranked suggestion / detail list.
* **`src/discovery.rs`** — the recommendation engine. Pure, testable, no LLM.
* **`src/ingest.rs`** — the only module that knows the external JSON schema.
* **`src/canvas.rs`** — a dependency-free force-directed graph rendered to
  `<canvas>` via `web-sys`, sized to its panel at device pixel ratio. No charting
  library in the bundle.
* **`src/model.rs`** — the domain types shared by server and client.

### Why islands matter here

A fully client-rendered version of this page would ship the corpus, the renderer
and *all* the markup to the browser. With islands, the browser downloads only the
explorer and the graph. That is the difference between a fund partner seeing a
populated dashboard instantly and watching a spinner.

---

## How the recommendations work (ResearchRabbit → patents)

ResearchRabbit is explicit that its recommendations come from the **structure**
of the literature, not from a language model. We keep that principle.

| ResearchRabbit | PatentGraveyard | Weight |
|---|---|---|
| Citation link | **Patent family** (same invention, different jurisdiction, matched by normalised title + inventor) | 3.0 |
| Co-authorship | **Shared inventor** | 1.2 |
| "Similar work" | **TF-IDF k-nearest neighbours** over title + abstract (two per patent) | cosine |

Edges are sparse by design: every patent contributes at most a family link, an
inventor link, and two text-neighbour links, so the map reads as a network rather
than a star or an all-pairs clique. The graph is scored, normalised to `0..1`,
and every edge carries the relation that produced it. That explainability is
a deliberate improvement on ResearchRabbit's "some additional magic": a diligence
partner must be able to audit a recommendation.

Clusters are found with a deterministic label-propagation pass over the
neighbourhood — the patent analogue of ResearchRabbit's observation that papers
group into subtopics.

---

## Data

There is no bundled corpus. The session starts empty and the analyst grows it
live: typing a term in the command bar and pressing `<GO>` calls the server's
`/api/search`, which fetches a
[SerpApi Google Patents](https://serpapi.com/google-patents-api) page, merges it
into the in-memory session (de-duplicated by publication number), and re-seeds
the map over every lapsed result. One process, one session: everything lives in
RAM until the server restarts — the seam where durable storage will plug in
when this moves to its AWS host.

The server reads the SerpApi key from its environment, so the key never reaches
the browser:

```bash
SERPAPI_KEY=xxxx cargo run --no-default-features --features ssr
# (SERPAPI_BASE can override the endpoint for local testing)
```

The mobile assistant adds a shared bearer token and a SQLite report file. Both
are server-side settings:

```bash
MOBILE_API_TOKEN=xxxx GRAVE_TENDER_DB=./gravetender.db \
  cargo run --no-default-features --features ssr
```

See [`MOBILE.md`](MOBILE.md) for the GraveTender phone app, native builds, and
the EC2/Caddy deployment.

---

## Running it

Prerequisites: Rust (with the `wasm32-unknown-unknown` target), `trunk`, and
`binaryen` (for `wasm-opt`).

```bash
# one-time
rustup target add wasm32-unknown-unknown
cargo install trunk
# binaryen: `brew install binaryen` on macOS

# build the WASM islands
trunk build            # or: trunk build --release

# build & run the server
cargo run --no-default-features --features ssr
# open http://127.0.0.1:3000
```

For live reload during development:

```bash
trunk watch            # rebuilds WASM on change
cargo run --no-default-features --features ssr
```

### Tests

```bash
cargo test --no-default-features --features ssr
```

The suite includes unit tests for ingest and the graph engine, plus an
end-to-end test (`tests/discovery_e2e.rs`) that drives the server's real
pipeline — `ingest_document` → `session::apply` → discovery walk — over inline
SerpApi-shaped fixtures and asserts the client's actual questions are answered.

---

## Mobile companion — GraveTender

The phone app is a separate Leptos CSR bundle wrapped by Tauri 2. It is an
assistant, not a second desk: chat, quick actions, report generation, report
reading/sharing, and settings. It deliberately omits the graph, patent table,
dossier editor, and seed workflow, and it never calls SerpApi.

```bash
# backend + mobile web preview
./run.sh ./target/release/patent-graveyard
cd mobile-ui && trunk serve --port 8080

# native shell (after Xcode/Android SDK are installed)
cargo tauri dev
```

The API lives at `/api/mobile/{health,bootstrap,chat,reports}`. Reports are
Markdown, grounded in the current session, persisted in SQLite, and shared with
the platform share sheet. Deployment, signing, TestFlight, and Android APK
instructions are in [`MOBILE.md`](MOBILE.md).

---

## Notes on production use

* **Live data.** `ingest.rs` is the only schema-aware module. Swapping SerpApi
  for USPTO bulk data, EPO OPS or PATSTAT means changing one file. The rest of
  the app sees only `model::Corpus`.
* **Scale.** The graph walk is recomputed client-side for instant re-seeding. On
  a corpus of millions this should move behind a `#[server]` function; the
  `DiscoveryConfig` seam already exists, and the islands never assume the corpus
  is local.
* **Legal notice.** `country_status` is reported by Google Patents and is **not**
  a substitute for a formal freedom-to-operate opinion. The app labels this
  clearly; diligence decisions should be confirmed against the USPTO register.

# https://docs.google.com/presentation/d/1TA8h913UMWZx_JO-u-4Edcf96RQgqNkrq8l4qZm5RZk/edit?usp=sharing
