//! A small UnrealScript interpreter for mods whose content is written as code: Augmentique's outfit and weapon
//! skin lists are long runs of calls (GlobalAddPartL(PS_Legs, 11, false, "default_p", "JCDentonTex3"); ...) in
//! functions with parameters, locals, if / else and a few helpers. The source comes from the mod's own package
//! (its TextBuffers) at run time, so nothing of it is copied here. Functions that touch the engine are "natives":
//! the `Host` answers them (and variable reads) in Rust; everything else runs from the script.
//!
//! Supported: function definitions (typed parameters, `optional`, `out` ignored), `local` declarations (arrays
//! included), statements `if` / `else`, blocks, `return`, `for (init; cond; step)`, `while`, assignments to locals
//! (`x = e;`, `a[i] = e;`), calls; expressions with literals (int, float, string, name, bool, none, texture'X'),
//! identifiers, array indexing, member access (`a[i].Name`), unary `!` `-`, binary `* / % + - $ @ < > <= >= == != ~=
//! && ||`. Anything else is skipped to the end of its statement and evaluates to None.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum V {
    None,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl V {
    pub fn truthy(&self) -> bool {
        match self {
            V::None => false,
            V::Int(i) => *i != 0,
            V::Float(f) => *f != 0.0,
            V::Bool(b) => *b,
            V::Str(s) => !s.is_empty(),
        }
    }
    pub fn int(&self) -> i64 {
        match self {
            V::Int(i) => *i,
            V::Float(f) => *f as i64,
            V::Bool(b) => *b as i64,
            V::Str(s) => s.trim().parse().unwrap_or(0),
            V::None => 0,
        }
    }
    pub fn float(&self) -> f64 {
        match self {
            V::Float(f) => *f,
            V::Int(i) => *i as f64,
            V::Bool(b) => *b as i64 as f64,
            V::Str(s) => s.trim().parse().unwrap_or(0.0),
            V::None => 0.0,
        }
    }
    pub fn str(&self) -> String {
        match self {
            V::Str(s) => s.clone(),
            V::Int(i) => i.to_string(),
            V::Float(f) => format!("{f:.6}"),
            V::Bool(b) => if *b { "True".into() } else { "False".into() },
            V::None => String::new(),
        }
    }
    pub fn is_none(&self) -> bool {
        matches!(self, V::None)
    }
}

/// What the interpreter asks the game.
pub trait Host {
    /// A native function: Some(result) when the host implements `name` (lowercase), None to run the script's own.
    fn call(&mut self, name: &str, args: &[V]) -> Option<V>;
    /// A variable that isn't a local (member variables, enum values such as PS_Legs, constants), lowercase.
    fn get(&mut self, name: &str) -> V;
    /// An element of a member array, and a field of it (`outfitInfos[3].Name`: field "name").
    fn get_index(&mut self, name: &str, index: i64, field: Option<&str>) -> V;
}

#[derive(Clone, Debug)]
enum Expr {
    Lit(V),
    Ident(String),
    Index(String, Box<Expr>),
    Field(Box<Expr>, String),
    Call(String, Vec<Option<Expr>>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Unknown,
}

#[derive(Clone, Debug)]
enum Stmt {
    Expr(Expr),
    Assign(String, Option<Expr>, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    For(Box<Stmt>, Expr, Box<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Return(Option<Expr>),
    /// `local type a, b[9];`: the names (arrays read as None until set, never from the host).
    Local(Vec<String>),
}

#[derive(Clone, Debug)]
struct Func {
    params: Vec<String>,
    body: Vec<Stmt>,
}

/// The functions of one or more classes' script text.
#[derive(Default)]
pub struct Script {
    funcs: HashMap<String, Func>,
    /// `const NAME = literal;` at class level.
    consts: HashMap<String, V>,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Int(i64),
    Float(f64),
    Str(String),
    /// A quoted object name, with the class prefix if any: texture'X' -> ("texture", "X").
    Name(String, String),
    P(&'static str),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    const OPS: &[&str] = &[
        "&&", "||", "==", "!=", "~=", "<=", ">=", "+=", "-=", "++", "--", "(", ")", "{", "}", "[", "]", ";", ",", "=", "!",
        ".", "<", ">", "+", "-", "*", "/", "%", "$", "@", ":", "?", "#",
    ];
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' && i + 1 < b.len() {
                    i += 1;
                }
                s.push(b[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c == '\'' {
            let s0 = i + 1;
            i = s0;
            while i < b.len() && b[i] != '\'' {
                i += 1;
            }
            out.push(Tok::Name(String::new(), b[s0..i].iter().collect()));
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let s0 = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let id: String = b[s0..i].iter().collect();
            if b.get(i) == Some(&'\'') {
                let s1 = i + 1;
                i = s1;
                while i < b.len() && b[i] != '\'' {
                    i += 1;
                }
                out.push(Tok::Name(id.to_ascii_lowercase(), b[s1..i].iter().collect()));
                i += 1;
            } else {
                out.push(Tok::Id(id));
            }
        } else if c.is_ascii_digit() {
            let s0 = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '.') {
                i += 1;
            }
            let t: String = b[s0..i].iter().collect();
            let t = t.trim_end_matches(['f', 'F']);
            if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                out.push(Tok::Int(i64::from_str_radix(h, 16).unwrap_or(0)));
            } else if t.contains('.') {
                out.push(Tok::Float(t.parse().unwrap_or(0.0)));
            } else {
                out.push(Tok::Int(t.parse().unwrap_or(0)));
            }
        } else {
            let rest: String = b[i..(i + 2).min(b.len())].iter().collect();
            let op = OPS.iter().find(|o| rest.starts_with(**o)).copied().unwrap_or("?");
            out.push(Tok::P(op));
            i += op.len().max(1);
        }
    }
    out
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(q)) if q.eq_ignore_ascii_case(k))
    }
    fn skip_to(&mut self, end: &str) {
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t {
                Tok::P("(") | Tok::P("[") => depth += 1,
                Tok::P(")") | Tok::P("]") => depth -= 1,
                Tok::P("{") if depth <= 0 => return,
                Tok::P("}") if depth <= 0 => return,
                Tok::P(p) if *p == end && depth <= 0 => {
                    self.i += 1;
                    return;
                }
                _ => {}
            }
            self.i += 1;
        }
    }
    fn block_body(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        while self.peek().is_some() && !self.is("}") {
            let before = self.i;
            if let Some(s) = self.stmt() {
                out.push(s);
            }
            if self.i == before {
                self.i += 1;
            }
        }
        self.eat("}");
        out
    }
    fn body(&mut self) -> Vec<Stmt> {
        if self.eat("{") {
            self.block_body()
        } else {
            self.stmt().into_iter().collect()
        }
    }
    fn stmt(&mut self) -> Option<Stmt> {
        if self.eat(";") {
            return None;
        }
        if self.is("{") {
            let b = self.body();
            return Some(Stmt::If(Expr::Lit(V::Bool(true)), b, Vec::new()));
        }
        if self.is_kw("local") {
            // local <type> a, b[9];  - the names are identifiers after the type, outside brackets.
            self.i += 1;
            let mut names = Vec::new();
            let mut first = true;
            let mut depth = 0;
            while let Some(t) = self.peek().cloned() {
                self.i += 1;
                match t {
                    Tok::P(";") => break,
                    Tok::P("[") | Tok::P("<") => depth += 1,
                    Tok::P("]") | Tok::P(">") => depth -= 1,
                    Tok::Id(n) if depth == 0 => {
                        if first {
                            first = false;
                        } else {
                            names.push(n.to_ascii_lowercase());
                        }
                    }
                    _ => {}
                }
            }
            return Some(Stmt::Local(names));
        }
        if self.is_kw("const") || self.is_kw("var") {
            self.skip_to(";");
            return None;
        }
        if self.is_kw("if") {
            self.i += 1;
            self.eat("(");
            let c = self.expr();
            self.eat(")");
            let a = self.body();
            let b = if self.is_kw("else") {
                self.i += 1;
                self.body()
            } else {
                Vec::new()
            };
            return Some(Stmt::If(c, a, b));
        }
        if self.is_kw("for") {
            self.i += 1;
            self.eat("(");
            let init = self.simple().unwrap_or(Stmt::Expr(Expr::Unknown));
            self.eat(";");
            let cond = self.expr();
            self.eat(";");
            let step = self.simple().unwrap_or(Stmt::Expr(Expr::Unknown));
            self.eat(")");
            let b = self.body();
            return Some(Stmt::For(Box::new(init), cond, Box::new(step), b));
        }
        if self.is_kw("while") {
            self.i += 1;
            self.eat("(");
            let c = self.expr();
            self.eat(")");
            let b = self.body();
            return Some(Stmt::While(c, b));
        }
        if self.is_kw("return") {
            self.i += 1;
            if self.eat(";") {
                return Some(Stmt::Return(None));
            }
            let e = self.expr();
            self.skip_to(";");
            return Some(Stmt::Return(Some(e)));
        }
        if self.is_kw("switch") || self.is_kw("foreach") || self.is_kw("do") {
            // Not used by the code we run: skip the statement and its block.
            self.skip_to(";");
            if self.eat("{") {
                let mut d = 1;
                while d > 0 && self.peek().is_some() {
                    if self.is("{") {
                        d += 1;
                    } else if self.is("}") {
                        d -= 1;
                    }
                    self.i += 1;
                }
            }
            return None;
        }
        let s = self.simple();
        self.skip_to(";");
        s
    }
    /// An assignment, increment or expression statement (no trailing ';').
    fn simple(&mut self) -> Option<Stmt> {
        if let Some(Tok::Id(name)) = self.peek().cloned() {
            let save = self.i;
            self.i += 1;
            let mut index = None;
            if self.eat("[") {
                index = Some(self.expr());
                self.eat("]");
            }
            let lname = name.to_ascii_lowercase();
            if self.eat("=") {
                let e = self.expr();
                return Some(Stmt::Assign(lname, index, e));
            }
            for (op, bin) in [("+=", "+"), ("-=", "-")] {
                if self.eat(op) {
                    let e = self.expr();
                    let cur = match &index {
                        Some(ix) => Expr::Index(lname.clone(), Box::new(ix.clone())),
                        None => Expr::Ident(lname.clone()),
                    };
                    return Some(Stmt::Assign(lname, index, Expr::Bin(bin, Box::new(cur), Box::new(e))));
                }
            }
            for (op, bin) in [("++", "+"), ("--", "-")] {
                if self.eat(op) {
                    let cur = match &index {
                        Some(ix) => Expr::Index(lname.clone(), Box::new(ix.clone())),
                        None => Expr::Ident(lname.clone()),
                    };
                    return Some(Stmt::Assign(lname, index, Expr::Bin(bin, Box::new(cur), Box::new(Expr::Lit(V::Int(1))))));
                }
            }
            self.i = save;
        }
        let e = self.expr();
        Some(Stmt::Expr(e))
    }
    fn expr(&mut self) -> Expr {
        self.bin(0)
    }
    fn bin(&mut self, level: usize) -> Expr {
        const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["==", "!=", "~="], &["<", ">", "<=", ">="], &["$", "@"], &["+", "-"], &["*", "/", "%"]];
        if level >= LEVELS.len() {
            return self.unary();
        }
        let mut a = self.bin(level + 1);
        loop {
            let op = LEVELS[level].iter().find(|o| self.is(o)).copied();
            let Some(op) = op else { break };
            self.i += 1;
            let b = self.bin(level + 1);
            a = Expr::Bin(op, Box::new(a), Box::new(b));
        }
        a
    }
    fn unary(&mut self) -> Expr {
        if self.eat("!") {
            return Expr::Not(Box::new(self.unary()));
        }
        if self.eat("-") {
            return Expr::Neg(Box::new(self.unary()));
        }
        let mut e = self.primary();
        loop {
            if self.eat(".") {
                match self.peek().cloned() {
                    Some(Tok::Id(f)) => {
                        self.i += 1;
                        if self.is("(") {
                            // A method call on an object: evaluate nothing, skip the arguments.
                            self.args();
                            e = Expr::Unknown;
                        } else {
                            e = Expr::Field(Box::new(e), f.to_ascii_lowercase());
                        }
                    }
                    _ => return Expr::Unknown,
                }
            } else {
                break;
            }
        }
        e
    }
    fn args(&mut self) -> Vec<Option<Expr>> {
        let mut out = Vec::new();
        if !self.eat("(") {
            return out;
        }
        if self.eat(")") {
            return out;
        }
        loop {
            if self.is(",") {
                out.push(None);
                self.i += 1;
                continue;
            }
            if self.is(")") {
                out.push(None);
                self.i += 1;
                break;
            }
            out.push(Some(self.expr()));
            if self.eat(",") {
                continue;
            }
            self.eat(")");
            break;
        }
        out
    }
    fn primary(&mut self) -> Expr {
        match self.peek().cloned() {
            Some(Tok::Int(v)) => {
                self.i += 1;
                Expr::Lit(V::Int(v))
            }
            Some(Tok::Float(v)) => {
                self.i += 1;
                Expr::Lit(V::Float(v))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Expr::Lit(V::Str(s))
            }
            Some(Tok::Name(_, n)) => {
                self.i += 1;
                Expr::Lit(V::Str(n))
            }
            Some(Tok::P("(")) => {
                self.i += 1;
                let e = self.expr();
                self.eat(")");
                e
            }
            Some(Tok::Id(id)) => {
                self.i += 1;
                let l = id.to_ascii_lowercase();
                match l.as_str() {
                    "true" => return Expr::Lit(V::Bool(true)),
                    "false" => return Expr::Lit(V::Bool(false)),
                    "none" => return Expr::Lit(V::None),
                    "self" | "super" | "default" | "static" | "class" => {
                        // super.X(...), default.X, class'Y'.static.Z(...): treat as calls by the final name.
                        if l == "super" && self.is("(") {
                            self.args();
                        }
                        if self.eat(".") {
                            if l == "static" || l == "default" {
                                return self.unary_rest();
                            }
                            return self.unary_rest();
                        }
                        return Expr::Unknown;
                    }
                    _ => {}
                }
                if self.is("(") {
                    // A cast such as int(x) / string(x) / Texture(x) passes its value through.
                    let a = self.args();
                    if ["int", "string", "float", "bool", "byte", "name", "texture", "mesh"].contains(&l.as_str()) {
                        return a.into_iter().next().flatten().unwrap_or(Expr::Unknown);
                    }
                    return Expr::Call(l, a);
                }
                if self.eat("[") {
                    let ix = self.expr();
                    self.eat("]");
                    return Expr::Index(l, Box::new(ix));
                }
                Expr::Ident(l)
            }
            _ => {
                self.i += 1;
                Expr::Unknown
            }
        }
    }
    /// After `default.` / `static.`: a member read or call by name.
    fn unary_rest(&mut self) -> Expr {
        match self.peek().cloned() {
            Some(Tok::Id(_)) => self.primary(),
            _ => Expr::Unknown,
        }
    }
}

impl Script {
    /// Read every `function` in this source (later definitions of the same name replace earlier ones).
    pub fn add_source(&mut self, src: &str) {
        let t = tokenize(src);
        let mut i = 0;
        while i < t.len() {
            if let (Tok::Id(k), Some(Tok::Id(n)), Some(Tok::P("=")), Some(v)) = (&t[i], t.get(i + 1), t.get(i + 2), t.get(i + 3)) {
                if k.eq_ignore_ascii_case("const") {
                    let val = match v {
                        Tok::Int(x) => V::Int(*x),
                        Tok::Float(x) => V::Float(*x),
                        Tok::Str(x) => V::Str(x.clone()),
                        Tok::Name(_, x) => V::Str(x.clone()),
                        _ => V::None,
                    };
                    self.consts.insert(n.to_ascii_lowercase(), val);
                    i += 4;
                    continue;
                }
            }
            if matches!(&t[i], Tok::Id(k) if k.eq_ignore_ascii_case("function") || k.eq_ignore_ascii_case("event")) {
                // function [type] Name(params) { body }   (modifiers before the keyword are fine)
                let mut j = i + 1;
                let mut name = None;
                while j < t.len() {
                    if let (Tok::Id(n), Some(Tok::P("("))) = (&t[j], t.get(j + 1)) {
                        name = Some(n.to_ascii_lowercase());
                        break;
                    }
                    j += 1;
                }
                let Some(name) = name else { break };
                // Parameters: the identifier before each ',' or ')' at depth 1.
                let mut params = Vec::new();
                let mut k = j + 2;
                let mut last_id: Option<String> = None;
                let mut depth = 1;
                while k < t.len() && depth > 0 {
                    match &t[k] {
                        Tok::P("(") => depth += 1,
                        Tok::P(")") => {
                            depth -= 1;
                            if depth == 0 {
                                if let Some(p) = last_id.take() {
                                    params.push(p);
                                }
                            }
                        }
                        Tok::P(",") if depth == 1 => {
                            if let Some(p) = last_id.take() {
                                params.push(p);
                            }
                        }
                        Tok::Id(x) if depth == 1 => last_id = Some(x.to_ascii_lowercase()),
                        Tok::P("[") => {
                            // param arrays: int seeds[30]
                            while k < t.len() && t[k] != Tok::P("]") {
                                k += 1;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                // Body (a declaration without one ends in ';').
                while k < t.len() && t[k] != Tok::P("{") && t[k] != Tok::P(";") {
                    k += 1;
                }
                if k < t.len() && t[k] == Tok::P("{") {
                    let mut p = Parser { t: &t, i: k + 1 };
                    let body = p.block_body();
                    self.funcs.insert(name, Func { params, body });
                    i = p.i;
                    continue;
                }
                i = k + 1;
                continue;
            }
            i += 1;
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.funcs.contains_key(&name.to_ascii_lowercase())
    }

    /// Call a function: the host's native, else the script's own (unknown ones do nothing).
    pub fn call(&self, host: &mut dyn Host, name: &str, args: &[V]) -> V {
        let l = name.to_ascii_lowercase();
        if let Some(v) = host.call(&l, args) {
            return v;
        }
        let Some(f) = self.funcs.get(&l) else { return V::None };
        let mut locals: HashMap<String, V> = HashMap::new();
        for (i, p) in f.params.iter().enumerate() {
            locals.insert(p.clone(), args.get(i).cloned().unwrap_or(V::None));
        }
        let mut steps = 0usize;
        match self.run(host, &f.body, &mut locals, &mut steps) {
            Flow::Return(v) => v,
            Flow::Next => V::None,
        }
    }

    fn run(&self, host: &mut dyn Host, body: &[Stmt], locals: &mut HashMap<String, V>, steps: &mut usize) -> Flow {
        for s in body {
            *steps += 1;
            if *steps > 2_000_000 {
                return Flow::Return(V::None);
            }
            match s {
                Stmt::Expr(e) => {
                    self.eval(host, e, locals);
                }
                Stmt::Assign(name, index, e) => {
                    let v = self.eval(host, e, locals);
                    let key = match index {
                        Some(ix) => format!("{name}[{}]", self.eval(host, ix, locals).int()),
                        None => name.clone(),
                    };
                    locals.insert(key, v);
                }
                Stmt::If(c, a, b) => {
                    let branch = if self.eval(host, c, locals).truthy() { a } else { b };
                    if let Flow::Return(v) = self.run(host, branch, locals, steps) {
                        return Flow::Return(v);
                    }
                }
                Stmt::For(init, c, step, b) => {
                    if let Flow::Return(v) = self.run(host, std::slice::from_ref(init), locals, steps) {
                        return Flow::Return(v);
                    }
                    let mut n = 0;
                    while self.eval(host, c, locals).truthy() && n < 100_000 {
                        if let Flow::Return(v) = self.run(host, b, locals, steps) {
                            return Flow::Return(v);
                        }
                        self.run(host, std::slice::from_ref(step), locals, steps);
                        n += 1;
                    }
                }
                Stmt::While(c, b) => {
                    let mut n = 0;
                    while self.eval(host, c, locals).truthy() && n < 100_000 {
                        if let Flow::Return(v) = self.run(host, b, locals, steps) {
                            return Flow::Return(v);
                        }
                        n += 1;
                    }
                }
                Stmt::Local(names) => {
                    for n in names {
                        locals.entry(n.clone()).or_insert(V::None);
                    }
                }
                Stmt::Return(e) => return Flow::Return(e.as_ref().map(|e| self.eval(host, e, locals)).unwrap_or(V::None)),
            }
        }
        Flow::Next
    }

    fn eval(&self, host: &mut dyn Host, e: &Expr, locals: &mut HashMap<String, V>) -> V {
        match e {
            Expr::Lit(v) => v.clone(),
            Expr::Unknown => V::None,
            Expr::Ident(n) => match locals.get(n) {
                Some(v) => v.clone(),
                None => match self.consts.get(n) {
                    Some(v) => v.clone(),
                    None => host.get(n),
                },
            },
            Expr::Index(n, ix) => {
                let i = self.eval(host, ix, locals).int();
                match locals.get(&format!("{n}[{i}]")) {
                    Some(v) => v.clone(),
                    None if locals.contains_key(n) => V::None,
                    None => host.get_index(n, i, None),
                }
            }
            Expr::Field(base, f) => match &**base {
                Expr::Index(n, ix) => {
                    let i = self.eval(host, ix, locals).int();
                    host.get_index(n, i, Some(f))
                }
                _ => V::None,
            },
            Expr::Call(n, args) => {
                let vals: Vec<V> = args.iter().map(|a| a.as_ref().map(|a| self.eval(host, a, locals)).unwrap_or(V::None)).collect();
                self.call(host, n, &vals)
            }
            Expr::Not(a) => V::Bool(!self.eval(host, a, locals).truthy()),
            Expr::Neg(a) => match self.eval(host, a, locals) {
                V::Float(f) => V::Float(-f),
                v => V::Int(-v.int()),
            },
            Expr::Bin(op, a, b) => {
                if *op == "&&" {
                    return V::Bool(self.eval(host, a, locals).truthy() && self.eval(host, b, locals).truthy());
                }
                if *op == "||" {
                    return V::Bool(self.eval(host, a, locals).truthy() || self.eval(host, b, locals).truthy());
                }
                let x = self.eval(host, a, locals);
                let y = self.eval(host, b, locals);
                let float = matches!(x, V::Float(_)) || matches!(y, V::Float(_));
                let strs = matches!(x, V::Str(_)) || matches!(y, V::Str(_));
                match *op {
                    "$" => V::Str(x.str() + &y.str()),
                    "@" => V::Str(format!("{} {}", x.str(), y.str())),
                    "==" if strs => V::Bool(x.str() == y.str()),
                    "!=" if strs => V::Bool(x.str() != y.str()),
                    "==" => V::Bool(if x.is_none() || y.is_none() { x.is_none() && y.is_none() || x.float() == y.float() && !x.is_none() } else { x.float() == y.float() }),
                    "!=" => V::Bool(if x.is_none() || y.is_none() { !(x.is_none() && y.is_none()) } else { x.float() != y.float() }),
                    "~=" => V::Bool(x.str().eq_ignore_ascii_case(&y.str())),
                    "<" => V::Bool(x.float() < y.float()),
                    ">" => V::Bool(x.float() > y.float()),
                    "<=" => V::Bool(x.float() <= y.float()),
                    ">=" => V::Bool(x.float() >= y.float()),
                    "+" if float => V::Float(x.float() + y.float()),
                    "-" if float => V::Float(x.float() - y.float()),
                    "*" if float => V::Float(x.float() * y.float()),
                    "/" if float => V::Float(x.float() / y.float()),
                    "+" => V::Int(x.int() + y.int()),
                    "-" => V::Int(x.int() - y.int()),
                    "*" => V::Int(x.int() * y.int()),
                    "/" => V::Int(if y.int() == 0 { 0 } else { x.int() / y.int() }),
                    "%" => V::Int(if y.int() == 0 { 0 } else { x.int() % y.int() }),
                    _ => V::None,
                }
            }
        }
    }
}

enum Flow {
    Next,
    Return(V),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct H {
        calls: Vec<(String, Vec<V>)>,
    }
    impl Host for H {
        fn call(&mut self, name: &str, args: &[V]) -> Option<V> {
            if name == "native" {
                self.calls.push((name.into(), args.to_vec()));
                return Some(V::None);
            }
            None
        }
        fn get(&mut self, name: &str) -> V {
            match name {
                "ps_legs" => V::Int(18),
                "bissetup" => V::Bool(false),
                _ => V::None,
            }
        }
        fn get_index(&mut self, name: &str, index: i64, field: Option<&str>) -> V {
            V::Str(format!("{name}[{index}].{}", field.unwrap_or("")))
        }
    }

    #[test]
    fn calls_and_ifs() {
        let src = r#"
        function Helper(int slot0, int slot1, bool bMale, string tex0, optional string tex1)
        {
            local string slots[9];
            if (bIsSetup)
                return;
            if (slot0 >= 0)
                slots[slot0] = tex0;
            if (slot1 >= 0)
                slots[slot1] = tex1;
            if (bMale)
            {
                Native(PS_Legs, slots[0], slots[1], slots[2]);
            }
            else
                Native(-1);
        }
        function Top()
        {
            //comment
            Helper(0,2,true,"A","B");
            Helper(1,-1,false,"C");
            Native(outfitInfos[3].Name, partNames[2] $ "x", "a" @ "b");
        }"#;
        let mut s = Script::default();
        s.add_source(src);
        let mut h = H { calls: Vec::new() };
        s.call(&mut h, "Top", &[]);
        assert_eq!(h.calls.len(), 3);
        assert_eq!(h.calls[0].1, vec![V::Int(18), V::Str("A".into()), V::None, V::Str("B".into())]);
        assert_eq!(h.calls[1].1, vec![V::Int(-1)]);
        assert_eq!(h.calls[2].1, vec![V::Str("outfitinfos[3].name".into()), V::Str("partnames[2].x".into()), V::Str("a b".into())]);
    }

    #[test]
    fn empty_args() {
        let mut s = Script::default();
        s.add_source("function T() { Native(1,,\"x\",,); }");
        let mut h = H { calls: Vec::new() };
        s.call(&mut h, "t", &[]);
        assert_eq!(h.calls[0].1, vec![V::Int(1), V::None, V::Str("x".into()), V::None, V::None]);
    }

    #[test]
    fn loops() {
        let mut s = Script::default();
        s.add_source("function T() { local int i; for (i = 0; i < 3; i++) Native(i); }");
        let mut h = H { calls: Vec::new() };
        s.call(&mut h, "t", &[]);
        assert_eq!(h.calls.len(), 3);
    }
}
