//! A small CSS subset for Razor panels: declarations (`style="..."`) and flat rules (`tag`, `.class`, `tag.class`, comma
//! lists). No nesting (SCSS), combinators, pseudo-classes, units other than `px`, or variables; such rules are ignored.

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
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Selector {
    tag: Option<String>,
    classes: Vec<String>,
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
        while let Some(open) = rest.find('{') {
            let sel = rest[..open].trim();
            // The body ends at the matching brace; a nested `{` makes the rule SCSS-style and it is skipped.
            let mut depth = 1;
            let mut end = None;
            for (i, c) in rest[open + 1..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(open + 1 + i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else { break };
            let body = &rest[open + 1..end];
            if !body.contains('{') {
                for s in sel.split(',') {
                    if let Some(sel) = Self::selector(s.trim()) {
                        sheet.rules.push((sel, parse_declarations(body)));
                    }
                }
            }
            rest = &rest[end + 1..];
        }
        sheet
    }

    fn selector(s: &str) -> Option<Selector> {
        if s.is_empty() || s.contains(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~' | ':' | '&' | '[' | '#' | '*' | '@')) {
            return None;
        }
        let mut parts = s.split('.');
        let tag = parts.next().filter(|t| !t.is_empty()).map(str::to_string);
        let classes: Vec<String> = parts.map(str::to_string).collect();
        Some(Selector { tag, classes })
    }

    /// Style of an element: tag rules, then class rules, then the inline declarations (later rules win within a level).
    pub fn compute(&self, tag: &str, classes: &[&str], inline: &str, mut base: Style) -> Style {
        let matches = |s: &Selector| s.tag.as_deref().map_or(true, |t| t == tag) && s.classes.iter().all(|c| classes.contains(&c.as_str()));
        // Specificity: tag-only < class rules.
        for pass in [false, true] {
            for (sel, decls) in &self.rules {
                if sel.classes.is_empty() == !pass && matches(sel) {
                    base.apply(decls);
                }
            }
        }
        base.apply(&parse_declarations(inline));
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
        s.apply(&parse_declarations("color: #00f; font-size: 28px; padding: 4px 8px; width: 120px; flex-direction: row; unknown: 1; gap: x"));
        assert_eq!(s.color, Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!((s.font_size, s.padding, s.width, s.gap), (Some(28.0), Some(4.0), Some(120.0), None));
        assert!(s.row);
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
        // Combinators, pseudo-classes and nested rules are ignored.
        assert_eq!(sheet.compute("div", &["a", "b", "x", "n", "m"], "", Style::default()).color, None);
    }
}
