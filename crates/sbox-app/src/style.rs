//! A small CSS subset for Razor panels: declarations (`style="..."`) and rules made of `tag`, `.class`, `tag.class`
//! compounds joined by descendant (`.a .b`) or child (`.a > .b`) combinators, in comma lists. SCSS nesting is flattened:
//! `&.on { }` inside `.btn { }` becomes `.btn.on`, a plain nested `.b { }` means `.btn .b`. Sibling combinators,
//! pseudo-classes, attribute / id / universal selectors, units other than `px`, and variables are not supported and such
//! rules are ignored. Later rules beat earlier ones of the same specificity (classes count ten, a tag one).

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub color: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub font_size: Option<f32>,
    pub padding: Option<f32>,
    pub gap: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// `flex-direction: row`.
    pub row: bool,
    /// `overflow: scroll` / `auto` (also `overflow-y`): the content scrolls inside the element's fixed height.
    pub scroll: bool,
    /// Tag and classes of the element and its ancestors, outermost first; what descendant selectors match against.
    pub path: Vec<(String, Vec<String>)>,
}

pub fn parse_color(v: &str) -> Option<[f32; 4]> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        if !h.is_ascii() {
            return None;
        }
        let d = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        let (r, g, b, a) = match h.len() {
            3 => (d(0, 1)? * 17, d(1, 1)? * 17, d(2, 1)? * 17, 255),
            6 => (d(0, 2)?, d(2, 2)?, d(4, 2)?, 255),
            8 => (d(0, 2)?, d(2, 2)?, d(4, 2)?, d(6, 2)?),
            _ => return None,
        };
        return Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]);
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let p: Vec<f32> = args.split([',', ' ', '/']).filter(|s| !s.is_empty()).filter_map(|s| s.trim_end_matches('%').parse().ok()).collect();
        if p.len() < 3 {
            return None;
        }
        return Some([p[0] / 255.0, p[1] / 255.0, p[2] / 255.0, p.get(3).copied().unwrap_or(1.0)]);
    }
    Some(match v.as_str() {
        "white" => [1.0; 4],
        "black" => [0.0, 0.0, 0.0, 1.0],
        "red" => [1.0, 0.0, 0.0, 1.0],
        "green" => [0.0, 0.5, 0.0, 1.0],
        "blue" => [0.0, 0.0, 1.0, 1.0],
        "yellow" => [1.0, 1.0, 0.0, 1.0],
        "gray" | "grey" => [0.5, 0.5, 0.5, 1.0],
        "transparent" => [0.0; 4],
        _ => return None,
    })
}

fn px(v: &str) -> Option<f32> {
    let v = v.trim();
    v.strip_suffix("px").unwrap_or(v).trim().parse().ok()
}

/// `a: b; c: d` into (property, value) pairs.
pub fn parse_declarations(src: &str) -> Vec<(String, String)> {
    src.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

impl Style {
    pub fn apply(&mut self, decls: &[(String, String)]) {
        for (k, v) in decls {
            match k.as_str() {
                "color" => self.color = parse_color(v).or(self.color),
                "background-color" | "background" => self.background = parse_color(v).or(self.background),
                "font-size" => self.font_size = px(v).or(self.font_size),
                // Only the single-value form; `padding: 4px 8px` uses its first value.
                "padding" => self.padding = px(v.split_whitespace().next().unwrap_or("")).or(self.padding),
                "gap" | "row-gap" => self.gap = px(v.split_whitespace().next().unwrap_or("")).or(self.gap),
                "width" => self.width = px(v).or(self.width),
                "height" => self.height = px(v).or(self.height),
                "flex-direction" => self.row = v == "row" || v == "row-reverse",
                "overflow" | "overflow-y" => self.scroll = v == "scroll" || v == "auto",
                _ => {}
            }
        }
    }
}

/// Split a block body into its own declarations and its nested rules (selector list, body).
fn split_block(src: &str) -> (String, Vec<(String, String)>) {
    let mut decls = String::new();
    let mut nested = Vec::new();
    let mut chunk = String::new();
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => {
                decls.push_str(&chunk);
                decls.push(';');
                chunk.clear();
            }
            '{' => {
                let mut depth = 1;
                let mut body = String::new();
                for c in chars.by_ref() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    body.push(c);
                }
                nested.push((chunk.trim().to_string(), body));
                chunk.clear();
            }
            _ => chunk.push(c),
        }
    }
    // A last declaration without its `;`.
    decls.push_str(&chunk);
    (decls, nested)
}

/// One compound selector: an optional tag and required classes.
#[derive(Clone, Debug, PartialEq)]
struct Simple {
    tag: Option<String>,
    classes: Vec<String>,
}

impl Simple {
    fn matches(&self, tag: &str, classes: &[String]) -> bool {
        self.tag.as_deref().map_or(true, |t| t == tag) && self.classes.iter().all(|c| classes.contains(c))
    }
}

/// A compound selector chain: `parts[0]` is the outermost; `child[i]` says whether `parts[i]` must be the direct parent
/// of `parts[i + 1]` (`>`) rather than any ancestor. The last part is the element itself.
#[derive(Clone, Debug, PartialEq)]
struct Selector {
    parts: Vec<Simple>,
    child: Vec<bool>,
}

impl Selector {
    fn specificity(&self) -> usize {
        self.parts.iter().map(|p| p.classes.len() * 10 + p.tag.is_some() as usize).sum()
    }

    fn matches(&self, tag: &str, classes: &[String], ancestors: &[(String, Vec<String>)]) -> bool {
        let (last, rest) = self.parts.split_last().expect("a selector has a part");
        if !last.matches(tag, classes) {
            return false;
        }
        // Walk outwards: each remaining part takes the nearest ancestor that fits (exactly the parent for `>`).
        let mut up = ancestors.len();
        for (i, part) in rest.iter().enumerate().rev() {
            let direct = self.child[i];
            loop {
                if up == 0 {
                    return false;
                }
                up -= 1;
                let (t, c) = &ancestors[up];
                if part.matches(t, c) {
                    break;
                }
                if direct {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    rules: Vec<(Selector, Vec<(String, String)>)>,
}

impl Stylesheet {
    /// Parse flat rules; anything with nesting, combinators or pseudo-classes is skipped.
    pub fn parse(css: &str) -> Stylesheet {
        let mut sheet = Stylesheet::default();
        let mut rest = css;
        // Drop /* comments */.
        let cleaned;
        if css.contains("/*") {
            let mut out = String::new();
            let mut r = css;
            while let Some(a) = r.find("/*") {
                out.push_str(&r[..a]);
                r = r[a..].find("*/").map_or("", |b| &r[a + b + 2..]);
            }
            out.push_str(r);
            cleaned = out;
            rest = &cleaned;
        }
        sheet.add_rules(rest, &[]);
        sheet
    }

    /// Parse the rules in `src`, resolving nested selectors against `parents` (the enclosing rule's selectors).
    fn add_rules(&mut self, src: &str, parents: &[String]) {
        let (decls, nested) = split_block(src);
        if !parents.is_empty() && !decls.is_empty() {
            for sel in parents {
                if let Some(sel) = Self::selector(sel) {
                    self.rules.push((sel, parse_declarations(&decls)));
                }
            }
        }
        for (sel_list, body) in nested {
            let mut resolved = Vec::new();
            for s in sel_list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if parents.is_empty() {
                    resolved.push(s.to_string());
                } else {
                    for p in parents {
                        resolved.push(if s.contains('&') { s.replace('&', p) } else { format!("{p} {s}") });
                    }
                }
            }
            self.add_rules(&body, &resolved);
        }
    }

    fn selector(s: &str) -> Option<Selector> {
        if s.is_empty() || s.contains(|c: char| matches!(c, '+' | '~' | ':' | '[' | '#' | '*' | '@' | '&')) {
            return None;
        }
        let spaced = s.replace('>', " > ");
        let mut parts = Vec::new();
        let mut child = Vec::new();
        let mut pending_child = false;
        for tok in spaced.split_whitespace() {
            if tok == ">" {
                // A combinator needs a compound on both sides.
                if parts.is_empty() || pending_child {
                    return None;
                }
                pending_child = true;
                continue;
            }
            if !parts.is_empty() {
                child.push(pending_child);
            }
            pending_child = false;
            let mut it = tok.split('.');
            let tag = it.next().filter(|t| !t.is_empty()).map(str::to_string);
            let classes: Vec<String> = it.map(str::to_string).collect();
            if classes.iter().any(String::is_empty) || (tag.is_none() && classes.is_empty()) {
                return None;
            }
            parts.push(Simple { tag, classes });
        }
        if parts.is_empty() || pending_child {
            return None;
        }
        Some(Selector { parts, child })
    }

    /// Style of an element: the matching rules from least to most specific (later wins among equals), then the inline
    /// declarations. The ancestors to match against are `base.path` (set by the caller from the parent); the result's
    /// path ends with this element.
    pub fn compute(&self, tag: &str, classes: &[&str], inline: &str, mut base: Style) -> Style {
        let own: Vec<String> = classes.iter().map(|c| c.to_string()).collect();
        let mut hits: Vec<(usize, usize)> = self.rules.iter().enumerate().filter(|(_, (sel, _))| sel.matches(tag, &own, &base.path)).map(|(i, (sel, _))| (sel.specificity(), i)).collect();
        hits.sort_unstable();
        for (_, i) in hits {
            base.apply(&self.rules[i].1);
        }
        base.apply(&parse_declarations(inline));
        base.path.push((tag.to_string(), own));
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some([1.0; 4]));
        assert_eq!(parse_color("#ff000080"), Some([1.0, 0.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some([1.0, 0.0, 0.0, 0.5]));
        assert_eq!(parse_color("rgb(0 255 0)"), Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(parse_color("White"), Some([1.0; 4]));
        assert_eq!(parse_color("notacolor"), None);
        assert_eq!(parse_color("#12"), None);
    }

    #[test]
    fn declarations_apply_to_a_style() {
        let mut s = Style::default();
        s.apply(&parse_declarations("color: #00f; font-size: 28px; padding: 4px 8px; width: 120px; flex-direction: row; unknown: 1; gap: x; overflow-y: auto"));
        assert_eq!(s.color, Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!((s.font_size, s.padding, s.width, s.gap), (Some(28.0), Some(4.0), Some(120.0), None));
        assert!(s.row && s.scroll);
    }

    #[test]
    fn cascade_tag_then_class_then_inline() {
        let sheet = Stylesheet::parse("/* c */ p { color: red; font-size: 10px } .big { font-size: 40px } p.big { color: blue } h1, .title { color: green } .a .b { color: black } .x:hover { color: white } .n { .m { color: red } }");
        let p = sheet.compute("p", &["big"], "", Style::default());
        assert_eq!((p.color, p.font_size), (Some([0.0, 0.0, 1.0, 1.0]), Some(40.0)));
        let plain = sheet.compute("p", &[], "font-size: 12px", Style::default());
        assert_eq!((plain.color, plain.font_size), (Some([1.0, 0.0, 0.0, 1.0]), Some(12.0)));
        assert_eq!(sheet.compute("h1", &[], "", Style::default()).color, Some([0.0, 0.5, 0.0, 1.0]));
        assert_eq!(sheet.compute("div", &["title"], "", Style::default()).color, Some([0.0, 0.5, 0.0, 1.0]));
        // Pseudo-classes are ignored; `.a .b` needs an `.a` ancestor, which this bare element has not.
        assert_eq!(sheet.compute("div", &["a", "b", "x", "n", "m"], "", Style::default()).color, None);
    }

    #[test]
    fn descendant_and_child_combinators_use_the_ancestor_chain() {
        let sheet = Stylesheet::parse(".a .b { color: red } .a > .c { color: blue } .a .a { width: 7px } p { color: white } .a p { color: green } div.x > p.y { font-size: 9px } + .z { color: red } .q > { color: red }");
        let el = |parent: Style, tag: &str, classes: &[&str]| sheet.compute(tag, classes, "", Style { path: parent.path.clone(), ..Style::default() });
        let a = el(Style::default(), "div", &["a"]);
        let mid = el(a.clone(), "span", &[]);
        let deep_b = el(mid.clone(), "div", &["b"]);
        assert_eq!(deep_b.color, Some([1.0, 0.0, 0.0, 1.0]), "any ancestor");
        assert_eq!(el(a.clone(), "div", &["c"]).color, Some([0.0, 0.0, 1.0, 1.0]), "direct child");
        assert_eq!(el(mid.clone(), "div", &["c"]).color, None, "a grandchild is not a child");
        assert_eq!(el(Style::default(), "div", &["b"]).color, None);
        // The more specific `.a p` beats the plain `p` wherever it sits in the sheet.
        assert_eq!(el(mid, "p", &[]).color, Some([0.0, 0.5, 0.0, 1.0]));
        assert_eq!(el(Style::default(), "p", &[]).color, Some([1.0; 4]));
        let x = el(Style::default(), "div", &["x"]);
        assert_eq!(el(x.clone(), "p", &["y"]).font_size, Some(9.0));
        assert_eq!(el(el(x, "section", &[]), "p", &["y"]).font_size, None);
        assert_eq!(el(a.clone(), "div", &["a"]).width, Some(7.0), "the same class on parent and child");
        assert_eq!(el(Style::default(), "div", &["z"]).color, None, "dangling combinators are rejected");
    }

    #[test]
    fn scss_nesting_is_flattened() {
        let sheet = Stylesheet::parse(".btn { color: red; font-size: 10px; &.on { color: blue } &.big, &.huge { font-size: 40px; } .inner { color: green } &:hover { color: white } @media (x) { .q { color: black } } }\nlabel { &.t { width: 5px } }");
        let c = |tag: &str, cls: &[&str]| sheet.compute(tag, cls, "", Style::default());
        assert_eq!(c("div", &["btn"]).color, Some([1.0, 0.0, 0.0, 1.0]), "the rule's own declarations survive nesting");
        assert_eq!(c("div", &["btn", "on"]).color, Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(c("div", &["btn", "on"]).font_size, Some(10.0));
        assert_eq!(c("div", &["btn", "big"]).font_size, Some(40.0));
        assert_eq!(c("div", &["btn", "huge"]).font_size, Some(40.0));
        assert_eq!(c("div", &["inner"]).color, None, "`.btn .inner` needs a .btn ancestor");
        let inside = sheet.compute("div", &["inner"], "", sheet.compute("div", &["btn"], "", Style::default()));
        assert_eq!(inside.color, Some([0.0, 0.5, 0.0, 1.0]), "plain nesting is a descendant selector");
        assert_eq!(c("div", &["q"]).color, None);
        assert_eq!(c("label", &["t"]).width, Some(5.0), "& after a tag name");
        assert_eq!(c("label", &[]).width, None);
    }
}
