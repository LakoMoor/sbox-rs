//! Clicking a Razor page that is drawn on a quad in the world: the cursor becomes a ray from the camera, the ray is
//! intersected with the quad, and the hit point is looked up among the page's buttons.

use sbox_math::{Quat, Transform, Vec3};

use crate::razor_ui::Item;

/// World-space ray through the pixel (`x`, `y`) of a `width` x `height` window for a camera at `cam` with the given
/// horizontal field of view in degrees. The camera looks along its forward (+X) axis, with left = +Y and up = +Z.
pub fn camera_ray(cam: &Transform, fov_degrees: f32, width: f32, height: f32, x: f32, y: f32) -> (Vec3, Vec3) {
    let tan_x = (fov_degrees.to_radians() * 0.5).tan();
    let tan_y = tan_x * height / width;
    let ndc_x = x / width * 2.0 - 1.0;
    let ndc_y = 1.0 - y / height * 2.0;
    let local = Vec3::new(1.0, -ndc_x * tan_x, ndc_y * tan_y);
    (cam.position, (cam.rotation * local).normalize())
}

/// Where a ray hits the panel quad (in the panel object's local YZ plane, facing -X, `size` units wide and high, its
/// origin the middle of the top edge):
/// the point as fractions (u, v) of the page, with (0, 0) at its top left as seen from the front. `None` when the ray
/// points away, runs parallel, or misses the quad.
pub fn panel_uv(panel: &Transform, size: [f32; 2], origin: Vec3, dir: Vec3) -> Option<[f32; 2]> {
    let inv: Quat = panel.rotation.inverse();
    let o = inv * (origin - panel.position);
    let d = inv * dir;
    if d.x.abs() < 1e-6 {
        return None;
    }
    let t = -o.x / d.x;
    if t <= 0.0 {
        return None;
    }
    let hit = o + d * t;
    let hw = size[0] * 0.5;
    if hit.y.abs() > hw || hit.z > 0.0 || hit.z < -size[1] {
        return None;
    }
    Some([(hw - hit.y) / size[0], -hit.z / size[1]])
}

/// The button (topmost, so the last drawn) under a bitmap pixel.
pub fn hit_button(items: &[Item], x: f32, y: f32) -> Option<&str> {
    items.iter().rev().find_map(|i| match i {
        Item::Button { id, rect, .. } if x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3] => Some(id.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panel `x` ahead whose top edge is `top` above the origin.
    fn panel_at(x: f32, top: f32) -> Transform {
        Transform::from_position(Vec3::new(x, 0.0, top))
    }

    #[test]
    fn the_centre_ray_hits_the_middle_of_the_panel() {
        let cam = Transform::from_position(Vec3::ZERO);
        let (o, d) = camera_ray(&cam, 90.0, 800.0, 600.0, 400.0, 300.0);
        assert!((d - Vec3::X).length() < 1e-5);
        let uv = panel_uv(&panel_at(300.0, 50.0), [200.0, 100.0], o, d).unwrap();
        assert!((uv[0] - 0.5).abs() < 1e-5 && (uv[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn screen_left_and_up_map_to_the_page_left_and_top() {
        let cam = Transform::from_position(Vec3::ZERO);
        // At 90 degrees the half-width at distance 100 is 100 units; the panel is 200 x 200 there.
        let (o, d) = camera_ray(&cam, 90.0, 800.0, 800.0, 200.0, 200.0);
        let uv = panel_uv(&panel_at(100.0, 100.0), [200.0, 200.0], o, d).unwrap();
        assert!((uv[0] - 0.25).abs() < 1e-4 && (uv[1] - 0.25).abs() < 1e-4, "upper left quarter, got {uv:?}");
        let (o, d) = camera_ray(&cam, 90.0, 800.0, 800.0, 600.0, 600.0);
        let uv = panel_uv(&panel_at(100.0, 100.0), [200.0, 200.0], o, d).unwrap();
        assert!((uv[0] - 0.75).abs() < 1e-4 && (uv[1] - 0.75).abs() < 1e-4, "lower right quarter, got {uv:?}");
    }

    #[test]
    fn rays_that_miss_or_point_away_hit_nothing() {
        let o = Vec3::ZERO;
        assert!(panel_uv(&panel_at(300.0, 50.0), [100.0, 100.0], o, Vec3::new(1.0, 1.0, 0.0).normalize()).is_none(), "beside it");
        assert!(panel_uv(&panel_at(-300.0, 50.0), [100.0, 100.0], o, Vec3::X).is_none(), "behind the camera");
        assert!(panel_uv(&panel_at(300.0, 50.0), [100.0, 100.0], o, Vec3::Y).is_none(), "parallel");
    }

    #[test]
    fn the_topmost_button_under_the_point_is_found() {
        let b = |id: &str, r: [f32; 4]| Item::Button { id: id.into(), text: id.into(), rect: r, color: None };
        let items = [b("under", [0.0, 0.0, 100.0, 100.0]), b("over", [50.0, 50.0, 100.0, 100.0])];
        assert_eq!(hit_button(&items, 60.0, 60.0), Some("over"));
        assert_eq!(hit_button(&items, 10.0, 10.0), Some("under"));
        assert_eq!(hit_button(&items, 500.0, 10.0), None);
    }
}
