mod assets;
mod config;
mod renderer;
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
            renderer.render(&config, &pose);
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
}
impl App {
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
                    &gpu.view,
                    wgpu::FilterMode::Linear,
                ));
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
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.output_only = !self.output_only
        }
        if !self.output_only {
            egui::SidePanel::left("controls").default_width(310.).show(ctx,|ui|{
egui::ScrollArea::vertical().show(ui,|ui|{
ui.heading("KnightCast");
ui.label("Native GPU · local UDP · F1 hides controls");
ui.separator();
ui.strong(format!("{:?} · {:.0} FPS · {:.1} packets/s",self.pose.state,self.fps,self.packet_rate));
if let Some((p,t))=&s.valid{
ui.label(format!("Last valid: {:.0} ms · capture {:.0} ms · inference {:.0} ms",(time-t)*1000.,p.capture_age_ms,p.inference_ms));
ui.label(format!("Sequence {} · mouth {:.2}",p.seq,p.mouth_open));
}
ui.label(format!("Accepted {} / rejected {} / superseded {} / sequence resets {}",s.accepted,s.rejected,s.superseded,s.restarts));
if let Some(gpu)=&self.gpu{
ui.small(&gpu.backend);
for line in &gpu.report{
ui.small(line);
}
}

if let Some(e)=&s.error{
ui.colored_label(egui::Color32::RED,e);
}

 if ui.button("Calibrate neutral (mouth closed)").clicked(){
if self.pose.calibrate(&s,time,self.config.grace_ms/1000.){
self.config.neutral=self.pose.neutral.to_array();
self.config.mouth_baseline=self.pose.mouth_baseline}
else{
self.errors.push("Calibration requires a fresh tracked sample".into())}
}

 ui.add(egui::Slider::new(&mut self.config.smoothing_ms,0. ..=300.).text("Smoothing ms"));
ui.add(egui::Slider::new(&mut self.config.grace_ms,0. ..=1000.).text("Grace ms"));
ui.add(egui::Slider::new(&mut self.config.return_ms,0. ..=1500.).text("Return ms"));

 ui.add(egui::Slider::new(&mut self.config.mouth_gain,0.1..=10.).text("Mouth sensitivity"));

 ui.separator();
ui.label("External stream-kit asset root");
ui.text_edit_singleline(&mut self.config.asset_root);
if ui.button("Reload original assets").clicked(){
self.reload()}

 ui.label("Protocol: KnightCast JSON v1 (selected face at sender)");
ui.add(egui::DragValue::new(&mut self.config.port).range(1..=65535).prefix("UDP port "));
if ui.button("Reconnect receiver").clicked(){
self.receiver=None;
self.accepted_last=0;
self.packet_rate=0.;
self.rate_last=Instant::now();
match tracking::Receiver::bind(self.config.port){
Ok(r)=>{
self.receiver=Some(r);
self.accepted_last=0;
self.pose.state=tracking::State::Waiting}
,Err(e)=>self.errors.push(format!("UDP: {e}"))}
}

 ui.collapsing("Model and framing",|ui|{
ui.add(egui::Slider::new(&mut self.config.model_scale,0.5..=1.5).text("Helmet scale"));
ui.add(egui::Slider::new(&mut self.config.body_scale,0.5..=1.5).text("Body scale"));
ui.add(egui::Slider::new(&mut self.config.zoom,0.5..=2.).text("Camera zoom"));
for (i,label) in ["Offset X","Offset Y"].iter().enumerate(){
ui.add(egui::DragValue::new(&mut self.config.offset[i]).speed(0.01).prefix(*label));
}
for (i,label) in ["Base yaw ","Base pitch ","Base roll "].iter().enumerate(){
ui.add(egui::DragValue::new(&mut self.config.base_degrees[i]).speed(0.2).prefix(*label));
}
for (i,label) in ["Pivot X ","Pivot Y ","Pivot Z "].iter().enumerate(){
ui.add(egui::DragValue::new(&mut self.config.pivot[i]).speed(0.01).prefix(*label));
}
ui.label("Background RGBA (output texture only)");
ui.color_edit_button_rgba_unmultiplied(&mut self.config.background);
}
);

 ui.horizontal(|ui|{
if ui.button("Save settings").clicked() && let Err(e)=self.config.save(&self.path){
self.errors.push(format!("Save: {e:#}"))}

if ui.button("Reset transforms").clicked(){
let root=self.config.asset_root.clone();
let port=self.config.port;
self.config=config::Config{
asset_root:root,port,..Default::default()}
;
self.pose.neutral=glam::Quat::IDENTITY;
}
}
);
ui.small(self.path.display().to_string());
for error in &self.errors{
ui.colored_label(egui::Color32::LIGHT_RED,error);
}

if !self.errors.is_empty()&&ui.button("Clear messages").clicked(){
self.errors.clear();
}
ui.separator();
ui.small("Prototype: rigid body. Jaw and blink. Gaze/smile/wide/surprise, arm activity and capture integration pending. Camera is a separately launched adapter.");
}
);
}
);
        }
        if let Some(gpu) = &self.gpu {
            gpu.render(&self.config, &self.pose)
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
