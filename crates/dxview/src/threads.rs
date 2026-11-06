//! Worker threads. Bevy's default gives the compute pool every hardware thread the machine has, which on a
//! many-core workstation (a 14-core / 28-thread Xeon, or two of them: 56) spreads each frame's small parallel jobs
//! over dozens of threads - and over two sockets - so they spend longer handing work around than doing it, while
//! the main and render threads (the frame's critical path) compete with them for cores. A game frame has a few
//! milliseconds of parallel work: a handful of threads finish it as fast as fifty.
//!
//! Auto (setting WorkerThreads 0, the default): the compute pool gets half the hardware threads (about one per
//! physical core), at least 2 and at most 8; IO and async compute get 1 each (the game loads levels and decodes
//! sounds on its own threads). Measured on a Ryzen 5 3600 (12 threads), Modern: 8 workers 17.5 ms a frame, 6 workers
//! 15.2 ms - a worker on the main or render thread's SMT sibling slows the frame's critical path more than it helps. Settings > Enhancements > Display: CPU Worker Threads
//! (WorkerThreads, an index into STEPS) or env DXVIEW_THREADS=n sets the compute pool to n.

use bevy::app::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy};

/// The menu's choices (0 = Auto).
pub const STEPS: [usize; 7] = [0, 2, 4, 6, 8, 12, 16];

/// The menu index for a WorkerThreads setting.
pub fn step_index(v: f32) -> f32 {
    v.round().clamp(0.0, (STEPS.len() - 1) as f32)
}

/// Hardware threads the OS reports.
pub fn hardware() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// Compute threads for `hw` hardware threads and a requested count (0 = auto).
pub fn compute_threads(hw: usize, requested: usize) -> usize {
    if requested > 0 {
        return requested.clamp(1, hw.max(1) * 2);
    }
    (hw / 2).clamp(2, 8)
}

fn policy(n: usize) -> TaskPoolThreadAssignmentPolicy {
    TaskPoolThreadAssignmentPolicy {
        min_threads: n,
        max_threads: n,
        percent: 1.0,
        on_thread_spawn: None,
        on_thread_destroy: None,
    }
}

pub fn task_pools() -> TaskPoolPlugin {
    let hw = hardware();
    let requested = std::env::var("DXVIEW_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or_else(|| STEPS[step_index(setting!("WorkerThreads")) as usize]);
    let compute = compute_threads(hw, requested);
    println!("dxview: {hw} hardware threads, {compute} worker threads");
    TaskPoolPlugin {
        task_pool_options: TaskPoolOptions {
            min_total_threads: compute + 2,
            max_total_threads: compute + 2,
            io: TaskPoolThreadAssignmentPolicy { percent: 0.0, ..policy(1) },
            async_compute: TaskPoolThreadAssignmentPolicy { percent: 0.0, ..policy(1) },
            compute: policy(compute),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(compute_threads(4, 0), 2);
        assert_eq!(compute_threads(8, 0), 4); // Steam Deck
        assert_eq!(compute_threads(12, 0), 6); // Ryzen 5 3600
        assert_eq!(compute_threads(28, 0), 8); // Xeon E5-2680 v4
        assert_eq!(compute_threads(56, 0), 8); // two of them
        assert_eq!(compute_threads(28, 6), 6);
    }
}

/// The render thread's per-camera schedules (Core3d / Core2d: prepass, opaque, transparent, post passes) run on the
/// render thread itself instead of being handed out to the worker pool. Their systems are tiny and ordered (each
/// records its pass's commands) and the pool gained nothing from them; worse, a main-world system waiting on its
/// own parallel loop (Bevy's calculate_bounds) kept picking up those render tasks while it waited and couldn't
/// return until the render thread finished its cameras - 3.5 ms of the main thread's PostUpdate on a Ryzen 3600.
pub struct RenderSchedulesPlugin;

impl bevy::app::Plugin for RenderSchedulesPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        if env_on!("DXVIEW_MT_RENDER_GRAPH") {
            return;
        }
        let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) else { return };
        render.edit_schedule(bevy::core_pipeline::Core3d, |s| {
            s.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        });
        render.edit_schedule(bevy::core_pipeline::Core2d, |s| {
            s.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        });
        // The render world's own Render schedule (prepare, queue, sort) runs in order on the render thread too,
        // instead of handing each system to the worker pool the main world is busy with (their own parallel loops
        // still use it). Measured on a Ryzen 5 3600, Modern: 17.5 -> 16.0 ms a frame. DXVIEW_MT_RENDER=1 = Bevy's
        // multi-threaded executor.
        if !env_on!("DXVIEW_MT_RENDER") {
            render.edit_schedule(bevy::render::Render, |s| {
                s.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
            });
        }
    }
}

/// Which main-world schedules run their systems in order on the main thread instead of handing each one to the
/// worker pool. Our Update holds about 160 systems, most doing 2-10 µs of work: handed out one by one the
/// scheduler's own overhead (waking threads, waiting on each) was twice the work. Run in order they cost what they
/// do, and the heavy ones (Bevy's transform and visibility passes, our parallel loops) still spread their loops over
/// the workers. Env DXVIEW_ST_SCHEDULES="Update,PostUpdate" (or "none") overrides the default for measurements.
pub struct ExecutorPlugin;

/// The default set (see ExecutorPlugin).
const ST_DEFAULT: &str = "none";

impl bevy::app::Plugin for ExecutorPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        use bevy::app::{First, Last, PostUpdate, PreUpdate, Update};
        use bevy::ecs::schedule::SingleThreadedExecutor;
        let list = std::env::var("DXVIEW_ST_SCHEDULES").unwrap_or_else(|_| ST_DEFAULT.to_string());
        for name in list.split(',').map(|s| s.trim()) {
            match name {
                "First" => app.edit_schedule(First, |s| { s.set_executor(SingleThreadedExecutor::new()); }),
                "PreUpdate" => app.edit_schedule(PreUpdate, |s| { s.set_executor(SingleThreadedExecutor::new()); }),
                "Update" => app.edit_schedule(Update, |s| { s.set_executor(SingleThreadedExecutor::new()); }),
                "PostUpdate" => app.edit_schedule(PostUpdate, |s| { s.set_executor(SingleThreadedExecutor::new()); }),
                "Last" => app.edit_schedule(Last, |s| { s.set_executor(SingleThreadedExecutor::new()); }),
                _ => continue,
            };
            println!("dxview: schedule {name} runs single-threaded");
        }
    }
}
