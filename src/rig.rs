//! Rigid slot/bone/IK semantics ported from the user's stream-kit desk-rig/skeleton.js.
use crate::activity::Frame;
use anyhow::{Context, Result, bail};
use glam::{Mat4, Vec2, Vec3};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
#[derive(Clone, Deserialize)]
struct Bone {
    parent: Option<String>,
    x: f32,
    y: f32,
    #[serde(default)]
    rotation: f32,
}
#[derive(Clone, Deserialize)]
struct Chain {
    shoulder: String,
    upper: String,
    forearm: String,
    wrist: String,
    target: [f32; 2],
    bounds: [f32; 4],
    bend: f32,
    #[serde(rename = "fingerCount", default = "default_fingers")]
    finger_count: usize,
}
fn default_fingers() -> usize {
    4
}
#[derive(Clone, Deserialize)]
struct Bind {
    x: f32,
    y: f32,
    a: f32,
}
#[derive(Clone, Deserialize)]
struct Slot {
    bone: String,
    z: f32,
    visible: bool,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    rotation: f32,
    #[serde(rename = "flipX", default)]
    flip_x: bool,
    #[serde(rename = "flipY", default)]
    flip_y: bool,
    #[serde(rename = "sourcePosition", default)]
    source: [f32; 2],
    bind: Option<Bind>,
}
#[derive(Clone, Default, Deserialize)]
pub struct Rig {
    bones: BTreeMap<String, Bone>,
    ik: BTreeMap<String, Chain>,
    slots: Vec<Slot>,
    #[serde(skip)]
    rest: Vec<Mat4>,
}
#[derive(Clone, Copy, Debug)]
struct World {
    position: Vec2,
    angle: f32,
}
impl World {
    fn matrix(self) -> Mat4 {
        Mat4::from_translation(self.position.extend(0.)) * Mat4::from_rotation_z(self.angle)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct IkResult {
    pub elbow: Vec2,
    pub wrist: Vec2,
    pub upper_angle: f32,
    pub forearm_angle: f32,
}
pub fn solve_ik(shoulder: Vec2, target: Vec2, upper: f32, forearm: f32, bend: f32) -> IkResult {
    let delta = target - shoulder;
    let reach = delta
        .length()
        .clamp((upper - forearm).abs() + 0.01, upper + forearm - 0.01);
    let angle = delta.y.atan2(delta.x);
    let wrist = shoulder + Vec2::from_angle(angle) * reach;
    let offset = ((upper * upper + reach * reach - forearm * forearm) / (2. * upper * reach))
        .clamp(-1., 1.)
        .acos();
    let u = angle + bend * offset;
    let elbow = shoulder + Vec2::from_angle(u) * upper;
    let lower = wrist - elbow;
    IkResult {
        elbow,
        wrist,
        upper_angle: u - std::f32::consts::FRAC_PI_2,
        forearm_angle: lower.y.atan2(lower.x) - std::f32::consts::FRAC_PI_2,
    }
}
fn world(
    id: &str,
    bones: &BTreeMap<String, Bone>,
    cache: &mut HashMap<String, World>,
    depth: usize,
) -> Result<World> {
    if depth > 128 {
        bail!("bone cycle")
    }
    if let Some(w) = cache.get(id) {
        return Ok(*w);
    }
    let b = bones
        .get(id)
        .with_context(|| format!("missing bone {id}"))?;
    let p = if let Some(parent) = &b.parent {
        world(parent, bones, cache, depth + 1)?
    } else {
        World {
            position: Vec2::ZERO,
            angle: 0.,
        }
    };
    let rotation = Vec2::from_angle(p.angle);
    let local = Vec2::new(
        b.x * rotation.x - b.y * rotation.y,
        b.x * rotation.y + b.y * rotation.x,
    );
    let w = World {
        position: p.position + local,
        angle: p.angle + b.rotation.to_radians(),
    };
    cache.insert(id.into(), w);
    Ok(w)
}
fn slot_matrix(slot: &Slot, w: World) -> Mat4 {
    let translation = Mat4::from_translation(Vec3::new(slot.x, slot.y, 0.));
    let rotation = Mat4::from_rotation_z(slot.rotation.to_radians());
    if let Some(bind) = &slot.bind {
        Mat4::from_translation(w.position.extend(0.))
            * Mat4::from_rotation_z(w.angle - bind.a)
            * Mat4::from_translation(Vec3::new(-bind.x, -bind.y, 0.))
            * Mat4::from_translation(Vec3::new(slot.source[0], slot.source[1], 0.))
            * translation
            * rotation
    } else {
        w.matrix()
            * translation
            * rotation
            * Mat4::from_scale(Vec3::new(
                if slot.flip_x { -1. } else { 1. },
                if slot.flip_y { -1. } else { 1. },
                1.,
            ))
    }
}
impl Rig {
    pub fn from_value(value: &serde_json::Value) -> Result<Self> {
        let mut rig: Self =
            serde_json::from_value(value.clone()).context("parse rigid skeleton")?;
        rig.slots.retain(|s| s.visible && s.z < 70.);
        rig.slots.sort_by(|a, b| a.z.total_cmp(&b.z));
        let mut cache = HashMap::new();
        for id in rig.bones.keys() {
            world(id, &rig.bones, &mut cache, 0)?;
        }
        for (side, chain) in &rig.ik {
            if side != "left" && side != "right" {
                bail!("unknown hand side {side}")
            }
            for id in [&chain.shoulder, &chain.upper, &chain.forearm, &chain.wrist] {
                if !rig.bones.contains_key(id) {
                    bail!("missing IK bone {id}")
                }
            }
            if rig.bones[&chain.forearm].y <= 0.02 || rig.bones[&chain.wrist].y <= 0.02 {
                bail!("IK lengths must be positive")
            }
            if chain.finger_count > if side == "right" { 2 } else { 4 } || chain.finger_count == 0 {
                bail!("unsupported finger count")
            }
            if chain.bounds[0] > chain.bounds[1] || chain.bounds[2] > chain.bounds[3] {
                bail!("invalid desk bounds")
            }
            for i in 0..chain.finger_count {
                if !rig.bones.contains_key(&format!("{side}_finger_{i}")) {
                    bail!("missing {side} finger {i}")
                }
            }
        }
        for slot in &rig.slots {
            let w = *cache.get(&slot.bone).context("slot bone missing")?;
            rig.rest.push(slot_matrix(slot, w));
        }
        Ok(rig)
    }
    pub fn pointer_bounds(&self) -> [f32; 4] {
        self.ik
            .get("right")
            .map_or([-150., 24., -14., 8.], |c| c.bounds)
    }
    fn posed_world(&self, frame: &Frame) -> HashMap<String, World> {
        let mut local = self.bones.clone();
        let mut cache = HashMap::new();
        // Approved neutral torso settling keeps wrist targets reachable without changing bone lengths.
        let mut settle = 0f32;
        for chain in self.ik.values() {
            let shoulder = world(&chain.shoulder, &local, &mut cache, 0).expect("validated rig");
            let length = local[&chain.forearm].y + local[&chain.wrist].y - 0.01;
            let dx = chain.target[0] - shoulder.position.x;
            let vertical = (length * length - dx * dx).max(0.).sqrt();
            settle = settle.max(chain.target[1] - vertical - shoulder.position.y);
        }
        if settle > 0. {
            if let Some(chest) = local.get_mut("chest") {
                chest.y += settle;
            }
            cache.clear();
        }
        for (side, chain) in &self.ik {
            let right = side == "right";
            let shoulder = world(&chain.shoulder, &local, &mut cache, 0).expect("validated rig");
            let pointer = if right { frame.pointer } else { [0.; 2] };
            let target = Vec2::from_array(chain.target)
                + Vec2::new(
                    pointer[0].clamp(chain.bounds[0], chain.bounds[1]),
                    pointer[1].clamp(chain.bounds[2], chain.bounds[3]),
                );
            let solved = solve_ik(
                shoulder.position,
                target,
                local[&chain.forearm].y,
                local[&chain.wrist].y,
                chain.bend,
            );
            debug_assert!(
                (solved.elbow.distance(shoulder.position) - local[&chain.forearm].y).abs() < 0.01
            );
            debug_assert!(
                (solved.wrist.distance(solved.elbow) - local[&chain.wrist].y).abs() < 0.01
            );
            local.get_mut(&chain.upper).unwrap().rotation =
                (solved.upper_angle - shoulder.angle).to_degrees();
            local.get_mut(&chain.forearm).unwrap().rotation =
                (solved.forearm_angle - solved.upper_angle).to_degrees();
            local.get_mut(&chain.wrist).unwrap().rotation = -solved.forearm_angle.to_degrees();
            let hand = &frame.hands[usize::from(right)];
            for i in 0..chain.finger_count {
                let age = frame.time - hand.taps[i];
                let pulse = if (0. ..0.22).contains(&age) {
                    (std::f64::consts::PI * age / 0.22).sin() as f32
                } else {
                    0.
                };
                let held = hand.down
                    && if right {
                        hand.buttons[i]
                    } else {
                        i == hand.serial.wrapping_sub(1) as usize % chain.finger_count
                    };
                let press = pulse.max(if held { 0.45 } else { 0. });
                let finger = local.get_mut(&format!("{side}_finger_{i}")).unwrap();
                finger.y += press * 2.;
                finger.rotation += press * if right { 6. } else { -6. };
                if let Some(tip) = local.get_mut(&format!("{side}_tip_{i}")) {
                    tip.y -= press * 5.;
                    tip.rotation -= press * 22.;
                }
            }
        }
        for id in local.keys() {
            world(id, &local, &mut cache, 0).expect("validated rig");
        }
        cache
    }
    pub fn deltas(&self, frame: &Frame) -> Vec<Mat4> {
        let posed = self.posed_world(frame);
        let art_to_world = Mat4::from_translation(Vec3::new(-6.04, 7., 0.))
            * Mat4::from_scale(Vec3::new(0.01, -0.01, 1.));
        let world_to_art = art_to_world.inverse();
        self.slots
            .iter()
            .zip(&self.rest)
            .map(|(slot, rest)| {
                art_to_world * slot_matrix(slot, posed[&slot.bone]) * rest.inverse() * world_to_art
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ik_preserves_lengths_at_and_beyond_bounds() {
        for target in [
            Vec2::new(20., 60.),
            Vec2::new(1000., 1000.),
            Vec2::ZERO,
            Vec2::new(-1000., -2.),
        ] {
            for bend in [-1., 1.] {
                let r = solve_ik(Vec2::ZERO, target, 100., 80., bend);
                assert!((r.elbow.length() - 100.).abs() < 0.001);
                assert!(((r.wrist - r.elbow).length() - 80.).abs() < 0.001);
                assert!(r.wrist.length() <= 180.);
            }
        }
    }
    #[test]
    #[ignore = "requires an explicitly supplied fixture from the user's own stream-kit rig"]
    fn external_stream_kit_skeleton_parity() {
        let path = std::env::var("KNIGHTCAST_RIG_REFERENCE").expect("set KNIGHTCAST_RIG_REFERENCE");
        let reference: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let rig = Rig::from_value(&reference["rig"]).unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let mut frame = Frame {
                time: case["time"].as_f64().unwrap(),
                pointer: serde_json::from_value(case["pointer"].clone()).unwrap(),
                ..Default::default()
            };
            for (i, side) in ["left", "right"].iter().enumerate() {
                let h = &case["hands"][side];
                frame.hands[i].down = h["down"].as_bool().unwrap();
                frame.hands[i].serial = h["serial"].as_u64().unwrap();
                frame.hands[i].taps = serde_json::from_value(h["taps"].clone()).unwrap();
                frame.hands[i].buttons = serde_json::from_value(h["buttons"].clone()).unwrap();
            }
            let actual = rig.posed_world(&frame);
            for (id, want) in case["world"].as_object().unwrap() {
                let got = actual[id];
                let want_pos = Vec2::new(
                    want["x"].as_f64().unwrap() as f32,
                    want["y"].as_f64().unwrap() as f32,
                );
                assert!(
                    got.position.distance(want_pos) < 0.002,
                    "{id}: actual {:?} expected {want_pos:?}",
                    got.position
                );
                assert!(
                    (got.angle - want["a"].as_f64().unwrap() as f32).abs() < 0.0001,
                    "{id} angle"
                );
            }
        }
    }
    #[test]
    fn bind_matrix_preserves_source_coordinates_at_bind_pose() {
        let slot:Slot=serde_json::from_value(serde_json::json!({"bone":"upper","z":1,"visible":true,"sourcePosition":[300,450],"bind":{"x":400,"y":600,"a":0.5}})).unwrap();
        let m = slot_matrix(
            &slot,
            World {
                position: Vec2::new(400., 600.),
                angle: 0.5,
            },
        );
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(300., 450., 0.)).length() < 0.001);
    }
}
