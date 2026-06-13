//! Native audio engine. Runs on its own thread (rodio's `OutputStream` is
//! `!Send`), owns the playback queue, and exposes a command channel + a shared
//! state snapshot the UI reads each frame.
//!
//! Audio decoding/output is 100% native (rodio → cpal → WASAPI on Windows):
//! there is no Chromium/WebView anywhere, which is the whole point of this build
//! vs. the Tauri/WebView2 app — background playback is never throttled.

use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::api::{ApiClient, Track};

/// Open an output stream on the named device, or the system default if `pref`
/// is None / not found. Returns (stream, handle, resolved device name).
fn open_stream(pref: &Option<String>) -> Option<(OutputStream, OutputStreamHandle, String)> {
    let host = rodio::cpal::default_host();
    if let Some(want) = pref {
        if let Ok(devices) = host.output_devices() {
            for d in devices {
                if d.name().map(|n| &n == want).unwrap_or(false) {
                    if let Ok((s, h)) = OutputStream::try_from_device(&d) {
                        return Some((s, h, want.clone()));
                    }
                }
            }
        }
    }
    let name = host.default_output_device().and_then(|d| d.name().ok()).unwrap_or_else(|| "Default".into());
    OutputStream::try_default().ok().map(|(s, h)| (s, h, name))
}

fn list_devices() -> Vec<String> {
    let host = rodio::cpal::default_host();
    host.output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn default_device_name() -> String {
    rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default()
}

// --------------------------------------------------------------------------- #
// Native DSP: 10-band parametric EQ + L/R balance + a visualizer sample tap.
// A `Dsp` source wraps the decoded f32 stream; the UI mutates `DspShared` live.
// --------------------------------------------------------------------------- #
pub const EQ_FREQS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

#[derive(Clone)]
pub struct DspParams {
    pub eq_db: [f32; 10], // per-band gain in dB (-12..+12)
    pub balance: f32,     // -1 = full left .. +1 = full right
    pub eq_on: bool,
}

impl Default for DspParams {
    fn default() -> Self {
        DspParams { eq_db: [0.0; 10], balance: 0.0, eq_on: false }
    }
}

pub struct DspShared {
    pub params: Mutex<DspParams>,
    pub viz: Mutex<std::collections::VecDeque<f32>>, // recent first-channel samples
}

impl DspShared {
    fn new() -> Arc<Self> {
        Arc::new(DspShared {
            params: Mutex::new(DspParams::default()),
            viz: Mutex::new(std::collections::VecDeque::with_capacity(2048)),
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Default)]
struct BqState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// RBJ cookbook peaking EQ.
    fn peaking(fs: f32, f0: f32, q: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f0 / fs;
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cs) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cs) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
    #[inline]
    fn process(&self, st: &mut BqState, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * st.x1 + self.b2 * st.x2 - self.a1 * st.y1 - self.a2 * st.y2;
        st.x2 = st.x1;
        st.x1 = x;
        st.y2 = st.y1;
        st.y1 = y;
        y
    }
}

/// Source wrapper applying EQ + balance and tapping samples for the visualizer.
struct Dsp<S> {
    inner: S,
    shared: Arc<DspShared>,
    fs: f32,
    ch: u16,
    coeffs: [Biquad; 10],
    state: Vec<[BqState; 10]>,
    cur: u16,
    lg: f32,
    rg: f32,
    eq_on: bool,
    n: u64,
}

impl<S> Dsp<S>
where
    S: Source<Item = f32>,
{
    fn new(inner: S, shared: Arc<DspShared>) -> Self {
        let fs = inner.sample_rate() as f32;
        let ch = inner.channels().max(1);
        let mut d = Dsp {
            inner,
            shared,
            fs,
            ch,
            coeffs: [Biquad::default(); 10],
            state: vec![[BqState::default(); 10]; ch as usize],
            cur: 0,
            lg: 1.0,
            rg: 1.0,
            eq_on: false,
            n: 0,
        };
        d.reload();
        d
    }
    fn reload(&mut self) {
        let p = self.shared.params.lock().map(|g| g.clone()).unwrap_or_default();
        self.eq_on = p.eq_on;
        for i in 0..10 {
            self.coeffs[i] = Biquad::peaking(self.fs, EQ_FREQS[i], 1.1, p.eq_db[i]);
        }
        let b = p.balance.clamp(-1.0, 1.0);
        self.lg = if b > 0.0 { 1.0 - b } else { 1.0 };
        self.rg = if b < 0.0 { 1.0 + b } else { 1.0 };
    }
}

impl<S> Iterator for Dsp<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        let mut s = self.inner.next()?;
        let ch = self.cur as usize;
        if self.eq_on {
            let st = &mut self.state[ch];
            for i in 0..10 {
                s = self.coeffs[i].process(&mut st[i], s);
            }
            s = s.clamp(-1.0, 1.0);
        }
        if self.ch >= 2 {
            if ch == 0 {
                s *= self.lg;
            } else if ch == 1 {
                s *= self.rg;
            }
        }
        if ch == 0 {
            if let Ok(mut v) = self.shared.viz.try_lock() {
                v.push_back(s);
                if v.len() > 2048 {
                    v.pop_front();
                }
            }
        }
        self.cur = (self.cur + 1) % self.ch;
        self.n += 1;
        if self.n % 2048 == 0 {
            self.reload();
        }
        Some(s)
    }
}

impl<S> Source for Dsp<S>
where
    S: Source<Item = f32>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let r = self.inner.try_seek(pos);
        if r.is_ok() {
            // clear biquad history so the new position doesn't pop
            for st in self.state.iter_mut() {
                *st = [BqState::default(); 10];
            }
        }
        r
    }
}

/// Commands the UI sends to the engine thread.
pub enum Cmd {
    SetQueue { tracks: Vec<Track>, start: usize },
    EnqueueEnd(Vec<Track>),
    RadioReplace(Vec<Track>),
    PlayNext(Track),
    JumpTo(usize),
    RemoveAt(usize),
    Move(usize, usize), // reorder queue item from -> to
    Toggle,
    Pause,
    Next,
    Prev,
    Seek(f32),
    SetVolume(f32),
    Shuffle,
    CycleRepeat,
    SetSleep(Option<f32>), // seconds from now, or None to cancel
    SetDevice(Option<String>), // None = auto/system default
    Restore { tracks: Vec<Track>, index: usize, pos: f32 }, // load paused at pos
    Clear,
}

/// Snapshot of playback state, cloned by the UI each frame (cheap enough).
#[derive(Clone, Default)]
pub struct Shared {
    pub queue: Vec<Track>,
    pub index: usize,
    pub current: Option<Track>,
    pub playing: bool,
    pub position: f32,
    pub duration: f32,
    pub volume: f32,
    pub repeat: u8, // 0 = off, 1 = all, 2 = one
    pub sleep_left: Option<f32>, // seconds until sleep stops playback
    pub devices: Vec<String>,
    pub device: String,
    pub device_auto: bool,
    pub status: String,
}

pub struct Handle {
    pub tx: std::sync::mpsc::Sender<Cmd>,
    pub shared: Arc<Mutex<Shared>>,
    pub dsp: Arc<DspShared>,
}

/// Spawn the engine thread and return a handle for the UI.
pub fn spawn(api: ApiClient, ctx: eframe::egui::Context) -> Handle {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let shared = Arc::new(Mutex::new(Shared {
        volume: 0.7, // web parity: default to 70%
        ..Default::default()
    }));
    let shared_for_thread = shared.clone();
    let dsp = DspShared::new();
    let dsp_for_thread = dsp.clone();

    thread::spawn(move || {
        let (stream, handle, dev_name) = match open_stream(&None) {
            Some(x) => x,
            None => {
                if let Ok(mut s) = shared_for_thread.lock() {
                    s.status = "No audio output device".into();
                }
                ctx.request_repaint();
                return;
            }
        };
        let mut engine = Engine {
            _stream: stream,
            handle,
            sink: None,
            queue: Vec::new(),
            index: 0,
            volume: 0.7, // web parity: default to 70% (overridden by a restored session)
            active: false,
            duration: 0.0,
            repeat: 0,
            sleep_at: None,
            device_pref: None,
            cur_device: dev_name,
            devices: Vec::new(),
            last_dev_poll: std::time::Instant::now() - Duration::from_secs(10),
            dsp: dsp_for_thread,
            api,
            shared: shared_for_thread,
            ctx,
        };

        loop {
            match rx.recv_timeout(Duration::from_millis(120)) {
                Ok(cmd) => engine.handle(cmd),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            // auto-advance when the current track finished on its own
            if engine.active {
                if let Some(sink) = &engine.sink {
                    if sink.empty() {
                        engine.advance_ended();
                    }
                }
            }
            // sleep timer
            if let Some(at) = engine.sleep_at {
                if std::time::Instant::now() >= at {
                    engine.stop_all();
                    engine.sleep_at = None;
                }
            }
            // device refresh + auto-switch (e.g. Bluetooth plugged in)
            if engine.last_dev_poll.elapsed() >= Duration::from_secs(3) {
                engine.poll_devices();
            }
            engine.publish();
        }
    });

    Handle { tx, shared, dsp }
}

struct Engine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    device_pref: Option<String>,
    cur_device: String,
    devices: Vec<String>,
    last_dev_poll: std::time::Instant,
    sink: Option<Sink>,
    queue: Vec<Track>,
    index: usize,
    volume: f32,
    active: bool,
    duration: f32,
    repeat: u8,
    sleep_at: Option<std::time::Instant>,
    dsp: Arc<DspShared>,
    api: ApiClient,
    shared: Arc<Mutex<Shared>>,
    ctx: eframe::egui::Context,
}

impl Engine {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetQueue { tracks, start } => {
                self.queue = tracks;
                let start = start.min(self.queue.len().saturating_sub(1));
                if self.queue.is_empty() {
                    self.stop_all();
                } else {
                    self.start(start);
                }
            }
            Cmd::EnqueueEnd(mut tracks) => {
                let was_empty = self.queue.is_empty();
                self.queue.append(&mut tracks);
                if was_empty && !self.queue.is_empty() {
                    self.start(0);
                }
            }
            Cmd::RadioReplace(tracks) => {
                // sonic radio: keep the current track playing, replace everything below it
                let keep = (self.index + 1).min(self.queue.len());
                self.queue.truncate(keep);
                self.queue.extend(tracks);
                if !self.active && !self.queue.is_empty() {
                    let i = self.index.min(self.queue.len() - 1);
                    self.start(i);
                }
            }
            Cmd::PlayNext(t) => {
                let at = (self.index + 1).min(self.queue.len());
                self.queue.insert(at, t);
                if !self.active && !self.queue.is_empty() {
                    self.start(self.index);
                }
            }
            Cmd::JumpTo(i) => {
                if i < self.queue.len() {
                    self.start(i);
                }
            }
            Cmd::RemoveAt(i) => self.remove_at(i),
            Cmd::Move(from, to) => self.move_item(from, to),
            Cmd::Toggle => self.toggle(),
            Cmd::Pause => {
                if let Some(sink) = &self.sink {
                    sink.pause();
                }
            }
            Cmd::Next => self.next_manual(),
            Cmd::Prev => self.prev(),
            Cmd::Seek(s) => {
                if let Some(sink) = &self.sink {
                    let _ = sink.try_seek(Duration::from_secs_f32(s.max(0.0)));
                }
            }
            Cmd::SetVolume(v) => {
                self.volume = v.clamp(0.0, 2.0);
                if let Some(sink) = &self.sink {
                    sink.set_volume(self.volume);
                }
            }
            Cmd::Shuffle => self.shuffle_rest(),
            Cmd::CycleRepeat => self.repeat = (self.repeat + 1) % 3,
            Cmd::SetSleep(secs) => {
                self.sleep_at = secs.map(|s| std::time::Instant::now() + Duration::from_secs_f32(s));
            }
            Cmd::SetDevice(pref) => self.switch_device(pref),
            Cmd::Restore { tracks, index, pos } => {
                if !tracks.is_empty() {
                    self.queue = tracks;
                    let idx = index.min(self.queue.len() - 1);
                    self.start_opts(idx, false, pos);
                }
            }
            Cmd::Clear => {
                self.queue.clear();
                self.index = 0;
                self.stop_all();
            }
        }
    }

    /// Shuffle the queue items AFTER the current track (keeps what's playing).
    fn shuffle_rest(&mut self) {
        let start = self.index + 1;
        if self.queue.len().saturating_sub(start) < 2 {
            return;
        }
        // tiny LCG seeded from the clock (runtime randomness; no extra deps)
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in (start + 1..self.queue.len()).rev() {
            let j = start + (rng() as usize) % (i - start + 1);
            self.queue.swap(i, j);
        }
    }

    fn start(&mut self, idx: usize) {
        self.start_opts(idx, true, 0.0);
    }

    fn poll_devices(&mut self) {
        self.last_dev_poll = std::time::Instant::now();
        self.devices = list_devices();
        if self.device_pref.is_none() {
            let def = default_device_name();
            if !def.is_empty() && def != self.cur_device {
                self.switch_device(None); // follow the new system default (e.g. Bluetooth)
            }
        }
    }

    /// Rebuild the output stream on a new device, preserving the current track,
    /// position and play/pause state.
    fn switch_device(&mut self, pref: Option<String>) {
        let pos = self.sink.as_ref().map(|s| s.get_pos().as_secs_f32()).unwrap_or(0.0);
        let was_playing = self.active && self.sink.as_ref().map(|s| !s.is_paused()).unwrap_or(false);
        if let Some((stream, handle, name)) = open_stream(&pref) {
            self.sink = None; // drop the sink bound to the old device
            self._stream = stream;
            self.handle = handle;
            self.cur_device = name.clone();
            self.device_pref = pref;
            if self.active && self.index < self.queue.len() {
                self.start_opts(self.index, was_playing, pos);
            }
            self.set_status(format!("Output → {name}"));
        } else {
            self.set_status("Couldn't open that output device".into());
        }
    }

    /// Begin playback of queue[idx]: fetch meta + bytes, decode. `autoplay`
    /// controls play vs. paused (paused is used for session restore + device
    /// switching), `seek_to` resumes from a position.
    fn start_opts(&mut self, idx: usize, autoplay: bool, seek_to: f32) {
        if idx >= self.queue.len() {
            self.stop_all();
            return;
        }
        self.index = idx;
        let t = self.queue[idx].clone();
        self.set_status(format!("Loading: {} — {}", t.artist, t.title));
        self.publish();

        // duration: prefer the API's reported duration, fall back to the decoder
        let mut dur = self
            .api
            .track_meta(&t.rating_key)
            .ok()
            .and_then(|m| m.duration_ms)
            .map(|ms| ms as f32 / 1000.0)
            .unwrap_or(0.0);

        let bytes = match self.api.stream_bytes(&t.rating_key) {
            Ok(b) => b,
            Err(e) => {
                self.set_status(format!("Stream failed ({}): {e}", t.title));
                self.advance_ended();
                return;
            }
        };

        let src = match Decoder::new(Cursor::new(bytes)) {
            Ok(s) => s,
            Err(e) => {
                self.set_status(format!("Decode failed ({}): {e}", t.title));
                self.advance_ended();
                return;
            }
        };
        if dur <= 0.0 {
            if let Some(d) = src.total_duration() {
                dur = d.as_secs_f32();
            }
        }

        let sink = match Sink::try_new(&self.handle) {
            Ok(s) => s,
            Err(e) => {
                self.set_status(format!("Audio sink error: {e}"));
                return;
            }
        };
        sink.set_volume(self.volume);
        // insert the native DSP stage (EQ + balance + visualizer tap)
        let processed = Dsp::new(src.convert_samples::<f32>(), self.dsp.clone());
        sink.append(processed);
        if seek_to > 0.5 {
            let _ = sink.try_seek(Duration::from_secs_f32(seek_to));
        }
        if autoplay {
            sink.play();
        } else {
            sink.pause();
        }
        self.sink = Some(sink);
        self.active = true;
        self.duration = dur;
        self.set_status(String::new());

        if autoplay {
            // best-effort history log (mirrors the web player)
            let api = self.api.clone();
            let tt = t.clone();
            thread::spawn(move || api.log_play(&tt));
        }
    }

    fn toggle(&mut self) {
        if let Some(sink) = &self.sink {
            if sink.is_paused() {
                sink.play();
            } else {
                sink.pause();
            }
        } else if !self.queue.is_empty() {
            self.start(self.index);
        }
    }

    /// User pressed Next.
    fn next_manual(&mut self) {
        if self.index + 1 < self.queue.len() {
            self.start(self.index + 1);
        } else if self.repeat == 1 && !self.queue.is_empty() {
            self.start(0);
        } else {
            self.stop_all();
        }
    }

    /// Track ended by itself → advance, honoring repeat mode.
    fn advance_ended(&mut self) {
        if self.repeat == 2 {
            // repeat-one: replay current
            self.start(self.index);
        } else if self.index + 1 < self.queue.len() {
            self.start(self.index + 1);
        } else if self.repeat == 1 && !self.queue.is_empty() {
            self.start(0);
        } else {
            self.active = false;
            self.sink = None;
        }
    }

    fn prev(&mut self) {
        let pos = self
            .sink
            .as_ref()
            .map(|s| s.get_pos().as_secs_f32())
            .unwrap_or(0.0);
        if pos > 3.0 {
            if let Some(sink) = &self.sink {
                let _ = sink.try_seek(Duration::from_secs(0));
            }
        } else if self.index > 0 {
            self.start(self.index - 1);
        } else if let Some(sink) = &self.sink {
            let _ = sink.try_seek(Duration::from_secs(0));
        }
    }

    fn remove_at(&mut self, i: usize) {
        if i >= self.queue.len() {
            return;
        }
        let removing_current = i == self.index;
        self.queue.remove(i);
        if self.queue.is_empty() {
            self.index = 0;
            self.stop_all();
            return;
        }
        if removing_current {
            // current shifted out; play whatever now sits at this slot (clamped)
            let next = self.index.min(self.queue.len() - 1);
            self.start(next);
        } else if i < self.index {
            self.index -= 1;
        }
    }

    fn move_item(&mut self, from: usize, to: usize) {
        let n = self.queue.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let cur_rk = self.queue.get(self.index).map(|t| t.rating_key.clone());
        let item = self.queue.remove(from);
        self.queue.insert(to, item);
        if let Some(rk) = cur_rk {
            if let Some(pos) = self.queue.iter().position(|t| t.rating_key == rk) {
                self.index = pos;
            }
        }
    }

    fn stop_all(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.active = false;
        self.duration = 0.0;
    }

    fn set_status(&mut self, msg: String) {
        if let Ok(mut s) = self.shared.lock() {
            s.status = msg;
        }
    }

    fn publish(&self) {
        let position = self
            .sink
            .as_ref()
            .map(|s| s.get_pos().as_secs_f32())
            .unwrap_or(0.0);
        let playing = self
            .sink
            .as_ref()
            .map(|s| self.active && !s.is_paused() && !s.empty())
            .unwrap_or(false);
        let current = self.queue.get(self.index).cloned();
        if let Ok(mut s) = self.shared.lock() {
            s.queue = self.queue.clone();
            s.index = self.index;
            s.current = current;
            s.playing = playing;
            s.position = position;
            s.duration = self.duration;
            s.volume = self.volume;
            s.repeat = self.repeat;
            s.sleep_left = self
                .sleep_at
                .map(|a| a.saturating_duration_since(std::time::Instant::now()).as_secs_f32());
            s.devices = self.devices.clone();
            s.device = self.cur_device.clone();
            s.device_auto = self.device_pref.is_none();
        }
        self.ctx.request_repaint();
    }
}
