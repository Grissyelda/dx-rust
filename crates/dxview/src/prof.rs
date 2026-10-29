//! A small always-available profiler: `DXVIEW_PROF=1` times the main systems and frames and prints a table every
//! five seconds (average and worst ms per system, frame time average / 99th percentile / worst, frames over
//! 25 ms), so performance can be measured on the user's machine from the log.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static STATS: Mutex<Option<HashMap<&'static str, (f64, u32, f64)>>> = Mutex::new(None);
/// Always on (DXVIEW_NO_HITCHLOG turns it off): this frame's scope times, so a frame that takes over 400 ms is
/// written to hitch.log beside the exe with what it spent its time on.
static HITCH: AtomicBool = AtomicBool::new(true);
static FRAME: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());

static PROCESS_START: Mutex<Option<Instant>> = Mutex::new(None);

pub fn init() {
    if let Ok(mut p) = PROCESS_START.lock() {
        *p = Some(Instant::now());
    }
    ON.store(std::env::var("DXVIEW_PROF").is_ok(), Ordering::Relaxed);
    HITCH.store(std::env::var_os("DXVIEW_NO_HITCHLOG").is_none(), Ordering::Relaxed);
}

/// Note a slow piece of work outside the per-frame scopes (a level load, a view model build) in hitch.log.
pub fn hitch_note(what: &str, ms: f64) {
    if ms > 250.0 && HITCH.load(Ordering::Relaxed) {
        hitch_write(&format!("{what}: {ms:.0} ms\n"));
    }
}

fn hitch_write(text: &str) {
    use std::io::Write;
    // At most 300 lines a run (a slow GPU would otherwise log every frame).
    static LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if LINES.fetch_add(1, Ordering::Relaxed) >= 300 {
        return;
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("hitch.log")) {
        let _ = f.write_all(text.as_bytes());
    }
}

pub struct Scope(&'static str, Instant);

impl Drop for Scope {
    fn drop(&mut self) {
        let ms = self.1.elapsed().as_secs_f64() * 1000.0;
        if HITCH.load(Ordering::Relaxed) && ms > 1.0 {
            if let Ok(mut f) = FRAME.lock() {
                f.push((self.0, ms));
            }
        }
        if !ON.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut g) = STATS.lock() {
            let e = g.get_or_insert_with(HashMap::new).entry(self.0).or_insert((0.0, 0, 0.0));
            e.0 += ms;
            e.1 += 1;
            e.2 = e.2.max(ms);
        }
    }
}

/// Time the rest of the enclosing scope under `name` (no cost when profiling is off).
#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn scope(name: &'static str) -> Option<Scope> {
    (ON.load(Ordering::Relaxed) || HITCH.load(Ordering::Relaxed)).then(|| Scope(name, Instant::now()))
}

#[derive(Default)]
pub struct ProfState {
    frames: Vec<f64>,
    last: Option<Instant>,
    since: f64,
    hitch_last: Option<Instant>,
    started: Option<Instant>,
}

pub fn prof_report_system(
    mut st: bevy::prelude::Local<ProfState>,
    diags: Option<bevy::prelude::Res<bevy::diagnostic::DiagnosticsStore>>,
) {
    let now = Instant::now();
    // The hitch log: frames over 250 ms after the first 10 s (loading has its own lines).
    if HITCH.load(Ordering::Relaxed) {
        if st.started.is_none() {
            // Start-up: process start to the first frame.
            if let Some(p) = PROCESS_START.lock().ok().and_then(|p| *p) {
                hitch_write(&format!("--- start-up to the first frame: {:.0} ms\n", (now - p).as_secs_f64() * 1000.0));
            }
        }
        let start = *st.started.get_or_insert(now);
        if let Some(l) = st.hitch_last {
            let ms = (now - l).as_secs_f64() * 1000.0;
            let mut f = FRAME.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default();
            if ms > 400.0 && (now - start).as_secs_f64() > 10.0 {
                f.sort_by(|a, b| b.1.total_cmp(&a.1));
                let mut text = format!("frame {ms:.0} ms at {:.1} s:", (now - start).as_secs_f64());
                for (k, v) in f.iter().take(8) {
                    text.push_str(&format!(" {k} {v:.0}"));
                }
                text.push('\n');
                hitch_write(&text);
            }
        }
        st.hitch_last = Some(now);
    }
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    if let Some(l) = st.last {
        let ms = (now - l).as_secs_f64() * 1000.0;
        st.frames.push(ms);
        st.since += ms;
    }
    st.last = Some(now);
    if st.since < 5000.0 {
        return;
    }
    let mut f = std::mem::take(&mut st.frames);
    st.since = 0.0;
    let n = f.len().max(1);
    let avg = f.iter().sum::<f64>() / n as f64;
    f.sort_by(|a, b| a.total_cmp(b));
    let p99 = f[(n * 99 / 100).min(n - 1)];
    let worst = f.last().copied().unwrap_or(0.0);
    let spikes = f.iter().filter(|&&x| x > 25.0).count();
    let mut out = format!(
        "PROF frames {n}: avg {avg:.2} ms ({:.0} fps), p99 {p99:.2}, worst {worst:.2}, >25ms {spikes}\n",
        1000.0 / avg.max(0.001)
    );
    if let Ok(mut g) = STATS.lock() {
        let mut v: Vec<(&'static str, (f64, u32, f64))> = g.take().unwrap_or_default().into_iter().collect();
        v.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        for (k, (tot, cnt, max)) in v.iter().take(30) {
            out.push_str(&format!("PROF   {k:<36} {:7.3} ms/frame  max {max:7.2}  calls {cnt}\n", tot / n as f64));
        }
    }
    // RenderDiagnosticsPlugin: CPU and GPU time of each render pass.
    if let Some(d) = diags {
        let mut v: Vec<(String, f64)> = d
            .iter()
            .filter(|x| x.path().as_str().starts_with("render/"))
            .filter_map(|x| Some((x.path().as_str().to_string(), x.average()?)))
            .filter(|(p, _)| p.ends_with("elapsed_gpu") || p.ends_with("elapsed_cpu"))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (p, ms) in v.iter().take(24) {
            out.push_str(&format!("PROF   gpu {p:<60} {ms:7.3} ms\n"));
        }
    }
    print!("{out}");
    dump_spans(n);
}

/// With DXVIEW_PROF set: Bevy's per-pass render timings (CPU and GPU).
pub struct ProfRenderPlugins;

impl bevy::app::Plugin for ProfRenderPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        if std::env::var("DXVIEW_PROF").is_ok() {
            app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        }
    }
}

pub fn render_plugins() -> ProfRenderPlugins {
    ProfRenderPlugins
}

// ---- Phase timing: where the frame goes outside our own systems ----------------------------------------------

use bevy::ecs::schedule::ScheduleLabel;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct MarkA;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct MarkB;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct MarkC;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct MarkD;

static MARK: Mutex<Option<Instant>> = Mutex::new(None);
static RMARK: Mutex<Option<Instant>> = Mutex::new(None);

fn add(name: &'static str, ms: f64) {
    if let Ok(mut g) = STATS.lock() {
        let e = g.get_or_insert_with(HashMap::new).entry(name).or_insert((0.0, 0, 0.0));
        e.0 += ms;
        e.1 += 1;
        e.2 = e.2.max(ms);
    }
}

fn lap(m: &Mutex<Option<Instant>>, name: Option<&'static str>) {
    let now = Instant::now();
    if let Ok(mut g) = m.lock() {
        if let (Some(n), Some(t)) = (name, *g) {
            add(n, (now - t).as_secs_f64() * 1000.0);
        }
        *g = Some(now);
    }
}

fn mark_a() {
    lap(&MARK, Some("~phase: outside main update (extract/present/winit)"));
}
fn mark_b() {
    lap(&MARK, Some("~phase: main First..Update"));
}
fn mark_c() {
    lap(&MARK, Some("~phase: main PostUpdate"));
}
fn mark_d() {
    lap(&MARK, Some("~phase: main Last"));
}
fn r0() {
    lap(&RMARK, None);
}
fn r1() {
    lap(&RMARK, Some("~render: extract cmds + prepare assets/meshes"));
}
fn r2() {
    lap(&RMARK, Some("~render: views + specialize"));
}
fn r3() {
    lap(&RMARK, Some("~render: queue"));
}
fn r4() {
    lap(&RMARK, Some("~render: sort + prepare (batching, bind groups)"));
}
fn r5() {
    lap(&RMARK, Some("~render: graph (encode + submit)"));
}
fn r6() {
    lap(&RMARK, Some("~render: cleanup"));
}

/// With DXVIEW_PROF set: time the main schedule phases and the render world's stages.
pub struct ProfPhasePlugin;

impl bevy::app::Plugin for ProfPhasePlugin {
    fn build(&self, app: &mut bevy::app::App) {
        if std::env::var("DXVIEW_PROF").is_err() {
            return;
        }
        use bevy::app::{Last, MainScheduleOrder, PostUpdate, Update};
        use bevy::prelude::IntoScheduleConfigs;
        app.add_systems(MarkA, mark_a)
            .add_systems(MarkB, mark_b)
            .add_systems(MarkC, mark_c)
            .add_systems(MarkD, mark_d);
        {
            let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
            order.labels.insert(0, MarkA.intern());
            order.insert_after(Update, MarkB);
            order.insert_after(PostUpdate, MarkC);
            order.insert_after(Last, MarkD);
        }
        use bevy::render::{Render, RenderApp, RenderSystems as S};
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.add_systems(
                Render,
                (
                    r0.in_set(S::ExtractCommands),
                    r1.after(S::PrepareMeshes).before(S::CreateViews),
                    r2.after(S::PrepareViews).before(S::Queue),
                    r3.after(S::Queue).before(S::PhaseSort),
                    r4.after(S::Prepare).before(S::Render),
                    r5.after(S::Render).before(S::Cleanup),
                    r6.in_set(S::PostCleanup),
                ),
            );
        }
    }
}

// ---- Scene statistics: what the renderer is being asked to do --------------------------------------------------

#[derive(Default)]
pub struct SceneStats {
    frames: u32,
    mesh_mod: u64,
    mesh_bytes: u64,
    img_mod: u64,
    img_bytes: u64,
    mat_mod: u64,
    since: f64,
    last: Option<Instant>,
}

/// With DXVIEW_PROF set: every five seconds, the entity / draw / material counts and the per-frame asset
/// re-uploads (modified meshes, images and materials), which is what the render world's time scales with.
pub fn scene_stats_system(
    mut st: bevy::prelude::Local<SceneStats>,
    mut mesh_ev: bevy::prelude::MessageReader<bevy::asset::AssetEvent<bevy::prelude::Mesh>>,
    mut img_ev: bevy::prelude::MessageReader<bevy::asset::AssetEvent<bevy::prelude::Image>>,
    mut mat_ev: bevy::prelude::MessageReader<bevy::asset::AssetEvent<bevy::prelude::StandardMaterial>>,
    meshes: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Mesh>>,
    images: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Image>>,
    mats: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::StandardMaterial>>,
    draws: bevy::prelude::Query<(
        &bevy::prelude::Mesh3d,
        Option<&bevy::prelude::MeshMaterial3d<bevy::prelude::StandardMaterial>>,
        &bevy::prelude::ViewVisibility,
    )>,
    ents: bevy::prelude::Query<bevy::prelude::Entity>,
    ui: bevy::prelude::Query<&bevy::prelude::Node>,
    lights: bevy::prelude::Query<(), bevy::prelude::Or<(bevy::prelude::With<bevy::prelude::PointLight>, bevy::prelude::With<bevy::prelude::SpotLight>)>>,
) {
    use bevy::asset::AssetEvent;
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    st.frames += 1;
    for e in mesh_ev.read() {
        if let AssetEvent::Modified { id } = e {
            st.mesh_mod += 1;
            if let Some(m) = meshes.get(*id) {
                st.mesh_bytes += m.count_vertices() as u64 * 48;
            }
        }
    }
    for e in img_ev.read() {
        if let AssetEvent::Modified { id } = e {
            st.img_mod += 1;
            if let Some(i) = images.get(*id) {
                st.img_bytes += i.data.as_ref().map_or(0, |d| d.len()) as u64;
            }
        }
    }
    for e in mat_ev.read() {
        if let AssetEvent::Modified { .. } = e {
            st.mat_mod += 1;
        }
    }
    let now = Instant::now();
    if let Some(l) = st.last {
        st.since += (now - l).as_secs_f64();
    }
    st.last = Some(now);
    if st.since < 5.0 {
        return;
    }
    let f = st.frames.max(1) as f64;
    let (mut n, mut vis) = (0, 0);
    let mut vis_mats = std::collections::HashSet::new();
    let mut vis_meshes = std::collections::HashSet::new();
    for (m, mat, v) in &draws {
        n += 1;
        if v.get() {
            vis += 1;
            vis_meshes.insert(m.0.id());
            if let Some(mat) = mat {
                vis_mats.insert(mat.0.id());
            }
        }
    }
    println!(
        "PROF scene: {} entities, {} ui nodes, {} lights, {n} mesh draws ({vis} visible, {} distinct meshes, {} distinct materials); assets {} meshes {} images {} materials",
        ents.iter().count(),
        ui.iter().count(),
        lights.iter().count(),
        vis_meshes.len(),
        vis_mats.len(),
        meshes.len(),
        images.len(),
        mats.len()
    );
    println!(
        "PROF uploads/frame: {:.1} meshes ({:.0} KB), {:.1} images ({:.0} KB), {:.1} materials",
        st.mesh_mod as f64 / f,
        st.mesh_bytes as f64 / f / 1024.0,
        st.img_mod as f64 / f,
        st.img_bytes as f64 / f / 1024.0,
        st.mat_mod as f64 / f
    );
    *st = SceneStats { last: st.last, ..Default::default() };
}

// ---- A/B runs: DXVIEW_PROF_AB=1 cycles configurations so one profile run compares them --------------------------

pub static NO_POSE: AtomicBool = AtomicBool::new(false);

/// 0-20 s warm-up as is; then 10 s each: vsync, vsync without vertex posing, no vsync, no vsync without posing.
pub fn prof_ab_system(
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>,
    mut win: bevy::prelude::Query<&mut bevy::prelude::Window>,
    mut phase: bevy::prelude::Local<i32>,
) {
    if std::env::var("DXVIEW_PROF_AB").is_err() {
        return;
    }
    let t = time.elapsed_secs();
    let p = if t < 20.0 { 0 } else { (((t - 20.0) / 10.0) as i32 + 1).min(5) };
    if p == *phase {
        return;
    }
    *phase = p;
    let (vsync, pose) = match p {
        1 => (true, true),
        2 => (true, false),
        3 => (false, true),
        4 => (false, false),
        _ => (true, true),
    };
    NO_POSE.store(!pose, Ordering::Relaxed);
    for mut w in &mut win {
        w.present_mode = if vsync {
            bevy::window::PresentMode::AutoVsync
        } else {
            bevy::window::PresentMode::AutoNoVsync
        };
    }
    println!("PROF AB phase {p}: vsync {vsync}, posing {pose}");
}

// ---- Span totals: with the `spans` feature, every Bevy span (systems, render nodes, submit, present) ------------

use bevy::log::tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};
use bevy::log::tracing::{span, Subscriber};

static SPANS: Mutex<Option<HashMap<String, (f64, u64)>>> = Mutex::new(None);

struct SpanName(String);
struct SpanStart(Instant);

struct NameVisitor<'a>(&'a mut String);
impl bevy::log::tracing::field::Visit for NameVisitor<'_> {
    fn record_str(&mut self, f: &bevy::log::tracing::field::Field, v: &str) {
        if f.name() == "name" {
            self.0.push(' ');
            self.0.push_str(v);
        }
    }
    fn record_debug(&mut self, f: &bevy::log::tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "name" {
            self.0.push_str(&format!(" {v:?}"));
        }
    }
}

struct SpanTotals;

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanTotals {
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut name = attrs.metadata().name().to_string();
        attrs.record(&mut NameVisitor(&mut name));
        if let Some(s) = ctx.span(id) {
            s.extensions_mut().insert(SpanName(name));
        }
    }
    fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
        if let Some(s) = ctx.span(id) {
            s.extensions_mut().replace(SpanStart(Instant::now()));
        }
    }
    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        let Some(s) = ctx.span(id) else { return };
        let ext = s.extensions();
        let (Some(st), Some(n)) = (ext.get::<SpanStart>(), ext.get::<SpanName>()) else { return };
        let ms = st.0.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut g) = SPANS.lock() {
            let e = g.get_or_insert_with(HashMap::new).entry(n.0.clone()).or_insert((0.0, 0));
            e.0 += ms;
            e.1 += 1;
        }
    }
}

/// LogPlugin::custom_layer: the span totals layer when profiling (it only sees spans with the `spans` feature).
pub fn span_layer(_app: &mut bevy::app::App) -> Option<bevy::log::BoxedLayer> {
    std::env::var("DXVIEW_PROF").is_ok().then(|| Box::new(SpanTotals) as bevy::log::BoxedLayer)
}

/// Print the heaviest spans of the last report window (ms per frame over `frames` frames).
pub fn dump_spans(frames: usize) {
    let Ok(mut g) = SPANS.lock() else { return };
    let mut v: Vec<(String, (f64, u64))> = g.take().unwrap_or_default().into_iter().collect();
    v.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    for (k, (tot, n)) in v.iter().take(60) {
        let k: String = k.chars().take(90).collect();
        println!("PROF span {:7.3} ms/frame  x{:<6} {k}", tot / frames.max(1) as f64, n / frames.max(1) as u64);
    }
}

/// Developer aid: `DXVIEW_MESHLOG=1` - every five seconds, which kind of thing the re-uploaded meshes belong to
/// (by the components on the mesh entity's parent chain), to find who rewrites meshes every frame.
#[allow(clippy::type_complexity)]
pub fn mesh_owner_system(
    mut ev: bevy::prelude::MessageReader<bevy::asset::AssetEvent<bevy::prelude::Mesh>>,
    draws: bevy::prelude::Query<(bevy::prelude::Entity, &bevy::prelude::Mesh3d)>,
    parents: bevy::prelude::Query<&bevy::prelude::ChildOf>,
    kinds: bevy::prelude::Query<(
        bevy::prelude::Has<crate::ai::Npc>,
        bevy::prelude::Has<crate::actors::Deco>,
        bevy::prelude::Has<crate::weapons::ViewModelRoot>,
        bevy::prelude::Has<crate::ai::HeldRoot>,
        bevy::prelude::Has<crate::projectiles::Projectile>,
        bevy::prelude::Has<crate::particles::ParticleSprite>,
        bevy::prelude::Has<crate::actors::MeshActorRef>,
    )>,
    deco_names: bevy::prelude::Query<&crate::actors::Deco>,
    mut acc: bevy::prelude::Local<(HashMap<String, u32>, u32, Option<Instant>)>,
) {
    if std::env::var_os("DXVIEW_MESHLOG").is_none() {
        ev.clear();
        return;
    }
    let ids: std::collections::HashSet<bevy::asset::AssetId<bevy::prelude::Mesh>> = ev
        .read()
        .filter_map(|e| match e {
            bevy::asset::AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    acc.1 += 1;
    if !ids.is_empty() {
        for (e, m) in &draws {
            if !ids.contains(&m.0.id()) {
                continue;
            }
            let mut kind = "other".to_string();
            let mut cur = Some(e);
            let mut depth = 0;
            while let Some(c) = cur {
                if let Ok((npc, deco, vm, held, proj, part, ma)) = kinds.get(c) {
                    let k = if vm { "view model" } else if held { "npc weapon" } else if npc { "pawn" } else if proj { "projectile" } else if part { "particle" } else if deco { "decoration" } else if ma { "mesh actor" } else { "" };
                    if !k.is_empty() {
                        let name = deco_names.get(c).map(|d| d.name.clone()).unwrap_or_default();
                        kind = format!("{k} (depth {depth}) {name}");
                        break;
                    }
                }
                cur = parents.get(c).ok().map(|p| p.parent());
                depth += 1;
            }
            *acc.0.entry(kind).or_default() += 1;
        }
    }
    let now = Instant::now();
    let last = *acc.2.get_or_insert(now);
    if (now - last).as_secs_f64() >= 5.0 {
        let f = acc.1.max(1) as f64;
        let mut v: Vec<_> = acc.0.iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in v {
            println!("MESHLOG {k}: {:.1}/frame", n as f64 / f);
        }
        acc.0.clear();
        acc.1 = 0;
        acc.2 = Some(now);
    }
}
