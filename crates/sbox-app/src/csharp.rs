//! A very small slice of C# for Razor `@code` blocks: fields with literal initialisers become the page's starting values,
//! and parameterless `void` methods made of simple statements (`X++`, `X--`, `X = expr`, `X += expr`, `X -= expr`, with
//! literals, other values, `!X` and `a + b` / `a - b` as expressions) become click handlers. Anything else in the block
//! (loops, calls, lambdas, other types) is skipped, so a method with an unsupported statement is not registered at all.

use std::collections::HashMap;

type Values = HashMap<String, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `name op= expr` where op is `=`, `+` or `-`.
    Assign { name: String, op: char, expr: Expr },
    /// `Log.Info(expr)` / `Log.Warning(expr)` / `Log.Error(expr)`.
    Log { level: log::Level, expr: Expr },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(String),
    Var(String),
    Not(String),
    Bin(Box<Expr>, char, Box<Expr>),
    /// `$"text {Name} text"`: literal pieces and value names.
    Interp(Vec<Result<String, String>>),
}

pub struct Program {
    pub fields: Vec<(String, String)>,
    pub methods: HashMap<String, Vec<Stmt>>,
    /// Read-only `T Name => expr;` properties, evaluated against the current values whenever the page is drawn.
    pub computed: Vec<(String, Expr)>,
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_') && !s.chars().next().unwrap().is_ascii_digit()
}

fn literal(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(q) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return (!q.contains('"') && !q.contains('\\')).then(|| q.to_string());
    }
    if s == "true" || s == "false" {
        return Some(s.to_string());
    }
    let n = s.trim_end_matches(['f', 'F']);
    n.parse::<f64>().ok().map(|_| n.to_string())
}

fn expr(s: &str) -> Option<Expr> {
    let s = s.trim();
    if let Some(l) = literal(s) {
        return Some(Expr::Lit(l));
    }
    if let Some(body) = s.strip_prefix("$\"").and_then(|r| r.strip_suffix('"')) {
        let mut parts = Vec::new();
        let mut rest = body;
        while let Some(i) = rest.find('{') {
            if i > 0 {
                parts.push(Ok(rest[..i].to_string()));
            }
            let (name, after) = rest[i + 1..].split_once('}')?;
            if !is_ident(name.trim()) {
                return None;
            }
            parts.push(Err(name.trim().to_string()));
            rest = after;
        }
        if !rest.is_empty() {
            parts.push(Ok(rest.to_string()));
        }
        return Some(Expr::Interp(parts));
    }
    if let Some(v) = s.strip_prefix('!').map(str::trim).filter(|v| is_ident(v)) {
        return Some(Expr::Not(v.to_string()));
    }
    if is_ident(s) {
        return Some(Expr::Var(s.to_string()));
    }
    // One binary operator, left to right (enough for `Count + 1`).
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            return Some(Expr::Bin(Box::new(expr(&s[..i])?), c, Box::new(expr(&s[i + 1..])?)));
        }
    }
    None
}

fn statement(s: &str) -> Option<Stmt> {
    let s = s.trim();
    for (call, level) in [("Log.Info(", log::Level::Info), ("Log.Warning(", log::Level::Warn), ("Log.Error(", log::Level::Error)] {
        if let Some(arg) = s.strip_prefix(call).and_then(|r| r.strip_suffix(')')) {
            return Some(Stmt::Log { level, expr: expr(arg)? });
        }
    }
    if let Some(n) = s.strip_suffix("++").map(str::trim).filter(|n| is_ident(n)) {
        return Some(Stmt::Assign { name: n.to_string(), op: '+', expr: Expr::Lit("1".into()) });
    }
    if let Some(n) = s.strip_suffix("--").map(str::trim).filter(|n| is_ident(n)) {
        return Some(Stmt::Assign { name: n.to_string(), op: '-', expr: Expr::Lit("1".into()) });
    }
    for (pat, op) in [("+=", '+'), ("-=", '-'), ("=", '=')] {
        if let Some((l, r)) = s.split_once(pat) {
            if is_ident(l.trim()) && !r.starts_with('=') {
                return Some(Stmt::Assign { name: l.trim().to_string(), op, expr: expr(r)? });
            }
        }
    }
    None
}

fn strip_modifiers(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        let mut hit = false;
        for m in ["public ", "private ", "protected ", "internal ", "static "] {
            if let Some(r) = t.strip_prefix(m) {
                s = r;
                hit = true;
            }
        }
        if !hit {
            return t;
        }
    }
}

/// Split a code block into its top-level members, each as text up to the `;` or the closing `}` at depth 0.
fn members(code: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut in_str) = (Vec::new(), String::new(), 0i32, false);
    let mut prev = ' ';
    for c in code.chars() {
        cur.push(c);
        if c == '"' && prev != '\\' {
            in_str = !in_str;
        } else if !in_str {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    // An auto-property's accessor block may be followed by `= value;`: keep it in one member.
                    if depth == 0 && !cur.contains("get;") {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                ';' if depth == 0 => out.push(std::mem::take(&mut cur)),
                _ => {}
            }
        }
        prev = c;
    }
    out
}

pub fn parse(code: &str) -> Program {
    let mut p = Program { fields: Vec::new(), methods: HashMap::new(), computed: Vec::new() };
    for m in members(code) {
        let m = strip_modifiers(m.trim());
        let Some((ty, rest)) = m.split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        // `T Name => expr;`
        if let Some((head, body)) = rest.split_once("=>") {
            if ty != "void" && is_ident(head.trim()) {
                if let Some(e) = expr(body.trim_end_matches(';')) {
                    p.computed.push((head.trim().to_string(), e));
                }
                continue;
            }
        }
        if ty == "void" {
            let Some((head, body)) = rest.split_once('(') else { continue };
            let Some((params, body)) = body.split_once(')') else { continue };
            if !params.trim().is_empty() || !is_ident(head.trim()) {
                continue;
            }
            let body = body.trim();
            let stmts: Option<Vec<Stmt>> = if let Some(b) = body.strip_prefix("=>") {
                statement(b.trim_end_matches(';')).map(|s| vec![s])
            } else if let Some(b) = body.strip_prefix('{').and_then(|b| b.trim_end().strip_suffix('}')) {
                b.split(';').filter(|s| !s.trim().is_empty()).map(statement).collect()
            } else {
                None
            };
            if let Some(s) = stmts {
                p.methods.insert(head.trim().to_string(), s);
            }
        } else if matches!(ty.trim_end_matches('?'), "int" | "float" | "double" | "bool" | "string") {
            // `T Name = lit;` and the auto-property `T Name { get; set; } = lit;` both give a starting value.
            let rest = rest.trim_end_matches(';');
            let rest = match rest.find('{') {
                Some(i) if rest[i..].contains("get;") => {
                    let after = rest[i..].split_once('}').map_or("", |(_, a)| a.trim());
                    format!("{} {}", rest[..i].trim(), after)
                }
                _ => rest.to_string(),
            };
            if let Some((name, init)) = rest.split_once('=') {
                if let (true, Some(v)) = (is_ident(name.trim()), literal(init)) {
                    p.fields.push((name.trim().to_string(), v));
                }
            }
        }
    }
    p
}

fn num(s: &str) -> f64 {
    s.parse().unwrap_or(0.0)
}

fn show(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        v.to_string()
    }
}

pub fn eval(e: &Expr, v: &Values) -> String {
    match e {
        Expr::Interp(parts) => parts.iter().map(|p| match p {
            Ok(t) => t.clone(),
            Err(n) => v.get(n).cloned().unwrap_or_default(),
        }).collect(),
        Expr::Lit(s) => s.clone(),
        Expr::Var(n) => v.get(n).cloned().unwrap_or_default(),
        Expr::Not(n) => v.get(n).map_or(true, |s| s != "true").to_string(),
        Expr::Bin(a, op, b) => {
            let (x, y) = (eval(a, v), eval(b, v));
            if *op == '+' && (x.parse::<f64>().is_err() || y.parse::<f64>().is_err()) {
                format!("{x}{y}")
            } else {
                show(if *op == '+' { num(&x) + num(&y) } else { num(&x) - num(&y) })
            }
        }
    }
}

/// Run a method's statements against the page values.
pub fn run(stmts: &[Stmt], v: &mut Values) {
    for stmt in stmts {
        let (name, op, expr) = match stmt {
            Stmt::Log { level, expr } => {
                log::log!(*level, "razor: {}", eval(expr, v));
                continue;
            }
            Stmt::Assign { name, op, expr } => (name, op, expr),
        };
        let rhs = eval(expr, v);
        let new = match op {
            '=' => rhs,
            '+' => eval(&Expr::Bin(Box::new(Expr::Var(name.clone())), '+', Box::new(Expr::Lit(rhs))), v),
            _ => show(num(v.get(name).map_or("0", String::as_str)) - num(&rhs)),
        };
        v.insert(name.clone(), new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = r#"
        int Count = 3;
        public bool Open { get; set; } = false;
        string Title = "Hi";
        bool Show = true;
        void Increment() { Count++; }
        public void Toggle() => Show = !Show;
        void Reset() { Count = 0; Title = "Reset"; }
        void Add() { Count += 2; Open = true; }
        string Label => $"Count is {Count}!";
        int Next => Count + 1;
        void Noisy() { Log.Info($"c={Count}"); Count++; }
        void Fancy() { Foo(); }
        void WithArg(int x) { Count = x; }
    "#;

    #[test]
    fn literal_fields_and_simple_methods_are_found() {
        let p = parse(CODE);
        let names: Vec<_> = p.fields.iter().map(|(n, v)| format!("{n}={v}")).collect();
        assert_eq!(names, ["Count=3", "Open=false", "Title=Hi", "Show=true"], "an auto-property with an initialiser is a field too");
        let mut m: Vec<_> = p.methods.keys().cloned().collect();
        m.sort();
        assert_eq!(m, ["Add", "Increment", "Noisy", "Reset", "Toggle"], "calls and parameters are not supported");
        let mut v: Values = p.fields.iter().cloned().collect();
        let got: Vec<_> = p.computed.iter().map(|(n, e)| format!("{n}={}", eval(e, &v))).collect();
        assert_eq!(got, ["Label=Count is 3!", "Next=4"]);
        v.insert("Count".into(), "9".into());
        assert_eq!(eval(&p.computed[1].1, &v), "10");
    }

    /// Over the shipped pages: how many code blocks yield something, and that running every found method never panics.
    #[test]
    fn shipped_pages_yield_fields_and_methods() {
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out)
                } else if p.extension().is_some_and(|x| x == "razor") {
                    out.push(p)
                }
            }
        }
        let game = std::path::Path::new("E:/sbox-ref/game");
        if !game.exists() {
            return;
        }
        let mut files = Vec::new();
        walk(game, &mut files);
        let (mut with_code, mut with_methods, mut fields, mut methods, mut fully, mut computed) = (0, 0, 0, 0, 0, 0);
        for f in files {
            let Some(code) = crate::razor::parse(&std::fs::read_to_string(&f).unwrap_or_default()).ok().and_then(|d| d.code) else { continue };
            with_code += 1;
            let p = parse(&code);
            let declared = code.matches("void ").count();
            fields += p.fields.len();
            methods += p.methods.len();
            computed += p.computed.len();
            with_methods += (!p.methods.is_empty()) as usize;
            fully += (declared > 0 && declared == p.methods.len()) as usize;
            let mut v: Values = p.fields.iter().cloned().collect();
            for m in p.methods.values() {
                run(m, &mut v);
            }
        }
        eprintln!("shipped pages: {with_code} with code, {with_methods} with a runnable method, {fields} fields, {methods} methods, {fully} where every void method is supported, {computed} computed properties");
        assert!(with_code > 100, "the shipped pages were found");
    }

    #[test]
    fn methods_change_the_values() {
        let p = parse(CODE);
        let mut v: Values = p.fields.iter().cloned().collect();
        run(&p.methods["Increment"], &mut v);
        run(&p.methods["Add"], &mut v);
        assert_eq!(v["Count"], "6");
        assert_eq!(v["Open"], "true");
        run(&p.methods["Toggle"], &mut v);
        assert_eq!(v["Show"], "false");
        run(&p.methods["Reset"], &mut v);
        assert_eq!((v["Count"].as_str(), v["Title"].as_str()), ("0", "Reset"));
        v.insert("Count".into(), "10".into());
        let list: Vec<Stmt> = "Count -= 4; Count = Count + 1".split(';').filter_map(statement).collect();
        run(&list, &mut v);
        assert_eq!(v["Count"], "7");
    }
}
