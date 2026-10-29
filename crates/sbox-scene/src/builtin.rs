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
    /// Skinning matrices (model space, bind-relative). Empty = not skinned.
    pub bones: Vec<[[f32; 4]; 4]>,
}

impl ModelRenderer {
    pub fn new(mesh: MeshId) -> Self {
        Self { mesh, material: MaterialId(0), tint: [1.0; 4], cast_shadows: true, bones: Vec::new() }
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

/// Cone light shining along the object's forward (+X) axis; angles are half-angles in degrees.
pub struct SpotLight {
    pub color: [f32; 3],
    pub radius: f32,
    pub intensity: f32,
    pub cone_inner: f32,
    pub cone_outer: f32,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self { color: [1.0; 3], radius: 400.0, intensity: 1.0, cone_inner: 15.0, cone_outer: 45.0 }
    }
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

impl Component for SpotLight {
    impl_component_any!();
}

/// Opaque handle to a material owned by the renderer (0 = default white).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MaterialId(pub u32);

/// Screen-space bloom: bright areas (above `threshold`) are blurred and added back.
pub struct Bloom {
    pub strength: f32,
    pub threshold: f32,
    pub tint: [f32; 3],
}

impl Default for Bloom {
    fn default() -> Self {
        Self { strength: 1.0, threshold: 1.0, tint: [1.0; 3] }
    }
}

impl Component for Bloom {
    impl_component_any!();
}

/// Darkens the screen edges.
pub struct Vignette {
    pub intensity: f32,
    pub smoothness: f32,
    pub roundness: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { intensity: 1.0, smoothness: 1.0, roundness: 1.0 }
    }
}

impl Component for Vignette {
    impl_component_any!();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TonemapMode {
    #[default]
    Aces,
    AgX,
    Reinhard,
}

/// Selects the tone mapping curve and a fixed exposure (`exposure_compensation` in EV stops; auto exposure is not simulated).
pub struct Tonemapping {
    pub mode: TonemapMode,
    pub exposure_compensation: f32,
}

impl Default for Tonemapping {
    fn default() -> Self {
        Self { mode: TonemapMode::Aces, exposure_compensation: 0.0 }
    }
}

impl Component for Tonemapping {
    impl_component_any!();
}

/// Unsharp-mask style sharpening of the final image.
pub struct Sharpen {
    pub scale: f32,
}

impl Default for Sharpen {
    fn default() -> Self {
        Self { scale: 0.25 }
    }
}

impl Component for Sharpen {
    impl_component_any!();
}

/// Depth of field: pixels farther than `focal_distance` (behind) or nearer (in front) are blurred.
pub struct DepthOfField {
    pub focal_distance: f32,
    pub focus_range: f32,
    /// Maximum blur radius in pixels at a 1280x720 reference size.
    pub blur_size: f32,
    pub front_blur: bool,
    pub back_blur: bool,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self { focal_distance: 100.0, focus_range: 100.0, blur_size: 8.0, front_blur: false, back_blur: true }
    }
}

impl Component for DepthOfField {
    impl_component_any!();
}

/// Screen-space ambient occlusion computed from the depth buffer.
pub struct AmbientOcclusion {
    /// World-space sampling radius.
    pub radius: f32,
    pub intensity: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self { radius: 128.0, intensity: 1.0 }
    }
}

impl Component for AmbientOcclusion {
    impl_component_any!();
}

/// Splits the red and blue channels apart by `offset` pixels (x, y) in opposite directions.
pub struct ChromaticAberration {
    pub offset: [f32; 2],
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self { offset: [4.0, 6.0] }
    }
}

impl Component for ChromaticAberration {
    impl_component_any!();
}
