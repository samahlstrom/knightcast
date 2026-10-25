#![recursion_limit = "256"]
mod activity;
mod assets;
mod config;
mod expressions;
mod frame_worker;
mod output;
#[cfg(test)]
mod preview_tests;
mod renderer;
mod rig;
mod tracking;
use anyhow::{Context, Result};
use clap::Parser;
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Parser)]
#[command(
    version,
    about = "Native local-only GPU knight avatar; no camera starts automatically"
)]
struct Args {
    #[arg(long)]
    asset_root: Option<PathBuf>,
    /// Connect to stream-kit's existing local input/event hub.
    #[arg(long)]
    hub_port: Option<u16>,
    /// Serve transparent 600x660 RGBA video to an OBS Media Source.
    #[arg(long)]
    obs_port: Option<u16>,
    /// Render a PNG without opening a window.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Offscreen run duration; UDP receiver remains active. Requires --screenshot.
    #[arg(long, default_value_t = 0.)]
    seconds: f64,
    #[arg(long, default_value_t = 60)]
    fps: u32,
    #[arg(long, default_value_t = 1254)]
    width: u32,
    #[arg(long, default_value_t = 1300)]
    height: u32,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    transparent: bool,
    /// Synthetic initial pose for offscreen artwork inspection, degrees.
    #[arg(long, default_value_t = 0., allow_hyphen_values = true)]
    yaw: f32,
    #[arg(long, default_value_t = 0., allow_hyphen_values = true)]
    pitch: f32,
    #[arg(long, default_value_t = 0., allow_hyphen_values = true)]
    roll: f32,
    #[arg(long, default_value_t = 0.)]
    mouth: f32,
}
fn main() -> Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        (1..=4096).contains(&args.width) && (1..=4096).contains(&args.height),
        "dimensions must be 1..4096"
    );
    anyhow::ensure!(
        (1..=240).contains(&args.fps) && args.seconds.is_finite() && args.seconds >= 0.,
        "invalid FPS/duration"
    );
    let path = args.config.clone().unwrap_or_else(config::path);
    let (mut config, mut errors) = match config::Config::load(&path) {
        Ok(c) => (c, vec![]),
        Err(_) if !path.exists() => (config::Config::default(), vec![]),
        Err(e) => (
            config::Config::default(),
            vec![format!("Config: {e:#}; using defaults")],
        ),
    };
    if let Some(root) = args.asset_root {
        config.asset_root = root.display().to_string()
    }
    if let Some(port) = args.port {
        config.port = port
    }
    if args.transparent {
        config.background[3] = 0.
    }
    config.validate()?;
    if let Some(output) = args.screenshot {
        anyhow::ensure!(
            !config.asset_root.is_empty(),
            "provide --asset-root /path/to/stream-kit"
        );
        let assets = assets::Assets::load(std::path::Path::new(&config.asset_root))?;
        let renderer = pollster::block_on(renderer::Renderer::headless(
            assets,
            args.width,
            args.height,
        ))?;
        eprintln!("GPU: {}", renderer.backend);
        for line in &renderer.report {
            eprintln!("{line}")
        }
        let receiver = tracking::Receiver::bind(config.port)
            .with_context(|| format!("bind loopback UDP port {}", config.port))?;
        let mut pose = tracking::Pose {
            linked_eyes: config.linked_eyes,
            blink_baseline: config.blink_baseline,
            wide_baseline: config.wide_baseline,
            blink_smoothing_ms: config.blink_smoothing_ms,
            mouth_gain: config.mouth_gain,
            mouth_baseline: config.mouth_baseline,
            ..Default::default()
        };
        eprintln!("Loopback UDP port {}", receiver.port);
        pose.neutral = glam::Quat::from_array(config.neutral).normalize();
        pose.current = glam::Quat::from_euler(
            glam::EulerRot::YXZ,
            args.yaw.to_radians(),
            args.pitch.to_radians(),
            args.roll.to_radians(),
        );
        pose.mouth = args.mouth.clamp(0., 1.);
        let mut activity = args
            .hub_port
            .map(|port| activity::Client::start(port, renderer.pointer_bounds()));
        let mut native_output = args
            .obs_port
            .map(|port| output::NativeOutput::new(args.width, args.height, 60, port))
            .transpose()?;
        let start = Instant::now();
        let mut previous = start;
        let mut frames = 0u64;
        let mut report_at = 0.;
        loop {
            let now = Instant::now();
            let dt = now.duration_since(previous).as_secs_f32();
            previous = now;
            let snapshot = receiver.snapshot();
            if snapshot.valid.is_some() {
                pose.update(
                    &snapshot,
                    receiver.origin.elapsed().as_secs_f64(),
                    dt,
                    config.smoothing_ms / 1000.,
                    config.grace_ms / 1000.,
                    config.return_ms / 1000.,
                );
            }
            let hands = activity.as_mut().map(|a| a.frame(dt)).unwrap_or_default();
            renderer.render_with_activity(&config, &pose, &hands);
            if let Some(output) = &mut native_output
                && output.wants_frame()
            {
                output.offer(&renderer)?;
            }
            frames += 1;
            if args.seconds == 0. || start.elapsed().as_secs_f64() >= args.seconds {
                break;
            }
            if start.elapsed().as_secs_f64() >= report_at {
                let age = snapshot
                    .valid
                    .as_ref()
                    .map(|(_, t)| (receiver.origin.elapsed().as_secs_f64() - t) * 1000.);
                eprintln!(
                    "runtime {:.1}s frames {} fps {:.1} state {:?} accepted {} rejected {} superseded {} packet_age_ms {:?}",
                    start.elapsed().as_secs_f64(),
                    frames,
                    frames as f64 / start.elapsed().as_secs_f64(),
                    pose.state,
                    snapshot.accepted,
                    snapshot.rejected,
                    snapshot.superseded,
                    age
                );
                report_at += 10.;
            }
            let deadline = start + Duration::from_secs_f64(frames as f64 / args.fps as f64);
            if deadline > Instant::now() {
                std::thread::sleep(deadline - Instant::now());
            }
        }
        renderer.screenshot(&output)?;
        eprintln!(
            "Saved {} after {} frames / {:.3}s",
            output.display(),
            frames,
            start.elapsed().as_secs_f64()
        );
        return Ok(());
    }
    let receiver = match tracking::Receiver::bind(config.port) {
        Ok(r) => Some(r),
        Err(e) => {
            errors.push(format!("UDP {}: {e}", config.port));
            None
        }
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("KnightCast — native avatar")
            .with_inner_size([1120., 900.]),
        ..Default::default()
    };
    eframe::run_native(
        "KnightCast",
        options,
        Box::new(move |cc| {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .context("wgpu unavailable")?
                .clone();
            let mut app = App {
                config,
                path,
                receiver,
                pose: tracking::Pose::default(),
                gpu: None,
                texture: None,
                state,
                errors,
                last: Instant::now(),
                fps: 0.,
                packet_rate: 0.,
                rate_last: Instant::now(),
                accepted_last: 0,
                cap: args.fps,
                output_only: false,
                notice: String::new(),
                calibration: None,
                hub_port: args.hub_port,
                activity: None,
                obs_port: args.obs_port,
                obs_worker: None,
            };
            app.pose.neutral = glam::Quat::from_array(app.config.neutral).normalize();
            app.reload();
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("native window: {e}"))
}
struct App {
    config: config::Config,
    path: PathBuf,
    receiver: Option<tracking::Receiver>,
    pose: tracking::Pose,
    gpu: Option<renderer::Renderer>,
    texture: Option<egui::TextureId>,
    state: eframe::egui_wgpu::RenderState,
    errors: Vec<String>,
    last: Instant,
    fps: f32,
    packet_rate: f64,
    rate_last: Instant,
    accepted_last: u64,
    cap: u32,
    output_only: bool,
    notice: String,
    calibration: Option<Calibration>,
    hub_port: Option<u16>,
    activity: Option<activity::Client>,
    obs_port: Option<u16>,
    obs_worker: Option<frame_worker::FrameWorker>,
}
struct Calibration {
    started: Instant,
    last_sample: Option<f64>,
    samples: Vec<tracking::Packet>,
}
impl App {
    fn start_calibration(&mut self) {
        self.calibration = Some(Calibration {
            started: Instant::now(),
            last_sample: None,
            samples: vec![],
        });
        self.notice = "Look normally at your screen, eyes comfortably open and lips together for two seconds.".into();
        self.errors.clear();
    }

    fn update_calibration(&mut self, s: &tracking::Snapshot, time: f64) {
        let Some(calibration) = &mut self.calibration else {
            return;
        };
        if let Some((packet, received)) = &s.valid
            && time - received <= self.config.grace_ms as f64 / 1000.
            && calibration.last_sample != Some(*received)
            && s.latest.as_ref().is_some_and(|(p, _)| p.tracked)
        {
            calibration.samples.push(packet.clone());
            calibration.last_sample = Some(*received);
        }
        if calibration.started.elapsed().as_secs_f32() >= 2. {
            let calibration = self.calibration.take().unwrap();
            if self.pose.calibrate_samples(&calibration.samples) {
                self.config.neutral = self.pose.neutral.to_array();
                self.config.mouth_baseline = self.pose.mouth_baseline;
                self.config.blink_baseline = self.pose.blink_baseline;
                self.config.wide_baseline = self.pose.wide_baseline;
                self.notice =
                    "Face calibrated. Resting eyes, mouth and head position updated.".into();
            } else {
                self.notice =
                    "Not enough face samples. Face the camera and try Calibrate face again.".into();
            }
        }
    }

    fn reconnect(&mut self) {
        self.receiver = None;
        self.accepted_last = 0;
        self.packet_rate = 0.;
        self.rate_last = Instant::now();
        match tracking::Receiver::bind(self.config.port) {
            Ok(r) => {
                self.receiver = Some(r);
                self.notice = "Tracking connection reopened. Waiting for camera data.".into();
            }
            Err(e) => self.errors.push(format!("Tracking connection: {e}")),
        }
    }

    fn reload(&mut self) {
        match assets::Assets::load(std::path::Path::new(&self.config.asset_root)) {
            Ok(assets) => {
                let info = self.state.adapter.get_info();
                let gpu = renderer::Renderer::new(
                    self.state.device.clone(),
                    self.state.queue.clone(),
                    assets,
                    1254,
                    1300,
                    format!("{} / {:?}", info.name, info.backend),
                );
                let mut renderer = self.state.renderer.write();
                if let Some(id) = self.texture.take() {
                    renderer.free_texture(&id);
                }
                self.texture = Some(renderer.register_native_texture(
                    &gpu.device,
                    &gpu.preview_view(),
                    wgpu::FilterMode::Linear,
                ));
                if let Some(port) = self.hub_port {
                    self.activity = Some(activity::Client::start(port, gpu.pointer_bounds()));
                }
                if let Some(port) = self.obs_port {
                    match assets::Assets::load(std::path::Path::new(&self.config.asset_root)) {
                        Ok(assets) => {
                            let output_gpu = renderer::Renderer::new(
                                self.state.device.clone(),
                                self.state.queue.clone(),
                                assets,
                                600,
                                660,
                                gpu.backend.clone(),
                            );
                            self.obs_worker = None;
                            let worker =
                                output::NativeOutput::new(600, 660, 60, port).and_then(|output| {
                                    frame_worker::FrameWorker::start(
                                        output_gpu,
                                        output,
                                        &self.config,
                                        self.receiver.as_ref(),
                                        self.hub_port,
                                    )
                                });
                            match worker {
                                Ok(worker) => self.obs_worker = Some(worker),
                                Err(e) => self.errors.push(format!("OBS output: {e:#}")),
                            }
                        }
                        Err(e) => self.errors.push(format!("OBS assets: {e:#}")),
                    }
                }
                self.gpu = Some(gpu);
            }
            Err(e) => self.errors.push(format!("Assets: {e:#}")),
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        self.fps = self.fps * 0.9 + 0.1 / dt.max(0.00001);
        let s = self
            .receiver
            .as_ref()
            .map(|r| r.snapshot())
            .unwrap_or_default();
        let time = self
            .receiver
            .as_ref()
            .map(|r| r.origin.elapsed().as_secs_f64())
            .unwrap_or(0.);
        self.update_calibration(&s, time);
        self.pose.linked_eyes = self.config.linked_eyes;
        self.pose.blink_baseline = self.config.blink_baseline;
        self.pose.wide_baseline = self.config.wide_baseline;
        self.pose.blink_smoothing_ms = self.config.blink_smoothing_ms;
        self.pose.mouth_gain = self.config.mouth_gain;
        self.pose.mouth_baseline = self.config.mouth_baseline;
        self.pose.update(
            &s,
            time,
            dt,
            self.config.smoothing_ms / 1000.,
            self.config.grace_ms / 1000.,
            self.config.return_ms / 1000.,
        );
        if self.rate_last.elapsed().as_secs_f64() >= 1. {
            self.packet_rate = (s.accepted.saturating_sub(self.accepted_last)) as f64
                / self.rate_last.elapsed().as_secs_f64();
            self.rate_last = now;
            self.accepted_last = s.accepted;
        }
        let hands = self
            .activity
            .as_mut()
            .map(|a| a.frame(dt))
            .unwrap_or_default();
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.output_only = !self.output_only
        }
        if !self.output_only {
            egui::SidePanel::left("controls").default_width(330.).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(if self.hub_port.is_some() { "Stream-kit · KnightCast" } else { "KnightCast" });
                    if self.hub_port.is_some() {
                        ui.label(if hands.health.connected { "Stream-kit input connected" } else { "Connecting to stream-kit input…" });
                        ui.small(format!("Mouse / keyboard: {}", hands.health.hands_status));
                    }
                    if let Some(output) = &self.obs_worker {
                        let stats = output.stats();
                        ui.label(if stats.connected { "OBS video connected" } else { "OBS video ready · waiting for OBS" });
                        if let Some(error) = stats.error { ui.small(error); }
                    }
                    let status = match self.pose.state {
                        tracking::State::Tracking => "Face tracking active",
                        tracking::State::Grace => "Reacquiring face…",
                        tracking::State::Lost => "Face not found",
                        tracking::State::Waiting => "Waiting for camera tracking",
                    };
                    ui.strong(status);
                    ui.label("F1 hides or shows these controls.");
                    ui.separator();
                    if ui.add_enabled(self.calibration.is_none(), egui::Button::new("Calibrate face (2 seconds)").min_size(egui::vec2(0.,32.))).clicked() {
                        self.start_calibration();
                    }
                    ui.small("Sets your normal head position and resting eyes/mouth.");
                    if let Some(c) = &self.calibration {
                        ui.add(egui::ProgressBar::new((c.started.elapsed().as_secs_f32()/2.).min(1.)).text("Look normally at your screen"));
                    }
                    if !self.notice.is_empty() { ui.label(&self.notice); }
                    ui.separator();
                    ui.strong("Eyes and movement");
                    ui.small("Wide eyes raise black brows. Wide eyes + open mouth trigger the three-stage eye pop.");
                    ui.checkbox(&mut self.config.linked_eyes, "Keep both eyes in sync")
                        .on_hover_text("Recommended for a camera off to one side. Prevents the farther eye staying smaller; both eyes blink together. Turn off for independent winks.");
                    ui.add(egui::Slider::new(&mut self.config.blink_smoothing_ms, 0. ..=100.).text("Eye smoothing (ms)"));
                    ui.add(egui::Slider::new(&mut self.config.smoothing_ms, 0. ..=150.).text("Head smoothing (ms)"));
                    ui.add(egui::Slider::new(&mut self.config.mouth_gain,0.1..=10.).text("Mouth sensitivity"));
                    ui.separator();
                    ui.strong("Avatar size and position");
                    ui.add(egui::Slider::new(&mut self.config.zoom,0.5..=2.).text("Overall zoom"));
                    ui.add(egui::Slider::new(&mut self.config.model_scale,0.5..=1.5).text("Helmet size"));
                    ui.add(egui::Slider::new(&mut self.config.body_scale,0.5..=1.5).text("Body size"));
                    if ui.button("Reset avatar size and position").clicked() {
                        self.config.reset_framing();
                        self.notice = "Original avatar framing restored. Face calibration and tracking settings kept.".into();
                    }
                    ui.collapsing("Advanced positioning", |ui| {
                        for (i,label) in ["Offset X ","Offset Y "].iter().enumerate() { ui.add(egui::DragValue::new(&mut self.config.offset[i]).speed(0.01).prefix(*label)); }
                        for (i,label) in ["Base yaw ","Base pitch ","Base roll "].iter().enumerate() { ui.add(egui::DragValue::new(&mut self.config.base_degrees[i]).speed(0.2).prefix(*label)); }
                        for (i,label) in ["Pivot X ","Pivot Y ","Pivot Z "].iter().enumerate() { ui.add(egui::DragValue::new(&mut self.config.pivot[i]).speed(0.01).prefix(*label)); }
                        ui.label("Background color / transparency");
                        ui.color_edit_button_rgba_unmultiplied(&mut self.config.background);
                    });
                    ui.separator();
                    ui.strong("Settings");
                    ui.horizontal(|ui| {
                        if ui.button("Save settings").clicked() {
                            match self.config.save(&self.path) {
                                Ok(()) => self.notice = "Settings saved.".into(),
                                Err(e) => self.errors.push(format!("Save: {e:#}")),
                            }
                        }
                        if ui.button("Reload saved settings").clicked() {
                            match config::Config::load(&self.path) {
                                Ok(config) => {
                                    self.config = config;
                                    self.pose = tracking::Pose { neutral: glam::Quat::from_array(self.config.neutral).normalize(), ..Default::default() };
                                    self.calibration = None;
                                    self.reconnect(); self.reload();
                                    self.notice = "Saved settings restored.".into();
                                }
                                Err(e) => self.errors.push(format!("Reload settings: {e:#}")),
                            }
                        }
                    });
                    if ui.button("Reset all settings to defaults").clicked() {
                        self.config.reset_all();
                        self.pose = tracking::Pose::default();
                        self.calibration = None;
                        self.notice = "All settings reset; avatar files and tracking connection kept. Calibrate face, then Save settings when ready.".into();
                        self.errors.clear();
                    }
                    ui.small("Reset changes are saved only when you click Save settings.");
                    ui.collapsing("Camera connection and avatar files", |ui| {
                        ui.label("Camera capture runs in the separate native capture process.");
                        ui.add(egui::DragValue::new(&mut self.config.port).range(1..=65535).prefix("Tracking port "));
                        if ui.button("Reconnect tracking").clicked() { self.reconnect(); }
                        ui.add(egui::Slider::new(&mut self.config.grace_ms,0. ..=1000.).text("Hold during brief loss (ms)"));
                        ui.add(egui::Slider::new(&mut self.config.return_ms,0. ..=1500.).text("Return to neutral (ms)"));
                        ui.label("Avatar asset folder");
                        ui.text_edit_singleline(&mut self.config.asset_root);
                        if ui.button("Reload avatar files").clicked() { self.reload(); }
                    });
                    ui.collapsing("Diagnostics", |ui| {
                        ui.label(format!("{:.0} preview FPS · {:.1} tracking samples/s",self.fps,self.packet_rate));
                        if let Some((p,t)) = &s.valid {
                            ui.label(format!("Last pose {:.0} ms · capture {:.0} ms · inference {:.0} ms",(time-t)*1000.,p.capture_age_ms,p.inference_ms));
                            ui.label(format!("Eye widening {:.2} / {:.2} · surprise {:.2}",self.pose.wide[0],self.pose.wide[1],self.pose.surprise));
                            ui.label(format!("Raw eyes {:.2} / {:.2} · shown {:.2} / {:.2}",p.blink_left,p.blink_right,self.pose.blink[0],self.pose.blink[1]));
                        }
                        ui.label(format!("Accepted {} · rejected {} · superseded {}",s.accepted,s.rejected,s.superseded));
                        if let Some(output) = &self.obs_worker {
                            let x = output.stats();
                            ui.small(format!("OBS {} · frames {} · readbacks {} / {} · connections {} / {} · GPU busy {} · frame age {:?} ms", x.address, x.frames_written, x.readbacks, x.converted, x.connections, x.disconnects, x.readback_busy, x.frame_age_ms));
                        }
                        if let Some(gpu) = &self.gpu { ui.small(&gpu.backend); for line in &gpu.report { ui.small(line); } }
                        ui.small(self.path.display().to_string());
                    });
                    if let Some(e) = &s.error { ui.colored_label(egui::Color32::LIGHT_RED,e); }
                    for error in &self.errors { ui.colored_label(egui::Color32::LIGHT_RED,error); }
                    if !self.errors.is_empty() && ui.button("Dismiss old messages").clicked() { self.errors.clear(); }
                });
            });
        }
        if let Some(gpu) = &self.gpu {
            gpu.render_with_activity(&self.config, &self.pose, &hands)
        }
        if let Some(worker) = &self.obs_worker {
            worker.sync(&self.config, self.receiver.as_ref());
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                if let Some(id) = self.texture {
                    let available = ui.available_size();
                    let h = available.y.min(available.x * 1300. / 1254.);
                    ui.centered_and_justified(|ui| {
                        ui.image((id, egui::vec2(h * 1254. / 1300., h)));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Choose the external stream-kit asset root and reload.");
                    });
                }
            });
        ctx.request_repaint_after(Duration::from_secs_f64(1. / self.cap as f64));
    }
}
