//! Small helpers for hot paths: string tests that don't build lower-cased copies.

/// `s` contains `pat`, ASCII case-insensitive.
pub fn contains_ci(s: &str, pat: &str) -> bool {
    let (s, p) = (s.as_bytes(), pat.as_bytes());
    p.is_empty() || s.windows(p.len()).any(|w| w.eq_ignore_ascii_case(p))
}

/// `s` starts with `pat`, ASCII case-insensitive.
pub fn starts_with_ci(s: &str, pat: &str) -> bool {
    s.len() >= pat.len() && s.as_bytes()[..pat.len()].eq_ignore_ascii_case(pat.as_bytes())
}

/// `s` ends with `pat`, ASCII case-insensitive.
pub fn ends_with_ci(s: &str, pat: &str) -> bool {
    s.len() >= pat.len() && s.as_bytes()[s.len() - pat.len()..].eq_ignore_ascii_case(pat.as_bytes())
}

/// Run `f` on `s` lower-cased (ASCII), in a stack buffer when it fits (no allocation for names).
pub fn with_lower<R>(s: &str, f: impl FnOnce(&str) -> R) -> R {
    let mut buf = [0u8; 128];
    if s.len() <= buf.len() {
        let b = &mut buf[..s.len()];
        b.copy_from_slice(s.as_bytes());
        b.make_ascii_lowercase();
        // Only ASCII letters changed, so the bytes are still UTF-8.
        f(std::str::from_utf8(b).unwrap_or(""))
    } else {
        f(&s.to_ascii_lowercase())
    }
}

/// The one shared copy of a name used every frame (sound names held each frame, ...): looked up without
/// allocating; the first sight of a name keeps it for the rest of the run (a bounded set of names).
pub fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::RwLock;
    static SET: RwLock<Option<HashSet<&'static str>>> = RwLock::new(None);
    if let Ok(g) = SET.read() {
        if let Some(v) = g.as_ref().and_then(|m| m.get(s)) {
            return v;
        }
    }
    let mut g = SET.write().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashSet::new);
    if let Some(v) = m.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_owned().into_boxed_str());
    m.insert(v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci() {
        assert!(contains_ci("JCDentonCrouchWalk", "crouch"));
        assert!(!contains_ci("Walk", "crouch"));
        assert!(contains_ci("x", ""));
        assert!(starts_with_ci("GestureLeft", "gesture"));
        assert!(ends_with_ci("Glass.GlassFragment", "glassfragment"));
        assert!(!ends_with_ci("ss", "glass"));
        assert!(with_lower("JCDenton", |k| k == "jcdenton"));
        let a = intern(&String::from("Ambient.Hum"));
        assert!(std::ptr::eq(a, intern("Ambient.Hum")));
    }
}
