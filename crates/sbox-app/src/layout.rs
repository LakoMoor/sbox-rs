//! Flex-like stack layout for the UI: children are placed along one axis with padding, a gap, and either fixed sizes or
//! a share of the remaining space. Rectangles are `[x, y, width, height]` in pixels, origin top-left.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    Row,
    Column,
}

/// Size of a child along the stack axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Fixed(f32),
    /// Share of the space left after fixed children, padding and gaps (like `flex-grow`).
    Grow(f32),
}

/// Cross-axis placement of a child that is smaller than the container.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Start,
    Center,
    End,
    /// Fill the container's cross size.
    Stretch,
}

pub struct Stack {
    pub axis: Axis,
    pub padding: f32,
    pub gap: f32,
    pub align: Align,
}

impl Stack {
    pub fn new(axis: Axis) -> Self {
        Self { axis, padding: 0.0, gap: 0.0, align: Align::Stretch }
    }

    /// Rectangles of the children inside `rect`. `children` are (main size, cross size); a cross size of `None` fills
    /// the container, whatever the alignment.
    pub fn place(&self, rect: [f32; 4], children: &[(Size, Option<f32>)]) -> Vec<[f32; 4]> {
        let (main_len, cross_len) = match self.axis {
            Axis::Row => (rect[2], rect[3]),
            Axis::Column => (rect[3], rect[2]),
        };
        let inner_main = (main_len - 2.0 * self.padding).max(0.0);
        let inner_cross = (cross_len - 2.0 * self.padding).max(0.0);
        let gaps = self.gap * children.len().saturating_sub(1) as f32;
        let fixed: f32 = children.iter().map(|(s, _)| if let Size::Fixed(f) = s { *f } else { 0.0 }).sum();
        let grow: f32 = children.iter().map(|(s, _)| if let Size::Grow(g) = s { *g } else { 0.0 }).sum();
        let free = (inner_main - fixed - gaps).max(0.0);
        let mut cursor = self.padding;
        children
            .iter()
            .map(|(size, cross)| {
                let main = match size {
                    Size::Fixed(f) => *f,
                    Size::Grow(g) if grow > 0.0 => free * g / grow,
                    Size::Grow(_) => 0.0,
                };
                let cross_size = match (self.align, cross) {
                    (Align::Stretch, _) | (_, None) => inner_cross,
                    (_, Some(c)) => c.min(inner_cross),
                };
                let cross_pos = self.padding
                    + match self.align {
                        Align::Start | Align::Stretch => 0.0,
                        Align::Center => (inner_cross - cross_size) * 0.5,
                        Align::End => inner_cross - cross_size,
                    };
                let r = match self.axis {
                    Axis::Row => [rect[0] + cursor, rect[1] + cross_pos, main, cross_size],
                    Axis::Column => [rect[0] + cross_pos, rect[1] + cursor, cross_size, main],
                };
                cursor += main + self.gap;
                r
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_with_fixed_children_gap_and_padding() {
        let s = Stack { axis: Axis::Column, padding: 10.0, gap: 5.0, align: Align::Stretch };
        let r = s.place([100.0, 100.0, 200.0, 300.0], &[(Size::Fixed(50.0), None), (Size::Fixed(60.0), None)]);
        assert_eq!(r[0], [110.0, 110.0, 180.0, 50.0]);
        assert_eq!(r[1], [110.0, 165.0, 180.0, 60.0]);
    }

    #[test]
    fn grow_children_share_the_remaining_space() {
        let s = Stack { axis: Axis::Row, padding: 0.0, gap: 10.0, align: Align::Stretch };
        let r = s.place([0.0, 0.0, 330.0, 40.0], &[(Size::Fixed(100.0), None), (Size::Grow(1.0), None), (Size::Grow(3.0), None)]);
        // 330 - 100 - 2 gaps (20) = 210 -> 52.5 and 157.5
        assert_eq!(r[1], [110.0, 0.0, 52.5, 40.0]);
        assert_eq!(r[2], [172.5, 0.0, 157.5, 40.0]);
    }

    #[test]
    fn cross_axis_alignment() {
        let mut s = Stack::new(Axis::Row);
        s.align = Align::Center;
        let r = s.place([0.0, 0.0, 100.0, 100.0], &[(Size::Fixed(20.0), Some(40.0))]);
        assert_eq!(r[0], [0.0, 30.0, 20.0, 40.0]);
        s.align = Align::End;
        assert_eq!(s.place([0.0, 0.0, 100.0, 100.0], &[(Size::Fixed(20.0), Some(40.0))])[0], [0.0, 60.0, 20.0, 40.0]);
    }

    #[test]
    fn overflowing_fixed_children_leave_no_space_for_grow() {
        let s = Stack::new(Axis::Row);
        let r = s.place([0.0, 0.0, 50.0, 10.0], &[(Size::Fixed(80.0), None), (Size::Grow(1.0), None)]);
        assert_eq!(r[1][2], 0.0);
    }
}
