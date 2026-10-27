use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

// ---------------------------------------------------------------------------
// Types & Dummy Diligence Data
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Home,
    Chat,
    Report,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: &'static str,
    pub text: String,
    pub citations: Vec<&'static str>,
}

fn initial_messages() -> Vec<Message> {
    vec![
        Message {
            role: "user",
            text: "I am building a company for solar panels, can you find patents for me to use?".to_string(),
            citations: vec![],
        },
        Message {
            role: "assistant",
            text: "I analyzed the US patent corpus and extracted 5 strategic technology pillars for your solar manufacturing venture. All cited patents have lapsed due to non-payment of maintenance fees or expired terms, giving your startup 100% freedom to operate:\n\n1. **Perovskite-Silicon Tandem Cells** (US6201180B1, US5882434A)\n   Deploy copper front contact metallization and rear surface passivation to reach >28% conversion efficiency without royalty overhead.\n\n2. **High-Efficiency Resonant Inverters** (US6590793B1)\n   Soft-switching zero-voltage topologies that cut switching losses in half.\n\n3. **Dual-Axis Mechanical Tracking** (US6058930A)\n   Balanced pivot center-of-gravity tracking mounts that survive 120 mph winds.\n\n4. **Anti-Reflective Sol-Gel Coatings** (US6676870B1)\n   Mesoporous silica dip-coatings providing >95.5% optical transmittance.\n\n5. **DC-Coupled Battery Storage Interface** (US6710471B2)\n   Direct 400V DC bus coupling eliminating round-trip inverter losses.\n\nA full executive diligence report has been compiled and is ready in the Report tab.".to_string(),
            citations: vec!["US6201180B1", "US5882434A", "US6590793B1", "US6058930A", "US6676870B1", "US6710471B2"],
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn copy_text(text: String) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "No browser window.".to_string())?;
    let clipboard = window.navigator().clipboard();
    let write = js_sys::Reflect::get(&clipboard, &wasm_bindgen::JsValue::from_str("writeText"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| "Clipboard unavailable.".to_string())?;
    let promise = write
        .call1(&clipboard, &wasm_bindgen::JsValue::from_str(&text))
        .map_err(|e| format!("{e:?}"))?;
    let promise: js_sys::Promise = promise.dyn_into().map_err(|_| "Invalid promise".to_string())?;
    JsFuture::from(promise).await.map(|_| ()).map_err(|e| format!("{e:?}"))
}

async fn share_text(title: String, text: String) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "No browser window.".to_string())?;
    let navigator = window.navigator();
    let share = js_sys::Reflect::get(&navigator, &wasm_bindgen::JsValue::from_str("share"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| "Native share not supported".to_string())?;

    let data = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&data, &"title".into(), &title.into());
    let _ = js_sys::Reflect::set(&data, &"text".into(), &text.into());

    let promise = share.call1(&navigator, &data).map_err(|e| format!("{e:?}"))?;
    let promise: js_sys::Promise = promise.dyn_into().map_err(|_| "Invalid promise".to_string())?;
    JsFuture::from(promise).await.map(|_| ()).map_err(|e| format!("{e:?}"))
}

// ---------------------------------------------------------------------------
// Entry Point
// ---------------------------------------------------------------------------

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

// ---------------------------------------------------------------------------
// Main App Component
// ---------------------------------------------------------------------------

#[component]
pub fn App() -> impl IntoView {
    let view_mode = RwSignal::new(ViewMode::Home);
    let messages = RwSignal::new(initial_messages());
    let draft = RwSignal::new(String::new());
    let is_talking = RwSignal::new(false);
    let is_thinking = RwSignal::new(false);

    let send_message = Callback::new(move |text: String| {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        draft.set(String::new());
        messages.update(|list| {
            list.push(Message {
                role: "user",
                text: trimmed.clone(),
                citations: vec![],
            });
        });

        is_thinking.set(true);

        // Responsive simulated assistant response
        spawn_local(async move {
            let lower = trimmed.to_lowercase();
            let (reply, cites) = if lower.contains("battery") || lower.contains("electrolyte") {
                (
                    "Identified 3 foundational solid-state battery patents with expired maintenance fees:\n\n- **US6168885B1**: Lithium phosphorus oxynitride (LiPON) thin-film ceramic separator.\n- **US6242132B1**: Sulfide glass electrolyte formulation with 1.2 mS/cm room-temperature ionic conductivity.\n- **US5597660A**: Dry polymer-ceramic composite pressing technique bypassing solvent hazards.\n\nAll three are in the public domain and available for immediate prototype synthesis.",
                    vec!["US6168885B1", "US6242132B1", "US5597660A"],
                )
            } else if lower.contains("drone") || lower.contains("swarm") {
                (
                    "Retrieved 2 lapsed patents for autonomous swarm mesh navigation:\n\n- **US6826453B2**: Decentralized flocking algorithm utilizing local distance vectors without central controller.\n- **US7289906B2**: Optical flow ground-tracking stabilization for GPS-denied environments.\n\nBoth patent grants lapsed at year 8 due to non-payment of USPTO maintenance fees.",
                    vec!["US6826453B2", "US7289906B2"],
                )
            } else {
                (
                    "Reviewed your thesis against the USPTO status database. The foundational claims across this domain expired between 2018 and 2023. You have clear freedom-to-operate across mechanical assembly, thermal dissipation, and control circuitry.\n\nCheck the Report tab to review the complete legal verification and milestone roadmap.",
                    vec!["US6201180B1", "US6590793B1"],
                )
            };

            messages.update(|list| {
                list.push(Message {
                    role: "assistant",
                    text: reply.to_string(),
                    citations: cites,
                });
            });
            is_thinking.set(false);
        });
    });

    view! {
        <div class="shell">
            // Minimalist Top Bar
            <header class="topbar">
                <div class="brand" on:click=move |_| view_mode.set(ViewMode::Home)>
                    <span class="brand-badge">"GT"</span>
                    <span class="brand-title">"GRAVETENDER"</span>
                </div>
                <div class="mode-toggle">
                    <button
                        class="toggle-btn"
                        class:active=move || view_mode.get() == ViewMode::Home
                        on:click=move |_| view_mode.set(ViewMode::Home)
                    >
                        "Home"
                    </button>
                    <button
                        class="toggle-btn"
                        class:active=move || view_mode.get() == ViewMode::Chat
                        on:click=move |_| view_mode.set(ViewMode::Chat)
                    >
                        "Chat"
                    </button>
                    <button
                        class="toggle-btn"
                        class:active=move || view_mode.get() == ViewMode::Report
                        on:click=move |_| view_mode.set(ViewMode::Report)
                    >
                        "Report"
                    </button>
                </div>
            </header>

            // Active Viewport
            <main class="content">
                {move || match view_mode.get() {
                    ViewMode::Home => view! {
                        <HomeView
                            view_mode=view_mode
                            is_talking=is_talking
                        />
                    }.into_any(),
                    ViewMode::Chat => view! {
                        <ChatView
                            messages=messages
                            draft=draft
                            is_thinking=is_thinking
                            send_message=send_message
                            is_talking=is_talking
                        />
                    }.into_any(),
                    ViewMode::Report => view! {
                        <ReportView />
                    }.into_any(),
                }}
            </main>

            // "Talk to Me" Voice Modal Overlay
            {move || is_talking.get().then(|| {
                view! {
                    <div class="voice-overlay" on:click=move |_| is_talking.set(false)>
                        <div class="voice-modal" on:click=move |ev| ev.stop_propagation()>
                            <div class="voice-status">"LISTENING…"</div>
                            <h3 class="voice-title">"Speak your venture thesis"</h3>
                            <div class="voice-wave">
                                <span class="wave-bar b1"></span>
                                <span class="wave-bar b2"></span>
                                <span class="wave-bar b3"></span>
                                <span class="wave-bar b4"></span>
                                <span class="wave-bar b5"></span>
                            </div>
                            <p class="voice-hint">
                                "“I am building a high-efficiency solar panel manufacturing startup…”"
                            </p>
                            <div class="voice-actions">
                                <button
                                    class="voice-done-btn"
                                    on:click=move |_| {
                                        is_talking.set(false);
                                        send_message.run("I am building a company for solar panels, can you find patents for me to use?".to_string());
                                    }
                                >
                                    "Done Speaking"
                                </button>
                                <button
                                    class="voice-cancel-btn"
                                    on:click=move |_| is_talking.set(false)
                                >
                                    "Cancel"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Home View
// ---------------------------------------------------------------------------

#[component]
fn HomeView(
    view_mode: RwSignal<ViewMode>,
    is_talking: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="home-pane">
            <div class="home-scroll">
                // Hero Section
                <div class="home-hero">
                    <div class="home-logo-circle">
                        <span class="home-logo-text">"GT"</span>
                    </div>
                    <h1 class="home-hero-title">"GraveTender"</h1>
                    <p class="home-hero-sub">
                        "Public Domain Patent Intelligence"
                    </p>
                    <p class="home-hero-desc">
                        "Resurrect expired USPTO technology pillars with 100% verified freedom-to-operate. Build next-generation ventures without paying patent royalties."
                    </p>
                </div>

                // Metrics / Status strip
                <div class="home-stats-strip">
                    <div class="stat-cell">
                        <span class="stat-number">"2.1M+"</span>
                        <span class="stat-label">"Lapsed Patents"</span>
                    </div>
                    <div class="stat-divider"></div>
                    <div class="stat-cell">
                        <span class="stat-number">"100%"</span>
                        <span class="stat-label">"Freedom-To-Operate"</span>
                    </div>
                    <div class="stat-divider"></div>
                    <div class="stat-cell">
                        <span class="stat-number">"0%"</span>
                        <span class="stat-label">"Royalty Drag"</span>
                    </div>
                </div>

                // Primary Quick Actions (Cards)
                <div class="home-section-title">"QUICK ACCESS"</div>
                <div class="home-cards">
                    <div class="home-card" on:click=move |_| view_mode.set(ViewMode::Chat)>
                        <div class="card-icon-row">
                            <span class="card-badge">"CHAT"</span>
                            <span class="card-arrow">"→"</span>
                        </div>
                        <h3 class="card-title">"Patent AI Assistant"</h3>
                        <p class="card-desc">
                            "Ask technical questions, search prior art, and discover lapsed patents in battery, solar, robotics & RF."
                        </p>
                    </div>

                    <div class="home-card" on:click=move |_| view_mode.set(ViewMode::Report)>
                        <div class="card-icon-row">
                            <span class="card-badge report">"REPORT"</span>
                            <span class="card-arrow">"→"</span>
                        </div>
                        <h3 class="card-title">"Executive Diligence Memo"</h3>
                        <p class="card-desc">
                            "Review verified 5-pillar technical breakdown, legal USPTO status, and 3-year commercialization roadmap."
                        </p>
                    </div>

                    <div class="home-card voice" on:click=move |_| is_talking.set(true)>
                        <div class="card-icon-row">
                            <span class="card-badge voice">"VOICE"</span>
                            <span class="card-pulse-dot">"●"</span>
                        </div>
                        <h3 class="card-title">"Talk to Me"</h3>
                        <p class="card-desc">
                            "Dictate your venture thesis or hardware prototype idea to find matching expired patent architectures hands-free."
                        </p>
                    </div>
                </div>

                // Active Intelligence Highlight
                <div class="home-section-title">"ACTIVE RESEARCH PIPELINE"</div>
                <div class="home-featured-box">
                    <div class="featured-tag">"VERIFIED FTO · SOLAR PV"</div>
                    <h4 class="featured-title">"Silicon-Perovskite & Resonant Inverters"</h4>
                    <p class="featured-body">
                        "5 foundational patents (US6201180B1, US5882434A, US6590793B1) verified expired due to non-payment of maintenance fees."
                    </p>
                    <div class="featured-actions">
                        <button class="featured-btn primary" on:click=move |_| view_mode.set(ViewMode::Report)>
                            "Open Full Diligence"
                        </button>
                        <button class="featured-btn secondary" on:click=move |_| view_mode.set(ViewMode::Chat)>
                            "Query AI"
                        </button>
                    </div>
                </div>

                // Minimalist Footer
                <div class="home-footer">
                    <span>"GraveTender v0.1.0 · Public Domain Technology Engine"</span>
                </div>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Chat View
// ---------------------------------------------------------------------------

#[component]
fn ChatView(
    messages: RwSignal<Vec<Message>>,
    draft: RwSignal<String>,
    is_thinking: RwSignal<bool>,
    send_message: Callback<String>,
    is_talking: RwSignal<bool>,
) -> impl IntoView {
    let list_ref = NodeRef::<leptos::html::Div>::new();

    Effect::new(move |_| {
        let _ = messages.get();
        let _ = is_thinking.get();
        if let Some(el) = list_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    view! {
        <div class="chat-pane">
            <div class="message-list" node_ref=list_ref>
                {move || {
                    messages.get().into_iter().map(|msg| {
                        let is_user = msg.role == "user";
                        let citations = msg.citations;
                        view! {
                            <div class="message-row" class:user=is_user>
                                <div class="message-bubble" class:user=is_user>
                                    <div class="message-role">
                                        {if is_user { "YOU" } else { "PATENT ANALYST" }}
                                    </div>
                                    <div class="message-text">
                                        {msg.text}
                                    </div>
                                    {(!citations.is_empty()).then(|| view! {
                                        <div class="citation-strip">
                                            <span class="citation-label">"CITATIONS:"</span>
                                            {citations.into_iter().map(|c| {
                                                let url = format!("https://patents.google.com/patent/{}/en", c);
                                                view! {
                                                    <a class="citation-chip" href=url target="_blank" rel="noopener">
                                                        {c}
                                                        <span class="chip-status">" [LAPSED]"</span>
                                                    </a>
                                                }
                                            }).collect_view()}
                                        </div>
                                    })}
                                </div>
                            </div>
                        }
                    }).collect_view()
                }}

                {move || is_thinking.get().then(|| view! {
                    <div class="message-row">
                        <div class="message-bubble thinking">
                            <span class="typing-dot"></span>
                            <span class="typing-dot"></span>
                            <span class="typing-dot"></span>
                            <span class="typing-label">"Evaluating USPTO legal status…"</span>
                        </div>
                    </div>
                })}
            </div>

            // Bottom Input Controls: Text Bar + Prominent "Talk to Me" Button
            <div class="input-container">
                <div class="composer-row">
                    <input
                        type="text"
                        class="text-input"
                        placeholder="Ask a patent question…"
                        prop:value=move || draft.get()
                        on:input=move |ev| draft.set(event_target_value(&ev))
                        on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                let t = draft.get_untracked();
                                if !t.trim().is_empty() {
                                    send_message.run(t);
                                }
                            }
                        }
                    />
                    <button
                        class="send-btn"
                        prop:disabled=move || draft.get().trim().is_empty() || is_thinking.get()
                        on:click=move |_| {
                            let t = draft.get_untracked();
                            if !t.trim().is_empty() {
                                send_message.run(t);
                            }
                        }
                    >
                        "Send"
                    </button>
                </div>

                // Solid Minimalist "Talk to Me" Button
                <button
                    class="talk-btn"
                    on:click=move |_| is_talking.set(true)
                >
                    <span class="mic-icon">"●"</span>
                    <span class="talk-text">"Talk to me"</span>
                </button>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Report View
// ---------------------------------------------------------------------------

#[component]
fn ReportView() -> impl IntoView {
    let copied = RwSignal::new(false);

    let report_content = "# STRATEGIC DILIGENCE REPORT: SOLAR PV COMMERCIALIZATION\n\n\
Status: 100% Public Domain · Freedom to Operate Confirmed\n\
Venture Thesis: Manufacturing high-efficiency crystalline silicon and tandem solar modules by deploying expired US patent families across cell physics, power electronics, tracking systems, and balance-of-system integration.\n\n\
## 1. Key Lapsed Patent Assets\n\
- US6201180B1 (AstroPower Inc.): Screen-printed copper front contact metallization. Lapsed for fee non-payment.\n\
- US5882434A (SunPower Corp.): Rear-surface dielectric stack passivation (SiNx/AlOx) eliminating carrier recombination.\n\
- US6590793B1 (SatCon Tech): Zero-voltage soft-switching resonant DC-DC inverter topologies.\n\
- US6058930A (Northern Power Systems): Balanced center-of-gravity single/dual axis slew drive tracker.\n\
- US6676870B1 (PPG Industries): Mesoporous silica sol-gel anti-reflective front glass coating.\n\
- US6710471B2 (OutBack Power): 400V DC-coupled battery energy storage interface.\n\n\
## 2. Freedom to Operate Assessment\n\
All foundational patent claims for the above architectures have lapsed or expired under 35 U.S.C. 154/41. No foreign priority claims or active terminal disclaimers impede commercial manufacture and sale within the United States.\n\n\
## 3. Commercial Roadmap\n\
- Phase 1 (M1–M6): Coupon laboratory validation of expired contact recipes.\n\
- Phase 2 (M7–M18): Pilot line roll-to-roll sputtering and cell stringing.\n\
- Phase 3 (M19–M36): IEC 61215 certification and utility-scale field deployment.";

    let copy_report = Callback::new(move |_: ()| {
        let text = report_content.to_string();
        spawn_local(async move {
            if copy_text(text).await.is_ok() {
                copied.set(true);
            }
        });
    });

    let share_report = Callback::new(move |_: ()| {
        let title = "Strategic Patent Diligence Report".to_string();
        let text = report_content.to_string();
        spawn_local(async move {
            if share_text(title, text.clone()).await.is_err() && copy_text(text).await.is_ok() {
                copied.set(true);
            }
        });
    });

    view! {
        <div class="report-pane">
            <div class="report-header">
                <div>
                    <span class="report-tag">"EXECUTIVE MEMO"</span>
                    <h2 class="report-title">"Strategic Patent Diligence"</h2>
                    <span class="report-sub">"Solar Panel Manufacturing & Deployment"</span>
                </div>
                <div class="report-status-badge">
                    "✓ 100% Free to Operate"
                </div>
            </div>

            <div class="report-scroll">
                // Executive Overview Card
                <section class="report-section">
                    <h3 class="section-title">"1. Executive Thesis"</h3>
                    <p class="section-body">
                        "Your startup can construct a next-generation high-efficiency solar module manufacturing line \
                         without paying licensing royalties by deploying expired US patent families across cell passivating \
                         contacts, resonant power electronics, balanced tracking mounts, and sol-gel surface coatings."
                    </p>
                </section>

                // 5 Technology Pillars Card
                <section class="report-section">
                    <h3 class="section-title">"2. Five Strategic Technology Aspects"</h3>
                    <div class="aspect-table">
                        <div class="aspect-row">
                            <span class="aspect-num">"01"</span>
                            <div class="aspect-content">
                                <strong>"Perovskite-Silicon Tandem Cells"</strong>
                                <p>"Use expired PERC and dielectric stack passivation (US5882434A) to achieve >28% lab conversion efficiency."</p>
                                <span class="patent-pill">"US6201180B1 · US5882434A"</span>
                            </div>
                        </div>
                        <div class="aspect-row">
                            <span class="aspect-num">"02"</span>
                            <div class="aspect-content">
                                <strong>"Soft-Switching Power Electronics"</strong>
                                <p>"Zero-voltage switching topologies (US6590793B1) reducing inverter MOSFET switching losses by 50%."</p>
                                <span class="patent-pill">"US6590793B1 · US6882131B1"</span>
                            </div>
                        </div>
                        <div class="aspect-row">
                            <span class="aspect-num">"03"</span>
                            <div class="aspect-content">
                                <strong>"Dual-Axis Mechanical Tracking"</strong>
                                <p>"Balanced center-of-gravity rotational mounts (US6058930A) cutting motor power needs by 65%."</p>
                                <span class="patent-pill">"US6058930A · US6420646B1"</span>
                            </div>
                        </div>
                        <div class="aspect-row">
                            <span class="aspect-num">"04"</span>
                            <div class="aspect-content">
                                <strong>"Anti-Reflective & Hydrophobic Coatings"</strong>
                                <p>"Mesoporous silica sol-gel dip-coatings (US6676870B1) yielding >95.5% broadband optical transmittance."</p>
                                <span class="patent-pill">"US6676870B1 · US6387498B1"</span>
                            </div>
                        </div>
                        <div class="aspect-row">
                            <span class="aspect-num">"05"</span>
                            <div class="aspect-content">
                                <strong>"Direct DC Bus Storage Coupling"</strong>
                                <p>"Coupling solar strings and lithium battery packs directly onto a shared 400V DC bus (US6710471B2)."</p>
                                <span class="patent-pill">"US6710471B2 · US6882063B2"</span>
                            </div>
                        </div>
                    </div>
                </section>

                // Legal Status Verification
                <section class="report-section">
                    <h3 class="section-title">"3. Legal Status Verification"</h3>
                    <p class="section-body">
                        "Every cited patent has been verified against USPTO maintenance fee records. \
                         All terms have lapsed or reached statutory 20-year expiry under 35 U.S.C. 154. \
                         Freedom-to-operate in the United States is rated 100% Unencumbered."
                    </p>
                </section>

                // 3-Year Commercialization Roadmap
                <section class="report-section">
                    <h3 class="section-title">"4. Commercialization Roadmap"</h3>
                    <div class="roadmap-list">
                        <div class="roadmap-item">
                            <span class="phase-tag">"PHASE 1 (M1–M6)"</span>
                            <strong>"Lab Validation & Formulation"</strong>
                            <p>"Synthesize test coupons using expired PERC geometries and sol-gel recipes."</p>
                        </div>
                        <div class="roadmap-item">
                            <span class="phase-tag">"PHASE 2 (M7–M18)"</span>
                            <strong>"Pilot Line Fabrication"</strong>
                            <p>"Scale to M10 wafer format with automated copper screen-printing and stringing."</p>
                        </div>
                        <div class="roadmap-item">
                            <span class="phase-tag">"PHASE 3 (M19–M36)"</span>
                            <strong>"IEC Certification & Field Deployment"</strong>
                            <p>"Complete IEC 61215 module qualification and deploy 50MW reference tracker field."</p>
                        </div>
                    </div>
                </section>
            </div>

            // Action Buttons
            <div class="report-actions">
                <button class="action-btn secondary" on:click=move |_| copy_report.run(())>
                    {move || if copied.get() { "✓ Copied Markdown" } else { "Copy Markdown" }}
                </button>
                <button class="action-btn primary" on:click=move |_| share_report.run(())>
                    "Share Report"
                </button>
            </div>
        </div>
    }
}
