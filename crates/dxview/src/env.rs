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
