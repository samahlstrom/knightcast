//! Capture output must keep advancing when the native settings window is hidden.
//! This clock owns its pose filter, activity consumer, scene and output transport.
use crate::{
    activity,
    config::Config,
    output::{NativeOutput, OutputStats},
    renderer::Renderer,
    tracking::{Pose, Receiver, Snapshot},
};
use anyhow::{Context, Result};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct TrackingSource {
    origin: Instant,
    shared: Arc<Mutex<Snapshot>>,
}
impl TrackingSource {
    fn from_receiver(receiver: &Receiver) -> Self {
        Self {
            origin: receiver.origin,
            shared: receiver.shared.clone(),
        }
    }
    fn snapshot(&self) -> (Snapshot, f64) {
        let mut snapshot = self.shared.lock().unwrap();
        snapshot.consumed = snapshot.accepted;
        (snapshot.clone(), self.origin.elapsed().as_secs_f64())
    }
}
#[derive(Clone)]
struct Control {
    config: Config,
    tracking: Option<TrackingSource>,
}
impl Control {
    fn new(config: &Config, receiver: Option<&Receiver>) -> Self {
        Self {
            config: config.clone(),
            tracking: receiver.map(TrackingSource::from_receiver),
        }
    }
}
pub struct FrameWorker {
    control: Arc<Mutex<Control>>,
    stats: Arc<Mutex<OutputStats>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl FrameWorker {
    pub fn start(
        gpu: Renderer,
        mut output: NativeOutput,
        config: &Config,
        receiver: Option<&Receiver>,
        hub_port: Option<u16>,
    ) -> Result<Self> {
        config.validate().context("invalid output configuration")?;
        let control = Arc::new(Mutex::new(Control::new(config, receiver)));
        let stats = Arc::new(Mutex::new(output.stats()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared_control = control.clone();
        let shared_stats = stats.clone();
        let stopping = stop.clone();
        let worker=thread::Builder::new().name("knightcast-frames".into()).spawn(move|| {
            let mut pose=Pose::default();
            let mut activity=hub_port.map(|port|activity::Client::start(port,gpu.pointer_bounds()));
            let interval=Duration::from_secs_f64(1./60.);
            let mut last=Instant::now();let mut deadline=last;let mut report_at=last+Duration::from_secs(10);
            let mut frame_error=None;
            while !stopping.load(Ordering::Acquire) {
                let now=Instant::now();
                if now<deadline {thread::park_timeout(deadline-now);continue;}
                let dt=now.duration_since(last).as_secs_f32();last=now;
                deadline+=interval;
                if deadline<=now {deadline=now+interval;}
                // Copy under a short lock. Rendering and network work never hold the UI lock.
                let control=shared_control.lock().unwrap().clone();
                let mut config=control.config;config.background=[0.;4];
                let (snapshot,time)=control.tracking.as_ref().map(TrackingSource::snapshot).unwrap_or_default();
                pose.neutral=glam::Quat::from_array(config.neutral).normalize();
                pose.linked_eyes=config.linked_eyes;
                pose.blink_baseline=config.blink_baseline;
                pose.blink_smoothing_ms=config.blink_smoothing_ms;
                pose.mouth_gain=config.mouth_gain;
                pose.mouth_baseline=config.mouth_baseline;
                pose.update(&snapshot,time,dt,config.smoothing_ms/1000.,config.grace_ms/1000.,config.return_ms/1000.);
                let hands=activity.as_mut().map(|a|a.frame(dt)).unwrap_or_default();
                if output.wants_frame() {
                    gpu.render_with_activity(&config,&pose,&hands);
                    match output.offer(&gpu) {Ok(())=>frame_error=None,Err(e)=>frame_error=Some(format!("OBS frame: {e:#}"))}
                }
                let mut current_stats=output.stats();
                if frame_error.is_some(){current_stats.error=frame_error.clone();}
                if now>=report_at {
                    eprintln!("OBS worker connected={} frame_age_ms={:?} readbacks={} converted={} frames_written={} input_connected={} tracking={:?} error={:?}",current_stats.connected,current_stats.frame_age_ms,current_stats.readbacks,current_stats.converted,current_stats.frames_written,hands.health.connected,pose.state,current_stats.error);
                    report_at=now+Duration::from_secs(10);
                }
                *shared_stats.lock().unwrap()=current_stats;
            }
            // Drop transport and activity on this thread, outside control/stats locks.
            drop(output);
            drop(activity);
        }).context("start independent OBS frame worker")?;
        Ok(Self {
            control,
            stats,
            stop,
            worker: Some(worker),
        })
    }
    pub fn sync(&self, config: &Config, receiver: Option<&Receiver>) {
        if let Err(error) = config.validate() {
            self.stats.lock().unwrap().error = Some(format!("Output settings rejected: {error:#}"));
            return;
        }
        *self.control.lock().unwrap() = Control::new(config, receiver);
    }
    pub fn stats(&self) -> OutputStats {
        self.stats.lock().unwrap().clone()
    }
}
impl Drop for FrameWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::{Assets, Mesh, Part, Vertex},
        tracking::Packet,
    };
    use std::{
        net::UdpSocket,
        process::{Command, Stdio},
    };

    #[test]
    #[ignore = "requires native GPU, FFmpeg and isolated loopback sockets; no window or UI updates"]
    fn output_advances_and_tracks_packets_without_ui_updates() {
        let vertices = [[-1.5, 0.5, 1.], [0.5, 0.5, 1.], [-0.5, 3., 1.]]
            .map(|position| Vertex {
                position,
                uv: [0.; 2],
                color: [1.; 4],
            })
            .to_vec();
        let mesh = Mesh {
            vertices,
            indices: vec![0, 1, 2],
            texture: image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 40, 20, 255])),
            part: Part::Head,
            double_sided: true,
            center: glam::Vec3::ZERO,
            body_slot: None,
        };
        let gpu = pollster::block_on(Renderer::headless(
            Assets {
                meshes: vec![mesh],
                report: vec![],
                rig: crate::rig::Rig::default(),
            },
            32,
            32,
        ))
        .unwrap();
        let receiver = Receiver::bind(0).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let send = |packet: Packet| {
            sender
                .send_to(
                    &serde_json::to_vec(&packet).unwrap(),
                    (std::net::Ipv4Addr::LOCALHOST, receiver.port),
                )
                .unwrap()
        };
        send(Packet::default());
        let output = NativeOutput::new(32, 32, 60, 0).unwrap();
        let address = output.stats().address;
        let config = Config {
            smoothing_ms: 0.,
            grace_ms: 2000.,
            ..Default::default()
        };
        let worker = FrameWorker::start(gpu, output, &config, Some(&receiver), None).unwrap();
        let decoder = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "nut",
                "-i",
                &format!("tcp://{address}"),
                "-frames:v",
                "24",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let _ = tx.send(decoder.wait_with_output());
        });
        let started = Instant::now();
        while worker.stats().frames_written < 5 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "worker did not render without UI"
            );
            thread::sleep(Duration::from_millis(5));
        }
        send(Packet {
            seq: 1,
            rotation: glam::Quat::from_rotation_y(0.65).to_array(),
            ..Packet::default()
        });
        let decoded = rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        reader.join().unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout.len(), 32 * 32 * 4 * 24);
        let first = &decoded.stdout[..32 * 32 * 4];
        let last = &decoded.stdout[23 * 32 * 32 * 4..];
        assert_ne!(
            first, last,
            "new tracking must alter output with zero UI ticks"
        );
        assert!(
            first.as_chunks::<4>().0.iter().any(|p| p[3] == 255),
            "triangle visible"
        );
        assert_eq!(
            &first[..4],
            &[0, 0, 0, 0],
            "worker forces transparent background"
        );
        assert!(
            worker.stats().readbacks >= 10,
            "independent clock kept rendering"
        );
        let mut changed = config.clone();
        changed.zoom = 1.2;
        worker.sync(&changed, None);
        let stopped = Instant::now();
        drop(worker);
        assert!(stopped.elapsed() < Duration::from_secs(2));
    }
}
