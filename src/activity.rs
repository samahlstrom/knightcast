//! Read-only native client for the existing stream-kit event hub.
//! Only activity states are retained: never keyboard codes, raw events or a work queue.
use serde_json::Value;
use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, protocol::WebSocketConfig};

#[derive(Clone, Debug)]
pub struct Hand {
    pub down: bool,
    pub buttons: [bool; 2],
    pub serial: u64,
    pub taps: [f64; 4],
    down_at: f64,
    release_at: f64,
}
impl Default for Hand {
    fn default() -> Self {
        Self {
            down: false,
            buttons: [false; 2],
            serial: 0,
            taps: [-999.; 4],
            down_at: -999.,
            release_at: -999.,
        }
    }
}
impl Hand {
    fn event(&mut self, right: bool, down: bool, button: usize, now: f64) {
        if right {
            self.buttons[button] = down;
        }
        if down {
            let finger = if right {
                button
            } else {
                (self.serial % 4) as usize
            };
            self.taps[finger] = now;
            self.serial = self.serial.wrapping_add(1);
            self.down = true;
            self.down_at = now;
            self.release_at = -999.;
        } else if !right || !self.buttons.iter().any(|b| *b) {
            self.release_at = now;
        }
    }
    fn at_time(&self, now: f64) -> Self {
        let mut h = self.clone();
        if h.down && now >= h.down_at + 0.06 && h.release_at >= h.down_at {
            h.down = false;
        }
        h
    }
}
#[derive(Clone, Debug, Default)]
pub struct Health {
    pub connected: bool,
    pub hello_received: bool,
    pub hands_status: String,
    pub mic_status: String,
    pub voice_status: String,
    pub outfits: usize,
    pub hold: bool,
    pub accepted: u64,
    pub rejected: u64,
    pub ignored: u64,
    pub reconnects: u64,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expression {
    #[default]
    Neutral,
    Stars,
    Annoyed,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub time: f64,
    pub pointer: [f32; 2],
    pub hands: [Hand; 2],
    pub health: Health,
    pub expression: Expression,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            time: 0.,
            pointer: [0.; 2],
            hands: Default::default(),
            health: Health::default(),
            expression: Expression::Neutral,
        }
    }
}
#[derive(Default)]
struct State {
    pointer: [f32; 2],
    hands: [Hand; 2],
    health: Health,
    expression: Expression,
    expression_until: f64,
}
impl State {
    fn disconnected(&mut self, error: Option<String>) {
        self.health.connected = false;
        self.health.hello_received = false;
        self.health.error = error;
        self.pointer = [0.; 2];
        self.hands = Default::default();
        self.expression = Expression::Neutral;
        self.expression_until = 0.;
    }
    fn accept(&mut self, bytes: &[u8], now: f64, bounds: [f32; 4]) {
        if bytes.len() > 65536 {
            self.health.rejected += 1;
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            self.health.rejected += 1;
            return;
        };
        let valid = match v["t"].as_str() {
            Some("hello") => {
                if v["version"].as_u64() != Some(1) {
                    false
                } else {
                    let status = |name: &str| {
                        v[name]["state"]
                            .as_str()
                            .unwrap_or("unknown")
                            .chars()
                            .take(80)
                            .collect::<String>()
                    };
                    self.health.hello_received = true;
                    self.health.hands_status = status("hands");
                    self.health.mic_status = status("mic");
                    self.health.voice_status = status("voice");
                    self.health.outfits = v["outfits"].as_array().map_or(0, Vec::len);
                    self.health.hold = v["hold"]["on"].as_bool().unwrap_or(false);
                    true
                }
            }
            Some("pointer") => match (v["dx"].as_f64(), v["dy"].as_f64()) {
                (Some(dx), Some(dy))
                    if dx.is_finite() && dy.is_finite() && dx.abs() <= 1e6 && dy.abs() <= 1e6 =>
                {
                    // Same gain and per-message clamping as knight-puppet.html.
                    self.pointer[0] =
                        (self.pointer[0] + dx as f32 * 0.035).clamp(bounds[0], bounds[1]);
                    self.pointer[1] =
                        (self.pointer[1] + dy as f32 * 0.025).clamp(bounds[2], bounds[3]);
                    true
                }
                _ => false,
            },
            Some("hand") => match (
                v["side"].as_str(),
                v["down"].as_bool(),
                v["button"].as_str(),
            ) {
                (
                    Some(side @ ("left" | "right")),
                    Some(down),
                    button @ (None | Some("primary" | "secondary")),
                ) => {
                    let right = side == "right";
                    self.hands[usize::from(right)].event(
                        right,
                        down,
                        usize::from(button == Some("secondary")),
                        now,
                    );
                    true
                }
                _ => false,
            },
            Some("expr") => {
                let seconds = v["seconds"].as_f64().unwrap_or(4.);
                if !seconds.is_finite() || !(0. ..=120.).contains(&seconds) {
                    false
                } else {
                    let expression = match v["name"].as_str() {
                        Some("stars" | "excited") => Some(Expression::Stars),
                        Some("annoyed" | "boi") => Some(Expression::Annoyed),
                        Some("neutral" | "open") => Some(Expression::Neutral),
                        _ => None,
                    };
                    if let Some(expression) = expression {
                        if !self.health.hold {
                            self.expression = expression;
                            self.expression_until = now + if seconds == 0. { 4. } else { seconds };
                        }
                        true
                    } else {
                        self.health.ignored += 1;
                        return;
                    }
                }
            }
            Some("hold") => {
                if let Some(on) = v["on"].as_bool() {
                    self.health.hold = on;
                    true
                } else {
                    false
                }
            }
            // Deliberately no mic-to-jaw or pointer-to-head coupling. These remain
            // owned by their existing stream-kit consumers / explicit tracking bridge.
            Some("mic" | "face" | "body" | "event" | "outfit" | "voice_status") => {
                self.health.ignored += 1;
                return;
            }
            _ => false,
        };
        if valid {
            self.health.accepted += 1
        } else {
            self.health.rejected += 1
        }
    }
}
pub struct Client {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    origin: Instant,
    pointer: [f32; 2],
}
impl Client {
    pub fn start(port: u16, bounds: [f32; 4]) -> Self {
        // The only endpoint is loopback: this cannot become a remote activity feed.
        let bounds = if bounds.iter().all(|x| x.is_finite())
            && bounds[0] <= bounds[1]
            && bounds[2] <= bounds[3]
        {
            bounds
        } else {
            [-150., 24., -14., 8.]
        };
        let origin = Instant::now();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = state.clone();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            while !stopping.load(Ordering::Relaxed) {
                let connect = || -> anyhow::Result<_> {
                    let stream = TcpStream::connect_timeout(&address, Duration::from_millis(250))?;
                    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
                    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
                    let config = WebSocketConfig::default()
                        .max_message_size(Some(65536))
                        .max_frame_size(Some(65536))
                        .write_buffer_size(0)
                        .max_write_buffer_size(65536);
                    let (ws, _) = tungstenite::client::client_with_config(
                        format!("ws://127.0.0.1:{port}"),
                        stream,
                        Some(config),
                    )?;
                    ws.get_ref().set_nonblocking(true)?;
                    Ok(ws)
                };
                match connect() {
                    Ok(mut ws) => {
                        {
                            let mut s = shared.lock().unwrap();
                            s.health.connected = true;
                            s.health.error = None;
                            s.health.reconnects += 1;
                        }
                        while !stopping.load(Ordering::Relaxed) {
                            match ws.read() {
                                Ok(Message::Text(text)) => shared.lock().unwrap().accept(
                                    text.as_bytes(),
                                    origin.elapsed().as_secs_f64(),
                                    bounds,
                                ),
                                Ok(Message::Close(_)) => break,
                                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                                Ok(Message::Binary(_)) => {
                                    shared.lock().unwrap().health.rejected += 1
                                }
                                Err(tungstenite::Error::Io(e))
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) =>
                                {
                                    thread::park_timeout(Duration::from_millis(5))
                                }
                                Err(e) => {
                                    shared.lock().unwrap().health.error = Some(e.to_string());
                                    break;
                                }
                            }
                        }
                        let mut s = shared.lock().unwrap();
                        let error = s.health.error.clone();
                        s.disconnected(error);
                    }
                    Err(e) => shared
                        .lock()
                        .unwrap()
                        .disconnected(Some(format!("Stream-kit hub {address}: {e}"))),
                }
                if !stopping.load(Ordering::Relaxed) {
                    thread::park_timeout(Duration::from_millis(1000));
                }
            }
        });
        Self {
            state,
            stop,
            worker: Some(worker),
            origin,
            pointer: [0.; 2],
        }
    }
    pub fn frame(&mut self, dt: f32) -> Frame {
        let now = self.origin.elapsed().as_secs_f64();
        let s = self.state.lock().unwrap();
        let alpha = 1. - (-dt.max(0.) / 0.07).exp();
        for i in 0..2 {
            self.pointer[i] += (s.pointer[i] - self.pointer[i]) * alpha;
        }
        Frame {
            time: now,
            pointer: self.pointer,
            hands: std::array::from_fn(|i| s.hands[i].at_time(now)),
            health: s.health.clone(),
            expression: if now < s.expression_until {
                s.expression
            } else {
                Expression::Neutral
            },
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const BOUNDS: [f32; 4] = [-150., 24., -14., 8.];
    #[test]
    fn websocket_consumer_receives_and_releases_without_event_backlog() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (close_tx, close_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            for message in [
                r#"{"t":"hello","version":1,"hands":{"state":"ok"}}"#,
                r#"{"t":"pointer","dx":100,"dy":40}"#,
                r#"{"t":"hand","side":"right","down":true,"button":"secondary"}"#,
            ] {
                ws.send(Message::Text(message.into())).unwrap();
            }
            close_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let _ = ws.close(None);
        });
        let mut client = Client::start(port, BOUNDS);
        let start = Instant::now();
        loop {
            let f = client.frame(1.);
            if f.health.accepted == 3 {
                assert!(f.health.hello_received);
                assert_eq!(f.health.hands_status, "ok");
                assert!(f.hands[1].down);
                assert_eq!(f.hands[1].buttons, [false, true]);
                assert!((f.pointer[0] - 3.5).abs() < 0.001);
                assert!((f.pointer[1] - 1.).abs() < 0.001);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(2));
        }
        close_tx.send(()).unwrap();
        server.join().unwrap();
        loop {
            let f = client.frame(1.);
            if !f.health.connected {
                assert!(f.hands.iter().all(|h| !h.down));
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn voice_expression_is_timed_bounded_and_disconnect_safe() {
        let mut state = State::default();
        state.accept(br#"{"t":"expr","name":"stars","seconds":3.5}"#, 1., BOUNDS);
        assert_eq!(state.expression, Expression::Stars);
        assert_eq!(state.expression_until, 4.5);
        state.accept(br#"{"t":"expr","name":"annoyed","seconds":3}"#, 2., BOUNDS);
        assert_eq!(state.expression, Expression::Annoyed);
        assert_eq!(state.expression_until, 5.);
        state.accept(
            br#"{"t":"expr","name":"stars","seconds":999999}"#,
            2.,
            BOUNDS,
        );
        assert_eq!(state.expression, Expression::Annoyed);
        assert_eq!(state.health.rejected, 1);
        state.disconnected(None);
        assert_eq!(state.expression, Expression::Neutral);
    }
    #[test]
    fn pointer_is_bounded_and_coalesced() {
        let mut s = State::default();
        for _ in 0..10000 {
            s.accept(br#"{"t":"pointer","dx":100,"dy":100}"#, 0., BOUNDS)
        }
        assert_eq!(s.pointer, [24., 8.]);
        s.accept(br#"{"t":"pointer","dx":-10000,"dy":-10000}"#, 0., BOUNDS);
        assert_eq!(s.pointer, [-150., -14.]);
        assert_eq!(s.health.accepted, 10001);
    }
    #[test]
    fn button_mapping_and_disconnect_releases() {
        let mut s = State::default();
        s.accept(
            br#"{"t":"hand","side":"right","down":true,"button":"primary"}"#,
            1.,
            BOUNDS,
        );
        assert_eq!(s.hands[1].buttons, [true, false]);
        assert_eq!(s.hands[1].taps[0], 1.);
        s.accept(
            br#"{"t":"hand","side":"right","down":true,"button":"secondary"}"#,
            1.1,
            BOUNDS,
        );
        assert_eq!(s.hands[1].buttons, [true, true]);
        assert_eq!(s.hands[1].taps[1], 1.1);
        s.accept(
            br#"{"t":"hand","side":"right","down":false,"button":"primary"}"#,
            1.2,
            BOUNDS,
        );
        assert!(s.hands[1].at_time(2.).down);
        assert_eq!(s.hands[1].buttons, [false, true]);
        s.disconnected(None);
        assert!(
            s.hands
                .iter()
                .all(|h| !h.down && !h.buttons.iter().any(|b| *b))
        );
    }
    #[test]
    fn keyboard_cycles_and_minimum_release() {
        let mut s = State::default();
        for i in 0..5 {
            s.accept(
                br#"{"t":"hand","side":"left","down":true}"#,
                i as f64,
                BOUNDS,
            )
        }
        assert_eq!(s.hands[0].taps, [4., 1., 2., 3.]);
        s.accept(br#"{"t":"hand","side":"left","down":false}"#, 4.01, BOUNDS);
        assert!(s.hands[0].at_time(4.04).down);
        assert!(!s.hands[0].at_time(4.07).down);
    }
    #[test]
    fn hello_and_rejections_preserve_state() {
        let mut s = State::default();
        s.accept(br#"{"t":"hello","version":1,"hands":{"state":"ok"},"mic":{"state":"disabled"},"outfits":[],"hold":{"on":false}}"#,0.,BOUNDS);
        assert!(s.health.hello_received);
        assert_eq!(s.health.hands_status, "ok");
        for bytes in [
            b"bad".as_slice(),
            br#"{"t":"hand","side":"left","down":1}"#,
            br#"{"t":"pointer","dx":"bad","dy":0}"#,
        ] {
            s.accept(bytes, 0., BOUNDS)
        }
        assert_eq!(s.health.rejected, 3);
        assert_eq!(s.pointer, [0.; 2]);
    }
}
