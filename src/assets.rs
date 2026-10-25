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
                b * Mat4::from_rotation_z(-num(bind, "a"))
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
            let vertices = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
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
                indices: vec![0, 1, 2, 0, 2, 3],
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
