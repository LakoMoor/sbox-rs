//! A small retained-mode UI: filled panels, text labels and buttons with hover state.
//! Layout and hit testing are plain data (pixels, origin top-left); `rects` turns the tree into
//! [`UiRect`]s for `Renderer::overlay`.

use sbox_render::{TextureId, UiRect};

pub struct Panel {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

pub struct Label {
    pub texture: TextureId,
    pub rect: [f32; 4],
}

pub struct Button {
    pub id: String,
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub hover_color: [f32; 4],
    /// Text texture and its size in pixels, drawn centred in the button.
    pub label: Option<(TextureId, [f32; 2])>,
}

pub struct Ui {
    /// 1x1 white texture that filled rectangles are tinted from.
    white: TextureId,
    pub panels: Vec<Panel>,
    pub labels: Vec<Label>,
    pub buttons: Vec<Button>,
    hover: Option<usize>,
}

fn contains(rect: &[f32; 4], x: f32, y: f32) -> bool {
    x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]
}

impl Ui {
    pub fn new(white: TextureId) -> Self {
        Self { white, panels: vec![], labels: vec![], buttons: vec![], hover: None }
    }

    /// Topmost (last added) button under the point.
    pub fn button_at(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().rposition(|b| contains(&b.rect, x, y))
    }

    pub fn set_mouse(&mut self, x: f32, y: f32) {
        self.hover = self.button_at(x, y);
    }

    /// Id of the button a click at the point lands on.
    pub fn click(&self, x: f32, y: f32) -> Option<&str> {
        self.button_at(x, y).map(|i| self.buttons[i].id.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty() && self.labels.is_empty() && self.buttons.is_empty()
    }

    /// Draw order: panels, buttons (with their labels), free labels.
    pub fn rects(&self) -> Vec<UiRect> {
        let mut out = Vec::new();
        for p in &self.panels {
            out.push(UiRect { texture: self.white, rect: p.rect, tint: p.color });
        }
        for (i, b) in self.buttons.iter().enumerate() {
            let tint = if self.hover == Some(i) { b.hover_color } else { b.color };
            out.push(UiRect { texture: self.white, rect: b.rect, tint });
            if let Some((tex, [w, h])) = b.label {
                out.push(UiRect { texture: tex, rect: [b.rect[0] + (b.rect[2] - w) * 0.5, b.rect[1] + (b.rect[3] - h) * 0.5, w, h], tint: [1.0; 4] });
            }
        }
        for l in &self.labels {
            out.push(UiRect { texture: l.texture, rect: l.rect, tint: [1.0; 4] });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(id: &str, rect: [f32; 4]) -> Button {
        Button { id: id.into(), rect, color: [0.2; 4], hover_color: [0.6; 4], label: None }
    }

    #[test]
    fn click_hits_the_topmost_button_only() {
        let mut ui = Ui::new(TextureId(0));
        ui.buttons.push(button("back", [0.0, 0.0, 100.0, 100.0]));
        ui.buttons.push(button("front", [50.0, 50.0, 100.0, 100.0]));
        assert_eq!(ui.click(10.0, 10.0), Some("back"));
        assert_eq!(ui.click(75.0, 75.0), Some("front"));
        assert_eq!(ui.click(200.0, 200.0), None);
        // The right / bottom edges are exclusive.
        assert_eq!(ui.click(150.0, 150.0), None);
    }

    #[test]
    fn hover_switches_the_button_tint() {
        let mut ui = Ui::new(TextureId(0));
        ui.buttons.push(button("a", [0.0, 0.0, 10.0, 10.0]));
        assert_eq!(ui.rects()[0].tint, [0.2; 4]);
        ui.set_mouse(5.0, 5.0);
        assert_eq!(ui.rects()[0].tint, [0.6; 4]);
        ui.set_mouse(50.0, 50.0);
        assert_eq!(ui.rects()[0].tint, [0.2; 4]);
    }

    #[test]
    fn button_label_is_centred() {
        let mut ui = Ui::new(TextureId(0));
        let mut b = button("a", [100.0, 100.0, 200.0, 50.0]);
        b.label = Some((TextureId(7), [80.0, 20.0]));
        ui.buttons.push(b);
        let r = ui.rects();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].texture, TextureId(7));
        assert_eq!(r[1].rect, [160.0, 115.0, 80.0, 20.0]);
    }
}
