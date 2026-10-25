//! Versioned loopback protocol. All timestamps below are local monotonic durations.
use anyhow::{Result, bail};
use glam::Quat;
use serde::{Deserialize, Serialize};
use std::{
    net::UdpSocket,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Packet {
    pub version: u32,
    pub seq: u64,
    pub tracked: bool,
    pub rotation: [f32; 4],
    pub mouth_open: f32,
    pub blink_left: f32,
    pub blink_right: f32,
    pub eye_wide_left: f32,
    pub eye_wide_right: f32,
    pub smile: f32,
    pub gaze: [f32; 2],
    pub head_scale: f32,
    pub capture_age_ms: f32,
    pub inference_ms: f32,
}
impl Default for Packet {
    fn default() -> Self {
        Self {
            version: 1,
            seq: 0,
            tracked: true,
            rotation: [0., 0., 0., 1.],
            mouth_open: 0.,
            blink_left: 0.,
            blink_right: 0.,
            eye_wide_left: 0.,
            eye_wide_right: 0.,
            smile: 0.,
            gaze: [0., 0.],
            head_scale: 1.,
            capture_age_ms: 0.,
            inference_ms: 0.,
        }
    }
}
pub fn parse(bytes: &[u8]) -> Result<Packet> {
    if bytes.len() > 8192 {
        bail!("datagram exceeds 8192 bytes")
    }
    let mut p: Packet = serde_json::from_slice(bytes)?;
    if p.version != 1 || p.seq > 9_007_199_254_740_991 {
        bail!("unsupported version or sequence")
    }
    let q = Quat::from_array(p.rotation);
    let norm = q.length();
    if !q.is_finite() || !norm.is_finite() || norm < 0.000001 {
        bail!("invalid quaternion")
    }
    for x in [
        p.mouth_open,
        p.blink_left,
        p.blink_right,
        p.eye_wide_left,
        p.eye_wide_right,
        p.smile,
    ] {
        if !x.is_finite() || !(0. ..=1.).contains(&x) {
            bail!("expression outside [0,1]")
        }
    }
    for x in p.gaze {
        if !x.is_finite() || !(-1. ..=1.).contains(&x) {
            bail!("gaze outside [-1,1]")
        }
    }
    if !p.head_scale.is_finite() || !(0.1..=10.).contains(&p.head_scale) {
        bail!("head scale outside [0.1,10]")
    }
    for x in [p.capture_age_ms, p.inference_ms] {
        if !x.is_finite() || !(0. ..=60000.).contains(&x) {
            bail!("invalid timing duration")
        }
    }
    p.rotation = (q / norm).to_array();
    Ok(p)
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub latest: Option<(Packet, f64)>,
    pub valid: Option<(Packet, f64)>,
    pub accepted: u64,
    pub rejected: u64,
    pub superseded: u64,
    pub restarts: u64,
    pub consumed: u64,
    pub error: Option<String>,
}
impl Snapshot {
    pub fn publish(&mut self, bytes: &[u8], now: f64) {
        match parse(bytes) {
            Ok(p) => {
                if self.accepted > self.consumed {
                    self.superseded += 1
                }
                if self.latest.as_ref().is_some_and(|(old, _)| p.seq < old.seq) {
                    self.restarts += 1
                }
                self.accepted += 1;
                if p.tracked {
                    self.valid = Some((p.clone(), now))
                }
                self.latest = Some((p, now));
            }
            Err(_) => self.rejected += 1,
        }
    }
}
pub struct Receiver {
    pub shared: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    pub origin: Instant,
    pub port: u16,
}
impl Receiver {
    pub fn bind(port: u16) -> Result<Self> {
        let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
        let port = socket.local_addr()?.port();
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let origin = Instant::now();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let s = shared.clone();
        let t = stop.clone();
        let worker = thread::spawn(move || {
            let mut bytes = [0u8; 8193];
            while !t.load(Ordering::Relaxed) {
                match socket.recv_from(&mut bytes) {
                    Ok((n, peer)) => {
                        if peer.ip().is_loopback() {
                            s.lock()
                                .unwrap()
                                .publish(&bytes[..n], origin.elapsed().as_secs_f64());
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) => {
                        s.lock().unwrap().error = Some(e.to_string());
                        break;
                    }
                }
            }
        });
        Ok(Self {
            shared,
            stop,
            worker: Some(worker),
            origin,
            port,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.shared.lock().unwrap();
        s.consumed = s.accepted;
        s.clone()
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Tracking,
    Grace,
    Lost,
}
pub struct Pose {
    pub neutral: Quat,
    pub current: Quat,
    pub state: State,
    pub mouth: f32,
    pub mouth_baseline: f32,
    pub mouth_gain: f32,
    pub blink: [f32; 2],
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            neutral: Quat::IDENTITY,
            current: Quat::IDENTITY,
            state: State::Waiting,
            mouth: 0.,
            mouth_baseline: 0.,
            mouth_gain: 3.8,
            blink: [0.; 2],
        }
    }
}
pub fn smooth(a: Quat, mut b: Quat, dt: f32, tau: f32) -> Quat {
    if a.dot(b) < 0. {
        b = -b
    }
    a.slerp(
        b,
        if tau <= 0. {
            1.
        } else {
            1. - (-dt.max(0.) / tau).exp()
        },
    )
    .normalize()
}
impl Pose {
    pub fn calibrate(&mut self, s: &Snapshot, now: f64, grace: f32) -> bool {
        if let Some((p, t)) = &s.valid
            && now - t <= grace as f64
            && s.latest.as_ref().is_some_and(|(p, _)| p.tracked)
        {
            self.neutral = Quat::from_array(p.rotation);
            self.mouth_baseline = p.mouth_open;
            return true;
        }
        false
    }
    pub fn update(&mut self, s: &Snapshot, now: f64, dt: f32, tau: f32, grace: f32, ret: f32) {
        let Some((p, t)) = &s.valid else {
            self.state = State::Waiting;
            self.current = smooth(self.current, Quat::IDENTITY, dt, ret);
            self.mouth = 0.;
            self.blink = [0.; 2];
            return;
        };
        let age = (now - t).max(0.);
        let lost = age > grace as f64;
        self.state = if lost {
            State::Lost
        } else if s.latest.as_ref().is_some_and(|(p, _)| p.tracked) && age < 0.1 {
            State::Tracking
        } else {
            State::Grace
        };
        if lost {
            self.current = smooth(self.current, Quat::IDENTITY, dt, ret);
            self.mouth = 0.;
            self.blink = [0.; 2]
        } else if self.state == State::Tracking {
            let target = self.neutral.inverse() * Quat::from_array(p.rotation);
            self.current = smooth(self.current, target, dt, tau);
            self.mouth = mouth_response(p.mouth_open, self.mouth_baseline, self.mouth_gain);
            self.blink = [p.blink_left, p.blink_right]
        }
    }
}
pub fn mouth_response(raw: f32, baseline: f32, gain: f32) -> f32 {
    let x = (raw - baseline - 0.006).max(0.);
    (-(-1.5 * gain * x).exp_m1() / -(-1.5 * gain * 0.994).exp_m1()).clamp(0., 1.)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(p: Packet) -> Vec<u8> {
        serde_json::to_vec(&p).unwrap()
    }
    #[test]
    fn parser_rejection_preserves_valid() {
        let mut s = Snapshot::default();
        s.publish(&bytes(Packet::default()), 1.);
        for b in [
            vec![],
            vec![255],
            vec![0; 8193],
            b"{}".to_vec(),
            bytes(Packet {
                rotation: [0.; 4],
                ..Packet::default()
            }),
            bytes(Packet {
                mouth_open: 2.,
                ..Packet::default()
            }),
            bytes(Packet {
                version: 2,
                ..Packet::default()
            }),
        ] {
            s.publish(&b, 2.);
        }
        assert_eq!(s.rejected, 7);
        assert_eq!(s.valid.unwrap().1, 1.);
    }
    #[test]
    fn calibration_axes_and_combined() {
        for q in [
            Quat::from_rotation_x(0.2),
            Quat::from_rotation_y(0.3),
            Quat::from_rotation_z(-0.2),
            Quat::from_euler(glam::EulerRot::YXZ, 0.4, 0.3, -0.2),
        ] {
            let mut s = Snapshot::default();
            s.publish(
                &bytes(Packet {
                    rotation: q.to_array(),
                    ..Packet::default()
                }),
                1.,
            );
            let mut p = Pose::default();
            assert!(p.calibrate(&s, 1., 0.25));
            p.update(&s, 1., 0.1, 0., 0.25, 0.4);
            assert!(p.current.angle_between(Quat::IDENTITY) < 0.001);
            let next = q * Quat::from_rotation_y(0.1);
            s.publish(
                &bytes(Packet {
                    rotation: next.to_array(),
                    ..Packet::default()
                }),
                1.1,
            );
            p.update(&s, 1.1, 0.1, 0., 0.25, 0.4);
            assert!(p.current.angle_between(Quat::from_rotation_y(0.1)) < 0.001)
        }
    }
    #[test]
    fn smoothing_is_time_based_and_sign_safe() {
        let end = Quat::from_rotation_y(0.7);
        let expected = smooth(Quat::IDENTITY, end, 1., 0.3);
        for hz in [30, 60, 144] {
            let mut q = Quat::IDENTITY;
            for _ in 0..hz {
                q = smooth(q, end, 1. / hz as f32, 0.3)
            }
            assert!(q.angle_between(expected).to_degrees() < 0.5)
        }
        assert!(smooth(end, -end, 0.1, 0.2).angle_between(end) < 0.001)
    }
    #[test]
    fn loss_and_recovery() {
        let mut s = Snapshot::default();
        let mut p = Pose::default();
        s.publish(
            &bytes(Packet {
                rotation: Quat::from_rotation_y(0.5).to_array(),
                ..Packet::default()
            }),
            1.,
        );
        p.update(&s, 1., 0.1, 0., 0.25, 0.4);
        let held = p.current;
        s.publish(
            &bytes(Packet {
                tracked: false,
                ..Packet::default()
            }),
            1.1,
        );
        p.update(&s, 1.2, 0.1, 0.1, 0.25, 0.4);
        assert_eq!(p.state, State::Grace);
        assert!(p.current.angle_between(held) < 0.001);
        p.update(&s, 1.3, 0.1, 0.1, 0.25, 0.4);
        assert_eq!(p.state, State::Lost);
        assert!(p.current.angle_between(Quat::IDENTITY) < held.angle_between(Quat::IDENTITY));
        s.publish(
            &bytes(Packet {
                rotation: held.to_array(),
                ..Packet::default()
            }),
            1.4,
        );
        p.update(&s, 1.4, 0.01, 0.1, 0.25, 0.4);
        assert_eq!(p.state, State::Tracking);
        assert!(p.current.angle_between(held) > 0.001)
    }
    #[test]
    fn latest_slot_restart() {
        let mut s = Snapshot::default();
        for seq in 0..10000 {
            s.publish(
                &bytes(Packet {
                    seq,
                    ..Packet::default()
                }),
                seq as f64,
            )
        }
        assert_eq!(s.latest.as_ref().unwrap().0.seq, 9999);
        assert_eq!(s.superseded, 9999);
        s.publish(&bytes(Packet::default()), 10001.);
        assert_eq!(s.latest.unwrap().0.seq, 0);
        assert_eq!(s.restarts, 1)
    }
    #[test]
    fn grace_holds_displayed_pose_during_incomplete_smoothing() {
        let mut s = Snapshot::default();
        s.publish(
            &bytes(Packet {
                rotation: Quat::from_rotation_y(0.5).to_array(),
                ..Packet::default()
            }),
            1.,
        );
        let mut p = Pose::default();
        p.update(&s, 1., 0.01, 0.3, 0.25, 0.4);
        let held = p.current;
        s.publish(
            &bytes(Packet {
                tracked: false,
                ..Packet::default()
            }),
            1.01,
        );
        p.update(&s, 1.1, 0.1, 0.3, 0.25, 0.4);
        assert_eq!(p.state, State::Grace);
        assert!(p.current.angle_between(held) < 0.001);
        p.update(&Snapshot::default(), 1.2, 0.1, 0.3, 0.25, 0.4);
        assert_eq!(p.state, State::Waiting);
        assert_eq!(p.mouth, 0.);
        assert!(p.current.angle_between(Quat::IDENTITY) <= held.angle_between(Quat::IDENTITY));
    }
    #[test]
    fn mouth_deadzone_gain_and_bounds() {
        assert_eq!(mouth_response(0.1, 0.1, 3.8), 0.);
        assert_eq!(mouth_response(0.105, 0.1, 3.8), 0.);
        assert!(mouth_response(0.3, 0.1, 3.8) > mouth_response(0.3, 0.1, 1.));
        assert_eq!(mouth_response(1., 0., 3.8), 1.);
    }
    #[test]
    fn real_loopback_latest() {
        let r = Receiver::bind(0).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        for seq in 0..128 {
            sender
                .send_to(
                    &bytes(Packet {
                        seq,
                        ..Packet::default()
                    }),
                    (std::net::Ipv4Addr::LOCALHOST, r.port),
                )
                .unwrap();
        }
        let start = Instant::now();
        loop {
            let s = r.snapshot();
            if s.latest.as_ref().is_some_and(|(p, _)| p.seq == 127) {
                assert!(s.accepted > 0);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::yield_now();
        }
    }
}
