//! A self-contained, dependency-free network map rendered to `<canvas>`.
//!
//! We deliberately do **not** pull in a JavaScript charting library. The whole
//! visualisation is a small force-directed layout implemented in Rust and drawn
//! through `web-sys`, which keeps the WASM bundle tiny and the behaviour fully
//! under our control.
//!
//! Layout is deterministic (seeded by node id) so the server pass and the first
//! hydrated frame agree, and so two partners looking at the same seed set see
//! the same picture.
//!
//! On the server there is no canvas, so the SSR build renders the same markup
//! without drawing. The layout maths is shared and compiled in both builds; only
//! the drawing calls are gated behind the `hydrate` feature.
//!
//! Layout discipline: the solve is a one-shot, deterministic batch that runs
//! only when the graph's *shape* changes (node ids/roles, edges, stage size).
//! Hover, lens and search are pure paint and can never restart the simulation —
//! which is why the settled map never jitters.

use std::collections::HashMap;

use leptos::prelude::*;

use crate::discovery::{Edge, GraphNode};

/// Props for the canvas. The parent owns the reactive graph; the canvas draws it.
///
/// `focus` is the id of the entity currently in the lens: nodes that do not
/// belong to that entity are dimmed, so the user can read one actor's slice of
/// the landscape without leaving the map.
///
/// `query` is the free-text search: while it is non-empty, monuments that do
/// not match are dimmed. It is a *paint* signal — it never moves the stones.
#[component]
pub fn NetworkCanvas(
    nodes: Signal<Vec<GraphNode>>,
    edges: Signal<Vec<Edge>>,
    focus: Signal<Option<String>>,
    query: Signal<String>,
    on_node_click: Callback<String>,
) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    // Logical (CSS-pixel) size of the drawing surface, measured from the parent.
    let size = RwSignal::new((900.0_f64, 520.0_f64));
    // Nothing is drawn until the real size is known, so the first painted frame
    // is already correct.
    let measured = RwSignal::new(false);
    // Bumped to request (re)measurement — hydration retries and window resizes.
    let remeasure = RwSignal::new(0_u32);

    // Positions keyed by node id. They are rebuilt from scratch —
    // deterministically — whenever the *graph itself* changes, and never when
    // merely repaint signals change (hover, lens, search). This is what keeps
    // the map absolutely still while the cursor moves.
    let positions = StoredValue::new(HashMap::<String, (f64, f64)>::new());
    let last_sig = StoredValue::new(String::new());
    // The node under the cursor, for the tooltip.
    let hovered = RwSignal::new(None::<String>);
    // Pure paint viewport: pan offset in CSS px + zoom factor. Pan and zoom
    // only change the transform the stage is drawn through — the settled
    // stone layout is never re-solved because of them.
    let viewport = RwSignal::new((0.0_f64, 0.0_f64));
    let zoom = RwSignal::new(1.0_f64);
    // Drag state: the cursor position (CSS px) and the viewport when the drag
    // started. `None` between gestures.
    let dragging = RwSignal::new(None::<(f64, f64, f64, f64)>);
    // Where the last mousedown landed, so a click is only honoured when the
    // pointer barely moved — otherwise a drag ending on a stone would also
    // open that stone.
    let down_at = StoredValue::new((0.0_f64, 0.0_f64));

    // Measure the container on demand: once on mount, and again every time
    // `remeasure` ticks. Hydration can beat the browser's layout pass, so a
    // single measure-on-mount could catch a zero-height pane and blank the map
    // for the whole session — retries continue at display refresh rate until
    // the stage reports a real size, and resizes re-measure too.
    Effect::new(move |_| {
        let _ = remeasure.get();
        if let Some(dims) = measure(canvas_ref) {
            if dims != size.get_untracked() {
                size.set(dims);
            }
            measured.update(|m| *m = true);
        }
    });

    #[cfg(feature = "hydrate")]
    {
        drive_measurement(remeasure, measured);
        let tick = remeasure;
        let _ = window_event_listener(leptos::ev::resize, move |_| {
            tick.update(|t| *t = t.wrapping_add(1));
        });
        // A drag may end outside the canvas — release on the window.
        let drag_done = dragging;
        let _ = window_event_listener(leptos::ev::mouseup, move |_| {
            drag_done.set(None);
        });
    }

    // Redraw on graph / size / focus / search / hover / viewport changes.
    Effect::new(move |_| {
        let ns = nodes.get();
        let es = edges.get();
        let lens = focus.get();
        let needle = query.get();
        let hover = hovered.get();
        let pan = viewport.get();
        let zoomed = zoom.get();
        if ns.is_empty() || !measured.get() {
            return;
        }
        let dims = size.get();
        // Only the graph's *shape* may move the stones. Hover and search are
        // pure paint, so the settled layout never jitters or restarts.
        let sig = graph_signature(&ns, &es, dims);
        let stale = last_sig.with_value(|s| s.as_str() != sig);
        if stale {
            let fresh = solve_layout(&ns, &es, dims);
            last_sig.update_value(|s| *s = sig);
            positions.update_value(|p| *p = fresh);
        }
        let snapshot = positions.get_value();
        let needle = needle.trim().to_lowercase();
        draw(
            canvas_ref,
            &ns,
            &es,
            &snapshot,
            dims,
            lens.as_deref(),
            &needle,
            hover.as_deref(),
            pan,
            zoomed,
        );
    });

    // Stage-space (logical) coordinates of the cursor, i.e. CSS px relative
    // to the canvas corner minus the pan offset, then un-zoomed.
    let stage_xy = move |ev: &leptos::ev::MouseEvent| -> Option<(f64, f64)> {
        let origin = canvas_origin(canvas_ref)?;
        let (pan, z) = (viewport.get_untracked(), zoom.get_untracked());
        Some((
            (ev.client_x() as f64 - origin.0 - pan.0) / z,
            (ev.client_y() as f64 - origin.1 - pan.1) / z,
        ))
    };

    let pan_start = move |ev: leptos::ev::MouseEvent| {
        let Some(origin) = canvas_origin(canvas_ref) else {
            return;
        };
        let pan = viewport.get_untracked();
        dragging.set(Some((
            ev.client_x() as f64 - origin.0,
            ev.client_y() as f64 - origin.1,
            pan.0,
            pan.1,
        )));
        down_at.set_value((
            ev.client_x() as f64 - origin.0,
            ev.client_y() as f64 - origin.1,
        ));
    };

    let pan_move = move |ev: leptos::ev::MouseEvent| {
        let Some(origin) = canvas_origin(canvas_ref) else {
            return;
        };
        if let Some((sx, sy, px, py)) = dragging.get_untracked() {
            let dx = ev.client_x() as f64 - origin.0 - sx;
            let dy = ev.client_y() as f64 - origin.1 - sy;
            viewport.set((px + dx, py + dy));
            hovered.set(None);
            return;
        }
        // Track the closest node under the cursor (in stage px).
        if let Some((x, y)) = stage_xy(&ev) {
            let snapshot = positions.get_value();
            let mut best: Option<(String, f64)> = None;
            for (id, (nx, ny)) in snapshot.iter() {
                let d = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
                if d < 26.0 && best.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
                    best = Some((id.clone(), d));
                }
            }
            hovered.set(best.map(|(id, _)| id));
        }
    };

    let click = move |ev: leptos::ev::MouseEvent| {
        // A drag is not a click: only honour dabs.
        if let Some((ox, oy)) = canvas_origin(canvas_ref) {
            let (x, y) = (ev.client_x() as f64 - ox, ev.client_y() as f64 - oy);
            let (sx, sy) = down_at.get_value();
            if ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() > 4.0 {
                return;
            }
        }
        let Some((x, y)) = stage_xy(&ev) else {
            return;
        };
        let snapshot = positions.get_value();
        let mut best: Option<(String, f64)> = None;
        for (id, (nx, ny)) in snapshot.iter() {
            let d = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
            if d < 34.0 && best.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
                best = Some((id.clone(), d));
            }
        }
        if let Some((id, _)) = best {
            on_node_click.run(id);
        }
    };

    // Wheel zooms about the cursor: the point under the pointer stays put.
    let wheel_zoom = move |ev: leptos::ev::WheelEvent| {
        let Some(origin) = canvas_origin(canvas_ref) else {
            return;
        };
        let (px, py) = viewport.get_untracked();
        let z0 = zoom.get_untracked();
        let factor = if ev.delta_y() < 0.0 { 1.1 } else { 1.0 / 1.1 };
        let z1 = (z0 * factor).clamp(0.4, 3.0);
        let cx = ev.client_x() as f64 - origin.0;
        let cy = ev.client_y() as f64 - origin.1;
        viewport.set((cx - (cx - px) * z1 / z0, cy - (cy - py) * z1 / z0));
        zoom.set(z1);
        ev.prevent_default();
    };

    // Double-click anywhere off a drag puts the whole graveyard back in frame.
    let reset_view = move |_| {
        viewport.set((0.0, 0.0));
        zoom.set(1.0);
    };

    view! {
        <canvas
            node_ref=canvas_ref
            class="network-canvas"
            class:dragging=Signal::derive(move || dragging.get().is_some())
            on:click=click
            on:mousedown=pan_start
            on:mousemove=pan_move
            on:mouseleave=move |_| hovered.set(None)
            on:wheel=wheel_zoom
            on:dblclick=reset_view
        ></canvas>
    }
}

// ---------------------------------------------------------------------------
// Force-directed layout (Fruchterman–Reingold, converging batch solve)
// ---------------------------------------------------------------------------

fn hash_unit(s: &str) -> f64 {
    // Deterministic pseudo-random in [0,1) from a string.
    let mut h: u64 = 1469598103934665603;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    (h % 10_000) as f64 / 10_000.0
}

/// A stable fingerprint of everything the layout cares about: node ids and
/// roles, every edge (source, target, kind), and the stage size. Identical
/// fingerprints skip re-solving, which is why hovering or searching never
/// disturbs the settled map.
fn graph_signature(nodes: &[GraphNode], edges: &[Edge], (w, h): (f64, f64)) -> String {
    let mut marks: Vec<String> = nodes
        .iter()
        .map(|n| format!("{}:{}", n.id, n.role))
        .collect();
    marks.sort();
    let mut links: Vec<String> = edges
        .iter()
        .map(|e| format!("{}>{}{:?}", e.source, e.target, e.kind))
        .collect();
    links.sort();
    links.dedup();
    format!(
        "{}x{}|{}|{}",
        w as i32,
        h as i32,
        marks.join(","),
        links.join(",")
    )
}

/// Solve the whole layout to convergence in one deterministic batch.
///
/// The result depends only on the graph and the stage size — never on prior
/// positions — so the same graph always settles into the same constellation,
/// and repainting cannot make it creep or oscillate.
///
/// ResearchRabbit-style grouping: the graph is first split into communities
/// along its strongest edges (union-find, keeping the best links per node).
/// Every community grows outward from its own anchor on a sunflower spiral,
/// and a cluster-gravity term keeps each group dense while repulsion spreads
/// the clusters apart — so the closer two patents really are, the closer they
/// sit, and unrelated clusters read as separate archipelagos.
fn solve_layout(
    nodes: &[GraphNode],
    edges: &[Edge],
    (w, h): (f64, f64),
) -> HashMap<String, (f64, f64)> {
    let n = nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    let group_of = communities(nodes, edges);
    let group_count = group_of.iter().copied().max().map(|g| g + 1).unwrap_or(1);

    // One anchor per community, on a phyllotaxis (sunflower) spiral: cluster 0
    // dead centre, later clusters fanning out at the golden angle. Deterministic
    // and naturally spread, regardless of how many groups there are.
    let ring = (w.min(h) / 2.0) * 0.62;
    let mut anchors: Vec<(f64, f64)> = Vec::with_capacity(group_count);
    for gi in 0..group_count {
        let angle = gi as f64 * GOLDEN_ANGLE - std::f64::consts::FRAC_PI_2;
        let r = ring * (gi as f64).sqrt() / (group_count as f64).sqrt();
        anchors.push((w / 2.0 + angle.cos() * r, h / 2.0 + angle.sin() * r));
    }

    // Start every node near its community's anchor with a dash of hash jitter,
    // so internal FR jostling starts from a sane, cluster-coherent place.
    let mut px: Vec<f64> = Vec::with_capacity(n);
    let mut py: Vec<f64> = Vec::with_capacity(n);
    for (i, node) in nodes.iter().enumerate() {
        let (ax, ay) = anchors[group_of[i]];
        px.push(ax + (hash_unit(&node.id) - 0.5) * 70.0);
        py.push(ay + (hash_unit(&format!("r{}", node.id)) - 0.5) * 70.0);
    }
    let mut positions: HashMap<String, (f64, f64)> = HashMap::with_capacity(n);

    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();

    let area = w * h;
    let k = (area / n as f64).sqrt() * 0.62;
    let iterations = 220;
    let mut temperature = w.min(h) / 8.0;
    let cooling = temperature / (iterations as f64 + 1.0);

    for _ in 0..iterations {
        let mut dx = vec![0.0_f64; n];
        let mut dy = vec![0.0_f64; n];

        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let mut vx = px[i] - px[j];
                let mut vy = py[i] - py[j];
                let mut dist = (vx * vx + vy * vy).sqrt();
                if dist < 0.01 {
                    vx = 0.01;
                    vy = 0.01;
                    dist = 0.01;
                }
                let force = (k * k) / dist;
                let ux = vx / dist;
                let uy = vy / dist;
                dx[i] += ux * force;
                dy[i] += uy * force;
                dx[j] -= ux * force;
                dy[j] -= uy * force;
            }
        }

        // Attraction along edges, weighted so family links pull hardest.
        for e in edges {
            let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str()))
            else {
                continue;
            };
            let vx = px[a] - px[b];
            let vy = py[a] - py[b];
            let dist = (vx * vx + vy * vy).sqrt().max(0.01);
            let force = (dist * dist) / k * (0.35 + e.weight * 0.12);
            let ux = vx / dist;
            let uy = vy / dist;
            dx[a] -= ux * force;
            dy[a] -= uy * force;
            dx[b] += ux * force;
            dy[b] += uy * force;
        }

        // Cluster gravity: pull every node toward its own community's live
        // centroid (dense groups), plus a whisper of global centring so the
        // whole constellation can't drift to the rim.
        let mut cgx = vec![0.0_f64; group_count];
        let mut cgy = vec![0.0_f64; group_count];
        let mut cnt = vec![0.0_f64; group_count];
        for i in 0..n {
            let g = group_of[i];
            cgx[g] += px[i];
            cgy[g] += py[i];
            cnt[g] += 1.0;
        }
        for g in 0..group_count {
            if cnt[g] > 0.0 {
                cgx[g] /= cnt[g];
                cgy[g] /= cnt[g];
            }
        }
        for i in 0..n {
            let g = group_of[i];
            dx[i] += (cgx[g] - px[i]) * 0.04;
            dy[i] += (cgy[g] - py[i]) * 0.04;
            dx[i] += (w / 2.0 - px[i]) * 0.008;
            dy[i] += (h / 2.0 - py[i]) * 0.008;
        }

        let margin_x = (w * 0.35).min(90.0);
        let margin_y = (h * 0.30).min(60.0);
        for i in 0..n {
            let disp = (dx[i] * dx[i] + dy[i] * dy[i]).sqrt().max(0.01);
            let limited = disp.min(temperature);
            px[i] += dx[i] / disp * limited;
            py[i] += dy[i] / disp * limited;
            px[i] = px[i].clamp(margin_x, w - margin_x);
            py[i] = py[i].clamp(margin_y, h - margin_y);
        }
        temperature -= cooling;
        if temperature < 0.01 {
            break;
        }
    }

    for (i, node) in nodes.iter().enumerate() {
        positions.insert(node.id.clone(), (px[i], py[i]));
    }
    positions
}

/// The golden angle — spacing of the community anchors on the spiral.
const GOLDEN_ANGLE: f64 = 2.399963229728653;

/// Split the graph into communities by keeping only the strongest edges
/// (a ResearchRabbit "similarity" tier per node), then grouping by union-find.
/// Every node ends up in exactly one community; isolated nodes form
/// singletons. Deterministic: ties break on `(source, target)`.
fn communities(nodes: &[GraphNode], edges: &[Edge]) -> Vec<usize> {
    let n = nodes.len();
    if n == 0 {
        return std::vec::Vec::new();
    }
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();

    let mut cand: Vec<&Edge> = edges
        .iter()
        .filter(|e| index.contains_key(e.source.as_str()) && index.contains_key(e.target.as_str()))
        .collect();
    cand.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.target.cmp(&b.target))
    });

    // Union-find with path compression.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, mut x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        while parent[x] != root {
            let next = parent[x];
            parent[x] = root;
            x = next;
        }
        root
    }

    let mut degree = vec![0usize; n];
    const MAX_LINKS: usize = 3;
    for e in cand {
        let a = index[e.source.as_str()];
        let b = index[e.target.as_str()];
        if degree[a] >= MAX_LINKS || degree[b] >= MAX_LINKS {
            continue;
        }
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            parent[ra] = rb;
            degree[a] += 1;
            degree[b] += 1;
        }
    }

    // Renumber roots so communities get a stable, size-descending order
    // (bigger communities anchor first, nearer the stage centre).
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        members
            .entry(find(&mut parent, i))
            .or_insert_with(|| std::vec::Vec::new())
            .push(i);
    }
    let mut ordered: Vec<(usize, String, &Vec<usize>)> = members
        .iter()
        .map(|(root, list)| {
            let min_id = nodes[list.iter().copied().min().unwrap_or(0)].id.clone();
            (*root, min_id, list)
        })
        .collect();
    ordered.sort_by(|a, b| {
        b.2.len()
            .cmp(&a.2.len())
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut group_of = vec![0usize; n];
    for (rank, (_, _, list)) in ordered.iter().enumerate() {
        for i in list.iter() {
            group_of[*i] = rank;
        }
    }
    group_of
}

/// A node's colours: (fill, stroke). Tuned for the light paper stage.
/// Amber = where the walk started (seed), green = lapsed and free to build on,
/// red = still enforced, slate = no verdict.
#[cfg(feature = "hydrate")]
fn color_for(node: &GraphNode) -> (&'static str, &'static str) {
    if node.role == "seed" {
        ("#f6b73c", "#a16207")
    } else if node.us_graveyard {
        ("#37c464", "#15803d")
    } else if node.us_active {
        ("#f56b60", "#b91c1c")
    } else {
        ("#c9cfd8", "#8b93a1")
    }
}

// ---------------------------------------------------------------------------
// Browser-only rendering
// ---------------------------------------------------------------------------

/// Size the drawing buffer to its container (at device pixel ratio) and return
/// the logical size in CSS pixels. No-op on the server, where there is no DOM.
/// The buffer is only resized when its current size no longer matches, so a
/// re-measure alone never wipes a painted frame.
#[cfg(feature = "hydrate")]
fn measure(canvas_ref: NodeRef<leptos::html::Canvas>) -> Option<(f64, f64)> {
    use leptos::web_sys::HtmlCanvasElement;
    let canvas = canvas_ref.get()?;
    let canvas: &HtmlCanvasElement = canvas.as_ref();
    let parent = canvas.parent_element()?;
    let w = parent.client_width() as f64;
    let h = parent.client_height() as f64;
    if w < 20.0 || h < 20.0 {
        return None;
    }
    let dpr = leptos::web_sys::window()
        .map(|win| win.device_pixel_ratio())
        .unwrap_or(1.0);
    let want_w = (w * dpr).round() as u32;
    let want_h = (h * dpr).round() as u32;
    if canvas.width() != want_w || canvas.height() != want_h {
        canvas.set_width(want_w);
        canvas.set_height(want_h);
    }
    Some((w, h))
}

/// Nudge the measuring effect every animation frame for a fixed window, so
/// spells where the layout lands late (CSS, fonts) always get caught: even a
/// mediocre first measure gets corrected while this is still running.
#[cfg(feature = "hydrate")]
fn drive_measurement(remeasure: RwSignal<u32>, _measured: RwSignal<bool>) {
    request_animation_frame(move || {
        if remeasure.get_untracked() < 90 {
            remeasure.update(|t| *t = t.wrapping_add(1));
            drive_measurement(remeasure, _measured);
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn measure(_canvas_ref: NodeRef<leptos::html::Canvas>) -> Option<(f64, f64)> {
    None
}

#[cfg(feature = "hydrate")]
fn canvas_origin(canvas_ref: NodeRef<leptos::html::Canvas>) -> Option<(f64, f64)> {
    use leptos::web_sys::HtmlCanvasElement;
    let canvas = canvas_ref.get()?;
    let canvas: &HtmlCanvasElement = canvas.as_ref();
    let rect = canvas.get_bounding_client_rect();
    Some((rect.left(), rect.top()))
}

#[cfg(feature = "hydrate")]
fn draw(
    canvas_ref: NodeRef<leptos::html::Canvas>,
    nodes: &[GraphNode],
    edges: &[Edge],
    positions: &HashMap<String, (f64, f64)>,
    (w, h): (f64, f64),
    focus: Option<&str>,
    query: &str,
    hovered: Option<&str>,
    pan: (f64, f64),
    zoom: f64,
) {
    use leptos::wasm_bindgen::JsCast;
    use leptos::web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

    let Some(canvas) = canvas_ref.get() else {
        return;
    };
    let canvas: &HtmlCanvasElement = canvas.as_ref();
    let Ok(Some(ctx)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx) = ctx.dyn_into::<CanvasRenderingContext2d>() else {
        return;
    };

    let dpr = leptos::web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    // Clear in raw device space (the transform below may push the stage
    // around, so clearing in stage space can miss off-viewport regions),
    // then draw through the pan/zoom viewport transform.
    let _ = ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    let _ = ctx.clear_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);
    let _ = ctx.set_transform(dpr * zoom, 0.0, 0.0, dpr * zoom, dpr * pan.0, dpr * pan.1);

    // Is the lens (entity filter) on? If so, everything outside it dims.
    let dim = |node: Option<&GraphNode>| {
        focus.map(|entity| {
            let out = node
                .map(|n| !n.assignee.eq_ignore_ascii_case(entity))
                .unwrap_or(true);
            out
        })
    };

    // Is a search narrowing the map? Non-matching stones fall back, but
    // nothing moves — this is pure paint.
    let needle = query.trim().to_lowercase();
    let searching = !needle.is_empty();
    let matches_query = |node: &GraphNode| -> bool {
        !searching
            || node.label.to_lowercase().contains(&needle)
            || node.assignee.to_lowercase().contains(&needle)
            || node.id.to_lowercase().contains(&needle)
    };

    // Edges.
    for e in edges {
        let (Some(&(ax, ay)), Some(&(bx, by))) =
            (positions.get(&e.source), positions.get(&e.target))
        else {
            continue;
        };
        let src = nodes.iter().find(|n| n.id == e.source);
        let dst = nodes.iter().find(|n| n.id == e.target);
        let lens_drop = dim(src).or_else(|| dim(dst)).unwrap_or(false);
        let search_drop = searching
            && src.map(matches_query) != Some(true)
            && dst.map(matches_query) != Some(true);
        let alpha = (0.24 + e.weight * 0.12).min(0.85);
        let alpha = if lens_drop {
            alpha * 0.12
        } else if search_drop {
            alpha * 0.2
        } else {
            alpha
        };
        let stroke = match e.kind {
            crate::discovery::RelationKind::Family => format!("rgba(21, 128, 61, {alpha})"),
            crate::discovery::RelationKind::SharedAssignee => {
                format!("rgba(37, 99, 235, {alpha})")
            }
            crate::discovery::RelationKind::SharedInventor => {
                format!("rgba(180, 83, 9, {alpha})")
            }
            crate::discovery::RelationKind::CoCitation => format!("rgba(13, 148, 136, {alpha})"),
            crate::discovery::RelationKind::Lexical => format!("rgba(100, 116, 139, {alpha})"),
        };
        ctx.set_stroke_style_str(&stroke);
        ctx.set_line_width(if e.kind == crate::discovery::RelationKind::Family {
            2.4
        } else {
            1.0
        });
        ctx.begin_path();
        ctx.move_to(ax, ay);
        ctx.line_to(bx, by);
        ctx.stroke();
    }

    // Nodes — the graveyard metaphor, drawn literally. Every patent renders as
    // a monument:
    //
    //   • US-lapsed patents stand as arched **headstones**, a few of them
    //     leaning (deterministically, per id) like real graves — +
    //   • in-force patents stand straight as **obelisks**: still enforced.
    //   • patents with no US filing lie flat as ground **plaques**: no mark.
    //
    // Seeds glow softly so the map's origin stays readable at a glance.
    for node in nodes {
        let Some(&(x, y)) = positions.get(&node.id) else {
            continue;
        };
        let (fill, stroke) = color_for(node);
        let is_hover = hovered == Some(node.id.as_str());
        let lens_drop = dim(Some(node)).unwrap_or(false);
        let search_drop = searching && !matches_query(node);
        let global = if lens_drop {
            0.18
        } else if search_drop {
            0.3
        } else {
            1.0
        };
        let engraved = engraving(node);

        if node.role == "seed" {
            let (bw, bh) = stone_dims(node);
            let r = bw.max(bh) * 0.5 + 6.0;
            ctx.set_fill_style_str(&format!("rgba(217, 119, 6, {})", 0.16 * global));
            ctx.begin_path();
            let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
            ctx.fill();
            ctx.set_fill_style_str(&format!("rgba(217, 119, 6, {})", 0.10 * global));
            ctx.begin_path();
            let _ = ctx.arc(x, y, r + 5.0, 0.0, std::f64::consts::TAU);
            ctx.fill();
        }

        ctx.set_global_alpha(global);

        match monument_kind(node) {
            Monument::Headstone => {
                let (bw, bh) = stone_dims(node);
                let tilt = tilt_for(&node.id);
                ctx.save();
                // The rotation anchor is the base centre, where the stone
                // meets the soil — so the stone leans, not swims.
                let _ = ctx.translate(x, y + bh / 2.0);
                let _ = ctx.rotate(tilt);

                // Soil shadow.
                ctx.set_fill_style_str("rgba(24, 30, 38, 0.14)");
                ctx.begin_path();
                let _ = ctx.ellipse(
                    0.0,
                    2.0,
                    bw * 0.5 + 9.0,
                    3.2,
                    0.0,
                    0.0,
                    std::f64::consts::TAU,
                );
                ctx.fill();
                // Plinth.
                ctx.set_fill_style_str("#9aa2ad");
                ctx.fill_rect(-bw * 0.5 - 3.0, -4.5, bw + 6.0, 4.5);
                // Stone: straight sides, arched top, flat base on the plinth.
                let r = bw / 2.0;
                let top = -bh;
                ctx.begin_path();
                let _ = ctx.move_to(-r, -4.5);
                let _ = ctx.line_to(-r, top + r);
                let _ = ctx.arc(0.0, top + r, r, std::f64::consts::PI, 0.0);
                let _ = ctx.line_to(r, -4.5);
                ctx.close_path();
                ctx.set_fill_style_str(fill);
                ctx.set_stroke_style_str(stroke);
                ctx.set_line_width(if is_hover { 3.0 } else { 1.5 });
                ctx.fill();
                ctx.stroke();
                // Weathering: two faint horizontal streaks.
                ctx.set_stroke_style_str("rgba(24, 30, 38, 0.22)");
                ctx.set_line_width(1.0);
                let _ = ctx.begin_path();
                let _ = ctx.move_to(-r * 0.55, top + r + 10.0);
                let _ = ctx.line_to(r * 0.55, top + r + 10.0);
                let _ = ctx.move_to(-r * 0.3, top + r + 14.0);
                let _ = ctx.line_to(r * 0.4, top + r + 14.0);
                ctx.stroke();
                // Engraved year.
                if !engraved.is_empty() {
                    ctx.set_fill_style_str("rgba(20, 26, 34, 0.62)");
                    ctx.set_font("700 8px ui-monospace, monospace");
                    ctx.set_text_align("center");
                    let _ = ctx.fill_text(&engraved, 0.0, top + r + 24.0);
                }
                ctx.restore();
            }
            Monument::Obelisk => {
                let (bw, bh) = stone_dims(node);
                ctx.save();
                let _ = ctx.translate(x, y + bh / 2.0);
                // Soil + plinth — the obelisk is standing, never leaning.
                ctx.set_fill_style_str("rgba(24, 30, 38, 0.14)");
                ctx.begin_path();
                let _ = ctx.ellipse(
                    0.0,
                    2.0,
                    bw * 0.5 + 9.0,
                    3.0,
                    0.0,
                    0.0,
                    std::f64::consts::TAU,
                );
                ctx.fill();
                ctx.set_fill_style_str("#9aa2ad");
                ctx.fill_rect(-bw * 0.7 - 3.0, -4.5, bw * 1.4 + 6.0, 4.5);
                // Tapered pillar.
                let bot = -4.5;
                let shoulder = -bh * 0.66;
                let bhw = bw * 0.62;
                let thw = bw * 0.40;
                let tip = -bh * 0.94;
                ctx.begin_path();
                let _ = ctx.move_to(-bhw, bot);
                let _ = ctx.line_to(-thw, shoulder);
                let _ = ctx.line_to(0.0, tip);
                let _ = ctx.line_to(thw, shoulder);
                let _ = ctx.line_to(bhw, bot);
                ctx.close_path();
                ctx.set_fill_style_str(fill);
                ctx.set_stroke_style_str(stroke);
                ctx.set_line_width(if is_hover { 3.0 } else { 1.5 });
                ctx.fill();
                ctx.stroke();
                // Engraved year on the shaft.
                if !engraved.is_empty() {
                    ctx.set_fill_style_str("rgba(20, 26, 34, 0.62)");
                    ctx.set_font("700 8px ui-monospace, monospace");
                    ctx.set_text_align("center");
                    let _ = ctx.fill_text(&engraved, 0.0, shoulder + 16.0);
                }
                ctx.restore();
            }
            Monument::Plaque => {
                let (bw, bh) = stone_dims(node);
                ctx.save();
                let _ = ctx.translate(x, y);
                // Ground-level rounded marker.
                ctx.set_fill_style_str("rgba(24, 30, 38, 0.11)");
                ctx.begin_path();
                let _ = ctx.ellipse(
                    0.0,
                    0.0,
                    bw * 0.5 + 3.0,
                    bh * 0.5 + 2.0,
                    0.0,
                    0.0,
                    std::f64::consts::TAU,
                );
                ctx.fill();
                ctx.set_fill_style_str(fill);
                ctx.set_stroke_style_str(stroke);
                ctx.set_line_width(if is_hover { 3.0 } else { 1.5 });
                ctx.fill_rect(-bw * 0.5, -bh * 0.5, bw, bh);
                ctx.stroke_rect(-bw * 0.5, -bh * 0.5, bw, bh);
                if !engraved.is_empty() && bh > 7.0 {
                    ctx.set_fill_style_str("rgba(20, 26, 34, 0.62)");
                    ctx.set_font("700 7px ui-monospace, monospace");
                    ctx.set_text_align("center");
                    let _ = ctx.fill_text(&engraved, 0.0, 3.0);
                }
                ctx.restore();
            }
        }

        // Name below the monument, engraved-adjacent. Same reveal rule as
        // before: seeds, strong suggestions and the hovered stone get labels.
        if node.role == "seed" || nodes.len() <= 30 || node.score > 0.45 || is_hover {
            let (_, bh) = stone_dims(node);
            ctx.set_fill_style_str("#3f4550");
            ctx.set_font("11px ui-sans-serif, system-ui, sans-serif");
            ctx.set_text_align("center");
            let label = if node.label.chars().count() > 26 {
                let s: String = node.label.chars().take(25).collect();
                format!("{s}…")
            } else {
                node.label.clone()
            };
            let _ = ctx.fill_text(&label, x, y + bh / 2.0 + 14.0);
        }
        ctx.set_text_align("start");
        ctx.set_global_alpha(1.0);

        // Hover tooltip: id + jurisdiction, drawn on the canvas itself so the
        // whole interaction stays dependency-free.
        if is_hover {
            let text = format!("{} · {}", node.id, node.jurisdiction);
            ctx.set_font("600 10.5px ui-sans-serif, system-ui, sans-serif");
            let pad = 7.0;
            let tw = text.chars().count() as f64 * 5.4 + pad * 2.0;
            let bx = (x + 12.0).min(w - tw - 6.0);
            let by = (y + 12.0).min(h - 24.0);
            ctx.set_fill_style_str("rgba(28, 30, 34, 0.92)");
            ctx.fill_rect(bx, by, tw, 19.0);
            ctx.set_fill_style_str("#ffffff");
            let _ = ctx.fill_text(&text, bx + pad, by + 13.0);
        }
    }
    ctx.set_global_alpha(1.0);
}

/// Which monument this patent renders as.
#[cfg(feature = "hydrate")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Monument {
    /// US dead — a classic arched headstone.
    Headstone,
    /// US in force — a standing obelisk.
    Obelisk,
    /// No US filing / unknown — a flat ground plaque.
    Plaque,
}

#[cfg(feature = "hydrate")]
fn monument_kind(node: &GraphNode) -> Monument {
    if node.us_graveyard {
        Monument::Headstone
    } else if node.us_active {
        Monument::Obelisk
    } else {
        Monument::Plaque
    }
}

/// Size of a monument from its importance: seeds are big, suggestion stones
/// scale with their connection strength to the seed set.
#[cfg(feature = "hydrate")]
fn stone_scale(node: &GraphNode) -> f64 {
    if node.role == "seed" {
        1.35
    } else {
        0.8 + node.score * 0.5 + (node.degree_to_seeds.min(4)) as f64 * 0.07
    }
}

/// Width × height of the stone body, in CSS pixels.
#[cfg(feature = "hydrate")]
fn stone_dims(node: &GraphNode) -> (f64, f64) {
    let s = stone_scale(node);
    match monument_kind(node) {
        Monument::Headstone => (22.0 * s, 30.0 * s),
        Monument::Obelisk => (20.0 * s, 34.0 * s),
        Monument::Plaque => (26.0 * s, 10.0),
    }
}

/// The year engraved on the stone, or the gravemarker dagger when unknown.
#[cfg(feature = "hydrate")]
fn engraving(node: &GraphNode) -> String {
    match node.year {
        Some(y) => y.to_string(),
        None => "†".to_string(),
    }
}

/// Deterministic lean for a headstone, seeded by the patent id — the same
/// patent always leans by the same amount, on any machine, on any visit.
#[cfg(feature = "hydrate")]
fn tilt_for(id: &str) -> f64 {
    let hash: usize = id.bytes().map(|b| b as usize).sum();
    (-6.0 + (hash % 13) as f64) * (std::f64::consts::PI / 180.0)
}

/// The server never has a canvas; the layout is still computed so the first
/// hydrated frame is identical to what a pure-SSR page would have shown.
#[cfg(not(feature = "hydrate"))]
#[allow(clippy::too_many_arguments)]
fn draw(
    _canvas_ref: NodeRef<leptos::html::Canvas>,
    _nodes: &[GraphNode],
    _edges: &[Edge],
    _positions: &HashMap<String, (f64, f64)>,
    _dims: (f64, f64),
    _focus: Option<&str>,
    _query: &str,
    _hovered: Option<&str>,
    _pan: (f64, f64),
    _zoom: f64,
) {
}

#[cfg(not(feature = "hydrate"))]
fn canvas_origin(_canvas_ref: NodeRef<leptos::html::Canvas>) -> Option<(f64, f64)> {
    None
}
