//! Renderer-facing built-in components (CameraComponent, ModelRenderer, lights).
//! Mirrors Sandbox.Engine/Scene/Components/{Camera,Light,Render}.

use crate::{impl_component_any, Component};

/// Opaque handle to a mesh owned by the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshId(pub u32);

/// `CameraComponent`
pub struct CameraComponent {
    /// Vertical field of view in degrees (s&box exposes horizontal FOV; see `fov_y`).
    pub field_of_view: f32,
    pub z_near: f32,
    pub z_far: f32,
    pub background: [f32; 4],
    pub priority: i32,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self { field_of_view: 60.0, z_near: 10.0, z_far: 10000.0, background: [0.1, 0.12, 0.16, 1.0], priority: 0 }
    }
}

impl CameraComponent {
    /// Converts the horizontal FOV (s&box convention) to vertical for a given aspect ratio.
    pub fn fov_y(&self, aspect: f32) -> f32 {
        let h = (self.field_of_view.to_radians() * 0.5).tan();
        2.0 * (h / aspect).atan()
    }
}

impl Component for CameraComponent {
    impl_component_any!();
}

/// `ModelRenderer`, reduced to a mesh + tint until model/material loading lands.
pub struct ModelRenderer {
    pub mesh: MeshId,
    pub material: MaterialId,
    pub tint: [f32; 4],
    pub cast_shadows: bool,
}

impl ModelRenderer {
    pub fn new(mesh: MeshId) -> Self {
        Self { mesh, material: MaterialId(0), tint: [1.0; 4], cast_shadows: true }
    }
}

impl Component for ModelRenderer {
    impl_component_any!();
}

/// `DirectionalLight` — direction is the owner's forward vector.
pub struct DirectionalLight {
    pub color: [f32; 3],
    pub ambient: [f32; 3],
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self { color: [1.0, 0.96, 0.9], ambient: [0.15, 0.17, 0.2] }
    }
}

impl Component for DirectionalLight {
    impl_component_any!();
}

/// `PointLight`
pub struct PointLight {
    pub color: [f32; 3],
    pub radius: f32,
    pub intensity: f32,
}

impl Default for PointLight {
    fn default() -> Self {
        Self { color: [1.0; 3], radius: 400.0, intensity: 1.0 }
    }
}

impl Component for PointLight {
    impl_component_any!();
}

/// Opaque handle to a material owned by the renderer (0 = default white).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MaterialId(pub u32);
