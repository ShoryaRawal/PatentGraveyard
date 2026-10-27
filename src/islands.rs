//! The interactive strategy & diligence workspace island.
//!
//! Only code reachable from this `#[island]` is compiled to `wasm32` and
//! shipped to the browser.
//!
//! ## Phase 2 Architecture
//! * The graph canvas, the left patent bar, and the analyst chat bot have been removed.
//! * Prominent PlayStation Store top bar with 5 huge architectural aspect cards.
//! * Clicking any card triggers an island takeover replacing the cards with the
//!   deep-dive diligence island: Executive Synthesis, "Which Patents Can Be Used Where",
//!   Modeled Timeline Roadmap, and Curated Patent Docket.
//! * Powered by Groq (5 aspects) -> SerpApi (5 Google Patents searches) -> Groq (Synthesis & Timeline).

use leptos::prelude::*;

use crate::model::{AspectCard, AspectPatent, StrategySession};

// ---------------------------------------------------------------------------
// Shared Reactive State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct StrategyState {
    /// Active diligence session (contains original query, thesis, and 5 aspects)
    pub session: RwSignal<StrategySession>,
    /// Currently focused aspect (None = viewing the 5 PlayStation Store cards)
    pub active_aspect_id: RwSignal<Option<String>>,
    /// Prompt input in the command bar
    pub prompt: RwSignal<String>,
    /// True while Groq + SerpApi + Groq pipeline is in flight
    pub loading: RwSignal<bool>,
    /// Status message displayed during pipeline execution
    pub status_note: RwSignal<String>,
    /// Toast feedback notification
    pub toast: RwSignal<Option<String>>,
    /// Selected patent for modal inspection
    pub selected_patent: RwSignal<Option<AspectPatent>>,
}

impl StrategyState {
    pub fn say(&self, msg: &str) {
        self.toast.set(Some(msg.to_string()));
    }

    pub fn execute_diligence(&self) {
        let query = self.prompt.get_untracked().trim().to_string();
        if query.is_empty() {
            self.say("Please enter a venture idea or question first.");
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            if self.loading.get_untracked() {
                self.say("Diligence pipeline already running. Please wait...");
                return;
            }

            self.loading.set(true);
            self.status_note.set("Phase 1: Synthesizing 5 strategic outcomes with Groq...".to_string());
            self.active_aspect_id.set(None);

            let state = *self;
            let query_str = query.clone();

            leptos::task::spawn_local(async move {
                match fetch_strategy(&query_str).await {
                    Ok(new_session) => {
                        state.session.set(new_session);
                        state.loading.set(false);
                        state.status_note.set(String::new());
                        state.say(&format!("Strategy synthesized for \u{201c}{query_str}\u{201d} — 5 strategic outcomes ready."));
                    }
                    Err(err) => {
                        state.loading.set(false);
                        state.status_note.set(String::new());
                        state.say(&format!("Diligence failed: {err}"));
                    }
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.say("Diligence execution runs in the browser.");
        }
    }
}

// ---------------------------------------------------------------------------
// Browser Client API
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
async fn fetch_strategy(prompt: &str) -> Result<StrategySession, String> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or_else(|| "No browser window available".to_string())?;

    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);

    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .append("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    init.set_headers(&headers);

    let body_json = serde_json::json!({ "prompt": prompt }).to_string();
    init.set_body(&wasm_bindgen::JsValue::from_str(&body_json));

    let request = web_sys::Request::new_with_str_and_init("/api/strategy", &init)
        .map_err(|e| format!("{e:?}"))?;

    let resp_value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Network request failed: {e:?}"))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "Failed to parse Response object")?;

    let text_js = wasm_bindgen_futures::JsFuture::from(
        resp.text().map_err(|e| format!("{e:?}"))?,
    )
    .await
    .map_err(|e| format!("Failed to read response body: {e:?}"))?;

    let text = text_js.as_string().unwrap_or_default();

    #[derive(serde::Deserialize)]
    struct ApiResp {
        ok: bool,
        session: Option<StrategySession>,
        error: Option<String>,
    }

    let parsed: ApiResp = serde_json::from_str(&text)
        .map_err(|e| format!("Unparseable API reply ({e}): {text}"))?;

    if parsed.ok {
        parsed
            .session
            .ok_or_else(|| "Server returned ok but missing session".to_string())
    } else {
        Err(parsed
            .error
            .unwrap_or_else(|| "Unknown API error".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Workspace Root Island
// ---------------------------------------------------------------------------

#[island]
pub fn Workspace(initial_session: StrategySession) -> impl IntoView {
    let state = StrategyState {
        session: RwSignal::new(initial_session),
        active_aspect_id: RwSignal::new(None),
        prompt: RwSignal::new("I am building a company for solar panels, can you find patents for me to use?".to_string()),
        loading: RwSignal::new(false),
        status_note: RwSignal::new(String::new()),
        toast: RwSignal::new(None),
        selected_patent: RwSignal::new(None),
    };
    provide_context(state);

    // Global keyboard shortcuts for web-app feel (ESC to close island / modal)
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        if let Some(window) = web_sys::window() {
            let cb = Closure::<dyn Fn(web_sys::KeyboardEvent)>::wrap(Box::new(move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Escape" {
                    if state.selected_patent.get().is_some() {
                        state.selected_patent.set(None);
                    } else if state.active_aspect_id.get().is_some() {
                        state.active_aspect_id.set(None);
                    }
                }
            }));
            let _ = window.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
            cb.forget();
        }
    }

    view! {
        <div class="strategy-workspace">
            <StrategyCmdBar/>

            // Main section: either the PlayStation Store top bar cards or the Takeover Island
            <main class="strategy-main">
                {move || match state.active_aspect_id.get() {
                    None => view! { <PlayStationStoreDeck/> }.into_any(),
                    Some(aspect_id) => view! { <AspectTakeoverIsland aspect_id=aspect_id/> }.into_any(),
                }}
            </main>

            // Pinned web-app status footer
            <StrategyFooter/>

            // Patent detail modal if user inspects a specific patent
            {move || state.selected_patent.get().map(|pat| view! { <PatentModal patent=pat/> })}

            // Interactive toast alert
            {move || state.toast.get().map(|msg| {
                view! {
                    <div class="toast" role="status" on:click=move |_| state.toast.set(None)>
                        <span>{msg}</span>
                        <span class="toast-hint">"TAP TO DISMISS"</span>
                    </div>
                }
            })}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Strategy Command Bar & Search Input
// ---------------------------------------------------------------------------

#[component]
fn StrategyCmdBar() -> impl IntoView {
    let state = expect_context::<StrategyState>();

    let presets = [
        "I am building a company for solar panels, can you find patents for me to use?",
        "Building next-gen solid-state lithium battery startup with high energy density.",
        "Direct air carbon capture company using solid sorbents and thermal desorption.",
        "Autonomous drone delivery fleet with low-noise tilt-rotor aerodynamics.",
    ];

    view! {
        <header class="strategy-cmd-bar">
            <div class="cmd-input-container">
                <span class="cmd-prefix">"STRATEGY >"</span>
                <input
                    class="strategy-input"
                    type="text"
                    spellcheck="false"
                    placeholder="Enter your venture thesis: e.g. 'I am building a company for solar panels, can you find patents for me to use?'"
                    prop:value=state.prompt
                    on:input=move |ev| state.prompt.set(event_target_value(&ev))
                    on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            state.execute_diligence();
                        }
                    }
                />
                <button
                    class="btn btn-primary btn-execute"
                    disabled=move || state.loading.get()
                    on:click=move |_| state.execute_diligence()
                >
                    {move || if state.loading.get() {
                        "ANALYZING & HARVESTING..."
                    } else {
                        "RUN DILIGENCE"
                    }}
                </button>
            </div>

            // Presets and loading status indicators
            <div class="cmd-subbar">
                <div class="presets-row">
                    <span class="presets-label">"PROMPT PRESETS:"</span>
                    {presets.into_iter().map(|preset| {
                        let text = preset.to_string();
                        let tag = if preset.contains("solar") {
                            "Solar Panels"
                        } else if preset.contains("battery") {
                            "Solid-State Battery"
                        } else if preset.contains("carbon") {
                            "Direct Air Capture"
                        } else {
                            "Drone Delivery"
                        };
                        view! {
                            <button
                                class="preset-chip"
                                on:click=move |_| {
                                    state.prompt.set(text.clone());
                                    state.execute_diligence();
                                }
                            >
                                {tag}
                            </button>
                        }
                    }).collect_view()}
                </div>

                {move || {
                    if state.loading.get() {
                        view! {
                            <div class="loading-indicator">
                                <span class="spinner"></span>
                                <span class="loading-text">
                                    {state.status_note.get()}
                                </span>
                            </div>
                        }.into_any()
                    } else {
                        view! { <span class="cmd-engine-tag">"GROQ 70B · SERPAPI GOOGLE PATENTS · US GRAVEYARD CLASSIFIER"</span> }.into_any()
                    }
                }}
            </div>
        </header>
    }
}

// ---------------------------------------------------------------------------
// Large Strategic Outcomes (2 in a row, Minimalist & Professional)
// ---------------------------------------------------------------------------

#[component]
fn PlayStationStoreDeck() -> impl IntoView {
    let state = expect_context::<StrategyState>();

    view! {
        <section class="ps-store-section">
            <div class="ps-store-header">
                <div class="ps-store-title-group">
                    <span class="ps-badge">"AGENT DILIGENCE · SYNTHESIZED OUTCOMES"</span>
                    <h2 class="ps-store-heading">"Strategic Commercial Outcomes"</h2>
                    <p class="ps-store-sub">
                        {move || state.session.with(|s| s.general_thesis.clone())}
                    </p>
                </div>
                <div class="ps-store-meta">
                    <span class="ps-meta-pill">"5 DISCOVERED OUTCOMES"</span>
                    <span class="ps-meta-hint">"Select any outcome to inspect actionable patents, subsystems & timeline"</span>
                </div>
            </div>

            // 2-in-a-row large hero cards with left-to-right gradients & technical schematics
            <div class="ps-store-grid">
                {move || {
                    state.session.with(|s| {
                        let total = s.aspects.len();
                        s.aspects.iter().enumerate().map(|(idx, aspect)| {
                            let card = aspect.clone();
                            let aspect_id = card.id.clone();
                            let is_last_odd = idx == 4 && total == 5;
                            let theme_num = (idx % 5) + 1;
                            let card_classes = format!("ps-card ps-card-theme-{}", theme_num);
                            view! {
                                <div
                                    class=card_classes
                                    class:ps-card-span2=is_last_odd
                                    on:click=move |_| state.active_aspect_id.set(Some(aspect_id.clone()))
                                >
                                    // Left-to-right fading engineering schematic art layer
                                    <div class="ps-card-art" aria-hidden="true">
                                        {render_card_schematic(theme_num)}
                                    </div>

                                    <div class="ps-card-body">
                                        <div class="ps-card-header">
                                            <div class="ps-card-pills">
                                                <span class="ps-card-idx">
                                                    {format!("OUTCOME {:02}", idx + 1)}
                                                </span>
                                                <span class="ps-card-cat">
                                                    {card.category.clone()}
                                                </span>
                                            </div>
                                            <span class="ps-card-status">
                                                {format!("{} Lapsed (Public Domain)", card.graveyard_count)}
                                            </span>
                                        </div>

                                        <h3 class="ps-card-title">{card.title.clone()}</h3>
                                        <p class="ps-card-pitch">{card.pitch.clone()}</p>

                                        <div class="ps-card-stats">
                                            <span class="stat-badge">
                                                <span class="stat-bullet"></span>
                                                {format!("{} Patents Analyzed", card.total_patents)}
                                            </span>
                                            <span class="stat-badge stat-graveyard">
                                                <span class="stat-bullet green"></span>
                                                {format!("{} US Graveyard (100% Free)", card.graveyard_count)}
                                            </span>
                                            <span class="stat-badge">
                                                <span class="stat-bullet"></span>
                                                {format!("{} Roadmap Phases", card.timeline.len())}
                                            </span>
                                        </div>

                                        <div class="ps-card-footer">
                                            <span class="ps-card-action">
                                                "EXPLORE OUTCOME"
                                                <span class="arrow">" →"</span>
                                            </span>
                                        </div>
                                    </div>
                                </div>
                            }
                        }).collect_view()
                    })
                }}
            </div>
        </section>
    }
}

// ---------------------------------------------------------------------------
// Aspect Takeover Island (Takes over the card section)
// ---------------------------------------------------------------------------

#[component]
fn AspectTakeoverIsland(aspect_id: String) -> impl IntoView {
    let state = expect_context::<StrategyState>();
    let id_clone = aspect_id.clone();

    // Look up the active aspect from the session as a Copy Memo
    let aspect: Memo<Option<AspectCard>> = Memo::new(move |_| {
        state.session.with(|s| {
            s.aspects
                .iter()
                .find(|a| a.id == id_clone)
                .cloned()
        })
    });

    view! {
        <section class="takeover-island">
            // Island Navigation & Outcome Switcher
            <div class="takeover-nav">
                <div class="takeover-nav-left">
                    <button
                        class="btn btn-back"
                        on:click=move |_| state.active_aspect_id.set(None)
                    >
                        "← BACK TO ALL OUTCOMES"
                    </button>
                    <div class="takeover-title-box">
                        <span class="takeover-cat-badge">
                            {move || aspect.get().map(|a| a.category).unwrap_or_default()}
                        </span>
                        <h2 class="takeover-title">
                            {move || aspect.get().map(|a| a.title).unwrap_or_default()}
                        </h2>
                    </div>
                </div>

                // Quick switcher pills across the 5 outcomes
                <div class="takeover-switcher">
                    <span class="switcher-label">"SWITCH OUTCOME:"</span>
                    {move || {
                        state.session.with(|s| {
                            s.aspects.iter().enumerate().map(|(idx, a)| {
                                let target_id = a.id.clone();
                                let is_active = target_id == aspect_id;
                                let short_name = a.title.split('&').next().unwrap_or(&a.title).trim().to_string();
                                view! {
                                    <button
                                        class="switcher-pill"
                                        class:active=is_active
                                        on:click=move |_| state.active_aspect_id.set(Some(target_id.clone()))
                                    >
                                        <span class="switcher-idx">{format!("{:02}", idx + 1)}</span>
                                        <span class="switcher-name">{short_name}</span>
                                    </button>
                                }
                            }).collect_view()
                        })
                    }}
                </div>
            </div>

            // Takeover Island Body
            <div class="takeover-body">
                // Section 1: Executive Freedom-to-Operate Synthesis
                <div class="takeover-card synthesis-card">
                    <div class="section-badge">"01 / FREEDOM-TO-OPERATE OUTCOME SYNTHESIS"</div>
                    <p class="synthesis-text">
                        {move || aspect.get().map(|a| a.summary).unwrap_or_default()}
                    </p>
                    <div class="synthesis-footer">
                        <span class="query-tag">
                            "Google Patents Query: "
                            <code class="query-code">{move || aspect.get().map(|a| a.search_query).unwrap_or_default()}</code>
                        </span>
                        <span class="lapsed-highlight">
                            {move || {
                                aspect.get().map(|a| {
                                    format!("{} of {} patents are expired US graveyard patents free to build without licensing", a.graveyard_count, a.total_patents)
                                }).unwrap_or_default()
                            }}
                        </span>
                    </div>
                </div>

                // Section 2: "Which Patents Can Be Used Where" (Subsystem Matrix)
                <div class="takeover-card used-where-card">
                    <div class="section-badge">"02 / SUBSYSTEM IMPLEMENTATION (HOW TO EXECUTE THIS OUTCOME)"</div>
                    <div class="used-where-grid">
                        {move || {
                            aspect.get().map(|a| {
                                a.used_where.into_iter().map(|u| {
                                    let pub_num = u.patent_pub.clone();
                                    let pub_num_lookup = pub_num.clone();
                                    view! {
                                        <div class="subsystem-box">
                                            <div class="subsystem-top">
                                                <span class="subsystem-name">{u.subsystem.clone()}</span>
                                                <span class="fto-badge">{u.freedom_to_operate.clone()}</span>
                                            </div>
                                            <div class="subsystem-patent">
                                                <button
                                                    class="patent-ref-btn"
                                                    title="Inspect Patent"
                                                    on:click=move |_| {
                                                        // Find patent and open modal
                                                        if let Some(p) = state.session.with(|s| {
                                                            s.aspects.iter().flat_map(|asp| &asp.patents).find(|pat| pat.publication_number == pub_num_lookup).cloned()
                                                        }) {
                                                            state.selected_patent.set(Some(p));
                                                        }
                                                    }
                                                >
                                                    {pub_num}
                                                </button>
                                            </div>
                                            <p class="subsystem-app">{u.application.clone()}</p>
                                        </div>
                                    }
                                }).collect_view()
                            })
                        }}
                    </div>
                </div>

                // Section 3: Modeled Timeline & Where Each Patent Helps
                <div class="takeover-card timeline-card">
                    <div class="section-badge">"03 / PHASED IMPLEMENTATION TIMELINE FOR THIS OUTCOME"</div>
                    <div class="timeline-track">
                        {move || {
                            aspect.get().map(|a| {
                                a.timeline.into_iter().enumerate().map(|(idx, step)| {
                                    view! {
                                        <div class="timeline-step">
                                            <div class="timeline-marker">
                                                <span class="marker-circle">{idx + 1}</span>
                                                <span class="marker-line"></span>
                                            </div>
                                            <div class="timeline-content">
                                                <div class="timeline-heading">
                                                    <span class="timeline-phase">{step.phase.clone()}</span>
                                                    <span class="timeline-timeframe">{step.timeframe.clone()}</span>
                                                </div>
                                                <h4 class="timeline-objective">{step.objective.clone()}</h4>
                                                <div class="timeline-patents">
                                                    <span class="patents-label">"ACCELERATING PATENTS:"</span>
                                                    {step.patent_citations.into_iter().map(|cit| {
                                                        let cit_lookup = cit.clone();
                                                        view! {
                                                            <button
                                                                class="timeline-cit-chip"
                                                                on:click=move |_| {
                                                                    if let Some(p) = state.session.with(|s| {
                                                                        s.aspects.iter().flat_map(|asp| &asp.patents).find(|pat| pat.publication_number == cit_lookup).cloned()
                                                                    }) {
                                                                        state.selected_patent.set(Some(p));
                                                                    }
                                                                }
                                                            >
                                                                {cit}
                                                            </button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                                <p class="timeline-tactics">{step.tactical_guidance.clone()}</p>
                                            </div>
                                        </div>
                                    }
                                }).collect_view()
                            })
                        }}
                    </div>
                </div>

                // Section 4: Curated Patent Docket (Small section)
                <div class="takeover-card docket-card">
                    <div class="section-badge">"04 / CURATED PATENT DOCKET (HARVESTED PATENTS)"</div>
                    <div class="docket-list">
                        {move || {
                            aspect.get().map(|a| {
                                a.patents.into_iter().map(|pat| {
                                    let p_modal = pat.clone();
                                    view! {
                                        <div class="docket-row">
                                            <div class="docket-left">
                                                <div class="docket-top">
                                                    <span class="docket-pub mono">{pat.publication_number.clone()}</span>
                                                    <span class=pat.verdict_class.clone()>{pat.us_label.clone()}</span>
                                                    <span class="docket-date">{pat.priority_date.clone()}</span>
                                                </div>
                                                <h5 class="docket-title">{pat.title.clone()}</h5>
                                                <div class="docket-assignee">
                                                    <span class="assignee-label">"Assignee: "</span>
                                                    <span>{pat.assignee.clone()}</span>
                                                </div>
                                                <p class="docket-abstract">{pat.abstract_text.clone()}</p>
                                            </div>
                                            <div class="docket-actions">
                                                <button
                                                    class="btn btn-sm btn-inspect"
                                                    on:click=move |_| state.selected_patent.set(Some(p_modal.clone()))
                                                >
                                                    "INSPECT DOSSIER"
                                                </button>
                                                <a
                                                    class="btn btn-sm btn-link"
                                                    href=pat.link.clone()
                                                    target="_blank"
                                                    rel="noopener noreferrer"
                                                >
                                                    "GOOGLE PATENTS ↗"
                                                </a>
                                            </div>
                                        </div>
                                    }
                                }).collect_view()
                            })
                        }}
                    </div>
                </div>
            </div>
        </section>
    }
}

// ---------------------------------------------------------------------------
// Patent Dossier Modal
// ---------------------------------------------------------------------------

#[component]
fn PatentModal(patent: AspectPatent) -> impl IntoView {
    let state = expect_context::<StrategyState>();

    view! {
        <div class="modal-backdrop" on:click=move |_| state.selected_patent.set(None)>
            <div class="modal-dialog" on:click=move |ev| ev.stop_propagation()>
                <div class="modal-header">
                    <div>
                        <span class="modal-pub mono">{patent.publication_number.clone()}</span>
                        <span class=patent.verdict_class.clone()>{patent.us_label.clone()}</span>
                    </div>
                    <button class="modal-close" on:click=move |_| state.selected_patent.set(None)>
                        "✕"
                    </button>
                </div>

                <h3 class="modal-title">{patent.title.clone()}</h3>

                <div class="modal-meta-grid">
                    <div class="meta-item">
                        <span class="meta-lbl">"Assignee / Owner"</span>
                        <span class="meta-val">{patent.assignee.clone()}</span>
                    </div>
                    <div class="meta-item">
                        <span class="meta-lbl">"Priority Date"</span>
                        <span class="meta-val">{patent.priority_date.clone()}</span>
                    </div>
                    <div class="meta-item">
                        <span class="meta-lbl">"Jurisdiction"</span>
                        <span class="meta-val">{patent.jurisdiction.clone()}</span>
                    </div>
                    <div class="meta-item">
                        <span class="meta-lbl">"US Register Legal Status"</span>
                        <span class="meta-val">
                            {if patent.us_graveyard {
                                "Lapsed in US — Public Domain (100% Free to Build)"
                            } else {
                                "Active / In Force — Design Around Required"
                            }}
                        </span>
                    </div>
                </div>

                <div class="modal-section">
                    <h4>"Abstract & Core Disclosure"</h4>
                    <p class="modal-abstract">{patent.abstract_text.clone()}</p>
                </div>

                <div class="modal-footer">
                    <a
                        class="btn btn-primary"
                        href=patent.link.clone()
                        target="_blank"
                        rel="noopener noreferrer"
                    >
                        "OPEN ON GOOGLE PATENTS ↗"
                    </a>
                    <button
                        class="btn btn-ghost"
                        on:click=move |_| state.selected_patent.set(None)
                    >
                        "CLOSE"
                    </button>
                </div>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// App Terminal Footer (Pinned at Bottom of Web App)
// ---------------------------------------------------------------------------

#[component]
fn StrategyFooter() -> impl IntoView {
    let state = expect_context::<StrategyState>();

    view! {
        <footer class="strategy-footer">
            <div class="footer-left">
                <span class="footer-live-badge">
                    <span class="pulse-dot"></span>
                    {move || if state.loading.get() {
                        "SYNTHESIZING IP PIPELINE..."
                    } else if state.active_aspect_id.get().is_some() {
                        "OUTCOME DILIGENCE ACTIVE"
                    } else {
                        "TERMINAL READY"
                    }}
                </span>
                <span class="footer-sep">"|"</span>
                <span class="footer-subtext">
                    "GROQ LLAMA-3.3-70B · SERPAPI GOOGLE PATENTS · US GRAVEYARD REGISTRY"
                </span>
            </div>

            <div class="footer-center">
                <span class="footer-stat">
                    {move || {
                        state.session.with(|s| {
                            let total_patents: usize = s.aspects.iter().map(|a| a.total_patents).sum();
                            let free_patents: usize = s.aspects.iter().map(|a| a.graveyard_count).sum();
                            format!("5 STRATEGIC OUTCOMES · {} PATENTS ANALYZED · {} PUBLIC DOMAIN (100% FREE)", total_patents, free_patents)
                        })
                    }}
                </span>
            </div>

            <div class="footer-right">
                <span class="footer-shortcut">
                    <kbd class="kbd">"ENTER"</kbd>
                    " RUN"
                </span>
                {move || {
                    if state.active_aspect_id.get().is_some() {
                        view! {
                            <button
                                class="footer-back-chip"
                                on:click=move |_| state.active_aspect_id.set(None)
                            >
                                <kbd class="kbd">"ESC"</kbd>
                                " BACK TO OUTCOMES"
                            </button>
                        }.into_any()
                    } else {
                        view! {
                            <span class="footer-shortcut">
                                <span class="shield-icon">"🛡"</span>
                                " ZERO LICENSE LIABILITY"
                            </span>
                        }.into_any()
                    }
                }}
                <span class="footer-version">"PGY v2.4 PRO"</span>
            </div>
        </footer>
    }
}

// ---------------------------------------------------------------------------
// Vector Blueprint & Schematic SVG Artworks
// ---------------------------------------------------------------------------

fn render_card_schematic(theme: usize) -> impl IntoView {
    match theme {
        1 => view! {
            <svg class="schematic-svg" viewBox="0 0 440 280" fill="none">
                <circle cx="330" cy="140" r="110" class="schem-main" stroke-dasharray="4 2"/>
                <circle cx="330" cy="140" r="85" class="schem-dim"/>
                <circle cx="330" cy="140" r="55" class="schem-dim"/>
                <line x1="220" y1="60" x2="440" y2="60" class="schem-dim"/>
                <line x1="220" y1="100" x2="440" y2="100" class="schem-dim"/>
                <line x1="220" y1="140" x2="440" y2="140" class="schem-dim"/>
                <line x1="220" y1="180" x2="440" y2="180" class="schem-dim"/>
                <line x1="220" y1="220" x2="440" y2="220" class="schem-dim"/>
                <line x1="270" y1="30" x2="270" y2="250" class="schem-accent"/>
                <line x1="330" y1="25" x2="330" y2="255" class="schem-accent"/>
                <line x1="390" y1="30" x2="390" y2="250" class="schem-accent"/>
                <circle cx="330" cy="140" r="3" class="schem-node"/>
                <text x="240" y="265" class="schem-text">"CZ-Si WAFER · t=130μm · TOPCON"</text>
                <text x="290" y="22" class="schem-text">"16-BB METALLIZATION"</text>
            </svg>
        }.into_any(),
        2 => view! {
            <svg class="schematic-svg" viewBox="0 0 440 280" fill="none">
                <polygon points="310,50 350,70 350,110 310,130 270,110 270,70" class="schem-main"/>
                <polygon points="350,110 390,130 390,170 350,190 310,170 310,130" class="schem-main"/>
                <polygon points="270,110 310,130 310,170 270,190 230,170 230,130" class="schem-main"/>
                <polygon points="310,170 350,190 350,230 310,250 270,230 270,190" class="schem-dim"/>
                <circle cx="310" cy="130" r="5" class="schem-node"/>
                <circle cx="350" cy="110" r="3.5" class="schem-node"/>
                <circle cx="270" cy="110" r="3.5" class="schem-node"/>
                <circle cx="310" cy="170" r="4" class="schem-node"/>
                <path d="M220 50 Q260 80 300 80 M240 30 Q280 60 320 60" class="schem-dim"/>
                <text x="225" y="265" class="schem-text">"CVD VAPOR FLUX · CH3NH3PbI3 · Eg=1.55eV"</text>
                <text x="290" y="25" class="schem-text">"TANDEM PEROVSKITE CAGE"</text>
            </svg>
        }.into_any(),
        3 => view! {
            <svg class="schematic-svg" viewBox="0 0 440 280" fill="none">
                <path d="M200 140 Q240 50 280 140 T360 140 T440 140" class="schem-accent"/>
                <path d="M220 190 H235 V170 H250 V190 H265 V170 H280 V190 H295 V170 H310 V190 H325 V170 H340 V190" class="schem-dim"/>
                <circle cx="280" cy="140" r="4" class="schem-node"/>
                <circle cx="360" cy="140" r="4" class="schem-node"/>
                <rect x="325" y="60" width="60" height="36" rx="3" class="schem-main"/>
                <text x="333" y="82" class="schem-text">"MPPT μC"</text>
                <circle cx="310" cy="95" r="16" class="schem-dim"/>
                <text x="230" y="265" class="schem-text">"TRANSFORMERLESS H-BRIDGE · η=99.2%"</text>
                <text x="280" y="25" class="schem-text">"DYNAMIC MPPT TRACKING"</text>
            </svg>
        }.into_any(),
        4 => view! {
            <svg class="schematic-svg" viewBox="0 0 440 280" fill="none">
                <line x1="240" y1="200" x2="410" y2="95" class="schem-accent"/>
                <rect x="230" y="185" width="20" height="28" rx="2" class="schem-main"/>
                <circle cx="325" cy="148" r="14" class="schem-main"/>
                <circle cx="325" cy="148" r="4" class="schem-node"/>
                <line x1="325" y1="148" x2="325" y2="235" class="schem-main"/>
                <path d="M220 235 L300 235 L320 170 M260 235 L340 235 L360 150" class="schem-dim"/>
                <path d="M355 148 A30 30 0 0 0 345 128" class="schem-main"/>
                <text x="230" y="265" class="schem-text">"BIFACIAL ALBEDO +22% · STOW TILT: 0°"</text>
                <text x="270" y="25" class="schem-text">"TORQUE TUBE STRUCTURAL DYNAMICS"</text>
            </svg>
        }.into_any(),
        _ => view! {
            <svg class="schematic-svg" viewBox="0 0 440 280" fill="none">
                <polygon points="260,150 290,210 320,150" class="schem-main"/>
                <polygon points="320,150 350,210 380,150" class="schem-main"/>
                <polygon points="380,150 410,210 440,150" class="schem-main"/>
                <path d="M275 70 L285 180 L305 160 L315 200" class="schem-accent"/>
                <path d="M335 70 L345 180 L365 160 L375 200" class="schem-accent"/>
                <line x1="220" y1="110" x2="440" y2="110" class="schem-dim"/>
                <line x1="220" y1="130" x2="440" y2="130" class="schem-dim"/>
                <text x="235" y="265" class="schem-text">"RAY TRAPPING PYRAMIDS · POE CROSSLINK"</text>
                <text x="290" y="25" class="schem-text">"DUAL-LAYER ARC · R < 0.45%"</text>
            </svg>
        }.into_any(),
    }
}
