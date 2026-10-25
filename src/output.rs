//! Native straight-alpha RGBA output for OBS's built-in FFmpeg Media Source.
//!
//! One asynchronous GPU readback, one latest CPU frame, and bounded OS pipes.
//! No render-thread readback wait, encoding, socket write, or child process wait.
//! Each TCP connection receives a fresh NUT stream from a new owned FFmpeg muxer.

use crate::renderer::Renderer;
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const IO_DEADLINE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct OutputStats {
    pub address: SocketAddr,
    pub connected: bool,
    pub readbacks: u64,
    pub converted: u64,
    pub frames_written: u64,
    pub connections: u64,
    pub disconnects: u64,
    pub readback_busy: bool,
    pub frame_age_ms: Option<f64>,
    pub error: Option<String>,
}

struct Frame {
    rgba: Vec<u8>,
    captured: Instant,
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    connected: AtomicBool,
    busy: AtomicBool,
    readbacks: AtomicU64,
    converted: AtomicU64,
    frames_written: AtomicU64,
    connections: AtomicU64,
    disconnects: AtomicU64,
    latest: Mutex<Option<Arc<Frame>>>,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn error(&self, error: impl ToString) {
        *self.error.lock().unwrap() = Some(error.to_string());
    }

    fn publish(&self, rgba: Vec<u8>, captured: Instant) {
        *self.latest.lock().unwrap() = Some(Arc::new(Frame { rgba, captured }));
        self.converted.fetch_add(1, Ordering::Relaxed);
    }
}

struct Readback {
    buffer: wgpu::Buffer,
    device: wgpu::Device,
    ready: Receiver<std::result::Result<(), wgpu::BufferAsyncError>>,
    width: u32,
    height: u32,
    row: u32,
    captured: Instant,
}

pub struct NativeOutput {
    width: u32,
    height: u32,
    interval: Duration,
    next_offer: Instant,
    address: SocketAddr,
    shared: Arc<Shared>,
    jobs: SyncSender<Readback>,
    staging: Option<(wgpu::Device, wgpu::Buffer)>,
    pixel_worker: Option<JoinHandle<()>>,
    transport_worker: Option<JoinHandle<()>>,
}

impl NativeOutput {
    /// Bind loopback only. Port 0 is useful for isolated integration tests.
    pub fn new(width: u32, height: u32, fps: u32, port: u16) -> Result<Self> {
        ensure!(
            (1..=4096).contains(&width) && (1..=4096).contains(&height),
            "output dimensions must be 1..4096"
        );
        ensure!((1..=240).contains(&fps), "output FPS must be 1..240");
        let available = Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("native output requires the installed ffmpeg executable")?;
        ensure!(available.success(), "ffmpeg -version failed");
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .with_context(|| format!("cannot bind native OBS output on 127.0.0.1:{port}"))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared::default());
        let (jobs, receiver) = mpsc::sync_channel(1);
        let pixel_shared = shared.clone();
        let pixel_worker = thread::Builder::new()
            .name("output-pixels".into())
            .spawn(move || pixel_loop(receiver, pixel_shared))?;
        let transport_shared = shared.clone();
        let transport_worker = match thread::Builder::new()
            .name("output-transport".into())
            .spawn(move || transport_loop(listener, width, height, fps, transport_shared))
        {
            Ok(worker) => worker,
            Err(error) => {
                shared.stop.store(true, Ordering::Release);
                let _ = pixel_worker.join();
                return Err(error.into());
            }
        };
        let interval = Duration::from_secs_f64(1. / fps as f64);
        Ok(Self {
            width,
            height,
            interval,
            next_offer: Instant::now(),
            address,
            shared,
            jobs,
            staging: None,
            pixel_worker: Some(pixel_worker),
            transport_worker: Some(transport_worker),
        })
    }

    /// Gate a separate transparent output renderer with this before rendering.
    /// No GPU work is requested while no client is connected or readback is busy.
    pub fn wants_frame(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
            && !self.shared.busy.load(Ordering::Acquire)
            && Instant::now() >= self.next_offer
    }

    /// Submit a copy of an already-rendered RGBA8 sRGB texture. Returns without
    /// waiting for GPU completion; a worker polls and converts mapped pixels.
    pub fn offer(&mut self, renderer: &Renderer) -> Result<()> {
        if !self.wants_frame() {
            return Ok(());
        }
        ensure!(
            renderer.width == self.width && renderer.height == self.height,
            "output renderer must remain {}x{}",
            self.width,
            self.height
        );
        ensure!(
            renderer.texture.format() == wgpu::TextureFormat::Rgba8UnormSrgb,
            "output requires the renderer's RGBA8 sRGB texture"
        );
        let row = (self.width * 4).div_ceil(256) * 256;
        if self
            .staging
            .as_ref()
            .is_none_or(|(device, _)| *device != renderer.device)
        {
            let buffer = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("native-output-readback"),
                size: u64::from(row) * u64::from(self.height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.staging = Some((renderer.device.clone(), buffer));
        }
        let buffer = self.staging.as_ref().unwrap().1.clone();
        self.shared.busy.store(true, Ordering::Release);
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("native-output-copy"),
            });
        encoder.copy_texture_to_buffer(
            renderer.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        renderer.queue.submit([encoder.finish()]);
        let (tx, ready) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let captured = Instant::now();
        let job = Readback {
            buffer,
            device: renderer.device.clone(),
            ready,
            width: self.width,
            height: self.height,
            row,
            captured,
        };
        if let Err(error) = self.jobs.try_send(job) {
            let job = match error {
                mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
            };
            job.buffer.unmap();
            self.shared.busy.store(false, Ordering::Release);
            anyhow::bail!("output pixel worker unavailable");
        }
        self.next_offer += self.interval;
        if self.next_offer < captured - self.interval {
            self.next_offer = captured + self.interval;
        }
        self.shared.readbacks.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn stats(&self) -> OutputStats {
        OutputStats {
            address: self.address,
            connected: self.shared.connected.load(Ordering::Acquire),
            readbacks: self.shared.readbacks.load(Ordering::Relaxed),
            converted: self.shared.converted.load(Ordering::Relaxed),
            frames_written: self.shared.frames_written.load(Ordering::Relaxed),
            connections: self.shared.connections.load(Ordering::Relaxed),
            disconnects: self.shared.disconnects.load(Ordering::Relaxed),
            readback_busy: self.shared.busy.load(Ordering::Acquire),
            frame_age_ms: self
                .shared
                .latest
                .lock()
                .unwrap()
                .as_ref()
                .map(|frame| frame.captured.elapsed().as_secs_f64() * 1000.),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }
}

impl Drop for NativeOutput {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.transport_worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.pixel_worker.take() {
            let _ = worker.join();
        }
    }
}

/// GPU stores sRGB-encoded, linear-premultiplied color; the wire carries sRGB
/// straight color and unchanged linear coverage alpha. 64KiB LUT avoids pow()
/// for every channel of every frame. Index is alpha*256 + encoded channel.
fn unpremultiply_lut() -> Vec<u8> {
    let mut table = vec![0; 256 * 256];
    for alpha in 1..256 {
        for channel in 0..256 {
            let encoded = channel as f64 / 255.;
            let linear = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
            let straight = (linear / (alpha as f64 / 255.)).clamp(0., 1.);
            let encoded = if straight <= 0.0031308 {
                straight * 12.92
            } else {
                1.055 * straight.powf(1. / 2.4) - 0.055
            };
            table[alpha * 256 + channel] = (encoded * 255.).round() as u8;
        }
    }
    table
}

fn convert_pixels(mapped: &[u8], width: u32, height: u32, row: u32, lut: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity((width * height * 4) as usize);
    for line in mapped.chunks_exact(row as usize).take(height as usize) {
        for pixel in line[..width as usize * 4].as_chunks::<4>().0 {
            let alpha = pixel[3] as usize * 256;
            result.extend_from_slice(&[
                lut[alpha + pixel[0] as usize],
                lut[alpha + pixel[1] as usize],
                lut[alpha + pixel[2] as usize],
                pixel[3],
            ]);
        }
    }
    result
}

fn pixel_loop(jobs: Receiver<Readback>, shared: Arc<Shared>) {
    let lut = unpremultiply_lut();
    while !shared.stop.load(Ordering::Acquire) {
        let job = match jobs.recv_timeout(Duration::from_millis(20)) {
            Ok(job) => job,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        let mapped = loop {
            if shared.stop.load(Ordering::Acquire) {
                break false;
            }
            if let Err(error) = job.device.poll(wgpu::PollType::Poll) {
                shared.error(format!("output GPU poll: {error}"));
                break false;
            }
            match job.ready.try_recv() {
                Ok(Ok(())) => break true,
                Ok(Err(error)) => {
                    shared.error(format!("output readback: {error}"));
                    break false;
                }
                Err(mpsc::TryRecvError::Disconnected) => break false,
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(1)),
            }
        };
        if mapped {
            let bytes = job.buffer.slice(..).get_mapped_range();
            let rgba = convert_pixels(&bytes, job.width, job.height, job.row, &lut);
            drop(bytes);
            shared.publish(rgba, job.captured);
        }
        job.buffer.unmap();
        shared.busy.store(false, Ordering::Release);
    }
}

fn mux_command(width: u32, height: u32, fps: u32) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &format!("{width}x{height}"),
            "-framerate",
            &fps.to_string(),
            "-thread_queue_size",
            "1",
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "copy",
            "-f",
            "nut",
            "-flush_packets",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    command
}

fn transport_loop(listener: TcpListener, width: u32, height: u32, fps: u32, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Acquire) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) => {
                shared.error(format!("output accept: {error}"));
                break;
            }
        };
        // A reconnect starts with new live pixels and a new container header.
        *shared.latest.lock().unwrap() = None;
        shared.connected.store(true, Ordering::Release);
        shared.connections.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = serve_connection(stream, width, height, fps, &shared)
            && !shared.stop.load(Ordering::Acquire)
        {
            shared.error(error);
        }
        shared.connected.store(false, Ordering::Release);
        shared.disconnects.fetch_add(1, Ordering::Relaxed);
    }
}

fn write_chunk(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    shared: &Shared,
    stopped: &AtomicBool,
) -> std::io::Result<()> {
    let deadline = Instant::now() + IO_DEADLINE;
    while !bytes.is_empty() {
        if shared.stop.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match stream.write(bytes) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn serve_connection(
    mut stream: TcpStream,
    width: u32,
    height: u32,
    fps: u32,
    shared: &Arc<Shared>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    stream.set_nonblocking(true)?;
    let connection_probe = stream.try_clone()?;
    let mut child = mux_command(width, height, fps)
        .spawn()
        .context("start native output FFmpeg muxer")?;
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let feed_shared = shared.clone();
    let feed_stopped = stopped.clone();
    let feeder = thread::spawn(move || {
        let interval = Duration::from_secs_f64(1. / fps as f64);
        let mut deadline = Instant::now();
        while !feed_shared.stop.load(Ordering::Acquire) && !feed_stopped.load(Ordering::Acquire) {
            let now = Instant::now();
            if now < deadline {
                thread::sleep((deadline - now).min(Duration::from_millis(5)));
                continue;
            }
            let frame = feed_shared.latest.lock().unwrap().clone();
            if let Some(frame) = frame {
                // At a fixed media clock, repeat the latest frame if rendering is
                // slower; never replay superseded frames or accumulate a queue.
                if input.write_all(&frame.rgba).is_err() {
                    break;
                }
                feed_shared.frames_written.fetch_add(1, Ordering::Relaxed);
                deadline += interval;
                if deadline < Instant::now() - interval {
                    deadline = Instant::now() + interval;
                }
            } else {
                thread::sleep(Duration::from_millis(2));
                deadline = Instant::now();
            }
        }
        feed_stopped.store(true, Ordering::Release);
    });
    let pump_shared = shared.clone();
    let pump_stopped = stopped.clone();
    let pump = thread::spawn(move || {
        let mut bytes = [0_u8; 65536];
        loop {
            match output.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    if write_chunk(&mut stream, &bytes[..count], &pump_shared, &pump_stopped)
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        pump_stopped.store(true, Ordering::Release);
    });
    let mut exit_error = None;
    while !shared.stop.load(Ordering::Acquire) && !stopped.load(Ordering::Acquire) {
        // Detect a decoder closing even before the first rendered frame exists.
        // Otherwise FFmpeg could wait for stdin indefinitely with a dead client.
        match connection_probe.peek(&mut [0_u8; 1]) {
            Ok(0) => break,
            Ok(_) => break, // This transport is output-only; clients send no data.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    exit_error = Some(format!("FFmpeg muxer exited: {status}"));
                }
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => {
                exit_error = Some(error.to_string());
                break;
            }
        }
    }
    stopped.store(true, Ordering::Release);
    // Kill only our own child. Closing its pipes releases both blocked workers.
    let _ = child.kill();
    let _ = child.wait();
    let _ = feeder.join();
    let _ = pump.join();
    if let Some(error) = exit_error {
        anyhow::bail!(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_conversion_is_linear_and_strips_row_padding() {
        let lut = unpremultiply_lut();
        // Linear .5 red encoded in sRGB is 188, at alpha128 gives straight255.
        let input = [
            188, 0, 0, 128, 123, 45, 67, 0, 99, 99, 99, 99, 12, 34, 56, 255, 0, 188, 0, 128, 99,
            99, 99, 99,
        ];
        assert_eq!(
            convert_pixels(&input, 2, 2, 12, &lut),
            [255, 0, 0, 128, 0, 0, 0, 0, 12, 34, 56, 255, 0, 255, 0, 128]
        );
        for channel in 0..256 {
            assert_eq!(lut[255 * 256 + channel], channel as u8);
        }
    }

    #[test]
    fn cpu_mailbox_overwrites_and_does_not_queue() {
        let shared = Shared::default();
        for value in 0..10000_u32 {
            shared.publish(value.to_le_bytes().to_vec(), Instant::now());
        }
        assert_eq!(shared.converted.load(Ordering::Relaxed), 10000);
        assert_eq!(
            shared.latest.lock().unwrap().as_ref().unwrap().rgba,
            9999_u32.to_le_bytes()
        );
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "requires installed FFmpeg and permission to create local TCP sockets"]
    fn ffmpeg_tcp_alpha_roundtrip_reconnect_and_clean_shutdown() {
        let output = NativeOutput::new(4, 1, 60, 0).unwrap();
        let expected = vec![
            11, 22, 33, 0, 40, 50, 60, 64, 70, 80, 90, 128, 100, 110, 120, 255,
        ];
        for connection in 1..=2 {
            let mut decoder = Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "nut",
                    "-i",
                    &format!("tcp://{}", output.address),
                    "-frames:v",
                    "2",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "pipe:1",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            wait_until(|| output.stats().connections == connection);
            output.shared.publish(expected.clone(), Instant::now());
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if decoder.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() > deadline {
                    let _ = decoder.kill();
                    panic!("decoder timeout");
                }
                thread::sleep(Duration::from_millis(10));
            }
            let result = decoder.wait_with_output().unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, expected.repeat(2));
            wait_until(|| output.stats().disconnects >= connection);
        }
        let address = output.address;
        let start = Instant::now();
        drop(output);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(TcpStream::connect(address).is_err());
    }

    #[test]
    #[ignore = "requires installed FFmpeg and permission to create local TCP sockets"]
    fn stalled_consumer_disconnects_and_idle_child_shutdown_is_bounded() {
        let output = NativeOutput::new(600, 660, 60, 0).unwrap();
        let stalled = TcpStream::connect(output.address).unwrap();
        wait_until(|| output.stats().connected);
        output
            .shared
            .publish(vec![0; 600 * 660 * 4], Instant::now());
        wait_until(|| output.stats().disconnects == 1);
        assert!(!output.stats().connected);
        drop(stalled);
        let idle = TcpStream::connect(output.address).unwrap();
        wait_until(|| output.stats().connections == 2);
        drop(idle);
        wait_until(|| output.stats().disconnects == 2);
        let idle = TcpStream::connect(output.address).unwrap();
        wait_until(|| output.stats().connections == 3);
        let started = Instant::now();
        drop(output);
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(idle);
    }

    #[test]
    #[ignore = "requires native GPU, installed FFmpeg, and local TCP sockets; opens no window"]
    fn gpu_readback_to_tcp_preserves_straight_alpha() {
        let gpu = pollster::block_on(Renderer::headless(
            crate::assets::Assets {
                meshes: vec![],
                report: vec![],
                rig: crate::rig::Rig::default(),
            },
            16,
            16,
        ))
        .unwrap();
        let view = gpu.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("alpha-output-test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.5,
                            g: 0.,
                            b: 0.,
                            a: 0.5,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        gpu.queue.submit([encoder.finish()]);
        let mut output = NativeOutput::new(16, 16, 60, 0).unwrap();
        let mut decoder = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "nut",
                "-i",
                &format!("tcp://{}", output.address),
                "-frames:v",
                "2",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        wait_until(|| output.wants_frame());
        output.offer(&gpu).unwrap();
        wait_until(|| output.stats().converted == 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if decoder.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() > deadline {
                let _ = decoder.kill();
                panic!("decoder timeout");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let result = decoder.wait_with_output().unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout.len(), 16 * 16 * 4 * 2);
        // A .5 GPU clear can quantize to 127 or 128 depending on backend.
        for pixel in result.stdout.as_chunks::<4>().0 {
            assert_eq!(&pixel[..3], &[255, 0, 0]);
            assert!(pixel[3].abs_diff(128) <= 1);
        }
        assert_eq!(output.stats().readbacks, 1);
    }
}
