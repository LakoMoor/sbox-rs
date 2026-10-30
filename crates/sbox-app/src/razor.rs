//! A parser for s&box Razor panels (`.razor`): directives, the `@code` block and the markup tree. C# is never executed:
//! expressions (`@Name`, `@(a + b)`), `@if` / `@foreach` heads and the `@code` body are kept as source text, so the result
//! describes the layout and styling hooks (tags, classes, attributes, text) but not the logic.

use anyhow::{bail, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// Attribute without a value (`<input disabled>`).
    Flag,
    Text(String),
    /// C# expression (`value=@Name`, `value=@(1 + 2)`), source text without the `@`.
    Expr(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub name: String,
    pub value: AttrValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element { tag: String, attrs: Vec<Attr>, children: Vec<Node> },
    /// Literal text with whitespace collapsed.
    Text(String),
    /// Inline C# expression.
    Expr(String),
    /// `@if (...) { ... }`, `@foreach (...) { ... }`, `else { ... }`: the C# head and the markup inside.
    Block { head: String, children: Vec<Node> },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    /// `@using`, `@namespace`, `@inherits`, `@attribute`, ... as (name, rest of the line).
    pub directives: Vec<(String, String)>,
    /// Body of `@code { ... }` / `@functions { ... }`, if any.
    pub code: Option<String>,
    pub nodes: Vec<Node>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&AttrValue> {
        match self {
            Node::Element { attrs, .. } => attrs.iter().find(|a| a.name == name).map(|a| &a.value),
            _ => None,
        }
    }

    /// CSS classes of an element (`class="a b"`).
    pub fn classes(&self) -> Vec<&str> {
        match self.attr("class") {
            Some(AttrValue::Text(t)) => t.split_whitespace().collect(),
            _ => vec![],
        }
    }

    /// Literal text of the subtree, with expressions left out.
    pub fn text(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::Element { children, .. } | Node::Block { children, .. } => children.iter().map(Node::text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
            Node::Expr(_) => String::new(),
        }
    }
}

const VOID: [&str; 6] = ["br", "hr", "img", "input", "meta", "link"];
const BLOCK_KEYWORDS: [&str; 6] = ["if", "foreach", "for", "while", "switch", "lock"];

struct P<'a> {
    s: &'a str,
    i: usize,
    /// `@code` / `@functions` body found while parsing markup.
    code: Option<String>,
}

#[derive(PartialEq)]
enum End<'a> {
    Eof,
    Tag(&'a str),
    Brace,
}

impl<'a> P<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn starts(&self, t: &str) -> bool {
        self.rest().starts_with(t)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }
    fn ident(&mut self) -> &'a str {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.bump();
        }
        &self.s[start..self.i]
    }

    /// Text between a balanced pair of `open` / `close`, skipping string and char literals. Positioned on `open`.
    fn balanced(&mut self, open: char, close: char) -> Result<&'a str> {
        debug_assert_eq!(self.peek(), Some(open));
        self.bump();
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.bump() {
            match c {
                '"' => {
                    // verbatim / regular string: a backslash escapes the next char
                    while let Some(d) = self.bump() {
                        if d == '\\' {
                            self.bump();
                        } else if d == '"' {
                            break;
                        }
                    }
                }
                '/' if self.peek() == Some('/') => {
                    while self.peek().is_some_and(|d| d != '\n') {
                        self.bump();
                    }
                }
                '/' if self.peek() == Some('*') => {
                    self.skip_until("*/")?;
                }
                // A char literal is 'x' or '\x...'; any other apostrophe is not one.
                '\'' => {
                    let mut it = self.rest().chars();
                    match (it.next(), it.next()) {
                        (Some('\\'), _) => {
                            while let Some(d) = self.bump() {
                                if d == '\\' {
                                    self.bump();
                                } else if d == '\'' {
                                    break;
                                }
                            }
                        }
                        (Some(a), Some('\'')) if a != '\'' => {
                            self.bump();
                            self.bump();
                        }
                        _ => {}
                    }
                }
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(&self.s[start..self.i - 1]);
                    }
                }
                _ => {}
            }
        }
        bail!("unbalanced '{open}' at byte {start}")
    }

    fn skip_until(&mut self, end: &str) -> Result<()> {
        match self.rest().find(end) {
            Some(p) => {
                self.i += p + end.len();
                Ok(())
            }
            None => bail!("unterminated comment, expected {end:?}"),
        }
    }

    /// Implicit expression after `@`: `Name`, `Name.Prop`, `Fn(args)`, `list[0]`.
    fn implicit_expr(&mut self) -> Result<String> {
        let start = self.i;
        self.ident();
        loop {
            match self.peek() {
                Some('.') if self.rest()[1..].chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') => {
                    self.bump();
                    self.ident();
                }
                Some('(') => {
                    self.balanced('(', ')')?;
                }
                Some('[') => {
                    self.balanced('[', ']')?;
                }
                _ => break,
            }
        }
        Ok(self.s[start..self.i].to_string())
    }

    fn nodes(&mut self, end: End) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        let mut text = String::new();
        macro_rules! flush {
            () => {
                let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !t.is_empty() {
                    out.push(Node::Text(t));
                }
                text.clear();
            };
        }
        loop {
            let Some(c) = self.peek() else {
                flush!();
                return if end == End::Eof { Ok(out) } else { bail!("unexpected end of file") };
            };
            if self.starts("<!--") {
                self.skip_until("-->")?;
            } else if self.starts("@*") {
                self.skip_until("*@")?;
            } else if self.starts("</") {
                flush!();
                self.i += 2;
                let name = self.s[self.i..].split('>').next().unwrap_or("").trim();
                let name_len = self.s[self.i..].find('>').map_or(0, |p| p + 1);
                match end {
                    End::Tag(t) if t == name => {
                        self.i += name_len;
                        return Ok(out);
                    }
                    _ => bail!("unexpected closing tag </{name}> on line {}", self.s[..self.i].matches('\n').count() + 1),
                }
            } else if c == '<'
                && self.rest()[1..].chars().next().is_some_and(|c| c.is_alphabetic())
                // `Foo<Bar>(` in the C# of a code block is a generic argument list, not an element.
                && !self.s[..self.i].chars().next_back().is_some_and(|p| p.is_alphanumeric() || p == '_')
            {
                flush!();
                out.push(self.element()?);
            } else if c == '}' && end == End::Brace {
                flush!();
                self.bump();
                return Ok(out);
            } else if c == '@' {
                self.bump();
                match self.peek() {
                    Some('@') => {
                        self.bump();
                        text.push('@');
                    }
                    Some('(') => {
                        flush!();
                        out.push(Node::Expr(self.balanced('(', ')')?.to_string()));
                    }
                    Some('{') => {
                        // Statement block; the markup between its statements is kept, the C# around it shows up as text.
                        flush!();
                        self.bump();
                        let children = self.nodes(End::Brace)?;
                        out.push(Node::Block { head: String::new(), children });
                    }
                    Some(c) if c.is_alphabetic() || c == '_' => {
                        flush!();
                        let kw = {
                            let save = self.i;
                            let id = self.ident();
                            self.i = save;
                            id
                        };
                        if kw == "code" || kw == "functions" {
                            self.ident();
                            self.skip_ws();
                            if self.peek() != Some('{') {
                                bail!("expected '{{' after @{kw}");
                            }
                            self.code = Some(self.balanced('{', '}')?.trim().to_string());
                        } else if BLOCK_KEYWORDS.contains(&kw) {
                            out.push(self.block()?);
                            // `} else if (...) {` / `} else {` continuations
                            loop {
                                let save = self.i;
                                self.skip_ws();
                                if self.starts("else") && !self.rest()[4..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                                    out.push(self.block()?);
                                } else {
                                    self.i = save;
                                    break;
                                }
                            }
                        } else {
                            out.push(Node::Expr(self.implicit_expr()?));
                        }
                    }
                    _ => text.push('@'),
                }
            } else {
                text.push(c);
                self.bump();
            }
        }
    }

    /// `if (...) { markup }` with any `else if` / `else` continuations, positioned on the keyword.
    fn block(&mut self) -> Result<Node> {
        let start = self.i;
        if self.ident() == "else" {
            self.skip_ws();
            if self.starts("if") {
                self.ident();
                self.skip_ws();
            }
        } else {
            self.skip_ws();
        }
        if self.peek() == Some('(') {
            self.balanced('(', ')')?;
        }
        let head = self.s[start..self.i].trim().to_string();
        self.skip_ws();
        if self.peek() != Some('{') {
            // A single statement without braces (`@if (x) return;`): C#, not markup.
            while self.peek().is_some_and(|c| c != '\n' && c != ';') {
                self.bump();
            }
            self.bump();
            return Ok(Node::Block { head, children: vec![] });
        }
        self.bump();
        let children = self.nodes(End::Brace)?;
        Ok(Node::Block { head, children })
    }

    fn element(&mut self) -> Result<Node> {
        self.bump(); // <
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')) {
            self.bump();
        }
        let tag = self.s[start..self.i].to_string();
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => bail!("unterminated <{tag}>"),
                Some('/') if self.starts("/>") => {
                    self.i += 2;
                    return Ok(Node::Element { tag, attrs, children: vec![] });
                }
                Some('>') => {
                    self.bump();
                    break;
                }
                _ => {}
            }
            let n0 = self.i;
            while self.peek().is_some_and(|c| !c.is_whitespace() && !matches!(c, '=' | '>' | '/')) {
                self.bump();
            }
            let name = self.s[n0..self.i].to_string();
            if name.is_empty() {
                bail!("bad attribute in <{tag}> at byte {}", self.i);
            }
            self.skip_ws();
            let value = if self.peek() == Some('=') {
                self.bump();
                self.skip_ws();
                match self.peek() {
                    Some(q @ ('"' | '\'')) => {
                        self.bump();
                        let v0 = self.i;
                        // `Css="@($"a {b}")"`: a parenthesised expression may contain the quote character.
                        while self.peek().is_some_and(|c| c != q) {
                            if self.starts("@(") {
                                self.bump();
                                self.balanced('(', ')')?;
                            } else {
                                self.bump();
                            }
                        }
                        let v = self.s[v0..self.i].to_string();
                        self.bump();
                        AttrValue::Text(v)
                    }
                    Some('@') => {
                        self.bump();
                        AttrValue::Expr(if self.peek() == Some('(') { self.balanced('(', ')')?.to_string() } else { self.implicit_expr()? })
                    }
                    _ => {
                        let v0 = self.i;
                        while self.peek().is_some_and(|c| !c.is_whitespace() && c != '>') {
                            self.bump();
                        }
                        AttrValue::Text(self.s[v0..self.i].to_string())
                    }
                }
            } else {
                AttrValue::Flag
            };
            attrs.push(Attr { name, value });
        }
        if VOID.contains(&tag.as_str()) {
            return Ok(Node::Element { tag, attrs, children: vec![] });
        }
        let children = self.nodes(End::Tag(&tag))?;
        Ok(Node::Element { tag, attrs, children })
    }
}

pub fn parse(src: &str) -> Result<Document> {
    let mut p = P { s: src, i: 0, code: None };
    let mut doc = Document::default();
    loop {
        p.skip_ws();
        if p.starts("@*") {
            p.skip_until("*@")?;
        } else if p.starts("@") && p.rest()[1..].starts_with(|c: char| c.is_alphabetic()) {
            p.bump();
            let name = p.ident().to_string();
            if name == "code" || name == "functions" {
                p.skip_ws();
                if p.peek() != Some('{') {
                    bail!("expected '{{' after @{name}");
                }
                p.code = Some(p.balanced('{', '}')?.trim().to_string());
            } else if BLOCK_KEYWORDS.contains(&name.as_str()) {
                // markup-level block at the top: rewind and let the node parser take it
                p.i -= name.len() + 1;
                break;
            } else {
                let line_end = p.rest().find('\n').unwrap_or(p.rest().len());
                doc.directives.push((name, p.rest()[..line_end].trim().trim_end_matches(';').trim().to_string()));
                p.i += line_end;
            }
        } else {
            break;
        }
    }
    doc.nodes = p.nodes(End::Eof)?;
    doc.code = p.code;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: &str, attrs: Vec<Attr>, children: Vec<Node>) -> Node {
        Node::Element { tag: tag.into(), attrs, children }
    }
    fn text_attr(n: &str, v: &str) -> Attr {
        Attr { name: n.into(), value: AttrValue::Text(v.into()) }
    }

    #[test]
    fn directives_code_and_markup() {
        let d = parse("@using Sandbox;\n@namespace My.Ui\n@inherits PanelComponent\n<root><h2>Hi   there</h2></root>\n@code\n{\n    int x = 1;\n    string s = \"}\";\n}\n").unwrap();
        assert_eq!(d.directives, vec![("using".into(), "Sandbox".into()), ("namespace".into(), "My.Ui".into()), ("inherits".into(), "PanelComponent".into())]);
        assert_eq!(d.nodes, vec![el("root", vec![], vec![el("h2", vec![], vec![Node::Text("Hi there".into())])])]);
        assert!(d.code.unwrap().contains("string s = \"}\";"), "braces inside strings do not end the block");
    }

    #[test]
    fn attributes_expressions_and_void_tags() {
        let d = parse(r#"<root><div class="a b" hidden id='x' value=@Name onclick=@(() => Go(1))>@Title<br><img src="p.png"/> n: @Count.ToString() @@ok</div></root>"#).unwrap();
        let Node::Element { children, .. } = &d.nodes[0] else { panic!() };
        let div = &children[0];
        assert_eq!(div.classes(), vec!["a", "b"]);
        assert_eq!(div.attr("hidden"), Some(&AttrValue::Flag));
        assert_eq!(div.attr("id"), Some(&AttrValue::Text("x".into())));
        assert_eq!(div.attr("value"), Some(&AttrValue::Expr("Name".into())));
        assert_eq!(div.attr("onclick"), Some(&AttrValue::Expr("() => Go(1)".into())));
        let Node::Element { children, .. } = div else { panic!() };
        assert_eq!(children[0], Node::Expr("Title".into()));
        assert_eq!(children[1], el("br", vec![], vec![]));
        assert_eq!(children[2], el("img", vec![text_attr("src", "p.png")], vec![]));
        assert_eq!(children[3], Node::Text("n:".into()));
        assert_eq!(children[4], Node::Expr("Count.ToString()".into()));
        assert_eq!(children[5], Node::Text("@ok".into()));
    }

    #[test]
    fn control_flow_blocks_comments_and_components() {
        let d = parse("<root>@* skip *@<!-- c --><Button Label=\"Go\" />@if (Show) {<p>yes</p>} else {<p>no</p>}@foreach (var x in Items) {<li>@x</li>}</root>").unwrap();
        let Node::Element { children, .. } = &d.nodes[0] else { panic!() };
        assert_eq!(children[0], el("Button", vec![text_attr("Label", "Go")], vec![]));
        assert_eq!(children[1], Node::Block { head: "if (Show)".into(), children: vec![el("p", vec![], vec![Node::Text("yes".into())])] });
        assert_eq!(children[2], Node::Block { head: "else".into(), children: vec![el("p", vec![], vec![Node::Text("no".into())])] });
        assert_eq!(children[3], Node::Block { head: "foreach (var x in Items)".into(), children: vec![el("li", vec![], vec![Node::Expr("x".into())])] });
    }

    #[test]
    fn malformed_markup_is_an_error() {
        assert!(parse("<root><div></root>").is_err());
        assert!(parse("<root>").is_err());
        assert!(parse("<root>@(1 + </root>").is_err());
        assert!(parse("</div>").is_err());
    }

    #[test]
    fn text_collects_literal_text() {
        let d = parse("<root><p>One <b>two</b> @x three</p></root>").unwrap();
        assert_eq!(d.nodes[0].text(), "One two three");
    }

    #[test]
    fn generic_arguments_in_code_blocks_are_not_tags() {
        let d = parse("<root>@{ foreach (var g in Enum.GetValues<Group>()) { if (a < b) continue; <div class=\"x\">@g</div> } }</root>").unwrap();
        let Node::Element { children, .. } = &d.nodes[0] else { panic!() };
        assert!(matches!(&children[0], Node::Block { .. }));
    }

    #[test]
    fn quotes_inside_expressions_in_attribute_values() {
        let d = parse("<root>@if ( !Loading && Modes is { Label: { } modeLabel } modes )
{
 var fit = modes.FitFor( 1 );
 <div class=\"mode @(fit.Fit == K.Poor ? \"mismatch\" : modes.M ? \"multi\" : \"\")\" tooltip=\"@(fit.Why ?? modes.Hint)\"><icon>@modes.Icon</icon></div>
}</root>").unwrap();
        let Node::Element { children, .. } = &d.nodes[0] else { panic!() };
        let Node::Block { children, .. } = &children[0] else { panic!("{children:?}") };
        assert!(children.iter().any(|n| matches!(n, Node::Element { tag, .. } if tag == "div")));
    }

    /// Every `.razor` file of a local s&box checkout parses (skipped when the checkout is absent).
    #[test]
    fn parses_the_shipped_razor_files() {
        fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(p) else { return };
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out)
                } else if path.extension().is_some_and(|x| x == "razor") {
                    out.push(path)
                }
            }
        }
        let mut files = vec![];
        walk(std::path::Path::new("E:/sbox-ref"), &mut files);
        let mut failed = vec![];
        for f in &files {
            let Ok(src) = std::fs::read_to_string(f) else { continue };
            if let Err(e) = parse(&src) {
                failed.push(format!("{}: {e:#}", f.display()));
            }
        }
        eprintln!("{} razor files, {} failed", files.len(), failed.len());
        assert!(failed.is_empty(), "razor files that do not parse:
{}", failed.join("
"));
        for f in failed.iter().take(10) {
            eprintln!("{f}");
        }
    }
}
