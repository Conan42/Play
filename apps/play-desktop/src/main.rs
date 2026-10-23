use std::collections::VecDeque;
use std::io::Cursor;
use std::num::NonZeroU32;
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
use std::sync::atomic::{AtomicBool, Ordering};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SPECTRUM_HEIGHT: f32 = 250.0;
const SPECTRUM_DOUBLE_GRAPH_WIDTH: f32 = 1_200.0;
const SPECTRUM_QUADRUPLE_GRAPH_WIDTH: f32 = 2_000.0;
const CONTENT_GAP: f32 = 12.0;
const PLAYER_CONTENT_GAP: f32 = 8.0;
const TRACK_LIST_INNER_MARGIN: f32 = 16.0;
const TRACK_INFO_HEIGHT: f32 = 152.0;
const TRACK_HEADER_HEIGHT: f32 = 22.0;
const TRACK_ROW_HEIGHT: f32 = 26.0;
const SPECTRUM_FREQUENCY_HEIGHT: f32 = 36.0;
const TRACK_COLUMN_COUNT: usize = 9;
const LED_COUNT: usize = 24;
const SPECTRUM_WINDOW_FRAMES: usize = 16_384;
const PITCH_WINDOW_FRAMES: usize = 4_096;
const BPM_ANALYSIS_SECONDS: usize = 120;
const ROOM_SPECTRUM_BANDS: usize = 256;
const ANALYSIS_BUFFER_FRAMES: usize = 16384;
const TABLATURE_NOTE_REPEAT_SECONDS: f32 = 0.35;
const TABLATURE_NOTE_SPACING_PX: f32 = 46.0;
const TABLATURE_HISTORY_LIMIT: usize = 256;
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

const MODE_BUTTONS: [(WorkspaceMode, &str, &str); 4] = [
    (WorkspaceMode::Player, "Player", "\u{e405}"),
    (WorkspaceMode::RoomAnalyzer, "Room analyzer", "\u{e01d}"),
    (WorkspaceMode::LiveTablature, "Live Tablature", "\u{e8b0}"),
    (WorkspaceMode::UiLab, "UI lab", "\u{e871}"),
];

fn setup_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "free_sans".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../../../assets/fonts/FreeSans.ttf"))),
    );
    fonts.font_data.insert(
        "material_icons".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/MaterialIcons-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "material_design_icons".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/materialdesignicons-webfont.ttf"
        ))),
    );
    fonts.font_data.insert(
        "font_awesome".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/fa-solid-900.ttf"
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

#[derive(serde::Deserialize, serde::Serialize)]
struct SavedPlaylist {
    paths: Vec<String>,
    #[serde(default)]
    column_order: Vec<TrackColumn>,
    #[serde(default)]
    visible_columns: Vec<TrackColumn>,
    #[serde(default)]
    layout_version: u32,
    #[serde(default)]
    selected_track: Option<usize>,
    #[serde(default)]
    position_seconds: f32,
    #[serde(default)]
    repeat_mode: Option<RepeatMode>,
    #[serde(default)]
    shuffle_enabled: bool,
    #[serde(default)]
    chain_mode: Option<ChainMode>,
    #[serde(default = "default_chain_transition_seconds")]
    chain_transition_seconds: f32,
    #[serde(default)]
    click_chain_enabled: bool,
}

fn default_chain_transition_seconds() -> f32 {
    30.0
}

fn playlist_storage_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    })?;
    Some(base.join("play").join("playlist.ron"))
}

struct LiveAnalysisBuffer {
    samples: VecDeque<f32>,
    channels: usize,
    sample_rate: f32,
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

fn basic_audio_metadata(
    path: &Path,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
) -> AudioMetadata {
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_uppercase())
        .unwrap_or_else(|| "AUDIO".to_owned());
    AudioMetadata {
        format,
        title: None,
        artist: None,
        album: None,
        year: None,
        bit_depth: None,
        sample_rate,
        channels,
        duration_seconds,
        bitrate_kbps: None,
        bpm: None,
        bpm_estimated: false,
        genre: None,
    }
}

fn read_audio_metadata(
    path: &Path,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
) -> AudioMetadata {
    let mut metadata = basic_audio_metadata(path, sample_rate, channels, duration_seconds);

    if let Ok(tagged_file) = lofty::read_from_path(path) {
        let properties = tagged_file.properties();
        metadata.title =
            tagged_file.tags().iter().find_map(|tag| tag.title().map(|value| value.into_owned()));
        metadata.artist =
            tagged_file.tags().iter().find_map(|tag| tag.artist().map(|value| value.into_owned()));
        metadata.album =
            tagged_file.tags().iter().find_map(|tag| tag.album().map(|value| value.into_owned()));
        metadata.year = tagged_file.tags().iter().find_map(|tag| tag.date().map(|date| date.year));
        metadata.bit_depth = properties.bit_depth();
        metadata.bitrate_kbps = properties.audio_bitrate();
        metadata.bpm = tagged_file.tags().iter().find_map(|tag| {
            tag.get_string(lofty::tag::ItemKey::Bpm)
                .or_else(|| tag.get_string(lofty::tag::ItemKey::IntegerBpm))
                .and_then(|value| value.replace(',', ".").parse::<f32>().ok())
        });
        metadata.genre = tagged_file
            .tags()
            .iter()
            .find_map(|tag| tag.get_string(lofty::tag::ItemKey::Genre).map(str::to_owned));
    }

    metadata
}

fn estimate_bpm_from_decoder<I>(decoder: I, sample_rate: u32, channels: usize) -> Option<f32>
where
    I: Iterator<Item = f32>,
{
    if sample_rate == 0 || channels == 0 {
        return None;
    }

    let max_samples =
        (sample_rate as usize).saturating_mul(BPM_ANALYSIS_SECONDS).saturating_mul(channels);
    let mut mono_samples =
        Vec::with_capacity((sample_rate as usize).saturating_mul(BPM_ANALYSIS_SECONDS));
    let mut channel_sum = 0.0_f32;
    let mut channel_count = 0_usize;

    for sample in decoder.take(max_samples) {
        channel_sum += sample;
        channel_count += 1;
        if channel_count == channels {
            mono_samples.push(channel_sum / channels as f32);
            channel_sum = 0.0;
            channel_count = 0;
        }
    }

    estimate_bpm_from_samples(&mono_samples, sample_rate as f32)
}

fn load_track_metadata(path: &Path) -> AudioMetadata {
    let mut sample_rate = 0;
    let mut channels = 0;
    let mut duration_seconds = 0.0;
    if let Ok(file) = std::fs::File::open(path)
        && let Ok(decoder) = rodio::Decoder::try_from(file)
    {
        sample_rate = decoder.sample_rate().get();
        channels = decoder.channels().get() as usize;
        duration_seconds =
            decoder.total_duration().map(|duration| duration.as_secs_f32()).unwrap_or(0.0);
    }
    let mut metadata = read_audio_metadata(path, sample_rate, channels, duration_seconds);
    if metadata.bpm.is_none()
        && let Ok(file) = std::fs::File::open(path)
        && let Ok(decoder) = rodio::Decoder::try_from(file)
        && let Some(bpm) = estimate_bpm_from_decoder(decoder, sample_rate, channels)
    {
        metadata.bpm = Some(bpm);
        metadata.bpm_estimated = true;
    }
    metadata
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

struct PlayDesktop {
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

impl PlayDesktop {
    fn new() -> Self {
        let egui_context = Context::default();
        setup_fonts(&egui_context);
        let toolbar_textures = ToolbarTextures {
            normal: load_toolbar_texture(
                &egui_context,
                "play_toolbar_button_normal",
                include_bytes!("../../../assets/texture/bouton_normal.png"),
            ),
            pressed: load_toolbar_texture(
                &egui_context,
                "play_toolbar_button_pressed",
                include_bytes!("../../../assets/texture/bouton_enfoncer.png"),
            ),
            menu_normal: load_toolbar_texture(
                &egui_context,
                "play_toolbar_menu_normal",
                include_bytes!("../../../assets/texture/menu_normal.png"),
            ),
            menu_pressed: load_toolbar_texture(
                &egui_context,
                "play_toolbar_menu_pressed",
                include_bytes!("../../../assets/texture/menu_enfoncer.png"),
            ),
        };

        let mut app = Self {
            window: None,
            egui_context,
            egui_state: None,
            painter: None,
            workspace: Workspace::default(),
            scene: Scene::default(),
            repeat_mode: None,
            shuffle_enabled: false,
            shuffle_state: 0x9e37_79b9_7f4a_7c15,
            chain_mode: None,
            chain_transition_seconds: 30.0,
            click_chain_enabled: false,
            show_music_loader: false,
            show_room_measurement_dialog: false,
            loaded_music: Vec::new(),
            track_covers: Vec::new(),
            track_cover_checked: Vec::new(),
            track_metadata: Vec::new(),
            track_metadata_queued: Vec::new(),
            track_column_order: TrackColumn::default_order(),
            track_column_visible: TrackColumn::default_visibility(),
            selected_track: None,
            resume_position_seconds: 0.0,
            last_saved_position_seconds: 0.0,
            track_scroll_target: None,
            audio_metadata: None,
            metadata_loading: None,
            metadata_sender: None,
            metadata_pending: 0,
            audio_output: None,
            audio_output_name: None,
            audio_player: None,
            pink_noise_player: None,
            chain_previous_player: None,
            chain_transition: None,
            chain_transition_started: None,
            chain_transition_trigger_position: None,
            chain_tempo_start_speed: None,
            chain_tempo_target_speed: None,
            audio_error: None,
            stereo_faders: [0.8; 2],
            stereo_link_drag_start: None,
            stereo_link_pointer_start: None,
            channel_gains: Arc::new(Mutex::new([0.251_188_64; 2])),
            analysis_buffer: None,
            channel_levels: [0.0; 2],
            stereo_spectrum: [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]],
            room_spectrum: [0.0; ROOM_SPECTRUM_BANDS],
            microphone_buffer: None,
            microphone_stop: None,
            microphone_thread: None,
            microphone_devices: Vec::new(),
            microphone_device_index: 0,
            microphone_profile: MicrophoneProfile::BehringerEcm8000,
            live_analysis_source: LiveAnalysisSource::Microphone,
            microphone_input_peak: 0.0,
            microphone_error: None,
            microphone_monitor_enabled: true,
            microphone_mono_enabled: false,
            microphone_monitor_output: None,
            microphone_monitor_player: None,
            microphone_monitor_output_name: None,
            microphone_pitch: None,
            microphone_chord: None,
            live_tab_strumming: false,
            metronome_enabled: false,
            metronome_bpm: 90.0,
            tablature_playing: true,
            tablature_recording: true,
            tablature_pause_started: None,
            tablature_notes: VecDeque::new(),
            tablature_last_position: None,
            tablature_last_recorded_at: None,
            tablature_measure_origin: None,
            tablature_display_mode: TablatureDisplayMode::Tablature,
            toolbar_textures,
        };
        app.restore_playlist();
        app
    }

    fn initialize_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Play")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 760.0))
                        .with_min_inner_size(winit::dpi::LogicalSize::new(900.0, 680.0)),
                )
                .expect("création de la fenêtre Play"),
        );
        let egui_state = State::new(
            self.egui_context.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let configuration = egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::from_display_handle(
                event_loop.owned_display_handle(),
            ),
            ..Default::default()
        };
        let mut painter = pollster::block_on(Painter::new(
            self.egui_context.clone(),
            configuration,
            false,
            RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
            .expect("initialisation du renderer wgpu");

        self.window = Some(window);
        self.egui_state = Some(egui_state);
        self.painter = Some(painter);
    }

    fn draw_spectrum(&self, ui: &mut egui::Ui) {
        let desired_size = egui::vec2(ui.available_width(), SPECTRUM_HEIGHT);
        let (response, painter) = ui.allocate_painter(desired_size, egui::Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 8.0, Color32::from_rgb(21, 25, 35));

        let content = rect.shrink2(vec2(8.0, 8.0));
        let frequency_height = SPECTRUM_FREQUENCY_HEIGHT;
        let graph = egui::Rect::from_min_max(
            egui::pos2(content.left() + 22.0, content.top()),
            egui::pos2(content.right(), content.bottom() - frequency_height),
        );
        painter.rect_filled(graph, 0.0, Color32::from_rgb(24, 38, 49));
        let axis_y = graph.center().y;
        painter.line_segment(
            [egui::pos2(graph.left(), axis_y), egui::pos2(graph.right(), axis_y)],
            egui::Stroke::new(1.0, Color32::from_rgb(65, 74, 84)),
        );
        painter.text(
            egui::pos2(content.left(), graph.center().y - graph.height() * 0.25),
            egui::Align2::LEFT_CENTER,
            "L",
            FontId::proportional(11.0),
            Color32::from_rgb(67, 183, 213),
        );
        painter.text(
            egui::pos2(content.left(), graph.center().y + graph.height() * 0.25),
            egui::Align2::LEFT_CENTER,
            "R",
            FontId::proportional(11.0),
            Color32::from_rgb(117, 97, 200),
        );

        let band_count = self.stereo_spectrum[0].len().min(self.stereo_spectrum[1].len()).max(1);
        let band_width = graph.width() / band_count as f32;
        let bar_width = (band_width - 1.0).max(1.0);
        let maximum_height = (graph.height() * 0.5 - 1.0).max(1.0);
        for band in 0..band_count {
            let x = graph.left() + band as f32 * band_width;
            let left_height =
                Self::spectrum_display_level(self.stereo_spectrum[0][band]) * maximum_height;
            let right_height =
                Self::spectrum_display_level(self.stereo_spectrum[1][band]) * maximum_height;
            if left_height > 0.0 {
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x, axis_y - left_height),
                        egui::pos2(x + bar_width, axis_y),
                    ),
                    0.0,
                    Color32::from_rgb(67, 183, 213),
                );
            }
            if right_height > 0.0 {
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x, axis_y),
                        egui::pos2(x + bar_width, axis_y + right_height),
                    ),
                    0.0,
                    Color32::from_rgb(117, 97, 200),
                );
            }
        }

        let frequency_rect = egui::Rect::from_min_max(
            egui::pos2(content.left() + 22.0, content.bottom() - frequency_height),
            content.right_bottom(),
        );
        for (percent, label) in SPECTRUM_FREQUENCY_LABELS {
            let normalized = percent as f32 / 100.0;
            let x = egui::lerp(frequency_rect.left()..=frequency_rect.right(), normalized);
            painter.line_segment(
                [egui::pos2(x, frequency_rect.top()), egui::pos2(x, frequency_rect.top() + 5.0)],
                egui::Stroke::new(1.0, Color32::from_gray(115)),
            );
            let (anchor, label_x) = if percent == 0 {
                (egui::Align2::LEFT_CENTER, x + 1.0)
            } else if percent == 100 {
                (egui::Align2::RIGHT_CENTER, x - 1.0)
            } else {
                (egui::Align2::CENTER_CENTER, x)
            };
            painter.text(
                egui::pos2(label_x, frequency_rect.center().y),
                anchor,
                label,
                FontId::proportional(10.0),
                Color32::from_gray(220),
            );
        }
    }

    fn draw_room_spectrum(&self, ui: &mut egui::Ui) {
        let desired_size = vec2(ui.available_width(), ui.available_height().max(320.0));
        let (response, painter) = ui.allocate_painter(desired_size, egui::Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 8.0, Color32::from_rgb(21, 25, 35));

        let content = rect.shrink2(vec2(8.0, 8.0));
        let frequency_height = SPECTRUM_FREQUENCY_HEIGHT;
        let graph = egui::Rect::from_min_max(
            egui::pos2(content.left() + 42.0, content.top()),
            egui::pos2(content.right(), content.bottom() - frequency_height),
        );
        painter.rect_filled(graph, 0.0, Color32::from_rgb(24, 38, 49));

        for step in 0..=5 {
            let level = step as f32 * 22.0;
            let y = graph.bottom() - graph.height() * step as f32 / 5.0;
            painter.line_segment(
                [egui::pos2(graph.left(), y), egui::pos2(graph.right(), y)],
                egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 130, 145, 75)),
            );
            painter.text(
                egui::pos2(content.left(), y),
                egui::Align2::LEFT_CENTER,
                format!("{level:.0} dB"),
                FontId::proportional(10.0),
                Color32::from_gray(205),
            );
        }

        let band_width = graph.width() / ROOM_SPECTRUM_BANDS as f32;
        let bar_width = (band_width - 0.5).max(1.0);
        for (band, amplitude) in self.room_spectrum.iter().enumerate() {
            let x = graph.left() + band as f32 * band_width;
            let height = Self::room_spectrum_display_level(*amplitude) * graph.height();
            if height > 0.0 {
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x, graph.bottom() - height),
                        egui::pos2(x + bar_width, graph.bottom()),
                    ),
                    0.0,
                    Color32::from_rgb(74, 205, 174),
                );
            }
        }

        let frequency_rect = egui::Rect::from_min_max(
            egui::pos2(graph.left(), content.bottom() - frequency_height),
            content.right_bottom(),
        );
        for (percent, label) in SPECTRUM_FREQUENCY_LABELS {
            let normalized = percent as f32 / 100.0;
            let x = egui::lerp(frequency_rect.left()..=frequency_rect.right(), normalized);
            painter.line_segment(
                [egui::pos2(x, frequency_rect.top()), egui::pos2(x, frequency_rect.top() + 5.0)],
                egui::Stroke::new(1.0, Color32::from_gray(115)),
            );
            let (anchor, label_x) = if percent == 0 {
                (egui::Align2::LEFT_CENTER, x + 1.0)
            } else if percent == 100 {
                (egui::Align2::RIGHT_CENTER, x - 1.0)
            } else {
                (egui::Align2::CENTER_CENTER, x)
            };
            painter.text(
                egui::pos2(label_x, frequency_rect.center().y),
                anchor,
                label,
                FontId::proportional(10.0),
                Color32::from_gray(220),
            );
        }

        if self.microphone_input_peak <= 0.000001 {
            painter.text(
                graph.center() - vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "Aucun signal reçu du microphone",
                FontId::proportional(18.0),
                Color32::from_rgb(245, 190, 105),
            );
            painter.text(
                graph.center() + vec2(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                "Sélectionnez Scarlett Solo, activez 48 V et vérifiez le câble XLR",
                FontId::proportional(12.0),
                Color32::from_gray(190),
            );
        }
    }

    fn spectrum_display_level(amplitude: f32) -> f32 {
        const FLOOR_DB: f32 = -60.0;
        if amplitude <= 0.000001 {
            0.0
        } else {
            ((20.0 * amplitude.clamp(0.0, 1.0).log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
        }
    }

    fn room_spectrum_display_level(amplitude: f32) -> f32 {
        const ROOM_DB_MAX: f32 = 110.0;
        if amplitude <= 0.000001 {
            0.0
        } else {
            ((20.0 * amplitude.clamp(0.0, 1.0).log10() + ROOM_DB_MAX) / ROOM_DB_MAX).clamp(0.0, 1.0)
        }
    }

    fn draw_toolbar(&mut self, ui: &mut Ui) {
        ui.set_height(42.0);
        ui.spacing_mut().item_spacing.x = 6.0;

        ui.horizontal(|ui| {
            ui.set_height(38.0);

            let menu_response =
                toolbar_icon_button(ui, &self.toolbar_textures, "\u{e5d2}", false, "Menu");
            egui::Popup::menu(&menu_response).show(|ui| {
                ui.set_min_width(170.0);
                for (mode, label, _) in MODE_BUTTONS {
                    if ui.selectable_label(self.scene.mode == mode, label).clicked() {
                        self.select_mode(mode);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Réinitialiser le transport").clicked() {
                    self.stop_playback();
                    ui.close();
                }
                if ui.button("Charger des musiques…").clicked() {
                    self.show_music_loader = true;
                    ui.close();
                }
                if ui.button("Analyser les BPM manquants").clicked() {
                    self.queue_missing_bpm_analysis();
                    ui.close();
                }
            });

            toolbar_separator(ui);

            // Les six commandes suivent le même ordre que le Player
            // d'Exec Modernisé : précédent, lecture, arrêt, suivant, boucle,
            // enchaînement.
            if toolbar_icon_button(
                ui,
                &self.toolbar_textures,
                "\u{e045}",
                false,
                "Piste précédente",
            )
            .clicked()
            {
                self.change_track(-1);
            }
            let play_icon = if self.workspace.transport == TransportState::Playing {
                "\u{e034}"
            } else {
                "\u{e037}"
            };
            if toolbar_icon_button(ui, &self.toolbar_textures, play_icon, false, "Lecture / pause")
                .clicked()
            {
                self.toggle_playback();
            }
            if toolbar_icon_button(ui, &self.toolbar_textures, "\u{e047}", false, "Arrêter")
                .clicked()
            {
                self.stop_playback();
            }
            if toolbar_icon_button(ui, &self.toolbar_textures, "\u{e044}", false, "Piste suivante")
                .clicked()
            {
                self.change_track(1);
            }
            let repeat_response =
                toolbar_menu_button(ui, &self.toolbar_textures, "\u{e040}", "Répéter");
            egui::Popup::menu(&repeat_response).show(|ui| {
                ui.set_min_width(190.0);
                let mut repeat_track = self.repeat_mode == Some(RepeatMode::Track);
                if ui.checkbox(&mut repeat_track, "Répéter seul").changed() {
                    self.repeat_mode = repeat_track.then_some(RepeatMode::Track);
                    self.save_playlist();
                    ui.close();
                }
                let mut repeat_all = self.repeat_mode == Some(RepeatMode::All);
                if ui.checkbox(&mut repeat_all, "Répéter tout").changed() {
                    self.repeat_mode = repeat_all.then_some(RepeatMode::All);
                    self.save_playlist();
                    ui.close();
                }
            });

            let chain_response =
                toolbar_menu_button(ui, &self.toolbar_textures, "\u{e05f}", "Enchaîner");
            egui::Popup::menu(&chain_response).show(|ui| {
                ui.set_min_width(250.0);
                let mut chain_normal = self.chain_mode.is_some();
                if ui.checkbox(&mut chain_normal, "Enchaîner").changed() {
                    if chain_normal {
                        self.chain_mode = Some(ChainMode::Normal);
                    } else {
                        self.chain_mode = None;
                    }
                    self.save_playlist();
                    ui.close();
                }
                let mut chain_volume = self.chain_mode.is_some_and(ChainMode::has_volume_fader);
                if ui.checkbox(&mut chain_volume, "Enchaîner avec fader volume").changed() {
                    let tempo = self.chain_mode.is_some_and(ChainMode::has_tempo_fader);
                    self.chain_mode = Some(ChainMode::from_faders(chain_volume, tempo));
                    self.save_playlist();
                    ui.close();
                }
                let mut chain_tempo = self.chain_mode.is_some_and(ChainMode::has_tempo_fader);
                if ui.checkbox(&mut chain_tempo, "Enchaîner avec fader tempo").changed() {
                    let volume = self.chain_mode.is_some_and(ChainMode::has_volume_fader);
                    self.chain_mode = Some(ChainMode::from_faders(volume, chain_tempo));
                    self.save_playlist();
                    ui.close();
                }
                if ui.checkbox(&mut self.shuffle_enabled, "Ordre aléatoire").changed() {
                    self.save_playlist();
                }
                if ui
                    .checkbox(&mut self.click_chain_enabled, "Fondu enchaîné au clic sur une piste")
                    .changed()
                {
                    self.save_playlist();
                }
                ui.separator();
                if ui
                    .add(
                        egui::Slider::new(&mut self.chain_transition_seconds, 0.0..=30.0)
                            .text("Durée")
                            .suffix(" s"),
                    )
                    .changed()
                {
                    self.save_playlist();
                }
            });

            toolbar_separator(ui);
            if toolbar_icon_button(
                ui,
                &self.toolbar_textures,
                "\u{e2c8}",
                false,
                "Charger des musiques",
            )
            .clicked()
            {
                self.import_music();
            }
        });
    }

    fn draw_microphone_profile_menu(&mut self, ui: &mut Ui) {
        let profile_response = toolbar_menu_button(
            ui,
            &self.toolbar_textures,
            "\u{e429}",
            "Sélectionner le type de microphone",
        );
        egui::Popup::menu(&profile_response).show(|ui| {
            ui.set_min_width(300.0);
            ui.label("Profil du microphone");
            ui.separator();
            for profile in MicrophoneProfile::ALL {
                if ui
                    .selectable_label(self.microphone_profile == profile, profile.label())
                    .clicked()
                {
                    self.microphone_profile = profile;
                    ui.close();
                }
            }
            ui.separator();
            ui.label(match self.microphone_profile {
                MicrophoneProfile::Flat => "Signal utilisé sans correction.",
                MicrophoneProfile::BehringerEcm8000 => {
                    "ECM8000 : alimentation fantôme 48 V requise."
                }
                MicrophoneProfile::ShureBeta58a => "Beta 58A : micro dynamique, 48 V non requis.",
            });
        });
    }

    fn draw_mode_toolbar(&mut self, ui: &mut Ui) {
        ui.set_height(42.0);
        ui.spacing_mut().item_spacing.x = 6.0;

        ui.horizontal(|ui| match self.scene.mode {
            WorkspaceMode::RoomAnalyzer => {
                let microphone_active = self.microphone_buffer.is_some();
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e029}",
                    microphone_active,
                    if microphone_active {
                        "Arrêter l'entrée audio"
                    } else {
                        "Démarrer l'entrée audio"
                    },
                )
                .clicked()
                {
                    if microphone_active {
                        self.stop_microphone();
                        self.stop_pink_noise();
                    } else {
                        self.start_microphone();
                    }
                }

                let pink_noise_active = self.pink_noise_player.is_some();
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e050}",
                    pink_noise_active,
                    if pink_noise_active {
                        "Arrêter le bruit rose"
                    } else {
                        "Démarrer le bruit rose"
                    },
                )
                .clicked()
                {
                    if pink_noise_active {
                        self.stop_pink_noise();
                    } else {
                        self.start_pink_noise();
                    }
                }

                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e5d5}",
                    false,
                    "Actualiser les entrées audio",
                )
                .clicked()
                {
                    self.stop_microphone();
                    self.refresh_microphone_devices();
                    self.start_microphone();
                }

                self.draw_microphone_profile_menu(ui);

                toolbar_separator(ui);
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e14c}",
                    false,
                    "Effacer l'analyse",
                )
                .clicked()
                {
                    self.room_spectrum = [0.0; ROOM_SPECTRUM_BANDS];
                    self.workspace.spectrum = Default::default();
                }
            }
            WorkspaceMode::LiveTablature => {
                let play_icon = if self.tablature_playing { "\u{e034}" } else { "\u{e037}" };
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    play_icon,
                    false,
                    if self.tablature_playing {
                        "Mettre la tablature en pause"
                    } else {
                        "Reprendre la tablature"
                    },
                )
                .clicked()
                {
                    self.toggle_tablature_playback();
                }
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e061}",
                    self.tablature_recording,
                    if self.tablature_recording {
                        "Arrêter l'enregistrement"
                    } else {
                        "Enregistrer les notes"
                    },
                )
                .clicked()
                {
                    self.toggle_tablature_recording();
                }
                if self.live_analysis_source == LiveAnalysisSource::Microphone
                    && toolbar_icon_button(
                        ui,
                        &self.toolbar_textures,
                        "\u{e310}",
                        self.microphone_monitor_enabled,
                        if self.microphone_monitor_enabled {
                            "MON : couper le retour casque"
                        } else {
                            "MON : activer le retour casque"
                        },
                    )
                    .clicked()
                {
                    self.set_microphone_monitoring(!self.microphone_monitor_enabled);
                }
                if self.live_analysis_source == LiveAnalysisSource::Microphone
                    && toolbar_label_button(
                        ui,
                        &self.toolbar_textures,
                        "MONO",
                        self.microphone_mono_enabled,
                        if self.microphone_mono_enabled {
                            "MONO : désactiver le mélange mono"
                        } else {
                            "MONO : activer le mélange mono"
                        },
                    )
                    .clicked()
                {
                    self.microphone_mono_enabled = !self.microphone_mono_enabled;
                    if self.microphone_buffer.is_some() {
                        self.stop_microphone();
                        self.start_microphone();
                    }
                }
                self.draw_microphone_profile_menu(ui);
                toolbar_separator(ui);
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e8b0}",
                    !self.live_tab_strumming,
                    "Basculer tablature / strumming",
                )
                .clicked()
                {
                    self.live_tab_strumming = !self.live_tab_strumming;
                }
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e429}",
                    false,
                    "Changer l'affichage tablature",
                )
                .clicked()
                {
                    self.tablature_display_mode = match self.tablature_display_mode {
                        TablatureDisplayMode::Tablature => TablatureDisplayMode::Staff,
                        TablatureDisplayMode::Staff => TablatureDisplayMode::Both,
                        TablatureDisplayMode::Both => TablatureDisplayMode::Tablature,
                    };
                }
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e405}",
                    self.metronome_enabled,
                    if self.metronome_enabled {
                        "Désactiver le métronome"
                    } else {
                        "Activer le métronome"
                    },
                )
                .clicked()
                {
                    self.metronome_enabled = !self.metronome_enabled;
                }
                toolbar_separator(ui);
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e14c}",
                    false,
                    "Effacer la tablature",
                )
                .clicked()
                {
                    self.clear_tablature();
                }
            }
            _ => {}
        });
    }

    fn select_mode(&mut self, mode: WorkspaceMode) {
        self.scene.mode = mode;
        self.workspace.document.mode = mode;
        if mode == WorkspaceMode::LiveTablature {
            self.tablature_playing = true;
            self.tablature_recording = true;
            self.tablature_pause_started = None;
        }
        if mode == WorkspaceMode::RoomAnalyzer {
            self.stop_pink_noise();
            self.stop_microphone();
            self.show_room_measurement_dialog = true;
        } else if mode == WorkspaceMode::LiveTablature {
            self.show_room_measurement_dialog = false;
            self.stop_pink_noise();
            if self.live_analysis_source == LiveAnalysisSource::Microphone {
                self.start_microphone();
            }
        } else {
            self.show_room_measurement_dialog = false;
            self.stop_pink_noise();
            self.stop_microphone();
        }
    }

    fn start_room_measurement(&mut self) {
        self.show_room_measurement_dialog = false;
        // Le bruit rose doit suivre l'interface choisie pour le micro,
        // notamment vers la sortie casque de la Scarlett Solo.
        self.stop_pink_noise();
        let track_is_playing = self.track_is_playing();
        if !track_is_playing {
            // Lorsque la piste est arrêtée, reconstruire la sortie permet de
            // sélectionner la Scarlett pour le bruit rose. Si elle joue déjà,
            // conserver son mixer évite de couper la lecture.
            self.audio_output = None;
        }
        self.start_microphone();
        if !track_is_playing {
            self.start_pink_noise();
        }
    }

    fn start_microphone(&mut self) {
        if self.microphone_buffer.is_some() {
            return;
        }
        self.microphone_error = None;
        if self.microphone_devices.is_empty() {
            self.refresh_microphone_devices();
        }
        let Some(device) = self.microphone_devices.get(self.microphone_device_index).cloned()
        else {
            self.microphone_error =
                Some("Aucune entrée audio USB ou microphone disponible".to_owned());
            return;
        };
        let builder = match rodio::microphone::MicrophoneBuilder::new().device(device) {
            Ok(builder) => builder,
            Err(error) => {
                self.microphone_error = Some(format!("Micro : {error}"));
                return;
            }
        };
        let builder = match builder.default_config() {
            Ok(builder) => builder,
            Err(error) => {
                self.microphone_error = Some(format!("Configuration micro : {error}"));
                return;
            }
        };
        let config = builder.get_config();
        let microphone = match builder.open_stream() {
            Ok(microphone) => microphone,
            Err(error) => {
                self.microphone_error = Some(format!("Ouverture du micro : {error}"));
                return;
            }
        };
        let channels = config.channel_count.get() as usize;
        let sample_rate = config.sample_rate.get() as f32;
        let buffer = Arc::new(Mutex::new(LiveAnalysisBuffer::new(channels, sample_rate)));
        let monitor_input = self.scene.mode == WorkspaceMode::LiveTablature
            && self.microphone_monitor_enabled
            && self.live_analysis_source == LiveAnalysisSource::Microphone;
        if monitor_input {
            match self.open_room_audio_output() {
                Ok(mut output) => {
                    output.log_on_drop(false);
                    let player = rodio::Player::connect_new(output.mixer());
                    player.set_volume(0.75);
                    player.append(MicrophoneAnalysisSource::new(
                        microphone,
                        buffer.clone(),
                        config.channel_count,
                        config.sample_rate,
                        self.microphone_mono_enabled,
                        self.channel_gains.clone(),
                    ));
                    self.microphone_monitor_output_name = self.preferred_room_output_name();
                    self.microphone_monitor_output = Some(output);
                    self.microphone_monitor_player = Some(player);
                    self.microphone_buffer = Some(buffer);
                    return;
                }
                Err(error) => {
                    self.audio_error = Some(format!("Monitoring casque : {error}"));
                }
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_buffer = buffer.clone();
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut microphone = microphone;
            let mut pending = Vec::with_capacity(channels * 512);
            while !thread_stop.load(Ordering::Relaxed) {
                let Some(sample) = microphone.next() else { break };
                pending.push(sample);
                if pending.len() >= channels * 512 {
                    if let Ok(mut analysis) = thread_buffer.lock() {
                        analysis.push_samples(&pending);
                    }
                    pending.clear();
                }
            }
            if !pending.is_empty() {
                if let Ok(mut analysis) = thread_buffer.lock() {
                    analysis.push_samples(&pending);
                }
            }
        });
        self.microphone_buffer = Some(buffer);
        self.microphone_stop = Some(stop);
        self.microphone_thread = Some(thread);
    }

    fn stop_microphone_monitor(&mut self) {
        if let Some(player) = self.microphone_monitor_player.take() {
            player.stop();
        }
        self.microphone_monitor_output = None;
        self.microphone_monitor_output_name = None;
    }

    fn set_microphone_monitoring(&mut self, enabled: bool) {
        self.microphone_monitor_enabled = enabled;
        if self.microphone_buffer.is_some()
            && self.scene.mode == WorkspaceMode::LiveTablature
            && self.live_analysis_source == LiveAnalysisSource::Microphone
        {
            self.stop_microphone();
            self.start_microphone();
        }
    }

    fn stop_microphone(&mut self) {
        self.stop_microphone_monitor();
        if let Some(stop) = self.microphone_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(thread) = self.microphone_thread.take() {
            let _ = thread.join();
        }
        self.microphone_buffer = None;
        self.microphone_input_peak = 0.0;
        self.microphone_pitch = None;
        self.microphone_chord = None;
        self.tablature_playing = true;
        self.tablature_recording = true;
        self.tablature_pause_started = None;
        self.tablature_notes.clear();
        self.tablature_last_position = None;
        self.tablature_last_recorded_at = None;
        self.tablature_measure_origin = None;
        if self.scene.mode != WorkspaceMode::Player {
            self.stereo_spectrum = [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]];
            self.room_spectrum = [0.0; ROOM_SPECTRUM_BANDS];
            self.channel_levels = [0.0; 2];
        }
    }

    fn refresh_microphone_devices(&mut self) {
        let previous_name =
            self.microphone_devices.get(self.microphone_device_index).map(ToString::to_string);
        match rodio::microphone::available_inputs() {
            Ok(devices) => {
                let previous_index = previous_name
                    .as_ref()
                    .and_then(|name| devices.iter().position(|device| device.to_string() == *name));
                let preferred_index = devices.iter().position(|device| {
                    let name = device.to_string().to_lowercase();
                    name.contains("scarlett") || name.contains("focusrite")
                });
                self.microphone_devices = devices;
                self.microphone_device_index = previous_index
                    .or(preferred_index)
                    .unwrap_or(0)
                    .min(self.microphone_devices.len().saturating_sub(1));
            }
            Err(error) => {
                self.microphone_devices.clear();
                self.microphone_error = Some(format!("Liste des entrées audio : {error}"));
            }
        }
    }

    fn change_track(&mut self, offset: isize) {
        let track_count = self.loaded_music.len();
        let Some(current) = self.selected_track else {
            if track_count > 0 {
                self.play_track(0);
            }
            return;
        };
        if self.shuffle_enabled && offset > 0 {
            let next = self.random_track_index(current, track_count);
            let transition = self.chain_transition_mode();
            self.play_track_with_transition(next, transition);
            return;
        }
        let next = current as isize + offset;
        if next >= 0 && (next as usize) < track_count {
            let transition = self.chain_transition_mode();
            self.play_track_with_transition(next as usize, transition);
        }
    }

    fn chain_transition_mode(&self) -> Option<ChainMode> {
        self.chain_mode
            .filter(|mode| *mode != ChainMode::Normal && self.chain_transition_seconds > 0.0)
    }

    fn random_track_index(&mut self, current: usize, track_count: usize) -> usize {
        if track_count <= 1 {
            return 0;
        }
        let mut state = self.shuffle_state;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.shuffle_state = state;
        let mut next = (state % track_count as u64) as usize;
        if next == current {
            next = (next + 1) % track_count;
        }
        next
    }

    fn restore_playlist(&mut self) {
        let Some(path) = playlist_storage_path() else { return };
        let Ok(contents) = std::fs::read_to_string(path) else { return };
        let Ok(saved) = ron::from_str::<SavedPlaylist>(&contents) else { return };
        let saved_selected_track = saved.selected_track;
        let saved_position_seconds = saved.position_seconds.max(0.0);
        let saved_repeat_mode = saved.repeat_mode;
        let saved_shuffle_enabled = saved.shuffle_enabled;
        let saved_chain_mode = saved.chain_mode;
        let saved_chain_transition_seconds = if saved.layout_version < 2 {
            default_chain_transition_seconds()
        } else if saved.chain_transition_seconds.is_finite() {
            saved.chain_transition_seconds.clamp(0.0, 30.0)
        } else {
            default_chain_transition_seconds()
        };
        if !saved.column_order.is_empty() {
            self.track_column_order = Self::normalize_column_order(saved.column_order);
        }
        if saved.layout_version > 0 && !saved.visible_columns.is_empty() {
            self.track_column_visible = [false; TRACK_COLUMN_COUNT];
            for column in saved.visible_columns {
                self.track_column_visible[column.index()] = true;
            }
            // La colonne BPM a été ajoutée après la première version du
            // réglage des colonnes : elle est activée lors de la migration.
            if saved.layout_version < 3 {
                self.track_column_visible[TrackColumn::Bpm.index()] = true;
            }
            if !self.track_column_visible.iter().any(|visible| *visible) {
                self.track_column_visible = TrackColumn::default_visibility();
            }
        }
        for path in saved.paths {
            self.add_music_path(PathBuf::from(path));
        }
        self.repeat_mode = saved_repeat_mode;
        self.shuffle_enabled = saved_shuffle_enabled;
        self.chain_mode = saved_chain_mode;
        self.chain_transition_seconds = saved_chain_transition_seconds;
        self.click_chain_enabled = saved.click_chain_enabled;
        self.selected_track = saved_selected_track
            .filter(|index| *index < self.loaded_music.len())
            .or(self.selected_track);
        self.resume_position_seconds =
            if self.selected_track.is_some() { saved_position_seconds } else { 0.0 };
        self.last_saved_position_seconds = self.resume_position_seconds;
    }

    fn normalize_column_order(order: Vec<TrackColumn>) -> Vec<TrackColumn> {
        let mut normalized = Vec::with_capacity(TrackColumn::default_order().len());
        for column in order.into_iter().chain(TrackColumn::default_order()) {
            if !normalized.contains(&column) {
                normalized.push(column);
            }
        }
        normalized
    }

    fn save_playlist(&self) {
        let Some(path) = playlist_storage_path() else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let saved = SavedPlaylist {
            paths: self
                .loaded_music
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            column_order: self.track_column_order.clone(),
            visible_columns: self
                .track_column_order
                .iter()
                .copied()
                .filter(|column| self.track_column_visible[column.index()])
                .collect(),
            layout_version: 3,
            selected_track: self.selected_track,
            position_seconds: self
                .audio_player
                .as_ref()
                .map(|player| player.get_pos().as_secs_f32())
                .unwrap_or(self.resume_position_seconds)
                .max(0.0),
            repeat_mode: self.repeat_mode,
            shuffle_enabled: self.shuffle_enabled,
            chain_mode: self.chain_mode,
            chain_transition_seconds: self.chain_transition_seconds,
            click_chain_enabled: self.click_chain_enabled,
        };
        let Ok(contents) = ron::to_string(&saved) else { return };
        let _ = std::fs::write(path, contents);
    }

    fn queue_metadata_load(&mut self, index: usize, path: PathBuf) {
        if let Some(queued) = self.track_metadata_queued.get_mut(index) {
            if *queued {
                return;
            }
            *queued = true;
        }
        if self.metadata_sender.is_none() {
            let (sender, receiver) = mpsc::channel();
            self.metadata_sender = Some(sender);
            self.metadata_loading = Some(receiver);
        }
        let Some(sender) = self.metadata_sender.as_ref().cloned() else {
            return;
        };
        self.metadata_pending += 1;
        std::thread::spawn(move || {
            let metadata = load_track_metadata(&path);
            let _ = sender.send((path, metadata));
        });
    }

    fn apply_track_metadata(&mut self, index: usize, metadata: AudioMetadata) {
        if let Some(slot) = self.track_metadata.get_mut(index) {
            *slot = Some(metadata.clone());
        }
        if let Some(queued) = self.track_metadata_queued.get_mut(index) {
            *queued = false;
        }
        if let Some(track) = self.workspace.document.tracks.get_mut(index) {
            track.duration_seconds = metadata.duration_seconds;
            if let Some(title) = &metadata.title {
                track.title = title.clone();
            }
            if let Some(artist) = &metadata.artist {
                track.artist = artist.clone();
            }
            if let Some(album) = &metadata.album {
                track.album = album.clone();
            }
            track.year = metadata.year;
        }
        if self.selected_track == Some(index) {
            self.audio_metadata = Some(metadata);
        }
    }

    fn add_music_path(&mut self, path: PathBuf) {
        if self.loaded_music.iter().any(|loaded| loaded == &path) {
            return;
        }
        let title = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Musique")
            .to_owned();
        self.workspace.document.tracks.push(Track {
            title,
            artist: "Fichier local".to_owned(),
            album: String::new(),
            year: None,
            duration_seconds: 0.0,
        });
        self.track_covers.push(None);
        self.track_cover_checked.push(false);
        self.track_metadata.push(None);
        self.track_metadata_queued.push(false);
        let index = self.loaded_music.len();
        self.loaded_music.push(path.clone());
        if self.selected_track.is_none() {
            self.selected_track = Some(index);
        }
        self.queue_metadata_load(index, path);
    }

    fn remove_track(&mut self, index: usize) {
        if index >= self.loaded_music.len() {
            return;
        }

        let removing_selected = self.selected_track == Some(index);
        if removing_selected {
            self.stop_playback();
        }

        self.loaded_music.remove(index);
        self.workspace.document.tracks.remove(index);
        self.track_covers.remove(index);
        self.track_cover_checked.remove(index);
        self.track_metadata.remove(index);
        self.track_metadata_queued.remove(index);

        self.selected_track = match self.selected_track {
            Some(_) if removing_selected => {
                (!self.loaded_music.is_empty()).then_some(index.min(self.loaded_music.len() - 1))
            }
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
        self.track_scroll_target = self.selected_track;
        self.save_playlist();
    }

    fn queue_missing_bpm_analysis(&mut self) {
        let pending = self
            .loaded_music
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                let missing_bpm = self
                    .track_metadata
                    .get(*index)
                    .and_then(|metadata| metadata.as_ref())
                    .is_none_or(|metadata| metadata.bpm.is_none());
                missing_bpm && !self.track_metadata_queued.get(*index).copied().unwrap_or(false)
            })
            .map(|(index, path)| (index, path.clone()))
            .collect::<Vec<_>>();
        for (index, path) in pending {
            self.queue_metadata_load(index, path);
        }
    }

    fn draw_track_column_menu_contents(&mut self, ui: &mut Ui) {
        let mut changed = false;
        ui.label(RichText::new("Afficher et réordonner").small().color(Color32::GRAY));
        ui.separator();
        let visible_count = self.track_column_visible.iter().filter(|visible| **visible).count();
        let order = self.track_column_order.clone();
        for (position, column) in order.into_iter().enumerate() {
            ui.horizontal(|ui| {
                let checked = self.track_column_visible[column.index()];
                let can_toggle = !checked || visible_count > 1;
                let mut next_checked = checked;
                if ui
                    .add_enabled(can_toggle, egui::Checkbox::new(&mut next_checked, column.label()))
                    .changed()
                {
                    self.track_column_visible[column.index()] = next_checked;
                    changed = true;
                }
                if ui.add_enabled(position > 0, egui::Button::new("↑").small()).clicked() {
                    self.track_column_order.swap(position, position - 1);
                    changed = true;
                }
                if ui
                    .add_enabled(
                        position + 1 < self.track_column_order.len(),
                        egui::Button::new("↓").small(),
                    )
                    .clicked()
                {
                    self.track_column_order.swap(position, position + 1);
                    changed = true;
                }
            });
        }
        ui.separator();
        if ui.button("Réinitialiser").clicked() {
            self.track_column_order = TrackColumn::default_order();
            self.track_column_visible = TrackColumn::default_visibility();
            changed = true;
            ui.close();
        }
        if changed {
            self.save_playlist();
        }
    }

    fn track_column_widths(
        columns: &[TrackColumn],
        available_width: f32,
    ) -> Vec<(TrackColumn, f32)> {
        let base_width: f32 = columns.iter().map(|column| column.width()).sum();
        let mut widths: Vec<_> =
            columns.iter().copied().map(|column| (column, column.width())).collect();
        if base_width < available_width && !widths.is_empty() {
            let target_index =
                columns.iter().position(|column| *column == TrackColumn::Title).unwrap_or(0);
            widths[target_index].1 += available_width - base_width;
        }
        widths
    }

    fn track_cell_text(
        column: TrackColumn,
        index: usize,
        track: &Track,
        path: Option<&Path>,
        metadata: Option<&AudioMetadata>,
    ) -> String {
        match column {
            TrackColumn::Number => format!("{:02}", index + 1),
            TrackColumn::Title => track.title.clone(),
            TrackColumn::Artist => track.artist.clone(),
            TrackColumn::Album => {
                if track.album.is_empty() {
                    "—".to_owned()
                } else {
                    track.album.clone()
                }
            }
            TrackColumn::Year => {
                track.year.map(|year| year.to_string()).unwrap_or_else(|| "—".to_owned())
            }
            TrackColumn::Duration => {
                if track.duration_seconds > 0.0 {
                    Self::format_time(track.duration_seconds)
                } else {
                    "—".to_owned()
                }
            }
            TrackColumn::Bpm => metadata
                .and_then(|metadata| metadata.bpm)
                .map(|bpm| format!("{bpm:.0}"))
                .unwrap_or_else(|| "—".to_owned()),
            TrackColumn::Format => {
                if let Some(metadata) = metadata {
                    let bit_depth = metadata
                        .bit_depth
                        .map(|bits| format!("{bits} bits"))
                        .unwrap_or_else(|| "— bits".to_owned());
                    format!(
                        "{} / {} / {}",
                        metadata.format,
                        bit_depth,
                        Self::format_sample_rate(metadata.sample_rate)
                    )
                } else {
                    path.and_then(|path| path.extension())
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.to_uppercase())
                        .unwrap_or_else(|| "—".to_owned())
                }
            }
            TrackColumn::File => path
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("—")
                .to_owned(),
        }
    }

    fn draw_track_cell(
        ui: &mut Ui,
        column: TrackColumn,
        width: f32,
        height: f32,
        text: impl Into<egui::WidgetText>,
    ) {
        // Réserver explicitement la largeur de la colonne avant de dessiner le
        // texte évite que le layout d'egui ne compacte les cellules à leur taille
        // minimale quand la liste est dans un ScrollArea.
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let text_rect = if column.alignment() == egui::Align::Min {
            rect.with_min_x((rect.left() + 4.0).min(rect.right()))
        } else {
            rect
        };
        ui.put(text_rect, egui::Label::new(text).halign(column.alignment()).truncate());
    }

    fn track_info_height(&self) -> f32 {
        TRACK_INFO_HEIGHT
    }

    fn draw_track_list(&mut self, ui: &mut Ui) {
        let track_info_height = self.track_info_height();
        let visible_height = (ui.clip_rect().bottom() - ui.cursor().top()).max(0.0);
        let content_height = (visible_height
            - SPECTRUM_HEIGHT
            - track_info_height
            - PLAYER_CONTENT_GAP * 2.0
            - TRACK_LIST_INNER_MARGIN)
            .max(TRACK_HEADER_HEIGHT);
        let mut clicked_track = None;
        let mut remove_requested = None;
        let scroll_target = self.track_scroll_target;
        let mut scrolled_to_target = false;
        let visible_columns: Vec<_> = self
            .track_column_order
            .iter()
            .copied()
            .filter(|column| self.track_column_visible[column.index()])
            .collect();
        let visible_columns =
            if visible_columns.is_empty() { vec![TrackColumn::Title] } else { visible_columns };
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                let content_width = ui.available_width();
                ui.allocate_ui_with_layout(
                    vec2(content_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if self.workspace.document.tracks.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(
                                    (content_height - TRACK_HEADER_HEIGHT - 24.0).max(0.0) * 0.5,
                                );
                                ui.label(
                                    RichText::new("Aucune piste chargée")
                                        .color(Color32::from_gray(170)),
                                );
                            });
                        } else {
                            let column_widths =
                                Self::track_column_widths(&visible_columns, ui.available_width());
                            let total_width =
                                column_widths.iter().map(|(_, width)| *width).sum::<f32>();
                            egui::ScrollArea::both()
                                .id_salt("track_list")
                                .auto_shrink([false, false])
                                .max_height(content_height)
                                .show(ui, |ui| {
                                    // L'en-tête et les lignes partagent le même conteneur et les
                                    // mêmes largeurs. Les colonnes conservent leur largeur
                                    // native : ScrollArea ajoute alors le défilement horizontal
                                    // quand la fenêtre est trop étroite.
                                    ui.set_min_width(total_width);
                                    let header = ui.allocate_ui_with_layout(
                                        vec2(total_width, TRACK_HEADER_HEIGHT),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.spacing_mut().item_spacing.x = 0.0;
                                            for (column, width) in &column_widths {
                                                Self::draw_track_cell(
                                                    ui,
                                                    *column,
                                                    *width,
                                                    TRACK_HEADER_HEIGHT,
                                                    RichText::new(column.label())
                                                        .strong()
                                                        .color(Color32::from_gray(185)),
                                                );
                                            }
                                        },
                                    );
                                    let header_response = ui.interact(
                                        header.response.rect,
                                        ui.id().with("track_column_headers"),
                                        Sense::click(),
                                    );
                                    egui::Popup::menu(&header_response)
                                        .show(|ui| self.draw_track_column_menu_contents(ui));
                                    ui.separator();
                                    for (index, track) in
                                        self.workspace.document.tracks.iter().enumerate()
                                    {
                                        let selected = self.selected_track == Some(index);
                                        let path =
                                            self.loaded_music.get(index).map(PathBuf::as_path);
                                        let row = ui.allocate_ui_with_layout(
                                            vec2(total_width, TRACK_ROW_HEIGHT),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                let row_rect = ui.max_rect().shrink(2.0);
                                                if selected {
                                                    ui.painter().rect_filled(
                                                        row_rect,
                                                        4.0,
                                                        Color32::from_rgb(49, 91, 95),
                                                    );
                                                }
                                                ui.spacing_mut().item_spacing.x = 0.0;
                                                for (column, width) in &column_widths {
                                                    let text = Self::track_cell_text(
                                                        *column,
                                                        index,
                                                        track,
                                                        path,
                                                        self.track_metadata
                                                            .get(index)
                                                            .and_then(|metadata| metadata.as_ref()),
                                                    );
                                                    Self::draw_track_cell(
                                                        ui,
                                                        *column,
                                                        *width,
                                                        TRACK_ROW_HEIGHT,
                                                        text,
                                                    );
                                                }
                                            },
                                        );
                                        let row_response = ui.interact(
                                            row.response.rect,
                                            ui.id().with(("track_row", index)),
                                            Sense::click(),
                                        );
                                        if scroll_target == Some(index) {
                                            ui.scroll_to_rect(
                                                row.response.rect,
                                                Some(egui::Align::Center),
                                            );
                                            scrolled_to_target = true;
                                        }
                                        if row_response.clicked() {
                                            clicked_track = Some(index);
                                        }
                                        egui::Popup::menu(&row_response).show(|ui| {
                                            ui.label(RichText::new(&track.title).strong());
                                            ui.separator();
                                            if ui.button("Supprimer de la liste").clicked() {
                                                remove_requested = Some(index);
                                                ui.close();
                                            }
                                        });
                                        if row_response.hovered() {
                                            ui.painter().rect_stroke(
                                                row_response.rect.shrink(1.0),
                                                4.0,
                                                egui::Stroke::new(
                                                    1.0,
                                                    Color32::from_rgb(72, 184, 166),
                                                ),
                                                egui::StrokeKind::Inside,
                                            );
                                        }
                                        if index + 1 < self.workspace.document.tracks.len() {
                                            ui.separator();
                                        }
                                    }
                                });
                        }
                    },
                );
            });

        if scrolled_to_target {
            self.track_scroll_target = None;
        }

        if let Some(index) = clicked_track {
            self.play_track(index);
        }
        if let Some(index) = remove_requested {
            self.remove_track(index);
        }
    }

    fn draw_track_info(&mut self, ui: &mut Ui) {
        let available_width = ui.available_width();
        let info_index = self
            .selected_track
            .or_else(|| (!self.workspace.document.tracks.is_empty()).then_some(0));
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                let content_width = (available_width - 16.0).max(0.0);
                ui.set_min_width(content_width);
                ui.set_width(content_width);
                ui.set_height((self.track_info_height() - 16.0).max(0.0));
                if let Some(index) = info_index {
                    if !self.track_cover_checked.get(index).copied().unwrap_or(true) {
                        let cover = self
                            .loaded_music
                            .get(index)
                            .and_then(|path| load_cover_texture(&self.egui_context, path, index));
                        if let Some(slot) = self.track_covers.get_mut(index) {
                            *slot = cover;
                        }
                        if let Some(checked) = self.track_cover_checked.get_mut(index) {
                            *checked = true;
                        }
                    }
                    let track_title =
                        self.workspace.document.tracks.get(index).map(|track| track.title.clone());
                    let track_artist =
                        self.workspace.document.tracks.get(index).map(|track| track.artist.clone());
                    let track_album =
                        self.workspace.document.tracks.get(index).map(|track| track.album.clone());
                    let track_year =
                        self.workspace.document.tracks.get(index).and_then(|track| track.year);
                    let track_duration = self
                        .workspace
                        .document
                        .tracks
                        .get(index)
                        .map(|track| track.duration_seconds)
                        .unwrap_or(0.0);
                    let cover = self.track_covers.get(index).and_then(Option::as_ref).cloned();
                    let metadata = self
                        .audio_metadata
                        .clone()
                        .or_else(|| self.track_metadata.get(index).and_then(Clone::clone));
                    let audio_error = self.audio_error.clone();
                    let playback_position = self
                        .audio_player
                        .as_ref()
                        .map(|player| player.get_pos().as_secs_f32())
                        .unwrap_or(self.resume_position_seconds);
                    let fader_status = self.fader_status_text();
                    let cover_size = (TRACK_INFO_HEIGHT - 16.0).max(0.0);
                    ui.horizontal(|ui| {
                        if let Some(cover) = cover.as_ref() {
                            ui.add(
                                egui::Image::new((cover.id(), vec2(cover_size, cover_size)))
                                    .corner_radius(4.0),
                            );
                        } else {
                            ui.allocate_ui(vec2(cover_size, cover_size), |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space((cover_size - 34.0).max(0.0) * 0.5);
                                    ui.label("♪");
                                    ui.small("Aucune jaquette");
                                });
                            });
                        }
                        ui.separator();
                        ui.vertical(|ui| {
                            let info_width = ui.available_width();
                            ui.set_min_width(info_width);
                            ui.set_width(info_width);
                            ui.columns(2, |columns| {
                                let (left_column, right_column) = columns.split_at_mut(1);
                                let left = &mut left_column[0];
                                let right = &mut right_column[0];
                                if let Some(title) = &track_title {
                                    left.heading(title);
                                }
                                if let Some(artist) = &track_artist {
                                    left.label(artist);
                                }
                                if let Some(album) = &track_album {
                                    if !album.is_empty() {
                                        left.small(format!("Album : {album}"));
                                    }
                                }
                                if let Some(year) = track_year {
                                    left.small(format!("Année : {year}"));
                                }
                                if let Some(metadata) = &metadata {
                                    let bit_depth = metadata
                                        .bit_depth
                                        .map(|bits| format!("{bits} bits"))
                                        .unwrap_or_else(|| "qualité inconnue".to_owned());
                                    let channels = if metadata.channels == 2 {
                                        "Stéréo".to_owned()
                                    } else {
                                        format!("{} canaux", metadata.channels)
                                    };
                                    let quality = format!(
                                        "{} • {} • {} • {}",
                                        metadata.format,
                                        bit_depth,
                                        Self::format_sample_rate(metadata.sample_rate),
                                        channels
                                    );
                                    right.colored_label(Color32::from_rgb(110, 220, 190), quality);
                                    let bpm_label = metadata
                                        .bpm
                                        .map(|bpm| {
                                            if metadata.bpm_estimated {
                                                format!("{bpm:.0} (analysé)")
                                            } else {
                                                format!("{bpm:.0}")
                                            }
                                        })
                                        .unwrap_or_else(|| "—".to_owned());
                                    right.small(format!(
                                        "Durée {} • BPM {}",
                                        Self::format_time(metadata.duration_seconds),
                                        bpm_label
                                    ));
                                    if let Some(bitrate) = metadata.bitrate_kbps {
                                        right.small(format!("Débit {bitrate} kb/s"));
                                    }
                                    if let Some(genre) = &metadata.genre {
                                        right.small(format!("Genre : {genre}"));
                                    }
                                }
                                let duration = metadata
                                    .as_ref()
                                    .map(|metadata| metadata.duration_seconds)
                                    .filter(|duration| *duration > 0.0)
                                    .unwrap_or(track_duration);
                                right.small(format!(
                                    "Position : {} / {}",
                                    Self::format_time(playback_position),
                                    Self::format_time(duration)
                                ));
                                if let Some(status) = &fader_status {
                                    right.colored_label(Color32::from_rgb(235, 190, 95), status);
                                }
                                if let Some(error) = &audio_error {
                                    right.colored_label(Color32::LIGHT_RED, error);
                                }
                            });
                            ui.add_space(2.0);
                            self.draw_playback_seek(ui);
                        });
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space((self.track_info_height() - 16.0 - 18.0).max(0.0) * 0.5);
                        ui.label("Aucune piste chargée");
                    });
                }
            });
    }

    fn format_time(seconds: f32) -> String {
        let seconds = seconds.max(0.0) as u64;
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }

    fn fader_status_text(&self) -> Option<String> {
        let mode = self.chain_mode?;
        let mode_label = match mode {
            ChainMode::VolumeFader => "volume",
            ChainMode::TempoFader => "tempo",
            ChainMode::VolumeTempoFader => "volume + tempo",
            ChainMode::Normal => return None,
        };
        let duration = self.chain_transition_seconds.max(0.01);
        let state = if let Some(started) = self.chain_transition_started {
            let elapsed = started.elapsed().as_secs_f32().min(duration);
            format!("actif {:.1}/{:.1} s", elapsed, duration)
        } else {
            format!("prêt, durée {:.1} s", duration)
        };
        let trigger = self
            .chain_transition_trigger_position
            .map(Self::format_time)
            .unwrap_or_else(|| "jamais".to_owned());
        Some(format!("Fader {mode_label} : {state} • déclenché à {trigger}"))
    }

    fn format_sample_rate(sample_rate: u32) -> String {
        if sample_rate % 1000 == 0 {
            format!("{} kHz", sample_rate / 1000)
        } else {
            format!("{:.1} kHz", sample_rate as f32 / 1000.0)
        }
    }

    fn draw_playback_seek(&mut self, ui: &mut Ui) {
        let info_index = self
            .selected_track
            .or_else(|| (!self.workspace.document.tracks.is_empty()).then_some(0));
        let duration = self
            .audio_metadata
            .as_ref()
            .map(|metadata| metadata.duration_seconds)
            .or_else(|| {
                info_index.and_then(|index| {
                    self.track_metadata
                        .get(index)
                        .and_then(|metadata| metadata.as_ref())
                        .map(|metadata| metadata.duration_seconds)
                })
            })
            .or_else(|| {
                info_index.and_then(|index| {
                    self.workspace.document.tracks.get(index).map(|track| track.duration_seconds)
                })
            })
            .filter(|duration| *duration > 0.0)
            .unwrap_or(0.0);
        let current = self
            .audio_player
            .as_ref()
            .map(|player| player.get_pos().as_secs_f32())
            .unwrap_or(self.resume_position_seconds)
            .min(duration);
        let mut position = current;

        ui.horizontal(|ui| {
            ui.small(Self::format_time(position));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(Self::format_time(duration));
            });
        });
        let slider_width = ui.available_width();
        let slider = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = slider_width;
                ui.spacing_mut().interact_size.y = 12.0;
                ui.style_mut()
                    .text_styles
                    .insert(egui::TextStyle::Body, FontId::proportional(12.0));
                ui.add(
                    egui::Slider::new(&mut position, 0.0..=duration.max(0.1))
                        .show_value(false)
                        .trailing_fill(true),
                )
            })
            .inner;
        if slider.changed()
            && let Some(player) = self.audio_player.as_ref()
            && let Err(error) = player.try_seek(Duration::from_secs_f32(position))
        {
            self.audio_error = Some(format!("Recherche impossible : {error}"));
        } else if slider.changed() {
            self.resume_position_seconds = position;
            self.save_playlist();
            self.last_saved_position_seconds = position;
        }
    }

    fn draw_led_meter(ui: &mut Ui, level: f32, height: f32) {
        ui.vertical(|ui| {
            ui.set_width(20.0);
            let (response, painter) = ui.allocate_painter(vec2(20.0, height), egui::Sense::hover());
            let rect = response.rect;
            let step = rect.height() / LED_COUNT as f32;
            let radius = (rect.width() * 0.32).min(step * 0.34);
            let decibels = if level > 0.0001 { 20.0 * level.log10() } else { -60.0 };
            for index in 0..LED_COUNT {
                let threshold =
                    -60.0 + index as f32 * 60.0 / (LED_COUNT.saturating_sub(1).max(1) as f32);
                let active = decibels >= threshold;
                let color = if index >= LED_COUNT - 4 {
                    if active { Color32::RED } else { Color32::from_rgb(55, 24, 28) }
                } else if index >= LED_COUNT - 8 {
                    if active { Color32::YELLOW } else { Color32::from_rgb(58, 55, 24) }
                } else if active {
                    Color32::from_rgb(72, 210, 166)
                } else {
                    Color32::from_rgb(26, 67, 63)
                };
                let center =
                    egui::pos2(rect.center().x, rect.bottom() - (index as f32 + 0.5) * step);
                painter.circle_filled(center, radius, color);
            }
        });
    }

    fn draw_decibel_scale(ui: &mut Ui, height: f32) {
        ui.vertical(|ui| {
            ui.set_width(28.0);
            let (response, painter) = ui.allocate_painter(vec2(28.0, height), egui::Sense::hover());
            let rect = response.rect;
            for decibels in [0, -6, -12, -18, -24, -36, -48, -60] {
                let normalized = (decibels as f32 + 60.0) / 60.0;
                let y = rect.bottom() - normalized * rect.height();
                let y = y.clamp(rect.top() + 6.0, rect.bottom() - 6.0);
                painter.text(
                    egui::pos2(rect.left() + 1.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("{decibels}"),
                    FontId::proportional(9.0),
                    Color32::from_gray(170),
                );
            }
        });
    }

    fn draw_stereo_fader(ui: &mut Ui, height: f32, value: &mut f32) -> egui::Rect {
        let mut fader_rect = egui::Rect::NOTHING;
        ui.vertical(|ui| {
            ui.set_width(30.0);
            let (response, painter) =
                ui.allocate_painter(vec2(30.0, height), Sense::click_and_drag());
            let rect = response.rect;
            fader_rect = rect;
            let track = egui::Rect::from_center_size(
                rect.center(),
                vec2(6.0, (rect.height() - 16.0).max(1.0)),
            );
            if response.is_pointer_button_down_on() || response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    *value = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
                }
            }
            painter.rect_filled(track, 3.0, Color32::from_rgb(65, 67, 73));
            let knob_y = track.bottom() - *value * track.height();
            let knob = egui::Rect::from_center_size(
                egui::pos2(track.center().x, knob_y),
                vec2(24.0, 12.0),
            );
            painter.rect_filled(knob, 3.0, Color32::from_rgb(180, 184, 190));
            painter.rect_stroke(
                knob,
                3.0,
                egui::Stroke::new(1.0, Color32::from_gray(230)),
                egui::StrokeKind::Inside,
            );
            let decibels = if *value > 0.0 { -60.0 + *value * 60.0 } else { -60.0 };
            ui.label(format!("{decibels:.0}"));
        });
        fader_rect
    }

    fn fader_to_gain(value: f32) -> f32 {
        if value <= 0.0 {
            0.0
        } else {
            let decibels = -60.0 + value.clamp(0.0, 1.0) * 60.0;
            10.0_f32.powf(decibels / 20.0)
        }
    }

    fn draw_stereo_meter_panel(&mut self, ui: &mut Ui) {
        ui.set_min_width(154.0);
        let meter_height = (ui.available_height() - 32.0).max(180.0);
        let levels = [
            (self.channel_levels[0] * Self::fader_to_gain(self.stereo_faders[0])).clamp(0.0, 1.0),
            (self.channel_levels[1] * Self::fader_to_gain(self.stereo_faders[1])).clamp(0.0, 1.0),
        ];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            Self::draw_decibel_scale(ui, meter_height);
            Self::draw_led_meter(ui, levels[0], meter_height);
            let left_fader = Self::draw_stereo_fader(ui, meter_height, &mut self.stereo_faders[0]);
            let right_fader = Self::draw_stereo_fader(ui, meter_height, &mut self.stereo_faders[1]);
            Self::draw_led_meter(ui, levels[1], meter_height);

            let middle = egui::Rect::from_min_max(
                egui::pos2(left_fader.right() - 10.0, left_fader.top()),
                egui::pos2(right_fader.left() + 10.0, right_fader.bottom()),
            );
            let response =
                ui.interact(middle, ui.id().with("stereo_fader_link"), Sense::click_and_drag());
            let link_track = egui::Rect::from_center_size(
                middle.center(),
                vec2(2.0, (middle.height() - 16.0).max(1.0)),
            );
            if response.drag_started()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                self.stereo_link_drag_start = Some(self.stereo_faders);
                self.stereo_link_pointer_start = Some(pointer.y);
            }
            if response.clicked()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let target =
                    ((link_track.bottom() - pointer.y) / link_track.height()).clamp(0.0, 1.0);
                let center = (self.stereo_faders[0] + self.stereo_faders[1]) * 0.5;
                let min_delta = -self.stereo_faders[0].min(self.stereo_faders[1]);
                let max_delta = 1.0 - self.stereo_faders[0].max(self.stereo_faders[1]);
                let delta = (target - center).clamp(min_delta, max_delta);
                self.stereo_faders[0] = (self.stereo_faders[0] + delta).clamp(0.0, 1.0);
                self.stereo_faders[1] = (self.stereo_faders[1] + delta).clamp(0.0, 1.0);
            } else if response.is_pointer_button_down_on()
                && let (Some(start), Some(pointer_start), Some(pointer)) = (
                    self.stereo_link_drag_start,
                    self.stereo_link_pointer_start,
                    response.interact_pointer_pos(),
                )
            {
                let delta = ((pointer_start - pointer.y) / link_track.height())
                    .clamp(-start[0].min(start[1]), 1.0 - start[0].max(start[1]));
                self.stereo_faders =
                    [(start[0] + delta).clamp(0.0, 1.0), (start[1] + delta).clamp(0.0, 1.0)];
            }
            if response.drag_stopped() || response.clicked() {
                self.stereo_link_drag_start = None;
                self.stereo_link_pointer_start = None;
            }
        });
        if let Ok(mut gains) = self.channel_gains.lock() {
            *gains = [
                Self::fader_to_gain(self.stereo_faders[0]),
                Self::fader_to_gain(self.stereo_faders[1]),
            ];
        }
        if let Some(player) = self.pink_noise_player.as_ref() {
            let average_gain = (Self::fader_to_gain(self.stereo_faders[0])
                + Self::fader_to_gain(self.stereo_faders[1]))
                * 0.5;
            player.set_volume(0.8 * average_gain);
        }
    }

    fn toggle_playback(&mut self) {
        match self.workspace.transport {
            TransportState::Playing => {
                if let Some(player) = self.audio_player.as_ref() {
                    player.pause();
                }
                self.workspace.transport = TransportState::Paused;
            }
            TransportState::Paused => {
                if let Some(player) = self.audio_player.as_ref() {
                    player.play();
                    self.workspace.transport = TransportState::Playing;
                } else if let Some(index) = self
                    .selected_track
                    .or_else(|| (!self.workspace.document.tracks.is_empty()).then_some(0))
                {
                    self.play_track(index);
                }
            }
            TransportState::Stopped => {
                if let Some(index) = self
                    .selected_track
                    .or_else(|| (!self.workspace.document.tracks.is_empty()).then_some(0))
                {
                    self.play_track(index);
                }
            }
        }
    }

    fn stop_playback(&mut self) {
        if let Some(player) = self.audio_player.take() {
            player.stop();
        }
        if let Some(player) = self.chain_previous_player.take() {
            player.stop();
        }
        self.chain_transition = None;
        self.chain_transition_started = None;
        self.chain_transition_trigger_position = None;
        self.chain_tempo_start_speed = None;
        self.chain_tempo_target_speed = None;
        self.resume_position_seconds = 0.0;
        self.analysis_buffer = None;
        self.audio_metadata = None;
        self.channel_levels = [0.0; 2];
        self.stereo_spectrum = [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]];
        self.workspace.stop();
        self.workspace.spectrum = Default::default();
    }

    fn default_output_device_name() -> Option<String> {
        rodio::cpal::default_host().default_output_device().and_then(|device| {
            device.description().ok().map(|description| description.name().to_owned())
        })
    }

    fn preferred_room_output_name(&self) -> Option<String> {
        let input_name = self
            .microphone_devices
            .get(self.microphone_device_index)
            .map(ToString::to_string)
            .unwrap_or_default()
            .to_lowercase();
        let prefers_interface = input_name.contains("scarlett") || input_name.contains("focusrite");
        if prefers_interface {
            if let Some(device) =
                rodio::cpal::default_host().output_devices().ok().and_then(|devices| {
                    devices.into_iter().find(|device| {
                        let name = device
                            .description()
                            .ok()
                            .map(|description| description.name().to_lowercase())
                            .unwrap_or_default();
                        name.contains("scarlett") || name.contains("focusrite")
                    })
                })
            {
                return device.description().ok().map(|description| description.name().to_owned());
            }
        }
        Self::default_output_device_name()
    }

    fn open_room_audio_output(&self) -> Result<rodio::MixerDeviceSink, String> {
        let input_name = self
            .microphone_devices
            .get(self.microphone_device_index)
            .map(ToString::to_string)
            .unwrap_or_default()
            .to_lowercase();
        let prefers_interface = input_name.contains("scarlett") || input_name.contains("focusrite");
        let output_device = if prefers_interface {
            rodio::cpal::default_host().output_devices().ok().and_then(|devices| {
                devices.into_iter().find(|device| {
                    let name = device
                        .description()
                        .ok()
                        .map(|description| description.name().to_lowercase())
                        .unwrap_or_default();
                    name.contains("scarlett") || name.contains("focusrite")
                })
            })
        } else {
            None
        };

        let builder = if let Some(device) = output_device {
            rodio::DeviceSinkBuilder::from_device(device)
                .map_err(|error| format!("Configuration de la sortie Scarlett : {error}"))?
        } else {
            rodio::DeviceSinkBuilder::from_default_device()
                .map_err(|error| format!("Impossible d'ouvrir la sortie audio : {error}"))?
        };
        builder
            .open_sink_or_fallback()
            .map_err(|error| format!("Impossible d'ouvrir la sortie audio : {error}"))
    }

    fn start_pink_noise(&mut self) {
        if self.pink_noise_player.is_some() {
            return;
        }

        let result = (|| -> Result<(), String> {
            if self.audio_output.is_none() {
                self.audio_output_name = self.preferred_room_output_name();
                let mut output = self.open_room_audio_output()?;
                output.log_on_drop(false);
                self.audio_output = Some(output);
            }
            let output = self.audio_output.as_ref().expect("sortie audio initialisée");
            let player = rodio::Player::connect_new(output.mixer());
            player.set_volume(0.8);
            let decoder = rodio::Decoder::try_from(Cursor::new(include_bytes!(
                "../../../assets/son/bruit_rose.wav"
            )))
            .map_err(|error| format!("Impossible de décoder le bruit rose : {error}"))?;
            player.append(decoder.repeat_infinite());
            self.pink_noise_player = Some(player);
            Ok(())
        })();

        if let Err(error) = result {
            self.audio_error = Some(error);
        }
    }

    fn stop_pink_noise(&mut self) {
        if let Some(player) = self.pink_noise_player.take() {
            player.stop();
        }
    }

    fn play_track(&mut self, index: usize) {
        let transition = (self.audio_player.is_some() && self.selected_track != Some(index))
            .then(|| {
                self.chain_transition_mode()
                    .or_else(|| self.click_chain_enabled.then_some(ChainMode::VolumeFader))
            })
            .flatten();
        self.play_track_with_transition(index, transition);
    }

    fn play_track_with_transition(&mut self, index: usize, transition: Option<ChainMode>) {
        let Some(path) = self.loaded_music.get(index).cloned() else {
            return;
        };
        let previous_bpm = self.audio_metadata.as_ref().and_then(|metadata| metadata.bpm);
        let transition = transition
            .filter(|mode| *mode != ChainMode::Normal && self.chain_transition_seconds > 0.0);
        let transition_trigger_position = transition.map(|_| {
            self.audio_player
                .as_ref()
                .map(|player| player.get_pos().as_secs_f32())
                .unwrap_or(self.resume_position_seconds)
        });

        let resume_position =
            if self.selected_track == Some(index) { self.resume_position_seconds } else { 0.0 };
        self.selected_track = Some(index);
        self.track_scroll_target = Some(index);
        self.resume_position_seconds = resume_position;
        self.last_saved_position_seconds = resume_position;
        if self.track_metadata.get(index).is_some_and(Option::is_none)
            && !self.track_metadata_queued.get(index).copied().unwrap_or(false)
        {
            self.queue_metadata_load(index, path.clone());
        }
        // Pendant un fader de volume, l'ancien lecteur reste dans le mixer. Son
        // volume descendra en même temps que celui de la nouvelle piste monte.
        // Toute ancienne transition encore active est interrompue avant de
        // commencer la nouvelle.
        if let Some(player) = self.chain_previous_player.take() {
            player.stop();
        }
        let previous_player = if transition.is_some() {
            self.audio_player.take().map(|player| {
                player.set_volume(1.0);
                player
            })
        } else {
            if let Some(player) = self.audio_player.take() {
                player.stop();
            }
            None
        };
        self.chain_previous_player = previous_player;
        self.chain_transition = None;
        self.chain_transition_started = None;
        self.chain_tempo_start_speed = None;
        self.chain_tempo_target_speed = None;
        self.analysis_buffer = None;
        self.audio_metadata = None;
        self.channel_levels = [0.0; 2];
        self.stereo_spectrum = [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]];
        self.workspace.stop();
        self.audio_error = None;

        let result = (|| {
            if self.audio_output.is_none() {
                self.audio_output_name = Self::default_output_device_name();
                let mut output = rodio::DeviceSinkBuilder::open_default_sink()
                    .map_err(|error| format!("Impossible d'ouvrir la sortie audio : {error}"))?;
                output.log_on_drop(false);
                self.audio_output = Some(output);
            }
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("Impossible d'ouvrir {} : {error}", path.display()))?;
            let decoder = rodio::Decoder::try_from(file)
                .map_err(|error| format!("Impossible de décoder {} : {error}", path.display()))?;
            let channels = decoder.channels();
            let sample_rate = decoder.sample_rate();
            let duration_seconds =
                decoder.total_duration().map(|duration| duration.as_secs_f32()).unwrap_or(0.0);
            let metadata =
                self.track_metadata.get(index).and_then(Option::clone).unwrap_or_else(|| {
                    basic_audio_metadata(
                        &path,
                        sample_rate.get(),
                        channels.get() as usize,
                        duration_seconds,
                    )
                });
            let analysis_buffer = Arc::new(Mutex::new(LiveAnalysisBuffer::new(
                channels.get() as usize,
                sample_rate.get() as f32,
            )));
            let source = ChannelGainSource::new(
                decoder,
                self.channel_gains.clone(),
                analysis_buffer.clone(),
            );
            let player = {
                let output = self.audio_output.as_ref().expect("sortie audio initialisée");
                rodio::Player::connect_new(output.mixer())
            };
            if transition.is_some_and(ChainMode::has_volume_fader) {
                player.set_volume(0.0);
            }
            player.append(source);
            self.audio_player = Some(player);
            self.analysis_buffer = Some(analysis_buffer);
            self.audio_metadata = Some(metadata);
            if transition.is_some_and(ChainMode::has_tempo_fader) {
                if let Some(previous_player) = self.chain_previous_player.as_ref() {
                    self.chain_tempo_start_speed = Some(previous_player.speed());
                }
                self.chain_tempo_target_speed = previous_bpm
                    .zip(self.audio_metadata.as_ref().and_then(|metadata| metadata.bpm))
                    .filter(|(previous_bpm, next_bpm)| {
                        previous_bpm.is_finite()
                            && next_bpm.is_finite()
                            && *previous_bpm > 0.0
                            && *next_bpm > 0.0
                    })
                    .map(|(previous_bpm, next_bpm)| (next_bpm / previous_bpm).clamp(0.5, 2.0));
            }
            if resume_position > 0.0 {
                let position = resume_position.min(duration_seconds.max(0.0));
                if position > 0.0
                    && let Some(player) = self.audio_player.as_ref()
                {
                    let _ = player.try_seek(Duration::from_secs_f32(position));
                }
            }
            if let Some(track) = self.workspace.document.tracks.get_mut(index) {
                track.duration_seconds = duration_seconds;
            }
            Ok::<(), String>(())
        })();

        match result {
            Ok(()) => {
                self.workspace.transport = TransportState::Playing;
                self.chain_transition = transition;
                self.chain_transition_started = transition.map(|_| Instant::now());
                self.chain_transition_trigger_position = transition_trigger_position;
            }
            Err(error) => {
                if let Some(player) = self.chain_previous_player.take() {
                    player.stop();
                }
                self.audio_error = Some(error);
                self.chain_transition = None;
                self.chain_transition_started = None;
                self.chain_transition_trigger_position = None;
                self.chain_tempo_start_speed = None;
                self.chain_tempo_target_speed = None;
                self.workspace.stop();
            }
        }
        self.save_playlist();
        self.last_saved_position_seconds = self.resume_position_seconds;
    }

    fn finish_track(&mut self) {
        let Some(current) = self.selected_track else {
            self.stop_playback();
            return;
        };
        let track_count = self.loaded_music.len();
        if current >= track_count || track_count == 0 {
            self.stop_playback();
            return;
        }

        let (next, transition) = match (self.chain_mode, self.repeat_mode) {
            // Sans Enchaîner, seule la répétition de la piste courante reste active.
            (None, Some(RepeatMode::Track)) => (Some(current), None),
            (None, Some(RepeatMode::All) | None) => (None, None),
            // Répéter tout parcourt la liste uniquement si Enchaîner est activé.
            (Some(_), Some(RepeatMode::Track)) => (Some(current), None),
            (Some(chain_mode), Some(RepeatMode::All)) => {
                let next = if self.shuffle_enabled {
                    self.random_track_index(current, track_count)
                } else {
                    (current + 1) % track_count
                };
                (Some(next), Some(chain_mode))
            }
            (Some(chain_mode), None) => {
                let next = if self.shuffle_enabled {
                    Some(self.random_track_index(current, track_count))
                } else {
                    (current + 1 < track_count).then_some(current + 1)
                };
                (next, Some(chain_mode))
            }
        };

        if let Some(next) = next {
            if next == current {
                self.resume_position_seconds = 0.0;
                self.last_saved_position_seconds = 0.0;
            }
            self.play_track_with_transition(next, transition);
        } else {
            self.audio_player = None;
            if let Some(player) = self.chain_previous_player.take() {
                player.stop();
            }
            self.analysis_buffer = None;
            self.chain_transition = None;
            self.chain_transition_started = None;
            self.chain_transition_trigger_position = None;
            self.chain_tempo_start_speed = None;
            self.chain_tempo_target_speed = None;
            self.workspace.stop();
        }
    }

    fn update_chain_transition(&mut self) {
        let Some(mode) = self.chain_transition else {
            return;
        };
        let Some(started) = self.chain_transition_started else {
            self.chain_transition = None;
            return;
        };
        let duration = self.chain_transition_seconds.max(0.01);
        let progress = (started.elapsed().as_secs_f32() / duration).clamp(0.0, 1.0);
        if mode.has_volume_fader() {
            if let Some(player) = self.chain_previous_player.as_ref() {
                player.set_volume(1.0 - progress);
            }
        }
        if let Some(player) = self.audio_player.as_ref() {
            if mode.has_volume_fader() {
                player.set_volume(progress);
            }
        }
        if mode.has_tempo_fader()
            && let (Some(start_speed), Some(target_speed)) =
                (self.chain_tempo_start_speed, self.chain_tempo_target_speed)
            && let Some(player) = self.chain_previous_player.as_ref()
        {
            player.set_speed(start_speed + (target_speed - start_speed) * progress);
        }
        if progress >= 1.0 {
            if let Some(player) = self.chain_previous_player.take() {
                player.stop();
            }
            self.chain_transition = None;
            self.chain_transition_started = None;
            self.chain_tempo_start_speed = None;
            self.chain_tempo_target_speed = None;
        }
    }

    fn poll_metadata_loading(&mut self) {
        let Some(receiver) = self.metadata_loading.take() else {
            return;
        };
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok((path, metadata)) => {
                    if let Some(index) = self.loaded_music.iter().position(|loaded| loaded == &path)
                    {
                        self.apply_track_metadata(index, metadata);
                    }
                    self.metadata_pending = self.metadata_pending.saturating_sub(1);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected || self.metadata_pending == 0 {
            self.metadata_sender = None;
            self.metadata_loading = None;
        } else {
            self.metadata_loading = Some(receiver);
        }
    }

    fn import_music(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Charger des musiques")
            .add_filter("Fichiers audio", &["mp3", "flac", "wav", "ogg", "m4a", "aac", "opus"])
            .pick_files()
        else {
            return;
        };

        self.select_mode(WorkspaceMode::Player);
        for path in paths {
            self.add_music_path(path);
        }
        self.save_playlist();
    }

    fn spectrum_band_count_for_graph_width(graph_width: f32) -> usize {
        if graph_width >= SPECTRUM_QUADRUPLE_GRAPH_WIDTH {
            SPECTRUM_BANDS * 4
        } else if graph_width >= SPECTRUM_DOUBLE_GRAPH_WIDTH {
            SPECTRUM_BANDS * 2
        } else {
            SPECTRUM_BANDS
        }
    }

    fn current_spectrum_band_count(&self) -> usize {
        let window_width = self
            .window
            .as_ref()
            .map(|window| window.inner_size().width as f32 / window.scale_factor() as f32)
            .unwrap_or(1_280.0);
        // Le panneau droit et les marges de la vue centrale ne font pas partie
        // du graphe. Ces valeurs gardent les 64 bandes d'origine dans la
        // fenêtre minimale et passent à 256 bandes en largeur 2K.
        let graph_width = (window_width - 170.0 - 38.0).max(0.0);
        Self::spectrum_band_count_for_graph_width(graph_width)
    }

    fn analyze_buffer(
        buffer: Option<&Arc<Mutex<LiveAnalysisBuffer>>>,
        band_count: usize,
    ) -> Option<([Vec<f32>; 2], [f32; 2], f32, Vec<f32>)> {
        let analysis = buffer?.lock().ok()?;
        let channels = analysis.channels.max(1);
        let sample_rate = analysis.sample_rate;
        let total_frames = analysis.samples.len() / channels;
        if total_frames == 0 || sample_rate <= 0.0 {
            return None;
        }
        let frame_count = total_frames.min(SPECTRUM_WINDOW_FRAMES);
        let start_frame = total_frames - frame_count;
        let pitch_count = total_frames.min(PITCH_WINDOW_FRAMES);
        let pitch_start = total_frames - pitch_count;
        let mut channel_samples =
            [vec![0.0; SPECTRUM_WINDOW_FRAMES], vec![0.0; SPECTRUM_WINDOW_FRAMES]];
        let mut mono_samples = Vec::with_capacity(pitch_count);
        let mut levels = [0.0_f32; 2];
        for offset in 0..frame_count {
            let frame = start_frame + offset;
            let first_sample = frame * channels;
            let left = analysis.samples[first_sample];
            let right = if channels > 1 { analysis.samples[first_sample + 1] } else { left };
            channel_samples[0][offset] = left;
            channel_samples[1][offset] = right;
            levels[0] = levels[0].max(left.abs());
            levels[1] = levels[1].max(right.abs());
        }
        for offset in 0..pitch_count {
            let frame = pitch_start + offset;
            let first_sample = frame * channels;
            let left = analysis.samples[first_sample];
            let right = if channels > 1 { analysis.samples[first_sample + 1] } else { left };
            mono_samples.push((left + right) * 0.5);
        }
        let analyzer = SpectrumAnalyzer::new(sample_rate);
        let left_bands = analyzer.analyze_with_band_count(&channel_samples[0], band_count).ok()?;
        let right_bands = analyzer.analyze_with_band_count(&channel_samples[1], band_count).ok()?;
        Some(([left_bands, right_bands], levels, sample_rate, mono_samples))
    }

    fn update_spectrum_from_buffer(
        &mut self,
        buffer: Option<&Arc<Mutex<LiveAnalysisBuffer>>>,
    ) -> Option<(f32, Vec<f32>)> {
        let analyzed = Self::analyze_buffer(buffer, self.current_spectrum_band_count());
        if let Some((stereo_bands, channel_levels, sample_rate, mono_samples)) = analyzed {
            let mut bands = [0.0; 32];
            let workspace_band_count = bands.len();
            for (index, band) in bands.iter_mut().enumerate() {
                let first = index * stereo_bands[0].len() / workspace_band_count;
                let last = ((index + 1) * stereo_bands[0].len() / workspace_band_count)
                    .max(first + 1)
                    .min(stereo_bands[0].len());
                let count = (last - first) as f32;
                *band = (stereo_bands[0][first..last].iter().sum::<f32>()
                    + stereo_bands[1][first..last].iter().sum::<f32>())
                    / (count * 2.0);
            }
            self.stereo_spectrum = stereo_bands;
            self.workspace.spectrum.bands = bands;
            self.workspace.spectrum.peak = bands.iter().copied().fold(0.0, f32::max);
            self.channel_levels = channel_levels;
            Some((sample_rate, mono_samples))
        } else {
            self.stereo_spectrum = [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]];
            self.workspace.spectrum = Default::default();
            self.channel_levels = [0.0; 2];
            None
        }
    }

    fn update_spectrum_from_playback(&mut self) {
        let buffer = self.analysis_buffer.clone();
        let _ = self.update_spectrum_from_buffer(buffer.as_ref());
    }

    fn update_spectrum_from_microphone(&mut self) {
        self.update_live_analysis_from_buffer(self.microphone_buffer.clone(), true);
    }

    fn update_live_analysis_from_buffer(
        &mut self,
        buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
        microphone_input: bool,
    ) {
        if let Some((sample_rate, samples)) = self.update_spectrum_from_buffer(buffer.as_ref()) {
            if microphone_input {
                self.microphone_input_peak =
                    self.channel_levels.iter().copied().fold(0.0, f32::max);
            }
            self.microphone_pitch = play_media::detect_pitch(&samples, sample_rate);
            self.microphone_chord = play_media::detect_chord(&samples, sample_rate);
            self.update_tablature_history();
            if microphone_input {
                self.update_room_spectrum_from_buffer(buffer.as_ref());
            }
        } else {
            if microphone_input {
                self.microphone_input_peak = 0.0;
                self.room_spectrum = [0.0; ROOM_SPECTRUM_BANDS];
            }
            self.microphone_pitch = None;
            self.microphone_chord = None;
            self.tablature_last_position = None;
        }
    }

    fn update_live_analysis(&mut self) {
        match self.live_analysis_source {
            LiveAnalysisSource::Microphone => self.update_spectrum_from_microphone(),
            LiveAnalysisSource::PlaybackOutput => {
                self.update_live_analysis_from_buffer(self.analysis_buffer.clone(), false);
            }
        }
    }

    fn update_tablature_history(&mut self) {
        if !self.tablature_playing || !self.tablature_recording {
            return;
        }
        let now = Instant::now();
        let Some(pitch) = self.microphone_pitch else {
            self.tablature_last_position = None;
            return;
        };
        let Some((string, fret)) = Self::guitar_position(pitch.midi_note) else {
            self.tablature_last_position = None;
            return;
        };
        let position = (Self::tablature_line(string), fret);
        let changed = self.tablature_last_position != Some(position);
        let repeat_due = self.tablature_last_recorded_at.is_none_or(|recorded_at| {
            recorded_at.elapsed().as_secs_f32() >= TABLATURE_NOTE_REPEAT_SECONDS
        });
        if changed || repeat_due {
            if self.tablature_measure_origin.is_none() {
                self.tablature_measure_origin = Some(now);
            }
            self.tablature_notes.push_back(TablatureNote {
                recorded_at: now,
                line: position.0,
                fret: position.1,
                midi_note: pitch.midi_note,
            });
            while self.tablature_notes.len() > TABLATURE_HISTORY_LIMIT {
                self.tablature_notes.pop_front();
            }
            self.tablature_last_position = Some(position);
            self.tablature_last_recorded_at = Some(now);
        }
    }

    fn toggle_tablature_playback(&mut self) {
        if self.tablature_playing {
            self.tablature_playing = false;
            self.tablature_pause_started = Some(Instant::now());
            return;
        }

        if let Some(paused_at) = self.tablature_pause_started.take() {
            let pause_duration = paused_at.elapsed();
            if let Some(origin) = &mut self.tablature_measure_origin {
                *origin += pause_duration;
            }
            for note in &mut self.tablature_notes {
                note.recorded_at += pause_duration;
            }
            if let Some(recorded_at) = &mut self.tablature_last_recorded_at {
                *recorded_at += pause_duration;
            }
        }
        self.tablature_playing = true;
    }

    fn toggle_tablature_recording(&mut self) {
        self.tablature_recording = !self.tablature_recording;
        if self.tablature_recording {
            self.tablature_last_position = None;
            self.tablature_last_recorded_at = None;
        }
    }

    fn clear_tablature(&mut self) {
        self.tablature_notes.clear();
        self.tablature_last_position = None;
        self.tablature_last_recorded_at = None;
        self.tablature_measure_origin = None;
    }

    fn microphone_compensation_db(profile: MicrophoneProfile, frequency: f32) -> f32 {
        // Cette courbe représente une réponse nominale utilisable pour une
        // première lecture. Les ECM8000 étant calibrés individuellement, elle
        // ne remplace pas le fichier de calibration de l'exemplaire utilisé.
        const ECM8000_COMPENSATION: [(f32, f32); 14] = [
            (20.0, -1.5),
            (31.5, -1.0),
            (63.0, -0.5),
            (125.0, 0.0),
            (250.0, 0.3),
            (500.0, 0.2),
            (1_000.0, 0.0),
            (2_000.0, -0.2),
            (4_000.0, 0.0),
            (8_000.0, -0.8),
            (10_000.0, -1.2),
            (12_500.0, -2.0),
            (16_000.0, -3.5),
            (20_000.0, -5.0),
        ];

        let MicrophoneProfile::BehringerEcm8000 = profile else {
            return 0.0;
        };
        if !frequency.is_finite() {
            return 0.0;
        }
        let frequency = frequency.clamp(
            ECM8000_COMPENSATION[0].0,
            ECM8000_COMPENSATION[ECM8000_COMPENSATION.len() - 1].0,
        );
        for pair in ECM8000_COMPENSATION.windows(2) {
            let [(lower_frequency, lower_db), (upper_frequency, upper_db)] = pair else {
                continue;
            };
            if frequency <= *upper_frequency {
                let position = (frequency.ln() - lower_frequency.ln())
                    / (upper_frequency.ln() - lower_frequency.ln());
                return lower_db + (upper_db - lower_db) * position.clamp(0.0, 1.0);
            }
        }
        ECM8000_COMPENSATION.last().map_or(0.0, |(_, db)| *db)
    }

    fn update_room_spectrum_from_buffer(
        &mut self,
        buffer: Option<&Arc<Mutex<LiveAnalysisBuffer>>>,
    ) {
        let Some(buffer) = buffer else {
            self.room_spectrum = [0.0; ROOM_SPECTRUM_BANDS];
            return;
        };
        let Ok(analysis) = buffer.lock() else {
            return;
        };
        let channels = analysis.channels.max(1);
        let total_frames = analysis.samples.len() / channels;
        let frame_count = total_frames.min(SPECTRUM_WINDOW_FRAMES);
        if frame_count == 0 || analysis.sample_rate <= 0.0 {
            self.room_spectrum = [0.0; ROOM_SPECTRUM_BANDS];
            return;
        }
        let sample_rate = analysis.sample_rate;
        let start_frame = total_frames - frame_count;
        let mut samples = Vec::with_capacity(frame_count);
        for frame in start_frame..total_frames {
            let first_sample = frame * channels;
            let left = analysis.samples[first_sample];
            let right = if channels > 1 { analysis.samples[first_sample + 1] } else { left };
            samples.push((left + right) * 0.5);
        }
        drop(analysis);

        let analyzer = SpectrumAnalyzer::new(sample_rate);
        let Ok(bands) = analyzer.analyze_with_band_count(&samples, ROOM_SPECTRUM_BANDS) else {
            return;
        };
        let max_frequency = (sample_rate * 0.5).min(SPECTRUM_MAX_FREQUENCY);
        let min_frequency = SPECTRUM_MIN_FREQUENCY.min(max_frequency * 0.5);
        if max_frequency <= min_frequency {
            self.room_spectrum.copy_from_slice(&bands);
            return;
        }
        let ratio = (max_frequency / min_frequency).powf(1.0 / ROOM_SPECTRUM_BANDS as f32);
        let profile = self.microphone_profile;
        for (band, level) in bands.into_iter().enumerate() {
            let lower = min_frequency * ratio.powf(band as f32);
            let upper = (min_frequency * ratio.powf(band as f32 + 1.0)).min(max_frequency);
            let center_frequency = (lower * upper).sqrt();
            let compensation_db = Self::microphone_compensation_db(profile, center_frequency);
            self.room_spectrum[band] =
                (level * 10.0_f32.powf(compensation_db / 20.0)).clamp(0.0, 1.0);
        }
    }

    fn draw_room_measurement_dialog(&mut self, ctx: &Context) {
        if !self.show_room_measurement_dialog {
            return;
        }

        let mut start_requested = false;
        let response = egui::Modal::new(egui::Id::new("room_measurement_setup")).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.heading("Avant la mesure");
            ui.add_space(6.0);
            ui.label("Branchez l'ECM8000 sur la Scarlett Solo et activez le bouton 48V.");
            ui.label("Play ne peut pas activer l'alimentation fantôme à votre place.");
            ui.add_space(8.0);
            ui.colored_label(
                Color32::from_rgb(255, 205, 95),
                "Sans 48V, l'ECM8000 ne transmettra aucun signal.",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("J'ai activé le 48V — démarrer").clicked() {
                    start_requested = true;
                }
                if ui.button("Annuler").clicked() {
                    ui.close();
                }
            });
        });

        if start_requested {
            self.start_room_measurement();
        } else if response.should_close() {
            self.show_room_measurement_dialog = false;
        }
    }

    fn draw_music_loader(&mut self, ctx: &Context) {
        if !self.show_music_loader {
            return;
        }

        let mut open = true;
        let mut browse_requested = false;
        let mut close_requested = false;
        egui::Window::new("Charger des musiques")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(560.0, 420.0))
            .min_size(vec2(420.0, 280.0))
            .show(ctx, |ui| {
                ui.label("Ajoutez les fichiers audio qui composeront la session Play.");
                ui.add_space(8.0);
                if ui.button("Parcourir les fichiers…").clicked() {
                    browse_requested = true;
                }
                ui.separator();

                if self.loaded_music.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Aucune musique chargée").strong());
                        ui.label("Formats : MP3, FLAC, WAV, OGG, M4A, AAC et Opus");
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("loaded_music_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (index, path) in self.loaded_music.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("\u{e405}").font(FontId::new(
                                        18.0,
                                        FontFamily::Name("material_icons".into()),
                                    )));
                                    ui.vertical(|ui| {
                                        ui.label(
                                            path.file_name()
                                                .and_then(|name| name.to_str())
                                                .unwrap_or("Musique"),
                                        );
                                        ui.small(path.to_string_lossy());
                                    });
                                });
                                if index + 1 < self.loaded_music.len() {
                                    ui.separator();
                                }
                            }
                        });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("{} fichier(s)", self.loaded_music.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Fermer").clicked() {
                            close_requested = true;
                        }
                    });
                });
            });

        if browse_requested {
            self.import_music();
        }
        self.show_music_loader = open && !close_requested;
    }

    fn note_name(midi_note: u8) -> &'static str {
        ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"][midi_note as usize % 12]
    }

    fn chord_name(chord: ChordEstimate) -> String {
        let suffix = match chord.quality {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Power => "5",
            ChordQuality::Sus2 => "sus2",
            ChordQuality::Sus4 => "sus4",
        };
        format!("{}{suffix}", Self::note_name(chord.root))
    }

    fn guitar_position(midi_note: u8) -> Option<(&'static str, u8)> {
        let strings =
            [(40_u8, "E grave"), (45, "A"), (50, "D"), (55, "G"), (59, "B"), (64, "E aiguë")];
        strings
            .into_iter()
            .filter_map(|(open, name)| {
                midi_note.checked_sub(open).filter(|fret| *fret <= 20).map(|fret| (name, fret))
            })
            .min_by_key(|(_, fret)| *fret)
    }

    fn tablature_line(string: &str) -> usize {
        match string {
            "E aiguë" => 0,
            "B" => 1,
            "G" => 2,
            "D" => 3,
            "A" => 4,
            "E grave" => 5,
            _ => 0,
        }
    }

    fn draw_tablature_staff(
        painter: &egui::Painter,
        rect: egui::Rect,
        note_markers: &[(f32, usize, u8)],
    ) {
        let strings = ["e", "B", "G", "D", "A", "E"];
        let line_left = rect.left() + 34.0;
        let line_right = rect.right();
        let line_top = rect.top() + 12.0;
        let line_bottom = rect.bottom() - 22.0;
        let line_gap = (line_bottom - line_top) / 5.0;
        for (index, label) in strings.iter().enumerate() {
            let y = line_top + index as f32 * line_gap;
            painter.line_segment(
                [egui::pos2(line_left, y), egui::pos2(line_right, y)],
                egui::Stroke::new(1.5, Color32::from_rgb(125, 145, 155)),
            );
            painter.text(
                egui::pos2(rect.left(), y),
                egui::Align2::LEFT_CENTER,
                *label,
                FontId::proportional(15.0),
                Color32::from_gray(210),
            );
        }

        let fret_count = 12;
        for division in 0..=fret_count {
            let x = line_left + (line_right - line_left) * division as f32 / fret_count as f32;
            painter.line_segment(
                [egui::pos2(x, line_top), egui::pos2(x, line_bottom)],
                egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 130, 145, 45)),
            );
        }

        for &(x, line, fret) in note_markers {
            let y = line_top + line as f32 * line_gap;
            painter.circle_filled(egui::pos2(x, y), 10.0, Color32::from_rgb(49, 126, 123));
            painter.text(
                egui::pos2(x, y),
                egui::Align2::CENTER_CENTER,
                fret.to_string(),
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }

    fn diatonic_staff_position(midi_note: u8) -> i32 {
        let note_in_octave = match midi_note % 12 {
            0 | 1 => 0,
            2 | 3 => 1,
            4 => 2,
            5 | 6 => 3,
            7 | 8 => 4,
            9 | 10 => 5,
            11 => 6,
            _ => 0,
        };
        let octave = midi_note as i32 / 12 - 1;
        (octave + 1) * 7 + note_in_octave - 37
    }

    fn draw_standard_staff(painter: &egui::Painter, rect: egui::Rect, note_markers: &[(f32, u8)]) {
        let painter = painter.with_clip_rect(rect);
        let line_left = rect.left() + 44.0;
        let line_right = rect.right();
        let line_gap = 6.0;
        let top_line = rect.center().y - line_gap * 2.0;
        let bottom_line = top_line + line_gap * 4.0;
        let line_color = Color32::from_rgb(165, 175, 185);

        for line in 0..5 {
            let y = top_line + line as f32 * line_gap;
            painter.line_segment(
                [egui::pos2(line_left, y), egui::pos2(line_right, y)],
                egui::Stroke::new(1.4, line_color),
            );
        }
        painter.text(
            // La boucle de la clé de sol doit se placer sur la deuxième ligne
            // en partant du bas, qui porte la note Sol.
            egui::pos2(rect.left() + 4.0, top_line + line_gap * 3.0),
            egui::Align2::LEFT_CENTER,
            "𝄞",
            FontId::proportional(42.0),
            Color32::from_gray(215),
        );

        for &(x, midi_note) in note_markers {
            let staff_position = Self::diatonic_staff_position(midi_note);
            let y = bottom_line - staff_position as f32 * (line_gap * 0.5);
            let note_color = Color32::from_rgb(110, 220, 190);

            if staff_position < 0 || staff_position > 8 {
                let first = if staff_position < 0 { staff_position } else { 8 };
                let last = if staff_position < 0 { 0 } else { staff_position };
                for ledger in (first..=last).step_by(2) {
                    let ledger_y = bottom_line - ledger as f32 * (line_gap * 0.5);
                    painter.line_segment(
                        [egui::pos2(x - 8.0, ledger_y), egui::pos2(x + 8.0, ledger_y)],
                        egui::Stroke::new(1.2, line_color),
                    );
                }
            }

            painter.circle_filled(egui::pos2(x, y), 5.5, note_color);
            painter.line_segment(
                [egui::pos2(x + 5.0, y), egui::pos2(x + 5.0, y - 25.0)],
                egui::Stroke::new(1.5, note_color),
            );
            if matches!(midi_note % 12, 1 | 3 | 6 | 8 | 10) {
                painter.text(
                    egui::pos2(x - 13.0, y),
                    egui::Align2::RIGHT_CENTER,
                    "♯",
                    FontId::proportional(15.0),
                    note_color,
                );
            }
        }
    }

    fn draw_measure_lines(
        painter: &egui::Painter,
        rect: egui::Rect,
        line_left_margin: f32,
        top_margin: f32,
        bottom_margin: f32,
        row_start_time: f32,
        seconds_per_row: f32,
        show_labels: bool,
        bpm: f32,
    ) {
        let beat_seconds = 60.0 / bpm.max(30.0);
        let measure_seconds = beat_seconds * 4.0;
        let line_left = rect.left() + line_left_margin;
        let line_right = rect.right();
        let line_width = line_right - line_left;
        let line_top = rect.top() + top_margin;
        let line_bottom = rect.bottom() - bottom_margin;

        let row_end_time = row_start_time + seconds_per_row;
        let first_bar_time = (row_start_time / measure_seconds).ceil().max(0.0) * measure_seconds;
        let mut bar_time = first_bar_time;
        while bar_time <= row_end_time + f32::EPSILON {
            for beat in 0..4 {
                let time = bar_time + beat as f32 * beat_seconds;
                if time < row_start_time || time > row_end_time {
                    continue;
                }
                let progress = (time - row_start_time) / seconds_per_row;
                let x = line_left + line_width * progress;
                let is_measure = beat == 0;
                painter.line_segment(
                    [egui::pos2(x, line_top), egui::pos2(x, line_bottom)],
                    egui::Stroke::new(
                        if is_measure { 1.8 } else { 0.8 },
                        if is_measure {
                            Color32::from_rgba_unmultiplied(190, 205, 215, 150)
                        } else {
                            Color32::from_rgba_unmultiplied(145, 165, 180, 55)
                        },
                    ),
                );
                if show_labels && is_measure {
                    let measure_number = (time / measure_seconds).floor() as usize + 1;
                    painter.text(
                        egui::pos2(x + 4.0, rect.top() + 4.0),
                        egui::Align2::LEFT_TOP,
                        format!("M{measure_number}"),
                        FontId::proportional(10.0),
                        Color32::from_gray(155),
                    );
                }
            }
            bar_time += measure_seconds;
        }
    }

    fn draw_live_tablature_panel(&self, ui: &mut Ui) {
        let desired_size = vec2(ui.available_width(), ui.available_height().max(320.0));
        let (response, painter) = ui.allocate_painter(desired_size, egui::Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 8.0, Color32::from_rgb(21, 25, 35));
        let content = rect.shrink2(vec2(18.0, 14.0));
        let show_standard_staff = matches!(
            self.tablature_display_mode,
            TablatureDisplayMode::Staff | TablatureDisplayMode::Both
        );
        let show_tablature = matches!(
            self.tablature_display_mode,
            TablatureDisplayMode::Tablature | TablatureDisplayMode::Both
        );
        painter.text(
            content.left_top(),
            egui::Align2::LEFT_TOP,
            match self.tablature_display_mode {
                TablatureDisplayMode::Tablature => "TABLATURE GUITARE",
                TablatureDisplayMode::Staff => "PORTÉE CLASSIQUE",
                TablatureDisplayMode::Both => "PORTÉE + TABLATURE GUITARE",
            },
            FontId::proportional(16.0),
            Color32::from_gray(210),
        );

        if let Some(chord) = self.microphone_chord {
            painter.text(
                content.right_top(),
                egui::Align2::RIGHT_TOP,
                format!("Accord : {}", Self::chord_name(chord)),
                FontId::proportional(14.0),
                Color32::from_rgb(110, 220, 190),
            );
        }

        let staff_area = egui::Rect::from_min_max(
            egui::pos2(content.left(), content.top() + 36.0),
            egui::pos2(content.right(), content.bottom() - 30.0),
        );
        let staff_height = match (show_standard_staff, show_tablature) {
            (true, true) => 310.0,
            (true, false) => 88.0,
            (false, true) => 112.0,
            (false, false) => 112.0,
        };
        let staff_step = match (show_standard_staff, show_tablature) {
            (true, true) => 320.0,
            (true, false) => 100.0,
            (false, true) => 145.0,
            (false, false) => 145.0,
        };
        let staff_count =
            ((staff_area.height() + staff_step - staff_height) / staff_step).floor() as usize;
        let visible_staff_count = staff_count.max(1);
        let mut notes_by_staff: Vec<Vec<(f32, usize, u8)>> = vec![Vec::new(); visible_staff_count];
        let mut standard_notes_by_staff: Vec<Vec<(f32, u8)>> =
            vec![Vec::new(); visible_staff_count];
        let line_left = staff_area.left() + 34.0;
        let line_right = staff_area.right();
        let seconds_per_staff = ((line_right - line_left) / TABLATURE_NOTE_SPACING_PX
            * TABLATURE_NOTE_REPEAT_SECONDS)
            .max(TABLATURE_NOTE_REPEAT_SECONDS);
        let now = Instant::now();
        let timeline_now =
            if self.tablature_playing { now } else { self.tablature_pause_started.unwrap_or(now) };
        let elapsed = self
            .tablature_measure_origin
            .map(|origin| timeline_now.duration_since(origin).as_secs_f32())
            .unwrap_or(0.0)
            .max(0.0);
        let current_staff = (elapsed / seconds_per_staff).floor() as usize;
        let first_visible_staff = current_staff.saturating_sub(visible_staff_count - 1);
        let last_visible_staff = first_visible_staff + visible_staff_count;
        for note in &self.tablature_notes {
            let Some(origin) = self.tablature_measure_origin else {
                continue;
            };
            let note_time = note
                .recorded_at
                .checked_duration_since(origin)
                .map(|duration| duration.as_secs_f32())
                .unwrap_or(0.0);
            let note_staff = (note_time / seconds_per_staff).floor() as usize;
            if note_staff < first_visible_staff || note_staff >= last_visible_staff {
                continue;
            }
            let row = note_staff - first_visible_staff;
            let progress = (note_time % seconds_per_staff) / seconds_per_staff;
            let x = line_left + (line_right - line_left) * progress;
            notes_by_staff[row].push((x, note.line, note.fret));
            standard_notes_by_staff[row].push((x, note.midi_note));
        }
        for row in 0..visible_staff_count {
            let top = staff_area.top() + row as f32 * staff_step;
            if top + staff_height > staff_area.bottom() {
                break;
            }
            if show_standard_staff {
                let standard_height = if show_tablature { 175.0 } else { staff_height };
                let standard_rect = egui::Rect::from_min_max(
                    egui::pos2(staff_area.left(), top),
                    egui::pos2(staff_area.right(), top + standard_height),
                );
                Self::draw_measure_lines(
                    &painter,
                    standard_rect,
                    44.0,
                    14.0,
                    8.0,
                    (first_visible_staff + row) as f32 * seconds_per_staff,
                    seconds_per_staff,
                    true,
                    self.metronome_bpm,
                );
                Self::draw_standard_staff(&painter, standard_rect, &standard_notes_by_staff[row]);
            }
            if show_tablature {
                let tablature_top = if show_standard_staff { top + 190.0 } else { top };
                let tablature_rect = egui::Rect::from_min_max(
                    egui::pos2(staff_area.left(), tablature_top),
                    egui::pos2(staff_area.right(), tablature_top + 112.0),
                );
                Self::draw_measure_lines(
                    &painter,
                    tablature_rect,
                    34.0,
                    12.0,
                    22.0,
                    (first_visible_staff + row) as f32 * seconds_per_staff,
                    seconds_per_staff,
                    !show_standard_staff,
                    self.metronome_bpm,
                );
                Self::draw_tablature_staff(&painter, tablature_rect, &notes_by_staff[row]);
            }
        }

        if let Some(pitch) = self.microphone_pitch {
            if let Some((_, fret)) = Self::guitar_position(pitch.midi_note) {
                painter.text(
                    egui::pos2(content.left(), content.bottom()),
                    egui::Align2::LEFT_BOTTOM,
                    format!(
                        "{}{} • {:.1} Hz • {:+.1} cents • case {}",
                        Self::note_name(pitch.midi_note),
                        pitch.midi_note / 12 - 1,
                        pitch.frequency,
                        pitch.cents,
                        fret
                    ),
                    FontId::proportional(13.0),
                    Color32::from_rgb(110, 220, 190),
                );
            }
        } else if self.tablature_notes.is_empty() {
            painter.text(
                content.center(),
                egui::Align2::CENTER_CENTER,
                "Chantez ou jouez une note",
                FontId::proportional(20.0),
                Color32::from_gray(145),
            );
        }
    }

    fn draw_microphone_controls(&mut self, ui: &mut Ui) {
        if self.microphone_devices.is_empty() {
            self.refresh_microphone_devices();
        }
        let names: Vec<String> = self.microphone_devices.iter().map(ToString::to_string).collect();
        let previous_index = self.microphone_device_index;
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label("Entrée audio");
            egui::ComboBox::from_id_salt("microphone_input_device")
                .selected_text(
                    names
                        .get(self.microphone_device_index)
                        .map(String::as_str)
                        .unwrap_or("Aucune entrée"),
                )
                .show_ui(ui, |ui| {
                    for (index, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.microphone_device_index, index, name);
                    }
                });
            if ui.button("Actualiser").clicked() {
                refresh = true;
            }
        });
        if refresh {
            self.refresh_microphone_devices();
        }
        if self.microphone_device_index != previous_index || refresh {
            self.stop_microphone();
            if matches!(self.scene.mode, WorkspaceMode::RoomAnalyzer | WorkspaceMode::LiveTablature)
            {
                self.start_microphone();
            }
        }
    }

    fn track_is_playing(&self) -> bool {
        self.workspace.transport == TransportState::Playing
            && self.audio_player.as_ref().is_some_and(|player| !player.empty())
    }

    fn draw_current_track_banner(&self, ui: &mut Ui) {
        let selected =
            self.selected_track.and_then(|index| self.workspace.document.tracks.get(index));
        let state = match self.workspace.transport {
            TransportState::Playing => "Lecture",
            TransportState::Paused => "Pause",
            TransportState::Stopped => "Arrêtée",
        };
        let position = self
            .audio_player
            .as_ref()
            .map(|player| player.get_pos().as_secs_f32())
            .unwrap_or(self.resume_position_seconds);
        let duration = selected.map(|track| track.duration_seconds).unwrap_or(0.0);

        Frame::new()
            .fill(Color32::from_rgb(38, 44, 54))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("\u{e405}")
                            .font(FontId::new(20.0, FontFamily::Name("material_icons".into()))),
                    );
                    ui.vertical(|ui| {
                        if let Some(track) = selected {
                            ui.label(RichText::new(&track.title).strong());
                            if !track.artist.is_empty() {
                                ui.small(&track.artist);
                            }
                        } else {
                            ui.label(RichText::new("Aucune piste sélectionnée").strong());
                            ui.small("Chargez une musique depuis la barre d'outils");
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            if self.track_is_playing() {
                                Color32::from_rgb(110, 220, 190)
                            } else {
                                Color32::from_gray(170)
                            },
                            state,
                        );
                        if duration > 0.0 {
                            ui.small(format!(
                                "{:02}:{:02} / {:02}:{:02}",
                                (position as u64) / 60,
                                (position as u64) % 60,
                                (duration as u64) / 60,
                                (duration as u64) % 60
                            ));
                        }
                        if let Some(output_name) = self.audio_output_name.as_deref() {
                            ui.small(format!("Sortie : {output_name}"));
                        }
                    });
                });
            });
    }

    fn draw_room_analyzer(&mut self, ui: &mut Ui) {
        self.draw_current_track_banner(ui);
        ui.add_space(8.0);
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Room Analyzer");
                    ui.separator();
                    if self.microphone_error.is_some() {
                        ui.colored_label(Color32::LIGHT_RED, "Micro indisponible");
                    } else {
                        ui.colored_label(Color32::from_rgb(110, 220, 190), "Micro actif");
                    }
                });
                if self.track_is_playing() {
                    ui.colored_label(
                        Color32::from_rgb(110, 220, 190),
                        "La piste en cours sert de source pour l'analyse du micro.",
                    );
                }
                self.draw_microphone_controls(ui);
                if self.microphone_profile == MicrophoneProfile::BehringerEcm8000 {
                    ui.colored_label(
                        Color32::from_rgb(255, 205, 95),
                        "ECM8000 : activez l'alimentation fantôme 48 V sur la Scarlett Solo.",
                    );
                }
                let input_peak_db = if self.microphone_input_peak <= 0.000001 {
                    "-∞ dBFS".to_owned()
                } else {
                    format!("{:.1} dBFS", 20.0 * self.microphone_input_peak.log10())
                };
                let input_status = if self.microphone_input_peak <= 0.000001 {
                    "aucun signal reçu — vérifiez le 48 V et l'entrée Scarlett"
                } else {
                    "signal reçu"
                };
                ui.colored_label(
                    if self.microphone_input_peak <= 0.000001 {
                        Color32::LIGHT_RED
                    } else {
                        Color32::from_rgb(110, 220, 190)
                    },
                    format!("Niveau micro : {input_peak_db} • {input_status}"),
                );
                if let Some(error) = &self.microphone_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if let Some(error) = &self.audio_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.add_space(6.0);
                self.draw_room_spectrum(ui);
            });
    }

    fn draw_live_tablature(&mut self, ui: &mut Ui) {
        self.draw_current_track_banner(ui);
        ui.add_space(8.0);
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Live Tablature");
                    ui.separator();
                    if self.live_analysis_source == LiveAnalysisSource::PlaybackOutput {
                        ui.colored_label(
                            Color32::from_rgb(110, 220, 190),
                            "Analyse de la sortie Play",
                        );
                    } else if self.microphone_error.is_some() {
                        ui.colored_label(Color32::LIGHT_RED, "Micro indisponible");
                    } else {
                        ui.colored_label(Color32::from_rgb(110, 220, 190), "Écoute du micro");
                    }
                });
                let previous_source = self.live_analysis_source;
                ui.horizontal(|ui| {
                    ui.label("Source analysée");
                    egui::ComboBox::from_id_salt("live_analysis_source")
                        .selected_text(self.live_analysis_source.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.live_analysis_source,
                                LiveAnalysisSource::Microphone,
                                LiveAnalysisSource::Microphone.label(),
                            );
                            ui.selectable_value(
                                &mut self.live_analysis_source,
                                LiveAnalysisSource::PlaybackOutput,
                                LiveAnalysisSource::PlaybackOutput.label(),
                            );
                        });
                });
                if self.live_analysis_source != previous_source {
                    self.stop_microphone();
                    if self.live_analysis_source == LiveAnalysisSource::Microphone {
                        self.start_microphone();
                    }
                    self.clear_tablature();
                }
                if self.live_analysis_source == LiveAnalysisSource::PlaybackOutput {
                    ui.colored_label(
                        if self.audio_player.is_some() {
                            Color32::from_rgb(110, 220, 190)
                        } else {
                            Color32::LIGHT_RED
                        },
                        if self.audio_player.is_some() {
                            "La source suit la musique jouée par Play."
                        } else {
                            "Aucune piste Play en lecture. Lancez une piste depuis la barre principale."
                        },
                    );
                    if let Some(output_name) = self.audio_output_name.as_deref() {
                        ui.small(format!("Sortie active : {output_name}"));
                    }
                } else if let Some(output_name) = self.microphone_monitor_output_name.as_deref() {
                    ui.colored_label(
                        Color32::from_rgb(110, 220, 190),
                        format!("Monitoring casque actif : {output_name}"),
                    );
                }
                ui.horizontal(|ui| {
                    let tablature = ui.selectable_label(!self.live_tab_strumming, "🎸 TABLATURE");
                    let strumming = ui.selectable_label(self.live_tab_strumming, "↕ STRUMMING");
                    if tablature.clicked() {
                        self.live_tab_strumming = false;
                    }
                    if strumming.clicked() {
                        self.live_tab_strumming = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Mode");
                    egui::ComboBox::from_id_salt("live_tab_mode")
                        .selected_text("Gamme / accords guitare")
                        .show_ui(ui, |ui| {
                            let _ = ui.selectable_label(true, "Gamme / accords guitare");
                            let _ = ui.selectable_label(false, "Notes seules");
                        });
                    ui.label("Affichage");
                    egui::ComboBox::from_id_salt("live_tab_display_mode")
                        .selected_text(self.tablature_display_mode.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.tablature_display_mode,
                                TablatureDisplayMode::Tablature,
                                "Tablature",
                            );
                            ui.selectable_value(
                                &mut self.tablature_display_mode,
                                TablatureDisplayMode::Staff,
                                "Portée",
                            );
                            ui.selectable_value(
                                &mut self.tablature_display_mode,
                                TablatureDisplayMode::Both,
                                "Portée + tablature",
                            );
                        });
                    let metronome_label =
                        if self.metronome_enabled { "Métronome : ON" } else { "Métronome : OFF" };
                    if ui.button(metronome_label).clicked() {
                        self.metronome_enabled = !self.metronome_enabled;
                    }
                });
                if self.live_analysis_source == LiveAnalysisSource::Microphone {
                    self.draw_microphone_controls(ui);
                    let input_peak_db = if self.microphone_input_peak <= 0.000001 {
                        "-∞ dBFS".to_owned()
                    } else {
                        format!("{:.1} dBFS", 20.0 * self.microphone_input_peak.log10())
                    };
                    ui.colored_label(
                        if self.microphone_input_peak <= 0.000001 {
                            Color32::LIGHT_RED
                        } else {
                            Color32::from_rgb(110, 220, 190)
                        },
                        format!("Niveau guitare : {input_peak_db}"),
                    );
                    match self.microphone_profile {
                        MicrophoneProfile::BehringerEcm8000 => {
                            ui.colored_label(
                                Color32::from_rgb(255, 205, 95),
                                "ECM8000 sélectionné : alimentation fantôme 48 V requise.",
                            );
                        }
                        MicrophoneProfile::ShureBeta58a => {
                            ui.colored_label(
                                Color32::from_rgb(110, 220, 190),
                                "Shure Beta 58A sélectionné : micro dynamique, 48 V non requis.",
                            );
                        }
                        MicrophoneProfile::Flat => {}
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Exemple immédiat • 8 notes").clicked() {
                        self.clear_tablature();
                        self.tablature_recording = true;
                        self.tablature_playing = true;
                    }
                    if ui.button("Nouvel exercice").clicked() {
                        self.clear_tablature();
                        self.tablature_recording = true;
                        self.tablature_playing = true;
                    }
                    if ui.button("Effacer").clicked() {
                        self.clear_tablature();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("BPM");
                    ui.add(
                        egui::DragValue::new(&mut self.metronome_bpm)
                            .range(30.0..=240.0)
                            .speed(1.0),
                    );
                    egui::ComboBox::from_id_salt("live_tab_time_signature")
                        .selected_text("4/4")
                        .show_ui(ui, |ui| {
                            let _ = ui.selectable_label(true, "4/4");
                            let _ = ui.selectable_label(false, "3/4");
                            let _ = ui.selectable_label(false, "6/8");
                        });
                    if ui
                        .button(if self.metronome_enabled {
                            "Arrêter rythme"
                        } else {
                            "Démarrer rythme"
                        })
                        .clicked()
                    {
                        self.metronome_enabled = !self.metronome_enabled;
                    }
                    if ui.button("T−").clicked() {
                        self.metronome_bpm = (self.metronome_bpm - 5.0).max(30.0);
                    }
                    let play_label = if self.tablature_playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() {
                        self.toggle_tablature_playback();
                    }
                    let record_label = if self.tablature_recording {
                        "■ Arrêter l'enregistrement"
                    } else {
                        "● Enregistrer les notes"
                    };
                    if ui.button(record_label).clicked() {
                        self.toggle_tablature_recording();
                    }
                    ui.colored_label(
                        if self.tablature_recording {
                            Color32::from_rgb(230, 95, 95)
                        } else {
                            Color32::from_gray(150)
                        },
                        if self.tablature_recording {
                            "Enregistrement des notes actif"
                        } else {
                            "Enregistrement arrêté"
                        },
                    );
                });
                if let Some(error) = &self.microphone_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if let Some(error) = &self.audio_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.add_space(6.0);
                self.draw_live_tablature_panel(ui);
            });
    }

    fn draw_ui(&mut self, root_ui: &mut egui::Ui) {
        self.poll_metadata_loading();
        if let Some(player) = self.audio_player.as_ref() {
            self.resume_position_seconds = player.get_pos().as_secs_f32();
            if (self.resume_position_seconds - self.last_saved_position_seconds).abs() >= 1.0 {
                self.save_playlist();
                self.last_saved_position_seconds = self.resume_position_seconds;
            }
        }
        let should_advance_track = if self.workspace.transport == TransportState::Playing {
            match self.audio_player.as_ref() {
                None => false,
                Some(player) if player.empty() => true,
                Some(player) if self.chain_transition.is_none() => {
                    let transition_enabled = self.chain_transition_mode().is_some();
                    let duration = self
                        .audio_metadata
                        .as_ref()
                        .map(|metadata| metadata.duration_seconds)
                        .or_else(|| {
                            self.selected_track.and_then(|index| {
                                self.workspace
                                    .document
                                    .tracks
                                    .get(index)
                                    .map(|track| track.duration_seconds)
                            })
                        })
                        .unwrap_or(0.0);
                    let position = player.get_pos().as_secs_f32();
                    transition_enabled
                        && duration > 0.0
                        && duration - position <= self.chain_transition_seconds.max(0.01)
                }
                Some(_) => false,
            }
        } else {
            false
        };
        if should_advance_track {
            self.finish_track();
        }
        self.update_chain_transition();

        if self.scene.mode == WorkspaceMode::Player {
            self.update_spectrum_from_playback();
        } else if self.scene.mode == WorkspaceMode::LiveTablature {
            self.update_live_analysis();
        } else {
            self.update_spectrum_from_microphone();
        }

        egui::Panel::top("top_toolbar")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(45, 45, 45))
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(root_ui, |ui| {
                self.draw_toolbar(ui);
            });
        if matches!(
            self.scene.mode,
            WorkspaceMode::Player | WorkspaceMode::RoomAnalyzer | WorkspaceMode::LiveTablature
        ) {
            egui::Panel::right("stereo_meter").exact_size(170.0).show(root_ui, |ui| {
                self.draw_stereo_meter_panel(ui);
            });
        }
        if matches!(self.scene.mode, WorkspaceMode::RoomAnalyzer | WorkspaceMode::LiveTablature) {
            egui::Panel::bottom("mode_toolbar")
                .frame(
                    Frame::new()
                        .fill(Color32::from_rgb(45, 45, 45))
                        .inner_margin(egui::Margin::symmetric(8, 4)),
                )
                .show(root_ui, |ui| {
                    self.draw_mode_toolbar(ui);
                });
        }
        egui::CentralPanel::default().show(root_ui, |ui| match self.scene.mode {
            WorkspaceMode::Player => {
                ui.add_space(PLAYER_CONTENT_GAP);
                self.draw_track_list(ui);
                ui.add_space(PLAYER_CONTENT_GAP);
                self.draw_track_info(ui);
                ui.add_space(PLAYER_CONTENT_GAP);
                self.draw_spectrum(ui);
            }
            WorkspaceMode::RoomAnalyzer => self.draw_room_analyzer(ui),
            WorkspaceMode::LiveTablature => self.draw_live_tablature(ui),
            WorkspaceMode::UiLab => {
                ui.add_space(CONTENT_GAP);
                ui.heading("UI Lab");
                ui.label("Mode de prototypage de l'interface.");
            }
        });
        self.draw_music_loader(&self.egui_context.clone());
        self.draw_room_measurement_dialog(&self.egui_context.clone());
        self.egui_context.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let raw_input = {
            let Some(egui_state) = self.egui_state.as_mut() else { return };
            egui_state.take_egui_input(&window)
        };
        let context = self.egui_context.clone();
        let mut full_output = context.run_ui(raw_input, |root_ui| self.draw_ui(root_ui));
        if let Some(egui_state) = self.egui_state.as_mut() {
            egui_state.handle_platform_output(&window, full_output.platform_output);
        }
        let clipped_primitives =
            self.egui_context.tessellate(full_output.shapes, full_output.pixels_per_point);
        let pixels_per_point = full_output.pixels_per_point;
        let Some(painter) = self.painter.as_mut() else { return };
        painter.paint_and_update_textures(
            ViewportId::ROOT,
            pixels_per_point,
            [0.055, 0.065, 0.09, 1.0],
            &clipped_primitives,
            &mut full_output.textures_delta,
            Vec::new(),
            &window,
        );
    }
}

impl Drop for PlayDesktop {
    fn drop(&mut self) {
        self.stop_microphone();
    }
}

impl ApplicationHandler for PlayDesktop {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.initialize_window(event_loop);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else { return };
        if let Some(state) = self.egui_state.as_mut() {
            let _ = state.on_window_event(&window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save_playlist();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(painter) = self.painter.as_mut()
                    && let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    painter.on_window_resized(ViewportId::ROOT, width, height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut PlayDesktop::new())
}
