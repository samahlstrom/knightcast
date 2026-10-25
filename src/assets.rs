use crate::rig::Rig;
use anyhow::{Context, Result, bail};
use glam::{Mat4, Vec3};
use serde_json::Value;
use std::{collections::HashMap, path::Path};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}
#[derive(Clone, Copy, PartialEq)]
pub enum Part {
    Body,
    Head,
    Jaw,
    EyeLeft,
    EyeRight,
    Neck,
    Interior,
    EyePop {
        stage: usize,
        side: usize,
    },
    Brow {
        side: usize,
    },
    ReactionEye {
        right: bool,
        expression: crate::activity::Expression,
    },
}
#[derive(Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texture: image::RgbaImage,
    pub part: Part,
    pub double_sided: bool,
    pub center: Vec3,
    pub body_slot: Option<usize>,
}
pub struct Assets {
    pub meshes: Vec<Mesh>,
    pub report: Vec<String>,
    pub rig: Rig,
}
fn num(v: &Value, k: &str) -> f32 {
    v[k].as_f64().unwrap_or(0.) as f32
}
fn pair(v: &Value) -> [f32; 2] {
    [
        v[0].as_f64().unwrap_or(0.) as f32,
        v[1].as_f64().unwrap_or(0.) as f32,
    ]
}
fn bone(id: &str, bones: &Value, cache: &mut HashMap<String, Mat4>, depth: usize) -> Result<Mat4> {
    if depth > 128 {
        bail!("bone cycle")
    };
    if let Some(m) = cache.get(id) {
        return Ok(*m);
    }
    let b = &bones[id];
    if b.is_null() {
        bail!("missing bone {id}")
    };
    let p = if let Some(parent) = b["parent"].as_str() {
        bone(parent, bones, cache, depth + 1)?
    } else {
        Mat4::IDENTITY
    };
    let m = p
        * Mat4::from_translation(Vec3::new(num(b, "x"), num(b, "y"), 0.))
        * Mat4::from_rotation_z(num(b, "rotation").to_radians());
    cache.insert(id.to_string(), m);
    Ok(m)
}
impl Assets {
    pub fn load(root: &Path) -> Result<Self> {
        let rig_dir = root.join("overlays/original-rig");
        let rig: Value = serde_json::from_slice(
            &std::fs::read(rig_dir.join("rig.json")).context("read original-rig/rig.json")?,
        )?;
        let mut out = Self {
            meshes: vec![],
            report: vec![],
            rig: Rig::from_value(&rig)?,
        };
        let mut slots = rig["slots"].as_array().context("rig slots")?.clone();
        slots.sort_by(|a, b| num(a, "z").total_cmp(&num(b, "z")));
        let mut bones = HashMap::new();
        for (order, s) in slots
            .iter()
            .filter(|s| s["visible"] == true && num(s, "z") < 70.)
            .enumerate()
        {
            let asset = &rig["assets"][s["asset"].as_str().context("slot asset")?];
            let mut texture =
                image::open(rig_dir.join(asset["file"].as_str().context("asset file")?))?
                    .to_rgba8();
            if let Some(r) = asset["sourceRect"].as_array() {
                texture = image::imageops::crop_imm(
                    &texture,
                    r[0].as_u64().context("crop x")? as u32,
                    r[1].as_u64().context("crop y")? as u32,
                    r[2].as_u64().context("crop width")? as u32,
                    r[3].as_u64().context("crop height")? as u32,
                )
                .to_image();
            }
            let b = bone(
                s["bone"].as_str().context("slot bone")?,
                &rig["bones"],
                &mut bones,
                0,
            )?;
            let trans = if s["bind"].is_object() {
                let bind = &s["bind"];
                let source = pair(&s["sourcePosition"]);
                // Fixed attachment proportions in bone space; never driven by motion.
                let fit = s["bindScale"]
                    .as_array()
                    .map(|_| pair(&s["bindScale"]))
                    .unwrap_or([1.; 2]);
                anyhow::ensure!(
                    fit.iter()
                        .all(|v| v.is_finite() && v.abs() >= 0.01 && v.abs() <= 10.),
                    "invalid attachment fit"
                );
                b * Mat4::from_scale(Vec3::new(fit[0], fit[1], 1.))
                    * Mat4::from_rotation_z(-num(bind, "a"))
                    * Mat4::from_translation(Vec3::new(-num(bind, "x"), -num(bind, "y"), 0.))
                    * Mat4::from_translation(Vec3::new(
                        source[0] + num(s, "x"),
                        source[1] + num(s, "y"),
                        0.,
                    ))
                    * Mat4::from_rotation_z(num(s, "rotation").to_radians())
            } else {
                b * Mat4::from_translation(Vec3::new(num(s, "x"), num(s, "y"), 0.))
                    * Mat4::from_rotation_z(num(s, "rotation").to_radians())
                    * Mat4::from_scale(Vec3::new(
                        if s["flipX"] == true { -1. } else { 1. },
                        if s["flipY"] == true { -1. } else { 1. },
                        1.,
                    ))
            };
            let pivot = pair(&s["pivot"]);
            let w = num(s, "width");
            let h = num(s, "height");
            // Optional convex cutout trims captured background scraps without
            // repainting or resampling the approved sprite pixels.
            let outline: Vec<[f32; 2]> = if let Some(points) = asset["convexOutline"].as_array() {
                anyhow::ensure!(
                    (3..=128).contains(&points.len()),
                    "invalid convex sprite outline"
                );
                let points: Vec<_> = points.iter().map(pair).collect();
                anyhow::ensure!(
                    points
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
                    "outline UV outside sprite"
                );
                points
            } else {
                vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            };
            let indices: Vec<u32> = (1..outline.len() - 1)
                .flat_map(|i| [0, i as u32, i as u32 + 1])
                .collect();
            let vertices = outline
                .into_iter()
                .map(|uv| {
                    let p = trans.transform_point3(Vec3::new(
                        (uv[0] - pivot[0]) * w,
                        (uv[1] - pivot[1]) * h,
                        0.,
                    ));
                    Vertex {
                        position: [
                            (p.x - 604.) / 100.,
                            (700. - p.y) / 100.,
                            order as f32 * 0.0001,
                        ],
                        uv,
                        color: [1.; 4],
                    }
                })
                .collect();
            out.meshes.push(Mesh {
                vertices,
                indices,
                texture,
                part: Part::Body,
                double_sided: true,
                center: Vec3::ZERO,
                body_slot: Some(order),
            });
        }
        let file = root.join("overlays/head-blender/samson-knight-head.glb");
        let (doc, buffers, images) =
            gltf::import(&file).with_context(|| format!("load {}", file.display()))?;
        for ext in doc.extensions_used() {
            if ext != "KHR_materials_unlit" {
                bail!("unsupported glTF extension {ext}")
            }
        }
        if doc.skins().next().is_some() || doc.animations().next().is_some() {
            bail!("animated/skinned glTF is not supported by this rigid-art renderer")
        }
        let scene = doc
            .default_scene()
            .or_else(|| doc.scenes().next())
            .context("GLB has no scene")?;
        for node in scene.nodes() {
            load_node(
                node,
                Mat4::IDENTITY,
                Part::Head,
                &buffers,
                &images,
                &mut out,
            )?;
        }
        let eye_centers: Vec<_> = out
            .meshes
            .iter()
            .filter_map(|m| match m.part {
                Part::EyeLeft => Some((m.center, false)),
                Part::EyeRight => Some((m.center, true)),
                _ => None,
            })
            .collect();
        for (center, right) in eye_centers {
            for expression in [
                crate::activity::Expression::Stars,
                crate::activity::Expression::Annoyed,
            ] {
                out.meshes.push(reaction_eye(center, right, expression));
            }
        }
        out.meshes.extend(eye_effect_meshes());
        out.report.push(format!("{} rigid GPU primitives; original body sprites; unlit base color, vertex color, sRGB textures, alpha and outlines",out.meshes.len()));
        Ok(out)
    }
}
fn load_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    mut part: Part,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    out: &mut Assets,
) -> Result<()> {
    let name = node.name().unwrap_or("");
    if name.contains("recessed black interior") {
        part = Part::Interior;
    } else if name == "Jaw" {
        part = Part::Jaw
    } else if name == "Eye left" {
        part = Part::EyeLeft
    } else if name == "Eye right" {
        part = Part::EyeRight
    } else if name.starts_with("Neck") && name != "NeckAnchor" {
        part = Part::Neck
    }
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let transform = parent * local;
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                bail!("{name}: only triangle glTF primitives supported")
            };
            let material = primitive.material();
            if !material.unlit() {
                bail!("{name}: material is not KHR_materials_unlit; lit PBR unsupported")
            };
            let pbr = material.pbr_metallic_roughness();
            let factor = pbr.base_color_factor();
            let texture = if let Some(t) = pbr.base_color_texture() {
                if t.tex_coord() != 0 {
                    bail!("unsupported texture coordinate set")
                };
                let i = &images[t.texture().source().index()];
                match i.format {
                    gltf::image::Format::R8G8B8A8 => {
                        image::RgbaImage::from_raw(i.width, i.height, i.pixels.clone())
                            .context("RGBA texture")?
                    }
                    gltf::image::Format::R8G8B8 => image::DynamicImage::ImageRgb8(
                        image::RgbImage::from_raw(i.width, i.height, i.pixels.clone())
                            .context("RGB texture")?,
                    )
                    .to_rgba8(),
                    _ => bail!("unsupported glTF texture format {:?}", i.format),
                }
            } else {
                image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]))
            };
            let reader = primitive.reader(|b| Some(&buffers[b.index()].0));
            let positions: Vec<_> = reader
                .read_positions()
                .context("missing positions")?
                .collect();
            let uvs: Vec<_> = reader
                .read_tex_coords(0)
                .map(|v| v.into_f32().collect())
                .unwrap_or_else(|| vec![[0.; 2]; positions.len()]);
            let colors: Vec<_> = reader
                .read_colors(0)
                .map(|v| v.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.; 4]; positions.len()]);
            let mut center = Vec3::ZERO;
            let vertices: Vec<_> = positions
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    let p = transform.transform_point3(Vec3::from_array(p));
                    center += p;
                    Vertex {
                        position: p.to_array(),
                        uv: uvs[i],
                        color: std::array::from_fn(|j| colors[i][j] * factor[j]),
                    }
                })
                .collect();
            center /= vertices.len().max(1) as f32;
            let indices = reader
                .read_indices()
                .map(|i| i.into_u32().collect())
                .unwrap_or_else(|| (0..vertices.len() as u32).collect());
            let mesh = Mesh {
                vertices,
                indices,
                texture,
                part,
                double_sided: material.double_sided(),
                center,
                body_slot: None,
            };
            // Preserve the user's existing fixed inner sleeve from live.mjs.
            // Only in-memory clone geometry changes; source artwork is never rewritten.
            if name.starts_with("Cylinder") {
                let mut sleeve = mesh.clone();
                sleeve.part = Part::Head;
                for vertex in &mut sleeve.vertices {
                    let [x, y, z] = vertex.position;
                    let side = x.atan2(z).sin().powi(2);
                    let bottom = 1.285 + 0.20 * side;
                    let top = 2.18 - 0.29 * side;
                    let weight = ((top - y) / (top - bottom)).clamp(0., 1.);
                    vertex.position = [x * 0.99, y - 0.54 * weight, z * 0.99];
                }
                out.meshes.push(sleeve);
            }
            out.meshes.push(mesh);
        }
    }
    for child in node.children() {
        load_node(child, transform, part, buffers, images, out)?;
    }
    Ok(())
}

// Same five-point star and 54×10px rounded bar as the user's drawEyes().
// Curved positions keep the native reaction painted onto the visor surface.
fn reaction_eye(center: Vec3, right: bool, expression: crate::activity::Expression) -> Mesh {
    let points: Vec<[f32; 2]> = if expression == crate::activity::Expression::Stars {
        (0..10)
            .map(|i| {
                let angle = std::f32::consts::FRAC_PI_2 - i as f32 * std::f32::consts::PI / 5.;
                let radius = if i % 2 == 0 { 0.36 } else { 0.15 };
                [radius * angle.cos(), radius * angle.sin()]
            })
            .collect()
    } else {
        (0..=12)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 12.;
                [0.22 + 0.05 * angle.cos(), -0.03 + 0.05 * angle.sin()]
            })
            .chain((0..=12).map(|i| {
                let angle = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 12.;
                [-0.22 + 0.05 * angle.cos(), -0.03 + 0.05 * angle.sin()]
            }))
            .collect()
    };
    let vertex = |xy: [f32; 2]| {
        let x = center.x + xy[0];
        let y = center.y + xy[1];
        Vertex {
            position: [x, y, (2.06f32.powi(2) - x * x).max(0.).sqrt()],
            uv: [0.; 2],
            color: [1.; 4],
        }
    };
    let mut vertices = vec![vertex(
        if expression == crate::activity::Expression::Annoyed {
            [0., -0.03]
        } else {
            [0.; 2]
        },
    )];
    vertices.extend(points.iter().map(|xy| vertex(*xy)));
    let mut indices = Vec::new();
    for i in 0..points.len() as u32 {
        indices.extend([0, i + 1, (i + 1) % points.len() as u32 + 1]);
    }
    Mesh {
        vertices,
        indices,
        texture: image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])),
        part: Part::ReactionEye { right, expression },
        double_sided: true,
        center,
        body_slot: None,
    }
}

// Ported from the user's own head-blender/eye-pop.mjs. Eye pops extend along
// helmet-local +Z only: head rotation is applied later to the whole assembly.
const POP_SIZES: [f32; 3] = [1., 1.55, 2.15];
const POP_DEPTHS: [f32; 3] = [0., 1.35, 2.85];
const BROW_RADIUS: f32 = 0.055;
pub fn eye_anchor(side: usize) -> Vec3 {
    let x = if side == 0 { -0.741 } else { 0.669 };
    Vec3::new(x, 1.685, (2.06f32.powi(2) - x * x).max(0.01).sqrt() + 0.025)
}
pub fn eye_pop_transform(stage: usize, side: usize, level: f32) -> Option<Mat4> {
    if stage >= 3 || side >= 2 || level <= 0.01 {
        return None;
    }
    let delay = stage as f32 * 0.08;
    let progress = ((level - delay) / (1. - delay)).clamp(0., 1.);
    if stage > 0 && progress <= 0.01 {
        return None;
    }
    let extension = progress + (progress * std::f32::consts::PI).sin() * 0.06;
    let size = 1. + (POP_SIZES[stage] - 1.) * progress;
    let position = eye_anchor(side) + Vec3::Z * (POP_DEPTHS[stage] * extension);
    Some(
        Mat4::from_translation(position)
            * Mat4::from_scale(Vec3::new(0.225 * size, 0.33 * size, 0.045)),
    )
}
pub fn brow_transform(side: usize, wide: f32) -> Option<(Mat4, f32)> {
    let level = ((wide - 0.1) / 0.45).clamp(0., 1.);
    if side >= 2 || level <= 0.01 {
        return None;
    }
    let anchor = eye_anchor(side);
    let position = anchor + Vec3::new(0., 0.33 + 0.29 + 0.12 * level, 0.02);
    let yaw = (anchor.x / 2.06).clamp(-1., 1.).asin();
    Some((
        Mat4::from_translation(position) * Mat4::from_rotation_y(yaw),
        level.sqrt(),
    ))
}
fn effect_mesh(vertices: Vec<Vertex>, indices: Vec<u32>, part: Part) -> Mesh {
    Mesh {
        vertices,
        indices,
        texture: image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])),
        part,
        double_sided: true,
        center: Vec3::ZERO,
        body_slot: None,
    }
}
fn eye_effect_meshes() -> Vec<Mesh> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=16 {
        let theta = std::f32::consts::PI * row as f32 / 16.;
        for column in 0..=24 {
            let phi = std::f32::consts::TAU * column as f32 / 24.;
            vertices.push(Vertex {
                position: [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ],
                uv: [0.; 2],
                color: [1.; 4],
            });
        }
    }
    for row in 0..16 {
        for column in 0..24 {
            let i = row * 25 + column;
            indices.extend([i, i + 1, i + 26, i, i + 26, i + 25]);
        }
    }
    let mut meshes = Vec::new();
    for stage in 0..3 {
        for side in 0..2 {
            meshes.push(effect_mesh(
                vertices.clone(),
                indices.clone(),
                Part::EyePop { stage, side },
            ));
        }
    }
    vertices.clear();
    indices.clear();
    // Quadratic control point height .16 gives a curve peak of .08. The tube
    // follows the original 20-segment/6-sided geometry; user requested thicker ink brows.
    let ink = [0., 0., 0., 1.];
    for segment in 0..=20 {
        let t = segment as f32 / 20.;
        let center = Vec3::new(-0.31 + 0.62 * t, 0.32 * t * (1. - t), 0.);
        let tangent = Vec3::new(0.62, 0.32 * (1. - 2. * t), 0.).normalize();
        let in_plane = Vec3::new(-tangent.y, tangent.x, 0.);
        for radial in 0..=6 {
            let angle = std::f32::consts::TAU * radial as f32 / 6.;
            let point = center + BROW_RADIUS * (angle.cos() * Vec3::Z + angle.sin() * in_plane);
            vertices.push(Vertex {
                position: point.to_array(),
                uv: [0.; 2],
                color: ink,
            });
        }
    }
    for segment in 0..20 {
        for radial in 0..6 {
            let i = segment * 7 + radial;
            indices.extend([i, i + 1, i + 8, i, i + 8, i + 7]);
        }
    }
    for side in 0..2 {
        meshes.push(effect_mesh(
            vertices.clone(),
            indices.clone(),
            Part::Brow { side },
        ));
    }
    meshes
}

#[cfg(test)]
mod eye_effect_tests {
    use super::*;
    #[test]
    fn exactly_three_white_pairs_with_original_first_dimensions() {
        let meshes = eye_effect_meshes();
        assert_eq!(
            meshes
                .iter()
                .filter(|m| matches!(m.part, Part::EyePop { .. }))
                .count(),
            6
        );
        assert_eq!(
            meshes
                .iter()
                .filter(|m| matches!(m.part, Part::Brow { .. }))
                .count(),
            2
        );
        for stage in 0..3 {
            for side in 0..2 {
                let transform = eye_pop_transform(stage, side, 1.).unwrap();
                let (scale, _, position) = transform.to_scale_rotation_translation();
                assert!(
                    (scale - Vec3::new(0.225 * POP_SIZES[stage], 0.33 * POP_SIZES[stage], 0.045))
                        .length()
                        < 0.00001
                );
                assert!(
                    (position - eye_anchor(side) - Vec3::Z * POP_DEPTHS[stage]).length() < 0.00001
                );
                let mesh = meshes
                    .iter()
                    .find(|m| m.part == Part::EyePop { stage, side })
                    .unwrap();
                assert!(mesh.vertices.iter().all(|v| v.color == [1.; 4]));
                let x_extent = mesh
                    .vertices
                    .iter()
                    .map(|v| {
                        transform.transform_point3(Vec3::from_array(v.position)).x - position.x
                    })
                    .fold(0f32, |a, b| a.max(b.abs()));
                assert!((x_extent - scale.x).abs() < 0.00001);
            }
        }
        assert!(eye_pop_transform(3, 0, 1.).is_none());
        assert!(eye_pop_transform(0, 0, 0.).is_none());
        assert!(eye_pop_transform(2, 0, 0.1).is_none());
    }
    #[test]
    fn stage_projection_follows_rotated_helmet_forward_axis() {
        for rotation in [
            glam::Quat::IDENTITY,
            glam::Quat::from_euler(glam::EulerRot::YXZ, 0.5, 0.2, -0.1),
        ] {
            let head = Mat4::from_rotation_translation(rotation, Vec3::new(0.3, 0.7, -0.2));
            for side in 0..2 {
                let first =
                    (head * eye_pop_transform(0, side, 1.).unwrap()).transform_point3(Vec3::ZERO);
                for (stage, depth) in POP_DEPTHS.iter().enumerate().skip(1) {
                    let projected = (head * eye_pop_transform(stage, side, 1.).unwrap())
                        .transform_point3(Vec3::ZERO);
                    let delta = projected - first;
                    assert!((delta - rotation * Vec3::Z * *depth).length() < 0.00001);
                    let local = rotation.inverse() * delta;
                    assert!(local.x.abs() < 0.00001 && local.y.abs() < 0.00001);
                    let camera = Mat4::orthographic_rh(-6.04, 6.50, -6., 7., 0.1, 100.)
                        * Mat4::look_at_rh(Vec3::new(0., 0., 20.), Vec3::ZERO, Vec3::Y);
                    let screen_delta =
                        camera.project_point3(projected) - camera.project_point3(first);
                    let forward = rotation * Vec3::Z;
                    assert!((screen_delta.x - forward.x * depth / 6.27).abs() < 0.00001);
                    assert!((screen_delta.y - forward.y * depth / 6.5).abs() < 0.00001);
                }
            }
        }
    }
    #[test]
    fn raised_brows_match_curve_thickness_ink_and_response() {
        assert!(brow_transform(0, 0.1).is_none());
        let (full, opacity) = brow_transform(0, 0.55).unwrap();
        assert_eq!(opacity, 1.);
        assert!(
            (full.transform_point3(Vec3::ZERO) - (eye_anchor(0) + Vec3::new(0., 0.74, 0.02)))
                .length()
                < 0.00001
        );
        let (_, half) = brow_transform(0, 0.325).unwrap();
        assert!((half - 0.5f32.sqrt()).abs() < 0.00001);
        let mesh = eye_effect_meshes()
            .into_iter()
            .find(|m| m.part == Part::Brow { side: 0 })
            .unwrap();
        assert_eq!(mesh.vertices.len(), 21 * 7);
        assert_eq!(mesh.indices.len(), 20 * 6 * 6);
        for (i, v) in mesh.vertices.iter().enumerate() {
            let t = (i / 7) as f32 / 20.;
            let curve = Vec3::new(-0.31 + 0.62 * t, 0.32 * t * (1. - t), 0.);
            assert!((Vec3::from_array(v.position).distance(curve) - 0.055).abs() < 0.00001);
            assert_eq!(v.color, [0., 0., 0., 1.]);
        }
    }
}
