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
    /// Multiplied with the (white) text texture; rgb in sRGB.
    pub tint: [f32; 4],
}

pub struct Button {
    pub id: String,
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub hover_color: [f32; 4],
    /// Text texture and its size in pixels, drawn centred in the button.
    pub label: Option<(TextureId, [f32; 2])>,
}

/// Single-line text input. The app rasterises `text` into `label` whenever [`Ui::take_dirty`] reports the field.
pub struct TextField {
    pub id: String,
    pub rect: [f32; 4],
    pub text: String,
    pub max_len: usize,
    pub color: [f32; 4],
    pub focus_color: [f32; 4],
    /// Rasterised `text` and its pixel size; `None` while the text is empty.
    pub label: Option<(TextureId, [f32; 2])>,
    /// Caret position as a character index into `text`.
    pub caret: usize,
    /// Pixel offset of the caret from the start of the text, measured by the app when the field is reported dirty.
    pub caret_x: f32,
}

/// Caret movement keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaretMove {
    Left,
    Right,
    Home,
    End,
}

pub struct Ui {
    /// 1x1 white texture that filled rectangles are tinted from.
    white: TextureId,
    pub panels: Vec<Panel>,
    pub labels: Vec<Label>,
    pub buttons: Vec<Button>,
    pub fields: Vec<TextField>,
    hover: Option<usize>,
    focus: Option<usize>,
    dirty: Vec<usize>,
}

fn contains(rect: &[f32; 4], x: f32, y: f32) -> bool {
    x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]
}

impl Ui {
    pub fn new(white: TextureId) -> Self {
        Self { white, panels: vec![], labels: vec![], buttons: vec![], fields: vec![], hover: None, focus: None, dirty: vec![] }
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
        self.panels.is_empty() && self.labels.is_empty() && self.buttons.is_empty() && self.fields.is_empty()
    }

    /// Give focus to the text field under the point (or clear it when the click lands elsewhere).
    /// Returns whether a field now has focus.
    pub fn focus_at(&mut self, x: f32, y: f32) -> bool {
        self.focus = self.fields.iter().rposition(|f| contains(&f.rect, x, y));
        if let Some(i) = self.focus {
            // The caret starts at the end of the text; `dirty` makes the app measure its offset.
            self.fields[i].caret = self.fields[i].text.chars().count();
            self.dirty.push(i);
        }
        self.focus.is_some()
    }

    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    pub fn focused(&self) -> Option<&TextField> {
        self.focus.map(|i| &self.fields[i])
    }

    fn byte_at(text: &str, chars: usize) -> usize {
        text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
    }

    /// Typed character for the focused field, inserted at the caret. Control characters are ignored.
    /// Returns whether it was consumed.
    pub fn type_char(&mut self, c: char) -> bool {
        let Some(i) = self.focus else { return false };
        let f = &mut self.fields[i];
        if !c.is_control() && f.text.chars().count() < f.max_len {
            let at = Self::byte_at(&f.text, f.caret);
            f.text.insert(at, c);
            f.caret += 1;
            self.dirty.push(i);
        }
        true
    }

    /// Delete the character before the caret.
    pub fn backspace(&mut self) -> bool {
        let Some(i) = self.focus else { return false };
        let f = &mut self.fields[i];
        if f.caret > 0 {
            f.caret -= 1;
            let at = Self::byte_at(&f.text, f.caret);
            f.text.remove(at);
            self.dirty.push(i);
        }
        true
    }

    /// Delete the character after the caret.
    pub fn delete_forward(&mut self) -> bool {
        let Some(i) = self.focus else { return false };
        let f = &mut self.fields[i];
        if f.caret < f.text.chars().count() {
            let at = Self::byte_at(&f.text, f.caret);
            f.text.remove(at);
            self.dirty.push(i);
        }
        true
    }

    pub fn move_caret(&mut self, to: CaretMove) -> bool {
        let Some(i) = self.focus else { return false };
        let f = &mut self.fields[i];
        let len = f.text.chars().count();
        f.caret = match to {
            CaretMove::Left => f.caret.saturating_sub(1),
            CaretMove::Right => (f.caret + 1).min(len),
            CaretMove::Home => 0,
            CaretMove::End => len,
        };
        self.dirty.push(i);
        true
    }

    /// Release focus (Escape / Enter) and return the text of the field that had it.
    pub fn submit(&mut self) -> Option<(&str, &str)> {
        let i = self.focus.take()?;
        let f = &self.fields[i];
        Some((f.id.as_str(), f.text.as_str()))
    }

    /// Fields whose text changed since the last call, so the app can re-rasterise their labels.
    pub fn take_dirty(&mut self) -> Vec<usize> {
        let mut d = std::mem::take(&mut self.dirty);
        d.sort_unstable();
        d.dedup();
        d
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
        for (i, f) in self.fields.iter().enumerate() {
            let focused = self.focus == Some(i);
            out.push(UiRect { texture: self.white, rect: f.rect, tint: if focused { f.focus_color } else { f.color } });
            let mut text_right = f.rect[0] + 8.0;
            if let Some((tex, [w, h])) = f.label {
                out.push(UiRect { texture: tex, rect: [f.rect[0] + 8.0, f.rect[1] + (f.rect[3] - h) * 0.5, w, h], tint: [1.0; 4] });
                text_right += w;
            }
            if focused {
                // Caret at its measured offset (after the text when the app has not measured yet).
                let x = if f.caret >= f.text.chars().count() { text_right } else { f.rect[0] + 8.0 + f.caret_x };
                out.push(UiRect { texture: self.white, rect: [x + 2.0, f.rect[1] + 8.0, 2.0, f.rect[3] - 16.0], tint: [1.0; 4] });
            }
        }
        for l in &self.labels {
            out.push(UiRect { texture: l.texture, rect: l.rect, tint: l.tint });
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

    fn field(id: &str, rect: [f32; 4]) -> TextField {
        TextField { id: id.into(), rect, text: String::new(), max_len: 5, color: [0.1; 4], focus_color: [0.3; 4], label: None, caret: 0, caret_x: 0.0 }
    }

    #[test]
    fn typing_goes_to_the_focused_field_only() {
        let mut ui = Ui::new(TextureId(0));
        ui.fields.push(field("a", [0.0, 0.0, 100.0, 30.0]));
        ui.fields.push(field("b", [0.0, 50.0, 100.0, 30.0]));
        assert!(!ui.type_char('x'), "nothing focused yet");
        assert!(ui.focus_at(10.0, 60.0));
        for c in "hello world".chars() {
            ui.type_char(c);
        }
        assert_eq!(ui.fields[1].text, "hello", "limited to max_len");
        assert_eq!(ui.fields[0].text, "");
        assert_eq!(ui.take_dirty(), vec![1]);
        assert!(ui.take_dirty().is_empty());
        ui.backspace();
        assert_eq!(ui.fields[1].text, "hell");
        ui.type_char('\n');
        assert_eq!(ui.fields[1].text, "hell", "control characters are ignored");
    }

    #[test]
    fn caret_inserts_and_deletes_in_the_middle() {
        let mut ui = Ui::new(TextureId(0));
        ui.fields.push(field("a", [0.0, 0.0, 100.0, 30.0]));
        ui.focus_at(5.0, 5.0);
        for c in "abcd".chars() {
            ui.type_char(c);
        }
        ui.move_caret(CaretMove::Left);
        ui.move_caret(CaretMove::Left);
        ui.type_char('X');
        assert_eq!(ui.fields[0].text, "abXcd");
        assert_eq!(ui.fields[0].caret, 3);
        ui.backspace();
        assert_eq!(ui.fields[0].text, "abcd");
        ui.delete_forward();
        assert_eq!(ui.fields[0].text, "abd");
        ui.move_caret(CaretMove::Home);
        ui.backspace();
        assert_eq!(ui.fields[0].text, "abd", "nothing before the caret");
        ui.move_caret(CaretMove::End);
        assert_eq!(ui.fields[0].caret, 3);
        ui.move_caret(CaretMove::Right);
        assert_eq!(ui.fields[0].caret, 3, "clamped to the end");
    }

    #[test]
    fn clicking_elsewhere_clears_focus_and_submit_returns_the_text() {
        let mut ui = Ui::new(TextureId(0));
        ui.fields.push(field("name", [0.0, 0.0, 100.0, 30.0]));
        ui.focus_at(5.0, 5.0);
        ui.type_char('a');
        assert_eq!(ui.submit(), Some(("name", "a")));
        assert!(!ui.has_focus());
        ui.focus_at(5.0, 5.0);
        assert!(!ui.focus_at(500.0, 500.0));
        assert!(!ui.backspace());
    }

    #[test]
    fn focused_field_draws_a_caret() {
        let mut ui = Ui::new(TextureId(0));
        ui.fields.push(field("a", [10.0, 10.0, 100.0, 30.0]));
        assert_eq!(ui.rects().len(), 1);
        ui.focus_at(20.0, 20.0);
        assert_eq!(ui.rects().len(), 2);
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
