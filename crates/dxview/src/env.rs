//! Development switches read from the environment (DXVIEW_*), cached: each call site reads its variable once
//! (std::env::var allocates and takes the environment lock, and many of these sit in per-frame systems).

/// Whether the environment variable is set (read once per call site).
macro_rules! env_on {
    ($name:literal) => {{
        static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *V.get_or_init(|| std::env::var_os($name).is_some())
    }};
}

/// The environment variable's value, if set (read once per call site).
macro_rules! env_val {
    ($name:literal) => {{
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        V.get_or_init(|| std::env::var($name).ok()).as_deref()
    }};
}

/// A setting by literal key, cached per call site until the settings change (`settings::get` takes a lock and a
/// map lookup; many of these sit in per-pawn, per-frame code).
macro_rules! setting {
    ($name:literal) => {{
        static G: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static V: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let g = crate::settings::generation();
        if G.load(std::sync::atomic::Ordering::Relaxed) != g {
            let v = crate::settings::get($name);
            V.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
            G.store(g, std::sync::atomic::Ordering::Relaxed);
        }
        f32::from_bits(V.load(std::sync::atomic::Ordering::Relaxed))
    }};
}
