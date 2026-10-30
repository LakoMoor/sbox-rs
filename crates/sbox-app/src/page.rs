//! A Razor page on screen: the parsed document, its bound values and the game-code handlers its `onclick=@Name` buttons
//! call. A handler changes the values; the page is laid out again and replaces its widgets in the UI.

use std::collections::HashMap;
use std::ops::Range;

use crate::razor::Document;
use crate::razor_ui::{self, Item};
use crate::ui::{self, Ui};
use sbox_render::Renderer;

/// What a click on a page button asks the app to do besides changing values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
}

pub type Handler = Box<dyn FnMut(&mut HashMap<String, String>) -> Effect>;

pub struct Page {
    doc: Document,
    pub values: HashMap<String, String>,
    /// Opacity and scale from the scene's `ScreenPanel` (1, 1 without one).
    pub opacity: f32,
    pub scale: f32,
    handlers: HashMap<String, Handler>,
    /// Where the page's widgets sit in the UI vectors, so a re-layout replaces them and leaves the rest alone.
    panels: Range<usize>,
    labels: Range<usize>,
    buttons: Range<usize>,
    /// How many scroll areas the UI had before the page added its own.
    areas: usize,
}

impl Page {
    pub fn new(doc: Document, values: HashMap<String, String>) -> Self {
        let program = doc.code.as_deref().map(crate::csharp::parse);
        let mut values = values;
        let mut p = Self { doc, values: HashMap::new(), opacity: 1.0, scale: 1.0, handlers: HashMap::new(), panels: 0..0, labels: 0..0, buttons: 0..0, areas: 0 };
        // Fields of the page's `@code` block give starting values (explicit ones win); its simple methods become handlers.
        if let Some(prog) = program {
            for (k, v) in prog.fields {
                values.entry(k).or_insert(v);
            }
            for (name, stmts) in prog.methods {
                p.on(&name, move |v| {
                    crate::csharp::run(&stmts, v);
                    Effect::None
                });
            }
        }
        p.values = values;
        p.register_defaults_if_missing();
        p
    }

    /// The built-in demo handlers, except where the page's own code already defines one of that name.
    fn register_defaults_if_missing(&mut self) {
        let mine: Vec<(String, Handler)> = self.handlers.drain().collect();
        self.register_defaults();
        self.handlers.extend(mine);
    }

    /// Handlers every page gets: `Quit`, and `Increment` / `Decrement` / `Toggle` acting on the `Count` and `Show` values.
    fn register_defaults(&mut self) {
        self.on("Quit", |_| Effect::Quit);
        self.on("Increment", |v| {
            bump(v, "Count", 1);
            Effect::None
        });
        self.on("Decrement", |v| {
            bump(v, "Count", -1);
            Effect::None
        });
        self.on("Toggle", |v| {
            let on = v.get("Show").is_some_and(|s| s == "true");
            v.insert("Show".into(), (!on).to_string());
            Effect::None
        });
    }

    /// Game code for the button whose `onclick` is `@name`; replaces an earlier handler of that name.
    pub fn on(&mut self, name: &str, handler: impl FnMut(&mut HashMap<String, String>) -> Effect + 'static) {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    /// Run the handler for a clicked button id. `None` when the page has no handler of that name.
    pub fn click(&mut self, id: &str) -> Option<Effect> {
        let handler = self.handlers.get_mut(id)?;
        Some(handler(&mut self.values))
    }

    /// The page laid out at `width` pixels from the origin, with the current values (for drawing it somewhere other than the screen).
    pub fn items(&self, width: f32) -> Vec<Item> {
        razor_ui::layout_bound(&self.doc, 0.0, 0.0, width, &self.values).0
    }

    /// (Re)build the page's widgets: whatever an earlier call added is removed first.
    pub fn populate(&mut self, ui: &mut Ui, r: &mut Renderer, font: &ab_glyph::FontVec) {
        ui.panels.drain(self.panels.clone());
        ui.labels.drain(self.labels.clone());
        ui.buttons.drain(self.buttons.clone());
        ui.areas.truncate(self.areas);
        let (k, a) = (self.scale, self.opacity);
        // Laid out at 1x and then scaled, so the page keeps its proportions.
        let (items, content) = razor_ui::layout_bound(&self.doc, 40.0, 40.0, 520.0, &self.values);
        ui.content_height = (40.0 + content + 40.0) * k;
        let r4 = |r: [f32; 4]| [r[0] * k, r[1] * k, r[2] * k, r[3] * k];
        let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * a];
        // The page is always the first thing in the UI after the HUD, so appending is where its old widgets were.
        self.panels = ui.panels.len()..ui.panels.len();
        self.labels = ui.labels.len()..ui.labels.len();
        self.buttons = ui.buttons.len()..ui.buttons.len();
        self.areas = ui.areas.len();
        // Areas being built: (rect, content height, where their widgets start).
        let mut open: Vec<([f32; 4], f32, usize, usize, usize)> = Vec::new();
        for item in items {
            match item {
                Item::AreaStart { rect, content_height } => open.push((rect, content_height, ui.panels.len(), ui.labels.len(), ui.buttons.len())),
                Item::AreaEnd => {
                    if let Some((rect, content_height, p, l, b)) = open.pop() {
                        ui.areas.push(ui::ScrollArea { rect: r4(rect), content_height: content_height * k, scroll_y: 0.0, panels: p..ui.panels.len(), labels: l..ui.labels.len(), buttons: b..ui.buttons.len() });
                    }
                }
                Item::Panel { rect, color } => ui.panels.push(ui::Panel { rect: r4(rect), color: fade(color) }),
                Item::Text { text, size, color, rect } => {
                    let (texture, [w, h]) = crate::App::text_texture(r, font, &text, size * k);
                    ui.labels.push(ui::Label { texture, rect: [rect[0] * k, rect[1] * k, w, h], tint: fade(color) });
                }
                Item::Button { id, text, rect, color } => ui.buttons.push(ui::Button {
                    id,
                    rect: r4(rect),
                    color: fade(color.unwrap_or([0.18, 0.2, 0.3, 1.0])),
                    hover_color: fade([0.3, 0.4, 0.7, 1.0]),
                    label: Some(crate::App::text_texture(r, font, &text, 24.0 * k)),
                }),
            }
        }
        self.panels.end = ui.panels.len();
        self.labels.end = ui.labels.len();
        self.buttons.end = ui.buttons.len();
    }
}

/// Draw laid-out items into an RGBA bitmap (straight alpha, sRGB bytes) of `w` x `h` pixels: panels and buttons as
/// filled rectangles over what is already there, text blitted from the 4x supersampled glyph raster with a box filter.
/// Without a font the text is left out. Scroll areas are drawn unscrolled and unclipped.
pub fn bitmap(items: &[Item], w: u32, h: u32, font: Option<&ab_glyph::FontVec>) -> Vec<u8> {
    let mut img = vec![0u8; (w * h * 4) as usize];
    let blend = |img: &mut Vec<u8>, x: i32, y: i32, c: [f32; 3], a: f32| {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 || a <= 0.0 {
            return;
        }
        let o = ((y as u32 * w + x as u32) * 4) as usize;
        let da = img[o + 3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for k in 0..3 {
            let d = img[o + k] as f32 / 255.0;
            img[o + k] = (((c[k] * a + d * da * (1.0 - a)) / oa.max(1e-6)) * 255.0).round() as u8;
        }
        img[o + 3] = (oa * 255.0).round() as u8;
    };
    let fill = |img: &mut Vec<u8>, rect: [f32; 4], color: [f32; 4]| {
        for y in rect[1].floor() as i32..(rect[1] + rect[3]).ceil() as i32 {
            for x in rect[0].floor() as i32..(rect[0] + rect[2]).ceil() as i32 {
                blend(img, x, y, [color[0], color[1], color[2]], color[3]);
            }
        }
    };
    let text = |img: &mut Vec<u8>, s: &str, size: f32, color: [f32; 4], at: [f32; 2]| {
        let Some(font) = font else { return };
        let st = crate::text::TextStyle { text: s.to_string(), color: [1.0; 4], font: "Poppins".into(), size, weight: 600, italic: false, line_height: 1.0, align: -1, scale: 1.0 };
        let (src, sw, sh) = crate::text::rasterise(font, &st);
        let (tw, th) = (sw / 4, sh / 4);
        for y in 0..th {
            for x in 0..tw {
                let mut cov = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        cov += src[(((y * 4 + sy) * sw + x * 4 + sx) * 4 + 3) as usize] as f32 / 255.0;
                    }
                }
                blend(img, at[0] as i32 + x as i32, at[1] as i32 + y as i32, [color[0], color[1], color[2]], cov / 16.0 * color[3]);
            }
        }
    };
    for item in items {
        match item {
            Item::Panel { rect, color } => fill(&mut img, *rect, *color),
            Item::Text { text: t, size, color, rect } => text(&mut img, t, *size, *color, [rect[0], rect[1]]),
            Item::Button { text: t, rect, color, .. } => {
                fill(&mut img, *rect, color.unwrap_or([0.18, 0.2, 0.3, 1.0]));
                if let Some(font) = font {
                    let width = crate::App::text_texture_width(font, t, 24.0) * 0.25;
                    text(&mut img, t, 24.0, [1.0; 4], [rect[0] + (rect[2] - width) * 0.5, rect[1] + (rect[3] - 24.0 * 1.3) * 0.5]);
                }
            }
            Item::AreaStart { .. } | Item::AreaEnd => {}
        }
    }
    img
}

fn bump(v: &mut HashMap<String, String>, key: &str, by: i64) {
    let n = v.get(key).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    v.insert(key.to_string(), (n + by).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(src: &str) -> Page {
        Page::new(crate::razor::parse(src).unwrap(), HashMap::new())
    }

    #[test]
    fn the_pages_own_code_block_sets_values_and_defines_handlers() {
        let mut p = page("<root><button onclick=@Bump>+</button>@Score</root>
@code
{
    int Score = 10;
    void Bump() { Score += 5; }
    void Increment() { Score = 0; }
}
");
        assert_eq!(p.values["Score"], "10", "the field's literal is the starting value");
        p.click("Bump");
        assert_eq!(p.values["Score"], "15");
        p.click("Increment");
        assert_eq!(p.values["Score"], "0", "the page's method wins over the built-in demo handler");
    }

    #[test]
    fn clicks_run_game_code_that_changes_the_values() {
        let mut p = page("<root><button onclick=@Increment>+</button>@Count</root>");
        assert_eq!(p.click("Increment"), Some(Effect::None));
        assert_eq!(p.click("Increment"), Some(Effect::None));
        p.click("Decrement");
        assert_eq!(p.values["Count"], "1");
        assert_eq!(p.click("Quit"), Some(Effect::Quit));
        assert_eq!(p.click("Nope"), None, "unknown handlers are left to the app");
        p.click("Toggle");
        assert_eq!(p.values["Show"], "true");
        p.click("Toggle");
        assert_eq!(p.values["Show"], "false");
    }

    #[test]
    fn a_bitmap_composites_panels_over_each_other() {
        let items = [
            Item::Panel { rect: [0.0, 0.0, 4.0, 4.0], color: [1.0, 0.0, 0.0, 1.0] },
            Item::Panel { rect: [2.0, 2.0, 2.0, 2.0], color: [0.0, 0.0, 1.0, 0.5] },
            Item::Button { id: "b".into(), text: "ok".into(), rect: [6.0, 0.0, 2.0, 2.0], color: Some([0.0, 1.0, 0.0, 1.0]) },
        ];
        let img = bitmap(&items, 8, 4, None);
        let px = |x: usize, y: usize| &img[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
        assert_eq!(px(0, 0), &[255, 0, 0, 255]);
        assert_eq!(px(3, 3), &[128, 0, 128, 255], "half blue over red");
        assert_eq!(px(5, 2), &[0, 0, 0, 0], "untouched stays transparent");
        assert_eq!(px(7, 1), &[0, 255, 0, 255], "a button is a filled rectangle");
        // Rectangles hanging over the edge are clipped, not wrapped.
        let img = bitmap(&[Item::Panel { rect: [6.0, 2.0, 10.0, 10.0], color: [1.0; 4] }], 8, 4, None);
        assert_eq!(&img[(3 * 8 + 7) * 4..(3 * 8 + 7) * 4 + 4], &[255; 4]);
        assert_eq!(&img[(3 * 8 + 5) * 4..(3 * 8 + 5) * 4 + 4], &[0; 4]);
    }

    #[test]
    fn screen_panel_settings_default_to_identity() {
        let p = page("<root/>");
        assert_eq!((p.opacity, p.scale), (1.0, 1.0));
    }

    #[test]
    fn custom_handlers_replace_defaults() {
        let mut p = page("<root/>");
        p.on("Increment", |v| {
            v.insert("Count".into(), "100".into());
            Effect::None
        });
        p.click("Increment");
        assert_eq!(p.values["Count"], "100");
    }

    #[test]
    fn changed_values_change_the_layout() {
        let mut p = page("<root><p>@if (Show) {<b>on</b>} else {<i>off</i>}</p></root>");
        let text = |p: &Page| -> Vec<String> {
            razor_ui::layout_bound(&p.doc, 0.0, 0.0, 400.0, &p.values).0.into_iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text) } else { None }).collect()
        };
        assert_eq!(text(&p), vec!["off"]);
        p.click("Toggle");
        assert_eq!(text(&p), vec!["on"]);
    }
}
