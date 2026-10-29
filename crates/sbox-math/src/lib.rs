//! Math types following s&box conventions (Sandbox.System/Math):
//! X = forward, Y = left, Z = up. Angles are pitch/yaw/roll in degrees.

pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

pub const FORWARD: Vec3 = Vec3::new(1.0, 0.0, 0.0);
pub const BACKWARD: Vec3 = Vec3::new(-1.0, 0.0, 0.0);
pub const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);
pub const DOWN: Vec3 = Vec3::new(0.0, 0.0, -1.0);
pub const RIGHT: Vec3 = Vec3::new(0.0, -1.0, 0.0);
pub const LEFT: Vec3 = Vec3::new(0.0, 1.0, 0.0);

/// Pitch / yaw / roll in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Angles {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

impl Angles {
    pub const ZERO: Angles = Angles { pitch: 0.0, yaw: 0.0, roll: 0.0 };
    pub const fn new(pitch: f32, yaw: f32, roll: f32) -> Self {
        Self { pitch, yaw, roll }
    }
    pub fn to_rotation(self) -> Rotation {
        Rotation::from_angles(self)
    }
}

pub type Rotation = Quat;

/// Extension methods matching `Sandbox.Rotation`.
pub trait RotationExt {
    fn from_angles(a: Angles) -> Quat;
    fn forward(&self) -> Vec3;
    fn backward(&self) -> Vec3;
    fn right(&self) -> Vec3;
    fn left(&self) -> Vec3;
    fn up(&self) -> Vec3;
    fn down(&self) -> Vec3;
    fn angles(&self) -> Angles;
    fn look_at(forward: Vec3, up: Vec3) -> Quat;
}

impl RotationExt for Quat {
    /// Exact port of `Rotation.From(pitch, yaw, roll)`.
    fn from_angles(a: Angles) -> Quat {
        let p = a.pitch.to_radians() * 0.5;
        let y = a.yaw.to_radians() * 0.5;
        let r = a.roll.to_radians() * 0.5;
        let (sp, cp) = p.sin_cos();
        let (sy, cy) = y.sin_cos();
        let (sr, cr) = r.sin_cos();
        let (sr_cp, cr_sp) = (sr * cp, cr * sp);
        let x = sr_cp * cy - cr_sp * sy;
        let yy = cr_sp * cy + sr_cp * sy;
        let (cr_cp, sr_sp) = (cr * cp, sr * sp);
        let z = cr_cp * sy - sr_sp * cy;
        let w = cr_cp * cy + sr_sp * sy;
        Quat::from_xyzw(x, yy, z, w)
    }
    fn forward(&self) -> Vec3 { *self * FORWARD }
    fn backward(&self) -> Vec3 { *self * BACKWARD }
    fn right(&self) -> Vec3 { *self * RIGHT }
    fn left(&self) -> Vec3 { *self * LEFT }
    fn up(&self) -> Vec3 { *self * UP }
    fn down(&self) -> Vec3 { *self * DOWN }
    fn angles(&self) -> Angles {
        let f = self.forward();
        let l = self.left();
        let u = self.up();
        let xy = (f.x * f.x + f.y * f.y).sqrt();
        if xy > 0.001 {
            Angles {
                yaw: f.y.atan2(f.x).to_degrees(),
                pitch: (-f.z).atan2(xy).to_degrees(),
                roll: l.z.atan2(u.z).to_degrees(),
            }
        } else {
            Angles {
                yaw: (-l.x).atan2(l.y).to_degrees(),
                pitch: (-f.z).atan2(xy).to_degrees(),
                roll: 0.0,
            }
        }
    }
    fn look_at(forward: Vec3, up: Vec3) -> Quat {
        let f = forward.normalize_or_zero();
        if f == Vec3::ZERO {
            return Quat::IDENTITY;
        }
        let mut l = up.cross(f);
        if l.length_squared() < 1e-8 {
            l = UP.cross(f);
            if l.length_squared() < 1e-8 {
                l = LEFT;
            }
        }
        let l = l.normalize();
        let u = f.cross(l);
        Quat::from_mat3(&Mat3::from_cols(f, l, u))
    }
}

/// Position / rotation / scale (Sandbox `Transform`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Transform {
    pub const ZERO: Transform = Transform { position: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE };

    pub fn new(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self { position, rotation, scale }
    }
    pub fn from_position(position: Vec3) -> Self {
        Self { position, ..Self::ZERO }
    }

    fn safe_scale(&self) -> Vec3 {
        let f = |v: f32| if v != 0.0 { v } else { 1.0 };
        Vec3::new(f(self.scale.x), f(self.scale.y), f(self.scale.z))
    }

    pub fn point_to_world(&self, p: Vec3) -> Vec3 {
        self.position + self.rotation * (p * self.scale)
    }
    pub fn point_to_local(&self, p: Vec3) -> Vec3 {
        (self.rotation.inverse() * (p - self.position)) / self.safe_scale()
    }
    pub fn normal_to_world(&self, n: Vec3) -> Vec3 {
        (self.rotation * n).normalize_or_zero()
    }
    pub fn normal_to_local(&self, n: Vec3) -> Vec3 {
        (self.rotation.inverse() * n).normalize_or_zero()
    }
    pub fn rotation_to_world(&self, r: Quat) -> Quat {
        self.rotation * r
    }
    pub fn rotation_to_local(&self, r: Quat) -> Quat {
        self.rotation.inverse() * r
    }

    /// `parent.to_world(&local)` — port of `Transform.ToWorld`.
    pub fn to_world(&self, local: &Transform) -> Transform {
        Transform {
            position: self.point_to_world(local.position),
            rotation: self.rotation * local.rotation,
            scale: self.scale * local.scale,
        }
    }
    /// `parent.to_local(&world)` — port of `Transform.ToLocal`.
    pub fn to_local(&self, world: &Transform) -> Transform {
        Transform {
            position: self.point_to_local(world.position),
            rotation: self.rotation.inverse() * world.rotation,
            scale: world.scale / self.safe_scale(),
        }
    }
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
    pub fn lerp(a: &Transform, b: &Transform, t: f32) -> Transform {
        Transform {
            position: a.position.lerp(b.position, t),
            rotation: a.rotation.slerp(b.rotation, t),
            scale: a.scale.lerp(b.scale, t),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox {
    pub mins: Vec3,
    pub maxs: Vec3,
}

impl BBox {
    pub fn new(mins: Vec3, maxs: Vec3) -> Self {
        Self { mins, maxs }
    }
    pub fn from_points(pts: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut it = pts.into_iter();
        let first = it.next()?;
        Some(it.fold(Self::new(first, first), |b, p| Self::new(b.mins.min(p), b.maxs.max(p))))
    }
    pub fn center(&self) -> Vec3 {
        (self.mins + self.maxs) * 0.5
    }
    pub fn size(&self) -> Vec3 {
        self.maxs - self.mins
    }
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.mins).all() && p.cmple(self.maxs).all()
    }
    pub fn union(&self, o: &BBox) -> BBox {
        BBox::new(self.mins.min(o.mins), self.maxs.max(o.maxs))
    }
    pub fn transform(&self, t: &Transform) -> BBox {
        let (a, b) = (self.mins, self.maxs);
        let corners = [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, b.y, b.z),
            Vec3::new(b.x, b.y, b.z),
        ];
        BBox::from_points(corners.iter().map(|c| t.point_to_world(*c))).unwrap()
    }
    /// Ray vs box (slab test). Returns distance along the ray.
    pub fn trace(&self, ray: &Ray) -> Option<f32> {
        let inv = Vec3::ONE / ray.direction;
        let t1 = (self.mins - ray.origin) * inv;
        let t2 = (self.maxs - ray.origin) * inv;
        let tmin = t1.min(t2).max_element();
        let tmax = t1.max(t2).min_element();
        (tmax >= tmin.max(0.0)).then_some(tmin.max(0.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self { origin, direction }
    }
    pub fn project(&self, d: f32) -> Vec3 {
        self.origin + self.direction * d
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
}

impl Plane {
    pub fn new(origin: Vec3, normal: Vec3) -> Self {
        let n = normal.normalize();
        Self { normal: n, distance: origin.dot(n) }
    }
    pub fn trace(&self, ray: &Ray) -> Option<Vec3> {
        let d = self.normal.dot(ray.direction);
        if d.abs() < 1e-6 {
            return None;
        }
        let t = (self.distance - self.normal.dot(ray.origin)) / d;
        (t >= 0.0).then(|| ray.project(t))
    }
}

/// Converts s&box world space (X fwd, Y left, Z up) to renderer view space
/// (right-handed, -Z forward, +Y up, +X right).
pub fn sbox_to_view() -> Mat4 {
    // view.x = -world.y, view.y = world.z, view.z = -world.x
    Mat4::from_cols(
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(-1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_90_turns_forward_to_left() {
        let f = Angles::new(0.0, 90.0, 0.0).to_rotation().forward();
        assert!((f - LEFT).length() < 1e-5, "{f:?}");
    }

    #[test]
    fn angles_roundtrip() {
        let a = Angles::new(20.0, 40.0, 10.0);
        let b = a.to_rotation().angles();
        assert!(
            (a.pitch - b.pitch).abs() < 1e-3 && (a.yaw - b.yaw).abs() < 1e-3 && (a.roll - b.roll).abs() < 1e-3,
            "{b:?}"
        );
    }

    #[test]
    fn transform_roundtrip() {
        let p = Transform::new(Vec3::new(1.0, 2.0, 3.0), Angles::new(10.0, 20.0, 30.0).to_rotation(), Vec3::splat(2.0));
        let c = Transform::new(Vec3::new(-4.0, 1.0, 0.5), Angles::new(0.0, 45.0, 0.0).to_rotation(), Vec3::ONE);
        let l = p.to_local(&p.to_world(&c));
        assert!((l.position - c.position).length() < 1e-4);
    }
}
