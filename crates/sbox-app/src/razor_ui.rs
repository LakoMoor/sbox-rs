//! Turns a parsed Razor document into positioned UI items. Only static markup is drawn: expressions and `@if` / `@foreach`
//! blocks are skipped, and there is no data binding or event handling. Styling is the subset in [`crate::style`]
//! (`<style>` rules and `style="..."`): colours, font size, padding, gap, width / height in px and `flex-direction: row`.
//! Containers stack their children in a column (or a row); headings and paragraphs become text, `<button>` a button.

use crate::layout::{Align, Axis, Size, Stack};
use crate::razor::{AttrValue, Document, Node};
use crate::style::{Style, Stylesheet};

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Background of a container: `root`, an element with a `panel` / `card` class, or any element with a background.
    Panel { rect: [f32; 4], color: [f32; 4] },
    Text { text: String, size: f32, color: [f32; 4], rect: [f32; 4] },
    Button { id: String, text: String, rect: [f32; 4], color: Option<[f32; 4]> },
}

const PADDING: f32 = 12.0;
const GAP: f32 = 8.0;
const BUTTON_HEIGHT: f32 = 48.0;
const PANEL_COLOR: [f32; 4] = [0.05, 0.06, 0.1, 0.85];

fn default_size(tag: &str) -> f32 {
    match tag {
        "h1" => 40.0,
        "h2" => 32.0,
        "h3" => 26.0,
        "h4" | "h5" | "h6" => 22.0,
        _ => 20.0,
    }
}

/// A node that is a single text-bearing leaf: heading, paragraph, label, span or bare text.
fn is_text_leaf(n: &Node) -> bool {
    match n {
        Node::Text(_) => true,
        Node::Element { tag, children, .. } => matches!(tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "label" | "span") || (tag == "div" && !children.is_empty() && children.iter().all(|c| matches!(c, Node::Text(_) | Node::Expr(_)))),
        _ => false,
    }
}

fn button_id(n: &Node) -> String {
    match n.attr("id") {
        Some(AttrValue::Text(t)) => t.clone(),
        _ => n.text().to_lowercase().replace(' ', "_"),
    }
}

/// Visible children: static elements and text; blocks and expressions are dropped.
fn visible(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| match n {
            Node::Text(_) => true,
            Node::Element { tag, .. } => !matches!(tag.as_str(), "style" | "script" | "head"),
            _ => false,
        })
        .filter(|n| !is_text_leaf(n) || !n.text().is_empty())
        .collect()
}

struct Ctx<'a> {
    sheet: &'a Stylesheet,
}

impl Ctx<'_> {
    /// Computed style of `n`; colour and font size are inherited from `parent`.
    fn style(&self, n: &Node, parent: &Style) -> Style {
        let inherited = Style { color: parent.color, font_size: parent.font_size, ..Style::default() };
        match n {
            Node::Element { tag, .. } => {
                let inline = match n.attr("style") {
                    Some(AttrValue::Text(t)) => t.as_str(),
                    _ => "",
                };
                self.sheet.compute(tag, &n.classes(), inline, inherited)
            }
            _ => inherited,
        }
    }

    fn font_size(n: &Node, st: &Style) -> f32 {
        match n {
            Node::Element { tag, .. } => st.font_size.unwrap_or_else(|| default_size(tag)),
            _ => st.font_size.unwrap_or(20.0),
        }
    }

    /// (width, height) `n` wants inside a container `avail` wide; `None` width means "share the free space".
    fn measure(&self, n: &Node, avail: f32, parent: &Style) -> (Option<f32>, f32) {
        let st = self.style(n, parent);
        let natural = match n {
            Node::Text(_) => Self::font_size(n, &st) * 1.3,
            Node::Element { tag, .. } if tag == "button" => BUTTON_HEIGHT,
            Node::Element { children, .. } => {
                if is_text_leaf(n) {
                    Self::font_size(n, &st) * 1.3
                } else {
                    let kids = visible(children);
                    if kids.is_empty() {
                        0.0
                    } else {
                        let pad = st.padding.unwrap_or(PADDING);
                        let gap = st.gap.unwrap_or(GAP);
                        let inner = (st.width.unwrap_or(avail) - 2.0 * pad).max(0.0);
                        if st.row {
                            let widths = Self::row_widths(&st, &kids.iter().map(|k| self.style(k, &st).width).collect::<Vec<_>>(), inner, gap);
                            kids.iter().zip(&widths).map(|(k, w)| self.measure(k, *w, &st).1).fold(0.0, f32::max) + 2.0 * pad
                        } else {
                            kids.iter().map(|k| self.measure(k, inner, &st).1).sum::<f32>() + gap * (kids.len() - 1) as f32 + 2.0 * pad
                        }
                    }
                }
            }
            _ => 0.0,
        };
        (st.width, st.height.unwrap_or(natural))
    }

    /// Widths of the children of a row: fixed ones as given, the rest share what is left.
    fn row_widths(_st: &Style, fixed: &[Option<f32>], inner: f32, gap: f32) -> Vec<f32> {
        let used: f32 = fixed.iter().flatten().sum::<f32>() + gap * fixed.len().saturating_sub(1) as f32;
        let flexible = fixed.iter().filter(|w| w.is_none()).count().max(1) as f32;
        let each = ((inner - used) / flexible).max(0.0);
        fixed.iter().map(|w| w.unwrap_or(each)).collect()
    }

    fn place(&self, n: &Node, rect: [f32; 4], parent: &Style, out: &mut Vec<Item>) {
        let st = self.style(n, parent);
        let mut rect = rect;
        if let Some(w) = st.width {
            rect[2] = w;
        }
        match n {
            Node::Text(t) => out.push(Item::Text { text: t.clone(), size: Self::font_size(n, &st), color: st.color.unwrap_or([1.0; 4]), rect }),
            Node::Element { tag, .. } if tag == "button" => {
                out.push(Item::Button { id: button_id(n), text: n.text(), rect, color: st.background });
            }
            Node::Element { tag, children, .. } => {
                if is_text_leaf(n) {
                    if let Some(bg) = st.background {
                        out.push(Item::Panel { rect, color: bg });
                    }
                    out.push(Item::Text { text: n.text(), size: Self::font_size(n, &st), color: st.color.unwrap_or([1.0; 4]), rect });
                    return;
                }
                let card = tag == "root" || n.classes().iter().any(|c| matches!(*c, "panel" | "card"));
                if let Some(bg) = st.background {
                    out.push(Item::Panel { rect, color: bg });
                } else if card {
                    out.push(Item::Panel { rect, color: PANEL_COLOR });
                }
                let kids = visible(children);
                let (pad, gap) = (st.padding.unwrap_or(PADDING), st.gap.unwrap_or(GAP));
                let inner_w = (rect[2] - 2.0 * pad).max(0.0);
                let stack = Stack { axis: if st.row { Axis::Row } else { Axis::Column }, padding: pad, gap, align: Align::Stretch };
                let sizes: Vec<_> = if st.row {
                    let widths = Self::row_widths(&st, &kids.iter().map(|k| self.style(k, &st).width).collect::<Vec<_>>(), inner_w, gap);
                    widths.into_iter().map(|w| (Size::Fixed(w), None)).collect()
                } else {
                    kids.iter().map(|k| (Size::Fixed(self.measure(k, inner_w, &st).1), None)).collect()
                };
                for (k, r) in kids.iter().zip(stack.place(rect, &sizes)) {
                    self.place(k, r, &st, out);
                }
            }
            _ => {}
        }
    }
}

/// Concatenated text of every `<style>` element in the document.
fn collect_css(nodes: &[Node], out: &mut String) {
    for n in nodes {
        if let Node::Element { tag, children, .. } = n {
            if tag == "style" {
                out.push_str(&n.text());
                out.push('\n');
            } else {
                collect_css(children, out);
            }
        }
    }
}

/// Lay out a document in a `width`-wide column starting at (`x`, `y`). Returns the items and the total height.
pub fn layout(doc: &Document, x: f32, y: f32, width: f32) -> (Vec<Item>, f32) {
    let mut css = String::new();
    collect_css(&doc.nodes, &mut css);
    let sheet = Stylesheet::parse(&css);
    let ctx = Ctx { sheet: &sheet };
    let root = Style::default();
    let mut out = Vec::new();
    let mut cursor = y;
    for n in visible(&doc.nodes) {
        let (_, h) = ctx.measure(n, width, &root);
        ctx.place(n, [x, cursor, width, h], &root, &mut out);
        cursor += h + GAP;
    }
    (out, (cursor - y - GAP).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::razor::parse;

    fn texts(items: &[Item]) -> Vec<&str> {
        items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect()
    }

    #[test]
    fn root_becomes_a_panel_with_stacked_text_and_a_button() {
        let doc = parse("<root><h1>Title</h1><p>Some text</p><div class=\"row\"><button id=\"go\">Go now</button></div></root>").unwrap();
        let (items, h) = layout(&doc, 100.0, 50.0, 400.0);
        assert_eq!(items[0], Item::Panel { rect: [100.0, 50.0, 400.0, h], color: PANEL_COLOR });
        let Item::Text { text, size, rect, .. } = &items[1] else { panic!("{items:?}") };
        assert_eq!((text.as_str(), *size), ("Title", 40.0));
        assert_eq!(rect[0], 112.0, "padding on the left");
        assert_eq!(rect[2], 376.0, "inner width");
        let Item::Text { text, rect: r2, .. } = &items[2] else { panic!() };
        assert_eq!(text, "Some text");
        assert!(r2[1] > rect[1] + rect[3], "stacked below the heading");
        assert!(items.iter().any(|i| matches!(i, Item::Button { id, text, .. } if id == "go" && text == "Go now")));
    }

    #[test]
    fn expressions_blocks_and_style_are_not_drawn() {
        let doc = parse("<root><style>.a { color: red }</style>@if (x) {<p>hidden</p>}<p>@Name</p><p>shown</p></root>").unwrap();
        let (items, _) = layout(&doc, 0.0, 0.0, 300.0);
        assert_eq!(texts(&items), vec!["shown"]);
    }

    #[test]
    fn heights_add_up() {
        let doc = parse("<root><p>a</p><p>b</p></root>").unwrap();
        let (_, h) = layout(&doc, 0.0, 0.0, 200.0);
        // padding 12 + 26 + gap 8 + 26 + padding 12
        assert!((h - (2.0 * 12.0 + 2.0 * 26.0 + 8.0)).abs() < 1e-3, "{h}");
    }

    #[test]
    fn styles_set_colour_size_and_inherit() {
        let doc = parse("<root style=\"color: #f00\"><style>.big { font-size: 36px } p { color: #00f }</style><h2 class=\"big\">Hi</h2><p>Blue</p><label style=\"color: #0f0; font-size: 10px\">G</label><span>Inherits</span></root>").unwrap();
        let (items, _) = layout(&doc, 0.0, 0.0, 300.0);
        let t = |name: &str| items.iter().find_map(|i| match i { Item::Text { text, size, color, .. } if text == name => Some((*size, *color)), _ => None }).unwrap();
        assert_eq!(t("Hi"), (36.0, [1.0, 0.0, 0.0, 1.0]), "class size, colour inherited from root");
        assert_eq!(t("Blue").1, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(t("G"), (10.0, [0.0, 1.0, 0.0, 1.0]));
        assert_eq!(t("Inherits").1, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn backgrounds_padding_and_fixed_size() {
        let doc = parse("<root style=\"padding: 0px; gap: 0px\"><div style=\"background-color: #ff0000; padding: 4px; height: 100px\"><p>x</p></div></root>").unwrap();
        let (items, _) = layout(&doc, 0.0, 0.0, 200.0);
        assert_eq!(items[1], Item::Panel { rect: [0.0, 0.0, 200.0, 100.0], color: [1.0, 0.0, 0.0, 1.0] });
        let Item::Text { rect, .. } = &items[2] else { panic!("{items:?}") };
        assert_eq!((rect[0], rect[1]), (4.0, 4.0));
    }

    #[test]
    fn rows_share_the_width_with_fixed_children() {
        let doc = parse("<root style=\"padding: 0px; gap: 10px\"><div style=\"flex-direction: row; padding: 0px; gap: 10px\"><div style=\"width: 50px\"><p>A</p></div><p>B</p><p>C</p></div></root>").unwrap();
        let (items, _) = layout(&doc, 0.0, 0.0, 210.0);
        let x = |name: &str| items.iter().find_map(|i| match i { Item::Text { text, rect, .. } if text == name => Some((rect[0], rect[2])), _ => None }).unwrap();
        // 210 - 50 fixed - 2 gaps (20) = 140 for two flexible children -> 70 each
        assert_eq!(x("A").0, 12.0 + 0.0, "padding of the width-50 box");
        assert_eq!(x("B"), (60.0, 70.0));
        assert_eq!(x("C"), (140.0, 70.0));
    }
}
