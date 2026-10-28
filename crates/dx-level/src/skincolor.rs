//! BeginPlay's `switch (SkinColor)` (decorations and items pick their skin, and sometimes more, by an enum set
//! in the map: Barrel1's explosive / poison / radioactive barrels, vending machines, cars, trees, flags, pool
//! balls, nano keys...). The original script text is read from the user's own DeusEx.u and its case blocks run
//! as simple property assignments: `Skin = Texture'X'`, `MultiSkins[n] = ...`, `bExplosive = True`, numbers,
//! LightType enums.

use std::collections::HashMap;
use std::sync::Mutex;
use unreal_pkg::classes::ClassDb;
use unreal_pkg::props::{find, Property, Value};

type Cases = Vec<Vec<(String, u32, Value)>>;

static CACHE: Mutex<Option<HashMap<String, Option<Cases>>>> = Mutex::new(None);

const LIGHT_TYPES: [&str; 8] = ["LT_None", "LT_Steady", "LT_Pulse", "LT_Blink", "LT_Flicker", "LT_Strobe", "LT_BackdropLight", "LT_SubtlePulse"];

fn strip_comments(s: &str) -> String {
    s.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

/// The enum's value names in order: `enum ESkinColor { SC_A, SC_B };`.
fn enum_names(src: &str) -> Option<Vec<String>> {
    let i = src.find("enum ESkinColor")?;
    let rest = &src[i..];
    let a = rest.find('{')?;
    let b = rest.find('}')?;
    Some(rest[a + 1..b].split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
}

fn parse_value(classes: &ClassDb, class: &str, rhs: &str, ty: Option<u8>) -> Option<Value> {
    let rhs = rhs.trim();
    if let Some(q) = rhs.find('\'') {
        let kind = rhs[..q].trim();
        let name = rhs[q + 1..].trim_end_matches('\'');
        let path = classes.resolve_ref(class, name, kind).unwrap_or_else(|| name.to_string());
        return Some(Value::Path(path));
    }
    if rhs.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if rhs.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if let Some(i) = LIGHT_TYPES.iter().position(|l| l.eq_ignore_ascii_case(rhs)) {
        return Some(Value::Byte(i as u8));
    }
    // Declared types: 1 byte, 2 int, 4 float.
    match ty {
        Some(1) => rhs.parse::<f32>().ok().map(|v| Value::Byte(v as u8)),
        Some(2) => rhs.parse::<f32>().ok().map(|v| Value::Int(v as i32)),
        Some(4) => rhs.parse::<f32>().ok().map(Value::Float),
        _ => None,
    }
}

fn parse(classes: &mut ClassDb, class: &str) -> Option<Cases> {
    let src = strip_comments(&classes.script_text(class)?);
    let names = enum_names(&src)?;
    let start = src.find("switch (SkinColor)").or_else(|| src.find("switch(SkinColor)"))?;
    let body = &src[start..];
    let open = body.find('{')?;
    // The switch body: up to the matching brace.
    let mut depth = 0;
    let mut end = body.len();
    for (i, c) in body[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let block = &body[open + 1..end];
    let pkg = classes.package_of(class)?;
    let declared = classes.declared(&pkg, class);
    let mut cases: Cases = vec![Vec::new(); names.len()];
    // "case SC_X:" ... statements ... until the next case.
    let parts: Vec<&str> = block.split("case ").skip(1).collect();
    for part in parts {
        let Some((label, stmts)) = part.split_once(':') else { continue };
        let Some(idx) = names.iter().position(|n| n.eq_ignore_ascii_case(label.trim())) else { continue };
        for st in stmts.split(';') {
            let st = st.trim();
            if st.is_empty() || st.starts_with("break") {
                continue;
            }
            let Some((lhs, rhs)) = st.split_once('=') else { continue };
            let lhs = lhs.trim();
            let (name, index) = match lhs.split_once('[') {
                Some((n, i)) => (n.trim().to_string(), i.trim_end_matches(']').trim().parse::<u32>().unwrap_or(0)),
                None => (lhs.to_string(), 0),
            };
            if name.contains(' ') || name.contains('.') || name.contains('(') {
                continue;
            }
            let ty = declared.get(&name.to_ascii_lowercase()).copied();
            if let Some(v) = parse_value(classes, class, rhs, ty) {
                cases[idx].push((name, index, v));
            }
        }
    }
    Some(cases)
}

/// The assignments BeginPlay makes for this actor's SkinColor (the class or the nearest ancestor with the switch).
pub fn apply(classes: &mut ClassDb, class: &str, props: &mut Vec<Property>) {
    let skin = match find(props, "SkinColor") {
        Some(Value::Byte(b)) => *b as usize,
        _ => 0,
    };
    for c in classes.ancestry(class) {
        let cases = {
            let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            let m = g.get_or_insert_with(HashMap::new);
            if !m.contains_key(&c) {
                let v = parse(classes, &c);
                m.insert(c.clone(), v);
            }
            m.get(&c).cloned().flatten()
        };
        let Some(cases) = cases else { continue };
        if let Some(list) = cases.get(skin) {
            for (name, index, v) in list {
                props.retain(|p| !(p.name.eq_ignore_ascii_case(name) && p.index == *index));
                props.push(Property { name: name.clone(), index: *index, value: v.clone() });
            }
        }
        return;
    }
}
