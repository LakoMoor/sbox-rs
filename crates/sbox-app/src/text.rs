//! `TextRenderer` support: rasterises text with the game's own fonts into an RGBA texture shown on a quad.

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use sbox_render::{MaterialDesc, MeshData, Renderer, Vertex};
use sbox_scene::{GoId, MaterialId, MeshId, ModelRenderer, Scene};

pub struct TextStyle {
    pub text: String,
    pub color: [f32; 4],
    pub font: String,
    pub size: f32,
    pub weight: i64,
    pub italic: bool,
    pub line_height: f32,
    /// -1 left, 0 centre, 1 right.
    pub align: i32,
    /// World units per rasterised pixel.
    pub scale: f32,
}

fn weight_name(w: i64) -> &'static str {
    match w {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    }
}

/// Font file candidates, in order, for a family / weight / italic combination.
pub fn font_files(family: &str, weight: i64, italic: bool) -> Vec<String> {
    let w = weight_name(weight);
    let it = if italic { "Italic" } else { "" };
    let mut v = Vec::new();
    for fam in [family, "Poppins", "Inter"] {
        v.push(format!("fonts/{fam}-{w}{it}.ttf"));
        if w == "Regular" && italic {
            v.push(format!("fonts/{fam}-Italic.ttf"));
        }
        v.push(format!("fonts/{fam}-{w}.ttf"));
    }
    v
}

/// Rasterise to a tightly sized RGBA image (white text, coverage in alpha). Returns (pixels, width, height).
pub fn rasterise(font: &FontVec, st: &TextStyle) -> (Vec<u8>, u32, u32) {
    let px = st.size * 4.0;
    let scaled = font.as_scaled(PxScale::from(px));
    let lines: Vec<&str> = st.text.split('\n').collect();
    let line_h = (scaled.height() + scaled.line_gap()) * st.line_height.max(0.1);
    let widths: Vec<f32> = lines.iter().map(|l| l.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()).collect();
    let pad = 4.0;
    let w = (widths.iter().cloned().fold(1.0, f32::max) + pad * 2.0).ceil() as u32;
    let h = (line_h * lines.len() as f32 + pad * 2.0).ceil() as u32;
    let mut img = vec![0u8; (w * h * 4) as usize];
    for (li, line) in lines.iter().enumerate() {
        let x0 = pad
            + match st.align {
                -1 => 0.0,
                1 => widths.iter().cloned().fold(1.0, f32::max) - widths[li],
                _ => (widths.iter().cloned().fold(1.0, f32::max) - widths[li]) * 0.5,
            };
        let base = pad + li as f32 * line_h + scaled.ascent();
        let mut x = x0;
        for c in line.chars() {
            let id = font.glyph_id(c);
            let glyph = id.with_scale_and_position(PxScale::from(px), point(x, base));
            x += scaled.h_advance(id);
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    let (ix, iy) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if ix >= 0 && iy >= 0 && (ix as u32) < w && (iy as u32) < h {
                        let o = ((iy as u32 * w + ix as u32) * 4) as usize;
                        img[o..o + 3].fill(255);
                        img[o + 3] = img[o + 3].max((cov * 255.0) as u8);
                    }
                });
            }
        }
    }
    (img, w, h)
}

/// Create the quad + material for a text object and attach a `ModelRenderer` to `go`.
pub fn spawn_text(scene: &mut Scene, r: &mut Renderer, go: GoId, font: &FontVec, st: &TextStyle) {
    let (img, w, h) = rasterise(font, st);
    let tex = r.upload_rgba8(&img, w, h, true);
    let mat: MaterialId = r.create_material_ex(&MaterialDesc {
        color: Some(tex),
        tint: st.color,
        unlit: true,
        translucent: true,
        ..Default::default()
    });
    let (hw, hh) = (w as f32 * st.scale * 0.5, h as f32 * st.scale * 0.5);
    // Quad in the local YZ plane facing +X: text reads left-to-right for a viewer looking down -X.
    let v = |y: f32, z: f32, u: f32, vv: f32| Vertex { position: [0.0, y, z], normal: [1.0, 0.0, 0.0], uv: [u, vv], joints: [0; 4], weights: [0.0; 4] };
    let mesh: MeshId = r.upload_mesh(&MeshData {
        vertices: vec![v(hw, hh, 0.0, 0.0), v(-hw, hh, 1.0, 0.0), v(-hw, -hh, 1.0, 1.0), v(hw, -hh, 0.0, 1.0)],
        indices: vec![0, 1, 2, 0, 2, 3],
    });
    let mut m = ModelRenderer::new(mesh);
    m.material = mat;
    m.cast_shadows = false;
    scene.add_component(go, m);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterises_game_font_with_coverage() {
        let Ok(bytes) = std::fs::read("E:/sbox-ref/game/core/fonts/Poppins-ExtraBold.ttf") else { return };
        let font = FontVec::try_from_vec(bytes).unwrap();
        let st = TextStyle { text: "Hi\nthere".into(), color: [1.0; 4], font: "Poppins".into(), size: 20.0, weight: 800, italic: false, line_height: 1.0, align: 0, scale: 1.0 };
        let (img, w, h) = rasterise(&font, &st);
        assert_eq!(img.len(), (w * h * 4) as usize);
        let covered = img.chunks(4).filter(|p| p[3] > 128).count();
        assert!(covered > 200, "only {covered} covered pixels");
        assert!(h > 2 * 60, "two lines should be taller than {h}");
    }

    #[test]
    fn weight_maps_to_font_files() {
        assert_eq!(font_files("Poppins", 800, false)[0], "fonts/Poppins-ExtraBold.ttf");
        assert_eq!(font_files("Poppins", 400, true)[0], "fonts/Poppins-RegularItalic.ttf");
        assert!(font_files("Poppins", 400, true).contains(&"fonts/Poppins-Italic.ttf".to_string()));
    }
}
