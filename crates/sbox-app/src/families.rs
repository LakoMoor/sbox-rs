//! Shader families. Source 2 materials name a compiled `.shader_c` (GPU bytecode that cannot be translated), so each
//! family the shipped materials use is approximated by hand on top of the one PBR shader: a family adjusts the material
//! parameters read from the `.vmat`. Counts below are from the 912 materials of an s&box install.

use sbox_render::MaterialDesc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// `complex` (472), `generic` (110), `simple`, `foliage`, `fur`: the PBR path as is.
    Standard,
    /// `skin` (26): no subsurface scattering, so a dielectric with a softer highlight.
    Skin,
    /// `water_simple` (10): translucent, nearly mirror smooth.
    Water,
    /// `glass` (6, helmet visors and gas-mask lenses): the reference refracts and blurs what is behind it; here a smooth,
    /// mostly transparent dielectric.
    Glass,
    /// `blendable` (19): several texture layers blended by vertex data; only the first layer is drawn.
    Blendable,
    /// `tools_*` (sprites, wireframes, solids): editor helpers that the game does not draw.
    Tools,
}

impl Family {
    pub fn of(shader: &str) -> Family {
        let name = shader.rsplit('/').next().unwrap_or(shader);
        let name = name.strip_suffix("_c").unwrap_or(name);
        let name = name.strip_suffix(".shader").unwrap_or(name);
        match name {
            n if n.starts_with("tools_") => Family::Tools,
            "skin" => Family::Skin,
            "water_simple" | "water" => Family::Water,
            "glass" => Family::Glass,
            "blendable" => Family::Blendable,
            _ => Family::Standard,
        }
    }

    /// Adjust the parameters read from the material.
    pub fn apply(self, d: &mut MaterialDesc) {
        match self {
            Family::Standard | Family::Blendable => {}
            Family::Skin => {
                d.metalness = 0.0;
                d.roughness = d.roughness.max(0.55);
            }
            Family::Water => {
                d.translucent = true;
                d.roughness = 0.05;
                d.metalness = 0.0;
                d.tint = [d.tint[0] * 0.3, d.tint[1] * 0.55, d.tint[2] * 0.65, 0.6];
            }
            Family::Glass => {
                d.translucent = true;
                d.metalness = 0.0;
                d.roughness = d.roughness.min(0.15);
                d.tint[3] = d.tint[3].min(0.35);
            }
            Family::Tools => {
                d.translucent = true;
                d.tint[3] = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_names_map_to_families() {
        assert_eq!(Family::of("shaders/complex.shader"), Family::Standard);
        assert_eq!(Family::of("shaders/complex.shader_c"), Family::Standard);
        assert_eq!(Family::of("shaders/generic.shader"), Family::Standard);
        assert_eq!(Family::of("shaders/skin.shader"), Family::Skin);
        assert_eq!(Family::of("shaders/water_simple.shader"), Family::Water);
        assert_eq!(Family::of("shaders/blendable.shader_c"), Family::Blendable);
        assert_eq!(Family::of("shaders/glass.shader"), Family::Glass);
        assert_eq!(Family::of("shaders/tools_sprite.shader"), Family::Tools);
        assert_eq!(Family::of("shaders/tools_wireframe.shader"), Family::Tools);
        assert_eq!(Family::of(""), Family::Standard);
    }

    #[test]
    fn families_adjust_the_material() {
        let base = MaterialDesc { tint: [1.0; 4], roughness: 0.2, metalness: 1.0, ..Default::default() };

        let mut glass = base.clone();
        Family::Glass.apply(&mut glass);
        assert!(glass.translucent && glass.metalness == 0.0 && glass.roughness == 0.15 && glass.tint[3] == 0.35);

        let mut skin = base.clone();
        Family::Skin.apply(&mut skin);
        assert_eq!((skin.metalness, skin.roughness), (0.0, 0.55));

        let mut water = base.clone();
        Family::Water.apply(&mut water);
        assert!(water.translucent && water.tint[3] < 1.0 && water.roughness < 0.1);

        let mut tools = base.clone();
        Family::Tools.apply(&mut tools);
        assert!(tools.translucent && tools.tint[3] == 0.0, "invisible");

        let mut std = base.clone();
        Family::Standard.apply(&mut std);
        assert_eq!((std.metalness, std.roughness, std.tint), (1.0, 0.2, [1.0; 4]));
    }
}
