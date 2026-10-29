//! Turns a parsed Razor document into positioned UI items. Only static markup is drawn: expressions and `@if` / `@foreach`
//! blocks are skipped, there is no stylesheet (SCSS), no data binding and no event handlers. The layout is one vertical
//! flow: containers stack their children in a column, headings and paragraphs become text, `<button>` a button.

use crate::layout::{Align, Axis, Size, Stack};
use crate::razor::{AttrValue, Document, Node};

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Background of a container (`root`, or any element with a `panel` / `card` class).
    Panel { rect: [f32; 4] },
    Text { text: String, size: f32, rect: [f32; 4] },
    Button { id: String, text: String, rect: [f32; 4] },
}

const PADDING: f32 = 12.0;
const GAP: f32 = 8.0;
const BUTTON_HEIGHT: f32 = 48.0;

fn heading_size(tag: &str) -> Option<f32> {
    Some(match tag {
        "h1" => 40.0,
        "h2" => 32.0,
        "h3" => 26.0,
        "h4" | "h5" | "h6" => 22.0,
        "p" | "label" | "span" | "div" => 20.0,
        _ => return None,
    })
}

/// Inline text of a node, joined with spaces (expressions contribute nothing).
fn inline_text(n: &Node) -> String {
    n.text()
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
        .filter(|n| !is_text_leaf(n) || !inline_text(n).is_empty())
        .collect()
}

/// Height `n` needs at the given width.
fn measure(n: &Node, width: f32) -> f32 {
    match n {
        Node::Text(_) => 20.0 * 1.3,
        Node::Element { tag, children, .. } if tag == "button" => {
            let _ = children;
            BUTTON_HEIGHT
        }
        Node::Element { tag, children, .. } => {
            if is_text_leaf(n) {
                return heading_size(tag).unwrap_or(20.0) * 1.3;
            }
            let kids = visible(children);
            if kids.is_empty() {
                return 0.0;
            }
            let inner = (width - 2.0 * PADDING).max(0.0);
            kids.iter().map(|k| measure(k, inner)).sum::<f32>() + GAP * (kids.len() - 1) as f32 + 2.0 * PADDING
        }
        _ => 0.0,
    }
}

fn place(n: &Node, rect: [f32; 4], out: &mut Vec<Item>) {
    match n {
        Node::Text(t) => out.push(Item::Text { text: t.clone(), size: 20.0, rect }),
        Node::Element { tag, children, .. } if tag == "button" => {
            out.push(Item::Button { id: button_id(n), text: inline_text(n), rect });
            let _ = children;
        }
        Node::Element { tag, children, .. } => {
            if is_text_leaf(n) {
                out.push(Item::Text { text: inline_text(n), size: heading_size(tag).unwrap_or(20.0), rect });
                return;
            }
            let kids = visible(children);
            let cards = tag == "root" || n.classes().iter().any(|c| matches!(*c, "panel" | "card"));
            if cards {
                out.push(Item::Panel { rect });
            }
            let inner_w = (rect[2] - 2.0 * PADDING).max(0.0);
            let sizes: Vec<_> = kids.iter().map(|k| (Size::Fixed(measure(k, inner_w)), None)).collect();
            let stack = Stack { axis: Axis::Column, padding: PADDING, gap: GAP, align: Align::Stretch };
            for (k, r) in kids.iter().zip(stack.place(rect, &sizes)) {
                place(k, r, out);
            }
        }
        _ => {}
    }
}

/// Lay out a document in a `width`-wide column starting at (`x`, `y`). Returns the items and the total height.
pub fn layout(doc: &Document, x: f32, y: f32, width: f32) -> (Vec<Item>, f32) {
    let mut out = Vec::new();
    let mut cursor = y;
    for n in visible(&doc.nodes) {
        let h = measure(n, width);
        place(n, [x, cursor, width, h], &mut out);
        cursor += h + GAP;
    }
    (out, (cursor - y - GAP).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::razor::parse;

    #[test]
    fn root_becomes_a_panel_with_stacked_text_and_a_button() {
        let doc = parse("<root><h1>Title</h1><p>Some text</p><div class=\"row\"><button id=\"go\">Go now</button></div></root>").unwrap();
        let (items, h) = layout(&doc, 100.0, 50.0, 400.0);
        assert_eq!(items[0], Item::Panel { rect: [100.0, 50.0, 400.0, h] });
        let Item::Text { text, size, rect } = &items[1] else { panic!("{items:?}") };
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
        let texts: Vec<_> = items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect();
        assert_eq!(texts, vec!["shown"]);
    }

    #[test]
    fn heights_add_up() {
        let doc = parse("<root><p>a</p><p>b</p></root>").unwrap();
        let (_, h) = layout(&doc, 0.0, 0.0, 200.0);
        // padding 12 + 26 + gap 8 + 26 + padding 12
        assert!((h - (2.0 * 12.0 + 2.0 * 26.0 + 8.0)).abs() < 1e-3, "{h}");
    }
}
