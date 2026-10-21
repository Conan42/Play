use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

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
use play_media::{SPECTRUM_BANDS, SpectrumAnalyzer};
use play_ui::Scene;
use rodio::Source;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SPECTRUM_HEIGHT: f32 = 250.0;
const CONTENT_GAP: f32 = 12.0;
const TRACK_LIST_INNER_MARGIN: f32 = 16.0;
const TRACK_INFO_HEIGHT: f32 = 124.0;
const TRACK_ROW_HEIGHT: f32 = 52.0;
const LED_COUNT: usize = 24;
const SPECTRUM_WINDOW_FRAMES: usize = 512;
const ANALYSIS_BUFFER_FRAMES: usize = 16384;
const SPECTRUM_FREQUENCY_LABELS: [(usize, &str); 10] = [
    (0, "20 Hz"),
    (13, "50"),
    (23, "100"),
    (33, "200"),
    (47, "500"),
    (56, "1 kHz"),
    (66, "2 kHz"),
    (80, "5 kHz"),
    (90, "10 kHz"),
    (100, "20 kHz"),
];

const MODE_BUTTONS: [(WorkspaceMode, &str, &str); 3] = [
    (WorkspaceMode::Player, "Player", "\u{e405}"),
    (WorkspaceMode::RoomAnalyzer, "Room analyzer", "\u{e01d}"),
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
}

#[derive(serde::Deserialize, serde::Serialize)]
struct SavedPlaylist {
    paths: Vec<String>,
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

#[derive(Clone)]
struct AudioMetadata {
    format: String,
    bit_depth: Option<u8>,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
    bitrate_kbps: Option<u32>,
    bpm: Option<f32>,
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

fn read_flac_cover(path: &Path) -> Option<Vec<u8>> {
    if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("flac")) {
        return None;
    }

    let tagged_file = lofty::read_from_path(path).ok()?;
    tagged_file
        .tags()
        .iter()
        .flat_map(|tag| tag.pictures())
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| tagged_file.tags().iter().flat_map(|tag| tag.pictures()).next())
        .map(|picture| picture.data().to_vec())
}

fn load_cover_texture(ctx: &Context, path: &Path, index: usize) -> Option<TextureHandle> {
    let bytes = read_flac_cover(path)?;
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
        bit_depth: None,
        sample_rate,
        channels,
        duration_seconds,
        bitrate_kbps: None,
        bpm: None,
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
    transport_options: [bool; 2],
    show_music_loader: bool,
    loaded_music: Vec<PathBuf>,
    track_covers: Vec<Option<TextureHandle>>,
    track_cover_checked: Vec<bool>,
    selected_track: Option<usize>,
    audio_metadata: Option<AudioMetadata>,
    metadata_loading: Option<mpsc::Receiver<AudioMetadata>>,
    audio_output: Option<rodio::MixerDeviceSink>,
    audio_player: Option<rodio::Player>,
    audio_error: Option<String>,
    stereo_faders: [f32; 2],
    channel_gains: Arc<Mutex<[f32; 2]>>,
    analysis_buffer: Option<Arc<Mutex<LiveAnalysisBuffer>>>,
    channel_levels: [f32; 2],
    stereo_spectrum: [[f32; SPECTRUM_BANDS]; 2],
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
        };

        let mut app = Self {
            window: None,
            egui_context,
            egui_state: None,
            painter: None,
            workspace: Workspace::default(),
            scene: Scene::default(),
            transport_options: [false; 2],
            show_music_loader: false,
            loaded_music: Vec::new(),
            track_covers: Vec::new(),
            track_cover_checked: Vec::new(),
            selected_track: None,
            audio_metadata: None,
            metadata_loading: None,
            audio_output: None,
            audio_player: None,
            audio_error: None,
            stereo_faders: [0.8; 2],
            channel_gains: Arc::new(Mutex::new([0.251_188_64; 2])),
            analysis_buffer: None,
            channel_levels: [0.0; 2],
            stereo_spectrum: [[0.0; SPECTRUM_BANDS]; 2],
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
                        .with_min_inner_size(winit::dpi::LogicalSize::new(900.0, 560.0)),
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

        let content = rect.shrink2(vec2(8.0, 6.0));
        let frequency_height = 20.0;
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

        let band_width = graph.width() / SPECTRUM_BANDS as f32;
        let bar_width = (band_width - 1.0).max(1.0);
        let maximum_height = (graph.height() * 0.5 - 1.0).max(1.0);
        for band in 0..SPECTRUM_BANDS {
            let x = graph.left() + band as f32 * band_width;
            let left_height = self.stereo_spectrum[0][band].clamp(0.0, 1.0) * maximum_height;
            let right_height = self.stereo_spectrum[1][band].clamp(0.0, 1.0) * maximum_height;
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
                [egui::pos2(x, frequency_rect.top()), egui::pos2(x, frequency_rect.top() + 4.0)],
                egui::Stroke::new(1.0, Color32::from_gray(115)),
            );
            painter.text(
                egui::pos2(x, frequency_rect.bottom()),
                egui::Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(9.0),
                Color32::from_gray(160),
            );
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
            for (selected, (icon, label)) in self
                .transport_options
                .iter_mut()
                .zip([("\u{e040}", "Répéter"), ("\u{e05f}", "Enchaîner")])
            {
                if toolbar_icon_button(ui, &self.toolbar_textures, icon, *selected, label).clicked()
                {
                    *selected = !*selected;
                }
            }

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

    fn select_mode(&mut self, mode: WorkspaceMode) {
        self.scene.mode = mode;
        self.workspace.document.mode = mode;
    }

    fn change_track(&mut self, offset: isize) {
        let track_count = self.loaded_music.len();
        let Some(current) = self.selected_track else {
            if track_count > 0 {
                self.play_track(0);
            }
            return;
        };
        let next = current as isize + offset;
        if next >= 0 && (next as usize) < track_count {
            self.play_track(next as usize);
        }
    }

    fn restore_playlist(&mut self) {
        let Some(path) = playlist_storage_path() else { return };
        let Ok(contents) = std::fs::read_to_string(path) else { return };
        let Ok(saved) = ron::from_str::<SavedPlaylist>(&contents) else { return };
        for path in saved.paths {
            self.add_music_path(PathBuf::from(path));
        }
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
        };
        let Ok(contents) = ron::to_string(&saved) else { return };
        let _ = std::fs::write(path, contents);
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
            duration_seconds: 0.0,
        });
        self.track_covers.push(None);
        self.track_cover_checked.push(false);
        self.loaded_music.push(path);
    }

    fn draw_track_list(&mut self, ui: &mut Ui) {
        let content_height = (ui.available_height()
            - SPECTRUM_HEIGHT
            - TRACK_INFO_HEIGHT
            - CONTENT_GAP * 2.0
            - TRACK_LIST_INNER_MARGIN)
            .max(120.0);
        let mut clicked_track = None;
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                // Réserve une hauteur fixe pour que le contenu de la liste ne
                // puisse pas repousser l'analyseur spectral vers le bas.
                ui.set_height(content_height);
                if self.workspace.document.tracks.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((content_height - 24.0).max(0.0) * 0.5);
                        ui.label(
                            RichText::new("Aucune piste chargée").color(Color32::from_gray(170)),
                        );
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("track_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (index, track) in self.workspace.document.tracks.iter().enumerate()
                            {
                                let selected = self.selected_track == Some(index);
                                let row = ui.allocate_ui_with_layout(
                                    vec2(ui.available_width(), TRACK_ROW_HEIGHT),
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
                                        ui.add_space(8.0);
                                        ui.label(format!("{:02}", index + 1));
                                        ui.vertical(|ui| {
                                            ui.label(&track.title);
                                            ui.small(&track.artist);
                                        });
                                    },
                                );
                                if row.response.interact(Sense::click()).clicked() {
                                    clicked_track = Some(index);
                                }
                                if row.response.hovered() {
                                    ui.painter().rect_stroke(
                                        row.response.rect.shrink(1.0),
                                        4.0,
                                        egui::Stroke::new(1.0, Color32::from_rgb(72, 184, 166)),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if index + 1 < self.workspace.document.tracks.len() {
                                    ui.separator();
                                }
                            }
                        });
                }
            });

        if let Some(index) = clicked_track {
            self.play_track(index);
        }
    }

    fn draw_track_info(&mut self, ui: &mut Ui) {
        let available_width = ui.available_width();
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                let content_width = (available_width - 16.0).max(0.0);
                ui.set_min_width(content_width);
                ui.set_width(content_width);
                ui.set_height((TRACK_INFO_HEIGHT - 16.0).max(0.0));
                if let Some(index) = self.selected_track {
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
                    let cover = self.track_covers.get(index).and_then(Option::as_ref).cloned();
                    let metadata = self.audio_metadata.clone();
                    let audio_error = self.audio_error.clone();
                    let transport = self.workspace.transport;
                    ui.horizontal(|ui| {
                        if let Some(cover) = cover.as_ref() {
                            ui.add(
                                egui::Image::new((cover.id(), vec2(96.0, 96.0))).corner_radius(4.0),
                            );
                        } else {
                            ui.allocate_ui(vec2(96.0, 96.0), |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(34.0);
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
                                left.label(format!("État : {:?}", transport));
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
                                    right.colored_label(
                                        Color32::from_rgb(110, 220, 190),
                                        format!(
                                            "{} • {} • {} • {}",
                                            metadata.format,
                                            bit_depth,
                                            Self::format_sample_rate(metadata.sample_rate),
                                            channels
                                        ),
                                    );
                                    right.small(format!(
                                        "Durée {} • BPM {}",
                                        Self::format_time(metadata.duration_seconds),
                                        metadata
                                            .bpm
                                            .map(|bpm| format!("{bpm:.1}"))
                                            .unwrap_or_else(|| "—".to_owned())
                                    ));
                                    if let Some(bitrate) = metadata.bitrate_kbps {
                                        right.small(format!("Débit {bitrate} kb/s"));
                                    }
                                    if let Some(genre) = &metadata.genre {
                                        right.small(format!("Genre : {genre}"));
                                    }
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
                        ui.add_space(36.0);
                        ui.label("Sélectionnez une piste pour afficher ses informations");
                    });
                }
            });
    }

    fn format_time(seconds: f32) -> String {
        let seconds = seconds.max(0.0) as u64;
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }

    fn format_sample_rate(sample_rate: u32) -> String {
        if sample_rate % 1000 == 0 {
            format!("{} kHz", sample_rate / 1000)
        } else {
            format!("{:.1} kHz", sample_rate as f32 / 1000.0)
        }
    }

    fn draw_playback_seek(&mut self, ui: &mut Ui) {
        let duration = self
            .audio_metadata
            .as_ref()
            .map(|metadata| metadata.duration_seconds)
            .filter(|duration| *duration > 0.0)
            .unwrap_or(0.0);
        let current = self
            .audio_player
            .as_ref()
            .map(|player| player.get_pos().as_secs_f32())
            .unwrap_or(0.0)
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
        }
    }

    fn draw_led_meter(ui: &mut Ui, level: f32, height: f32) {
        ui.vertical(|ui| {
            ui.set_width(30.0);
            let (response, painter) = ui.allocate_painter(vec2(30.0, height), egui::Sense::hover());
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
            ui.set_width(38.0);
            let (response, painter) = ui.allocate_painter(vec2(38.0, height), egui::Sense::hover());
            let rect = response.rect;
            for decibels in [0, -6, -12, -18, -24, -36, -48, -60] {
                let normalized = (decibels as f32 + 60.0) / 60.0;
                let y = rect.bottom() - normalized * rect.height();
                painter.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.left() + 8.0, y)],
                    egui::Stroke::new(1.0, Color32::from_gray(150)),
                );
                painter.text(
                    egui::pos2(rect.left() + 10.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("{decibels}"),
                    FontId::proportional(10.0),
                    Color32::from_gray(170),
                );
            }
        });
    }

    fn draw_stereo_fader(ui: &mut Ui, height: f32, value: &mut f32) -> egui::Rect {
        let mut fader_rect = egui::Rect::NOTHING;
        ui.vertical(|ui| {
            ui.set_width(42.0);
            let (response, painter) =
                ui.allocate_painter(vec2(42.0, height), Sense::click_and_drag());
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
                vec2(32.0, 12.0),
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
        ui.set_min_width(204.0);
        let meter_height = (ui.available_height() - 8.0).max(220.0);
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
            if (response.clicked() || response.is_pointer_button_down_on())
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let track = egui::Rect::from_center_size(
                    middle.center(),
                    vec2(2.0, (middle.height() - 16.0).max(1.0)),
                );
                let value = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
                self.stereo_faders = [value, value];
            }
        });
        if let Ok(mut gains) = self.channel_gains.lock() {
            *gains = [
                Self::fader_to_gain(self.stereo_faders[0]),
                Self::fader_to_gain(self.stereo_faders[1]),
            ];
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
        self.analysis_buffer = None;
        self.audio_metadata = None;
        self.metadata_loading = None;
        self.channel_levels = [0.0; 2];
        self.stereo_spectrum = [[0.0; SPECTRUM_BANDS]; 2];
        self.workspace.stop();
        self.workspace.spectrum = Default::default();
    }

    fn play_track(&mut self, index: usize) {
        let Some(path) = self.loaded_music.get(index).cloned() else {
            return;
        };

        self.selected_track = Some(index);
        if let Some(player) = self.audio_player.take() {
            player.stop();
        }
        self.analysis_buffer = None;
        self.audio_metadata = None;
        self.metadata_loading = None;
        self.channel_levels = [0.0; 2];
        self.stereo_spectrum = [[0.0; SPECTRUM_BANDS]; 2];
        self.workspace.stop();
        self.audio_error = None;

        let result = (|| {
            if self.audio_output.is_none() {
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
            let metadata = basic_audio_metadata(
                &path,
                sample_rate.get(),
                channels.get() as usize,
                duration_seconds,
            );
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
            player.append(source);
            self.audio_player = Some(player);
            self.analysis_buffer = Some(analysis_buffer);
            self.audio_metadata = Some(metadata);
            if let Some(track) = self.workspace.document.tracks.get_mut(index) {
                track.duration_seconds = duration_seconds;
            }
            Ok::<(), String>(())
        })();

        match result {
            Ok(()) => {
                let Some(metadata) = self.audio_metadata.as_ref() else {
                    self.workspace.transport = TransportState::Playing;
                    return;
                };
                let metadata_path = path.clone();
                let sample_rate = metadata.sample_rate;
                let channels = metadata.channels;
                let duration_seconds = metadata.duration_seconds;
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let metadata = read_audio_metadata(
                        &metadata_path,
                        sample_rate,
                        channels,
                        duration_seconds,
                    );
                    let _ = sender.send(metadata);
                });
                self.metadata_loading = Some(receiver);
                self.workspace.transport = TransportState::Playing;
            }
            Err(error) => {
                self.audio_error = Some(error);
                self.workspace.stop();
            }
        }
    }

    fn poll_metadata_loading(&mut self) {
        let Some(receiver) = self.metadata_loading.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(metadata) => {
                if let Some(index) = self.selected_track {
                    if let Some(track) = self.workspace.document.tracks.get_mut(index) {
                        track.duration_seconds = metadata.duration_seconds;
                    }
                }
                self.audio_metadata = Some(metadata);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.metadata_loading = Some(receiver);
            }
            Err(mpsc::TryRecvError::Disconnected) => {}
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

    fn update_spectrum_from_playback(&mut self) {
        let (analyzed_bands, channel_levels) = self
            .analysis_buffer
            .as_ref()
            .and_then(|analysis| {
                let analysis = analysis.lock().ok()?;
                let channels = analysis.channels.max(1);
                let sample_rate = analysis.sample_rate;
                let total_frames = analysis.samples.len() / channels;
                if total_frames == 0 {
                    return None;
                }
                let frame_count = total_frames.min(SPECTRUM_WINDOW_FRAMES);
                let start_frame = total_frames - frame_count;
                let mut channel_samples =
                    [vec![0.0; SPECTRUM_WINDOW_FRAMES], vec![0.0; SPECTRUM_WINDOW_FRAMES]];
                let mut levels = [0.0_f32; 2];
                for offset in 0..frame_count {
                    let frame = start_frame + offset;
                    let first_sample = frame * channels;
                    let left = analysis.samples[first_sample];
                    let right =
                        if channels > 1 { analysis.samples[first_sample + 1] } else { left };
                    channel_samples[0][offset] = left;
                    channel_samples[1][offset] = right;
                    levels[0] = levels[0].max(left.abs());
                    levels[1] = levels[1].max(right.abs());
                }
                let analyzer = SpectrumAnalyzer::new(sample_rate);
                let left_bands = analyzer.analyze(&channel_samples[0]).ok()?;
                let right_bands = analyzer.analyze(&channel_samples[1]).ok()?;
                Some((Some([left_bands, right_bands]), levels))
            })
            .unwrap_or((None, [0.0; 2]));

        if let Some(stereo_bands) = analyzed_bands {
            self.stereo_spectrum = stereo_bands;
            let mut bands = [0.0; 32];
            for (index, band) in bands.iter_mut().enumerate() {
                let first = index * 2;
                *band = (stereo_bands[0][first]
                    + stereo_bands[0][first + 1]
                    + stereo_bands[1][first]
                    + stereo_bands[1][first + 1])
                    * 0.25;
            }
            self.workspace.spectrum.bands = bands;
            self.workspace.spectrum.peak = bands.iter().copied().fold(0.0, f32::max);
            self.channel_levels = channel_levels;
        } else {
            self.stereo_spectrum = [[0.0; SPECTRUM_BANDS]; 2];
            self.workspace.spectrum = Default::default();
            self.channel_levels = [0.0; 2];
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

    fn draw_ui(&mut self, root_ui: &mut egui::Ui) {
        self.poll_metadata_loading();
        if self.workspace.transport == TransportState::Playing
            && self.audio_player.as_ref().is_some_and(rodio::Player::empty)
        {
            self.audio_player = None;
            self.analysis_buffer = None;
            self.workspace.stop();
        }

        self.update_spectrum_from_playback();

        egui::Panel::top("top_toolbar")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(45, 45, 45))
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(root_ui, |ui| {
                self.draw_toolbar(ui);
            });
        egui::Panel::right("stereo_meter").exact_size(220.0).show(root_ui, |ui| {
            self.draw_stereo_meter_panel(ui);
        });
        egui::CentralPanel::default().show(root_ui, |ui| {
            ui.add_space(CONTENT_GAP);
            self.draw_track_list(ui);
            ui.add_space(CONTENT_GAP);
            self.draw_track_info(ui);
            ui.add_space(CONTENT_GAP);
            self.draw_spectrum(ui);
        });
        self.draw_music_loader(&self.egui_context.clone());
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
            WindowEvent::CloseRequested => event_loop.exit(),
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
