//! Minimal `Input` resource: named actions (as in s&box `Input.Down("Forward")`) plus mouse delta.

use std::collections::HashSet;

#[derive(Default, Debug, Clone)]
pub struct Input {
    down: HashSet<String>,
    pressed: HashSet<String>,
    /// Accumulated since the last `end_frame` (pixels).
    pub mouse_delta: (f32, f32),
    pub mouse_captured: bool,
}

impl Input {
    pub fn set(&mut self, action: &str, is_down: bool) {
        if is_down {
            if self.down.insert(action.to_string()) {
                self.pressed.insert(action.to_string());
            }
        } else {
            self.down.remove(action);
        }
    }
    pub fn down(&self, action: &str) -> bool {
        self.down.contains(action)
    }
    /// True only on the frame the action went down.
    pub fn pressed(&self, action: &str) -> bool {
        self.pressed.contains(action)
    }
    /// Analog helper: +1 for `pos`, -1 for `neg`.
    pub fn axis(&self, pos: &str, neg: &str) -> f32 {
        self.down(pos) as i32 as f32 - self.down(neg) as i32 as f32
    }
    pub fn add_mouse(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }
    pub fn take_mouse(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.mouse_delta)
    }
    pub fn end_frame(&mut self) {
        self.pressed.clear();
    }
}
