use std::collections::VecDeque;
use std::io::Cursor;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{
    Color32, Context, CornerRadius, FontDefinitions, FontFamily, FontId, Frame, RichText, Sense,
    TextureHandle, TextureOptions, Ui, ViewportId, vec2,
};
use egui_wgpu::RendererOptions;
use egui_wgpu::winit::Painter;
use egui_winit::State;
use lofty::picture::PictureType;
use lofty::prelude::*;
use play_core::{Track, TransportState, Workspace, WorkspaceMode};
use play_media::{
    ChordEstimate, ChordQuality, PitchEstimate, SPECTRUM_BANDS, SPECTRUM_MAX_FREQUENCY,
    SPECTRUM_MIN_FREQUENCY, SpectrumAnalyzer, estimate_bpm_from_samples,
};
use play_ui::Scene;
use rodio::Source;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use soundtouch::{Setting as SoundTouchSetting, SoundTouch};
use std::sync::atomic::{AtomicBool, Ordering};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SPECTRUM_HEIGHT: f32 = 250.0;
const SPECTRUM_DOUBLE_GRAPH_WIDTH: f32 = 1_200.0;
const SPECTRUM_QUADRUPLE_GRAPH_WIDTH: f32 = 2_000.0;
const PLAYER_CONTENT_GAP: f32 = 8.0;
const TRACK_LIST_INNER_MARGIN: f32 = 16.0;
const TRACK_INFO_HEIGHT: f32 = 152.0;
const TRACK_HEADER_HEIGHT: f32 = 22.0;
const TRACK_ROW_HEIGHT: f32 = 26.0;
const SPECTRUM_FREQUENCY_HEIGHT: f32 = 36.0;
const TRACK_COLUMN_COUNT: usize = 9;
const LED_COUNT: usize = 24;
const SPECTRUM_WINDOW_FRAMES: usize = 16_384;
const METER_WINDOW_FRAMES: usize = 2_048;
const PITCH_WINDOW_FRAMES: usize = 4_096;
const ROOM_SPECTRUM_BANDS: usize = 256;
const ANALYSIS_BUFFER_FRAMES: usize = 16384;
const TABLATURE_NOTE_REPEAT_SECONDS: f32 = 0.35;
const TABLATURE_NOTE_SPACING_PX: f32 = 46.0;
const TABLATURE_HISTORY_LIMIT: usize = 256;
const FADER_MIN_DB: f32 = -60.0;
const FADER_MAX_DB: f32 = 10.0;
const FADER_DEFAULT_VALUE: f32 = 60.0 / 70.0;
const SPECTRUM_FREQUENCY_LABELS: [(usize, &str); 10] = [
    (0, "16 Hz"),
    (16, "50"),
    (25, "100"),
    (35, "200"),
    (47, "500"),
    (57, "1 kHz"),
    (66, "2 kHz"),
    (79, "5 kHz"),
    (89, "10 kHz"),
    (100, "23 kHz"),
];

const MODE_BUTTONS: [(WorkspaceMode, &str, &str); 5] = [
    (WorkspaceMode::Player, "Player", "\u{e405}"),
    (WorkspaceMode::RoomAnalyzer, "Room analyzer", "\u{e01d}"),
    (WorkspaceMode::LiveTablature, "Live Tablature", "\u{e8b0}"),
    (WorkspaceMode::BackingTrack, "BackingTrack", "\u{e3a1}"),
    (WorkspaceMode::Ripper, "Ripper", "\u{e2c6}"),
];

fn setup_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "free_sans".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/FreeSans.ttf"
        ))),
    );
    fonts.font_data.insert(
        "material_icons".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/MaterialIcons-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "material_design_icons".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/materialdesignicons-webfont.ttf"
        ))),
    );
    fonts.font_data.insert(
        "font_awesome".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/fa-solid-900.ttf"
        ))),
    );

    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "free_sans".to_owned());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "free_sans".to_owned());
    fonts
        .families
        .insert(FontFamily::Name("material_icons".into()), vec!["material_icons".to_owned()]);
    fonts.families.insert(
        FontFamily::Name("material_design_icons".into()),
        vec!["material_design_icons".to_owned()],
    );
    fonts.families.insert(FontFamily::Name("font_awesome".into()), vec!["font_awesome".to_owned()]);
    ctx.set_fonts(fonts);
}

struct ToolbarTextures {
    normal: TextureHandle,
    pressed: TextureHandle,
    menu_normal: TextureHandle,
    menu_pressed: TextureHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
enum RepeatMode {
    Track,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
enum ChainMode {
    Normal,
    VolumeFader,
    TempoFader,
    VolumeTempoFader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TablatureDisplayMode {
    Tablature,
    Staff,
    Both,
}

enum PendingBackingAction {
    NewProject,
    LoadProject,
    SelectMode(WorkspaceMode),
    CloseApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MicrophoneProfile {
    Flat,
    BehringerEcm8000,
    ShureBeta58a,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveAnalysisSource {
    Microphone,
    PlaybackOutput,
}

impl LiveAnalysisSource {
    fn label(self) -> &'static str {
        match self {
            Self::Microphone => "Microphone / entrée Scarlett",
            Self::PlaybackOutput => "Sortie de la piste Play",
        }
    }
}

impl MicrophoneProfile {
    const ALL: [Self; 3] = [Self::Flat, Self::BehringerEcm8000, Self::ShureBeta58a];

    fn label(self) -> &'static str {
        match self {
            Self::Flat => "Aucune correction",
            Self::BehringerEcm8000 => "Behringer ECM8000 — courbe nominale",
            Self::ShureBeta58a => "Shure Beta 58A — micro dynamique",
        }
    }
}

impl TablatureDisplayMode {
    fn label(self) -> &'static str {
        match self {
            Self::Tablature => "Tablature",
            Self::Staff => "Portée",
            Self::Both => "Portée + tablature",
        }
    }
}

impl ChainMode {
    fn has_volume_fader(self) -> bool {
        matches!(self, Self::VolumeFader | Self::VolumeTempoFader)
    }

    fn has_tempo_fader(self) -> bool {
        matches!(self, Self::TempoFader | Self::VolumeTempoFader)
    }

    fn from_faders(volume: bool, tempo: bool) -> Self {
        match (volume, tempo) {
            (true, true) => Self::VolumeTempoFader,
            (true, false) => Self::VolumeFader,
            (false, true) => Self::TempoFader,
            (false, false) => Self::Normal,
        }
    }
}

#[derive(Clone)]
struct BackingStem {
    name: String,
    role: String,
    path: Option<PathBuf>,
    reference: bool,
    sync_offset_seconds: f32,
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
}

#[derive(Clone, Copy)]
struct BackingStemMix {
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
}

type BackingWaveformResult = (PathBuf, Option<(Vec<f32>, f32)>);

enum BackingSyncMessage {
    Progress { completed: usize, total: usize },
    Finished(Result<Vec<Option<f32>>, String>),
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct SavedBackingStem {
    name: String,
    role: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    reference: bool,
    #[serde(default)]
    sync_offset_seconds: f32,
    #[serde(default = "default_backing_volume")]
    volume: f32,
    #[serde(default)]
    pan: f32,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    solo: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct SavedBackingProject {
    #[serde(default)]
    name: String,
    #[serde(default)]
    backing_stems: Vec<SavedBackingStem>,
    #[serde(default = "default_backing_bpm")]
    backing_bpm: f32,
    #[serde(default = "default_backing_bpm")]
    backing_original_bpm: f32,
    #[serde(default)]
    position_seconds: f32,
    #[serde(default = "default_stereo_faders")]
    stereo_faders: [f32; 2],
}

fn default_backing_volume() -> f32 {
    FADER_DEFAULT_VALUE
}

fn default_backing_bpm() -> f32 {
    120.0
}

fn default_stereo_faders() -> [f32; 2] {
    [FADER_DEFAULT_VALUE; 2]
}

fn fader_to_db(value: f32) -> f32 {
    FADER_MIN_DB + value.clamp(0.0, 1.0) * (FADER_MAX_DB - FADER_MIN_DB)
}

fn format_fader_db(decibels: f32) -> String {
    if decibels > 0.0 { format!("+{decibels:.0}") } else { format!("{decibels:.0}") }
}

fn fader_to_gain(value: f32) -> f32 {
    10.0_f32.powf(fader_to_db(value) / 20.0)
}

impl BackingStem {
    fn mix(&self) -> BackingStemMix {
        BackingStemMix { volume: self.volume, pan: self.pan, muted: self.muted, solo: self.solo }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
enum TrackColumn {
    Number,
    Title,
    Artist,
    Album,
    Year,
    Duration,
    Bpm,
    Format,
    File,
}

impl TrackColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Number => "#",
            Self::Title => "Titre",
            Self::Artist => "Artiste",
            Self::Album => "Album",
            Self::Year => "Année",
            Self::Duration => "Durée",
            Self::Bpm => "BPM",
            Self::Format => "Format",
            Self::File => "Fichier",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Number => 0,
            Self::Title => 1,
            Self::Artist => 2,
            Self::Album => 3,
            Self::Year => 4,
            Self::Duration => 5,
            Self::Bpm => 6,
            Self::Format => 7,
            Self::File => 8,
        }
    }

    fn default_order() -> Vec<Self> {
        vec![
            Self::Number,
            Self::Title,
            Self::Artist,
            Self::Album,
            Self::Year,
            Self::Duration,
            Self::Bpm,
            Self::Format,
            Self::File,
        ]
    }

    fn default_visibility() -> [bool; TRACK_COLUMN_COUNT] {
        [true, true, true, true, true, true, true, true, false]
    }

    fn width(self) -> f32 {
        match self {
            Self::Number => 48.0,
            Self::Title => 250.0,
            Self::Artist => 180.0,
            Self::Album => 190.0,
            Self::Year => 72.0,
            Self::Duration => 82.0,
            Self::Bpm => 72.0,
            Self::Format => 210.0,
            Self::File => 240.0,
        }
    }

    fn alignment(self) -> egui::Align {
        match self {
            Self::Title | Self::Artist | Self::Album | Self::File => egui::Align::Min,
            Self::Number | Self::Year | Self::Duration | Self::Bpm | Self::Format => {
                egui::Align::Center
            }
        }
    }
}

struct LiveAnalysisBuffer {
    samples: VecDeque<f32>,
    channels: usize,
    sample_rate: f32,
}

struct BackingStemSource<I> {
    input: I,
    settings: Arc<Mutex<BackingStemMix>>,
    solo_active: Arc<AtomicBool>,
    master_gains: Arc<Mutex<[f32; 2]>>,
    levels: Arc<Mutex<[f32; 2]>>,
    analysis: Arc<Mutex<LiveAnalysisBuffer>>,
    pending_analysis: Vec<f32>,
    channel: usize,
}

struct BackingStemOffsetSource<I> {
    input: I,
    delay_samples: usize,
    skip_samples: usize,
    channels: rodio::ChannelCount,
    sample_rate: rodio::SampleRate,
}

const BACKING_TIME_STRETCH_CHUNK_FRAMES: usize = 4096;

struct BackingTimeStretchSource<I> {
    input: I,
    processor: SoundTouch,
    output: VecDeque<f32>,
    input_buffer: Vec<f32>,
    process_buffer: Vec<f32>,
    channels: rodio::ChannelCount,
    sample_rate: rodio::SampleRate,
    input_finished: bool,
    flushed: bool,
    tempo_ratio: f32,
}

impl<I> BackingTimeStretchSource<I>
where
    I: rodio::Source<Item = f32>,
{
    fn new(
        input: I,
        channels: rodio::ChannelCount,
        sample_rate: rodio::SampleRate,
        tempo_ratio: f32,
    ) -> Self {
        let channel_count = channels.get() as usize;
        let tempo_ratio = tempo_ratio.clamp(0.5, 2.0);
        let mut processor = SoundTouch::new();
        processor
            .set_channels(channels.get() as u32)
            .set_sample_rate(sample_rate.get())
            .set_tempo(tempo_ratio as f64)
            .set_setting(SoundTouchSetting::UseQuickseek, 1);
        Self {
            input,
            processor,
            output: VecDeque::new(),
            input_buffer: Vec::with_capacity(BACKING_TIME_STRETCH_CHUNK_FRAMES * channel_count),
            process_buffer: vec![0.0; BACKING_TIME_STRETCH_CHUNK_FRAMES * channel_count],
            channels,
            sample_rate,
            input_finished: false,
            flushed: false,
            tempo_ratio,
        }
    }

    fn drain_processor(&mut self) {
        let channel_count = self.channels.get() as usize;
        let max_frames = self.process_buffer.len() / channel_count.max(1);
        loop {
            let frames = self.processor.receive_samples(&mut self.process_buffer, max_frames);
            if frames == 0 {
                break;
            }
            self.output.extend(self.process_buffer[..frames * channel_count].iter().copied());
            if frames < max_frames {
                break;
            }
        }
    }
}

impl<I> Iterator for BackingTimeStretchSource<I>
where
    I: rodio::Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.output.pop_front() {
                return Some(sample);
            }

            if !self.input_finished {
                self.input_buffer.clear();
                while self.input_buffer.len()
                    < BACKING_TIME_STRETCH_CHUNK_FRAMES * self.channels.get() as usize
                {
                    let Some(sample) = self.input.next() else {
                        break;
                    };
                    self.input_buffer.push(sample);
                }
                let channel_count = self.channels.get() as usize;
                let input_frames = self.input_buffer.len() / channel_count.max(1);
                if input_frames > 0 {
                    self.processor.put_samples(&self.input_buffer, input_frames);
                    self.drain_processor();
                    continue;
                }
                self.input_finished = true;
            }

            if !self.flushed {
                self.processor.flush();
                self.flushed = true;
                self.drain_processor();
                continue;
            }

            return None;
        }
    }
}

impl<I> rodio::Source for BackingTimeStretchSource<I>
where
    I: rodio::Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input
            .total_duration()
            .map(|duration| Duration::from_secs_f32(duration.as_secs_f32() / self.tempo_ratio))
    }
}

impl<I> BackingStemOffsetSource<I> {
    fn new(
        input: I,
        offset_seconds: f32,
        channels: rodio::ChannelCount,
        sample_rate: rodio::SampleRate,
    ) -> Self {
        let samples =
            (offset_seconds.abs().max(0.0) * sample_rate.get() as f32 * channels.get() as f32)
                .round() as usize;
        if offset_seconds >= 0.0 {
            Self { input, delay_samples: 0, skip_samples: samples, channels, sample_rate }
        } else {
            Self { input, delay_samples: samples, skip_samples: 0, channels, sample_rate }
        }
    }
}

impl<I> Iterator for BackingStemOffsetSource<I>
where
    I: Iterator<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.delay_samples > 0 {
            self.delay_samples -= 1;
            return Some(0.0);
        }
        while self.skip_samples > 0 {
            self.input.next()?;
            self.skip_samples -= 1;
        }
        self.input.next()
    }
}

impl<I> rodio::Source for BackingStemOffsetSource<I>
where
    I: rodio::Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration().map(|duration| {
            if self.delay_samples > 0 {
                duration
                    + Duration::from_secs_f32(
                        self.delay_samples as f32
                            / self.sample_rate.get() as f32
                            / self.channels.get() as f32,
                    )
            } else {
                duration
            }
        })
    }
}

impl<I> Iterator for BackingStemSource<I>
where
    I: rodio::Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.input.next()?;
        let channel = self.channel;
        self.channel = (self.channel + 1) % self.input.channels().get() as usize;
        let settings = self.settings.lock().map(|settings| *settings).unwrap_or(BackingStemMix {
            volume: FADER_DEFAULT_VALUE,
            pan: 0.0,
            muted: false,
            solo: false,
        });
        let audible =
            !settings.muted && (!self.solo_active.load(Ordering::Relaxed) || settings.solo);
        if !audible {
            return Some(0.0);
        }
        let gain = fader_to_gain(settings.volume);
        let master_gain =
            self.master_gains.lock().map(|gains| gains[channel.min(1)]).unwrap_or(1.0);
        let pan = settings.pan.clamp(-1.0, 1.0);
        let pan_gain = if channel == 0 {
            if pan > 0.0 { 1.0 - pan } else { 1.0 }
        } else if pan < 0.0 {
            1.0 + pan
        } else {
            1.0
        };
        let output = sample * gain * pan_gain * master_gain;
        if let Ok(mut levels) = self.levels.lock() {
            levels[channel.min(1)] = levels[channel.min(1)].max(output.abs());
        }
        self.pending_analysis.push(output);
        if self.pending_analysis.len() >= 2 * 512 {
            if let Ok(mut analysis) = self.analysis.lock() {
                analysis.push_samples(&self.pending_analysis);
            }
            self.pending_analysis.clear();
        }
        Some(output)
    }
}

impl<I> rodio::Source for BackingStemSource<I>
where
    I: rodio::Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }
}

struct MicrophoneAnalysisSource {
    microphone: rodio::microphone::Microphone,
    buffer: Arc<Mutex<LiveAnalysisBuffer>>,
    pending: Vec<f32>,
    chunk_size: usize,
    channels: rodio::ChannelCount,
    sample_rate: rodio::SampleRate,
    mono_enabled: bool,
    output_samples: VecDeque<f32>,
    monitor_gains: Arc<Mutex<[f32; 2]>>,
    output_channel_index: usize,
}

impl MicrophoneAnalysisSource {
    fn new(
        microphone: rodio::microphone::Microphone,
        buffer: Arc<Mutex<LiveAnalysisBuffer>>,
        channels: rodio::ChannelCount,
        sample_rate: rodio::SampleRate,
        mono_enabled: bool,
        monitor_gains: Arc<Mutex<[f32; 2]>>,
    ) -> Self {
        let chunk_size = channels.get() as usize * 512;
        Self {
            microphone,
            buffer,
            pending: Vec::with_capacity(chunk_size),
            chunk_size,
            channels,
            sample_rate,
            mono_enabled,
            output_samples: VecDeque::new(),
            monitor_gains,
            output_channel_index: 0,
        }
    }

    fn apply_monitor_gain(&mut self, sample: rodio::Sample) -> rodio::Sample {
        let channel = self.output_channel_index.min(1);
        self.output_channel_index =
            (self.output_channel_index + 1) % self.channels.get().max(1) as usize;
        let gain = self.monitor_gains.lock().map(|gains| gains[channel]).unwrap_or(1.0);
        sample * gain
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        if let Ok(mut analysis) = self.buffer.lock() {
            analysis.push_samples(&self.pending);
        }
        self.pending.clear();
    }
}

impl Iterator for MicrophoneAnalysisSource {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(sample) = self.output_samples.pop_front() {
            return Some(self.apply_monitor_gain(sample));
        }
        if self.mono_enabled && self.channels.get() > 1 {
            let mut frame = Vec::with_capacity(self.channels.get() as usize);
            for _ in 0..self.channels.get() {
                let Some(sample) = self.microphone.next() else {
                    self.flush_pending();
                    return None;
                };
                frame.push(sample);
            }
            let mono = frame
                .iter()
                .copied()
                .max_by(|left, right| left.abs().total_cmp(&right.abs()))
                .unwrap_or(0.0);
            for _ in 0..self.channels.get() {
                self.output_samples.push_back(mono);
                self.pending.push(mono);
            }
            if self.pending.len() >= self.chunk_size {
                self.flush_pending();
            }
            return self.output_samples.pop_front().map(|sample| self.apply_monitor_gain(sample));
        }
        let sample = self.microphone.next();
        if let Some(sample) = sample {
            self.pending.push(sample);
            if self.pending.len() >= self.chunk_size {
                self.flush_pending();
            }
        } else {
            self.flush_pending();
        }
        sample.map(|sample| self.apply_monitor_gain(sample))
    }
}

impl rodio::Source for MicrophoneAnalysisSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[derive(Clone, Copy)]
struct TablatureNote {
    recorded_at: Instant,
    line: usize,
    fret: u8,
    midi_note: u8,
}

#[derive(Clone)]
struct AudioMetadata {
    format: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<u16>,
    bit_depth: Option<u8>,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
    bitrate_kbps: Option<u32>,
    bpm: Option<f32>,
    bpm_estimated: bool,
    genre: Option<String>,
}

impl LiveAnalysisBuffer {
    fn new(channels: usize, sample_rate: f32) -> Self {
        Self { samples: VecDeque::new(), channels, sample_rate }
    }

    fn push_samples(&mut self, samples: &[f32]) {
        self.samples.extend(samples.iter().copied());
        let maximum_samples = ANALYSIS_BUFFER_FRAMES * self.channels.max(1);
        while self.samples.len() > maximum_samples {
            self.samples.pop_front();
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
    }
}

struct ChannelGainSource<I> {
    input: I,
    gains: Arc<Mutex<[f32; 2]>>,
    analysis: Arc<Mutex<LiveAnalysisBuffer>>,
    pending_analysis: Vec<f32>,
    channels: usize,
    channel: usize,
}

impl<I> ChannelGainSource<I> {
    fn new(input: I, gains: Arc<Mutex<[f32; 2]>>, analysis: Arc<Mutex<LiveAnalysisBuffer>>) -> Self
    where
        I: rodio::Source,
    {
        let channels = input.channels().get() as usize;
        Self {
            input,
            gains,
            analysis,
            pending_analysis: Vec::with_capacity(channels * 512),
            channels,
            channel: 0,
        }
    }

    fn flush_analysis(&mut self) {
        if self.pending_analysis.is_empty() {
            return;
        }
        if let Ok(mut analysis) = self.analysis.lock() {
            analysis.push_samples(&self.pending_analysis);
        }
        self.pending_analysis.clear();
    }
}

impl<I> Iterator for ChannelGainSource<I>
where
    I: rodio::Source,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = match self.input.next() {
            Some(sample) => sample,
            None => {
                self.flush_analysis();
                return None;
            }
        };
        self.pending_analysis.push(sample);
        if self.pending_analysis.len() >= self.channels * 512 {
            self.flush_analysis();
        }
        let gains = self.gains.lock().map(|gains| *gains).unwrap_or([1.0; 2]);
        let gain = match self.channel {
            0 => gains[0],
            1 => gains[1],
            _ => (gains[0] + gains[1]) * 0.5,
        };
        self.channel = (self.channel + 1) % self.channels;
        Some(sample * gain)
    }
}

impl<I> rodio::Source for ChannelGainSource<I>
where
    I: rodio::Source,
{
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.input.try_seek(position)?;
        self.channel = 0;
        self.pending_analysis.clear();
        if let Ok(mut analysis) = self.analysis.lock() {
            analysis.clear();
        }
        Ok(())
    }
}

fn load_toolbar_texture(ctx: &Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let image = image::load_from_memory(bytes)
        .unwrap_or_else(|error| panic!("chargement de la texture {name}: {error}"))
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        TextureOptions::NEAREST,
    )
}

fn read_audio_cover(path: &Path) -> Option<Vec<u8>> {
    if let Ok(tagged_file) = lofty::read_from_path(path) {
        if let Some(picture) = tagged_file
            .tags()
            .iter()
            .flat_map(|tag| tag.pictures())
            .find(|picture| picture.pic_type() == PictureType::CoverFront)
            .or_else(|| tagged_file.tags().iter().flat_map(|tag| tag.pictures()).next())
        {
            return Some(picture.data().to_vec());
        }
    }

    let parent = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    let candidates = [
        parent.join("cover.jpg"),
        parent.join("cover.jpeg"),
        parent.join("cover.png"),
        parent.join("folder.jpg"),
        parent.join("folder.jpeg"),
        parent.join("folder.png"),
        parent.join("front.jpg"),
        parent.join("front.jpeg"),
        parent.join("front.png"),
        parent.join(format!("{stem}.jpg")),
        parent.join(format!("{stem}.jpeg")),
        parent.join(format!("{stem}.png")),
    ];
    candidates.into_iter().find_map(|candidate| std::fs::read(candidate).ok())
}

fn load_cover_texture(ctx: &Context, path: &Path, index: usize) -> Option<TextureHandle> {
    let bytes = read_audio_cover(path)?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ctx.load_texture(
        format!("track_cover_{index}"),
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        TextureOptions::LINEAR,
    ))
}

fn toolbar_icon_button(
    ui: &mut Ui,
    textures: &ToolbarTextures,
    icon: &str,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    toolbar_icon_button_with_font(
        ui,
        textures,
        icon,
        selected,
        tooltip,
        FontFamily::Name("material_icons".into()),
    )
}

fn toolbar_icon_button_with_font(
    ui: &mut Ui,
    textures: &ToolbarTextures,
    icon: &str,
    selected: bool,
    tooltip: &str,
    icon_family: FontFamily,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
    let active = selected || response.hovered() || response.is_pointer_button_down_on();
    let painter = ui.painter();
    paint_toolbar_button_surface(
        painter,
        rect,
        selected || response.is_pointer_button_down_on(),
        textures,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(20.0, icon_family),
        if active { Color32::YELLOW } else { Color32::WHITE },
    );
    response.on_hover_text(tooltip)
}

fn toolbar_label_button(
    ui: &mut Ui,
    textures: &ToolbarTextures,
    label: &str,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(48.0, 38.0), Sense::click());
    let active = selected || response.hovered() || response.is_pointer_button_down_on();
    paint_toolbar_button_surface(
        ui.painter(),
        rect,
        selected || response.is_pointer_button_down_on(),
        textures,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::new(11.0, FontFamily::Proportional),
        if active { Color32::YELLOW } else { Color32::WHITE },
    );
    response.on_hover_text(tooltip)
}

fn toolbar_menu_button(
    ui: &mut Ui,
    textures: &ToolbarTextures,
    icon: &str,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(82.0, 38.0), Sense::click());
    let popup_id = egui::Popup::default_response_id(&response);
    let active = response.hovered()
        || response.is_pointer_button_down_on()
        || egui::Popup::is_id_open(ui.ctx(), popup_id);
    let icon_color = if active { Color32::YELLOW } else { Color32::WHITE };
    let icon_rect =
        egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + 58.0, rect.bottom()));
    let arrow_rect =
        egui::Rect::from_min_max(egui::pos2(rect.left() + 58.0, rect.top()), rect.right_bottom());
    let texture = if active { &textures.menu_pressed } else { &textures.menu_normal };
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        icon_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(20.0, FontFamily::Name("material_icons".into())),
        icon_color,
    );
    ui.painter().text(
        arrow_rect.center(),
        egui::Align2::CENTER_CENTER,
        "▼",
        FontId::new(10.0, FontFamily::Proportional),
        icon_color,
    );
    response.on_hover_text(tooltip)
}

fn toolbar_separator(ui: &mut Ui) {
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

fn paint_toolbar_button_surface(
    painter: &egui::Painter,
    rect: egui::Rect,
    pressed: bool,
    textures: &ToolbarTextures,
) {
    let texture = if pressed { &textures.pressed } else { &textures.normal };
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

pub(crate) struct PlayDesktop {
    window: Option<Arc<Window>>,
    egui_context: Context,
    egui_state: Option<State>,
    painter: Option<Painter>,
    workspace: Workspace,
    scene: Scene,
    repeat_mode: Option<RepeatMode>,
    shuffle_enabled: bool,
    shuffle_state: u64,
    chain_mode: Option<ChainMode>,
    chain_transition_seconds: f32,
    click_chain_enabled: bool,
    show_music_loader: bool,
    show_room_measurement_dialog: bool,
    backing_stems: Vec<BackingStem>,
    backing_waveforms: Vec<Option<Vec<f32>>>,
    backing_durations: Vec<Option<f32>>,
    backing_waveform_ready: Vec<bool>,
    backing_waveform_sender: mpsc::Sender<BackingWaveformResult>,
    backing_waveform_results: mpsc::Receiver<BackingWaveformResult>,
    backing_sync_sender: mpsc::Sender<BackingSyncMessage>,
    backing_sync_results: mpsc::Receiver<BackingSyncMessage>,
    backing_sync_progress: Option<(usize, usize)>,
    backing_sync_resume_playing: bool,
    backing_waveform_loading: Vec<PathBuf>,
    backing_bpm: f32,
    backing_original_bpm: f32,
    backing_position_seconds: f32,
    backing_player: Option<rodio::Player>,
    backing_playback_origin_seconds: f32,
    backing_project_path: Option<PathBuf>,
    backing_project_name: String,
    backing_project_dirty: bool,
    pending_backing_action: Option<PendingBackingAction>,
    backing_mix_settings: Vec<Arc<Mutex<BackingStemMix>>>,
    backing_solo_active: Arc<AtomicBool>,
    backing_analysis_buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
    backing_levels: Arc<Mutex<[f32; 2]>>,
    loaded_music: Vec<PathBuf>,
    track_covers: Vec<Option<TextureHandle>>,
    track_cover_checked: Vec<bool>,
    track_metadata: Vec<Option<AudioMetadata>>,
    track_metadata_queued: Vec<bool>,
    track_column_order: Vec<TrackColumn>,
    track_column_visible: [bool; TRACK_COLUMN_COUNT],
    selected_track: Option<usize>,
    resume_position_seconds: f32,
    last_saved_position_seconds: f32,
    track_scroll_target: Option<usize>,
    audio_metadata: Option<AudioMetadata>,
    metadata_loading: Option<mpsc::Receiver<(PathBuf, AudioMetadata)>>,
    metadata_sender: Option<mpsc::Sender<(PathBuf, AudioMetadata)>>,
    metadata_pending: usize,
    audio_output: Option<rodio::MixerDeviceSink>,
    audio_output_name: Option<String>,
    audio_player: Option<rodio::Player>,
    pink_noise_player: Option<rodio::Player>,
    chain_previous_player: Option<rodio::Player>,
    chain_transition: Option<ChainMode>,
    chain_transition_started: Option<Instant>,
    chain_transition_trigger_position: Option<f32>,
    chain_tempo_start_speed: Option<f32>,
    chain_tempo_target_speed: Option<f32>,
    audio_error: Option<String>,
    stereo_faders: [f32; 2],
    stereo_link_drag_start: Option<[f32; 2]>,
    stereo_link_pointer_start: Option<f32>,
    channel_gains: Arc<Mutex<[f32; 2]>>,
    analysis_buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
    channel_levels: [f32; 2],
    meter_display_levels: [f32; 2],
    stereo_spectrum: [Vec<f32>; 2],
    room_spectrum: [f32; ROOM_SPECTRUM_BANDS],
    microphone_buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
    microphone_stop: Option<Arc<AtomicBool>>,
    microphone_thread: Option<JoinHandle<()>>,
    microphone_devices: Vec<rodio::microphone::Input>,
    microphone_device_index: usize,
    microphone_profile: MicrophoneProfile,
    live_analysis_source: LiveAnalysisSource,
    microphone_input_peak: f32,
    microphone_error: Option<String>,
    microphone_monitor_enabled: bool,
    microphone_mono_enabled: bool,
    microphone_monitor_output: Option<rodio::MixerDeviceSink>,
    microphone_monitor_player: Option<rodio::Player>,
    microphone_monitor_output_name: Option<String>,
    system_capture_stop: Option<Arc<AtomicBool>>,
    system_capture_thread: Option<JoinHandle<Result<usize, String>>>,
    system_capture_buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
    system_capture_source_name: Option<String>,
    system_capture_path: Option<PathBuf>,
    system_capture_started_at: Option<Instant>,
    system_capture_error: Option<String>,
    microphone_pitch: Option<PitchEstimate>,
    microphone_chord: Option<ChordEstimate>,
    live_tab_strumming: bool,
    metronome_enabled: bool,
    metronome_bpm: f32,
    tablature_playing: bool,
    tablature_recording: bool,
    tablature_pause_started: Option<Instant>,
    tablature_notes: VecDeque<TablatureNote>,
    tablature_last_position: Option<(usize, u8)>,
    tablature_last_recorded_at: Option<Instant>,
    tablature_measure_origin: Option<Instant>,
    tablature_display_mode: TablatureDisplayMode,
    toolbar_textures: ToolbarTextures,
}

mod backing_track;
mod bpm;
mod config;
mod core;
mod live_tablature;
mod player;
mod ripper;
mod room_analyzer;
mod sync;
mod ui;

pub(crate) use core::run;
