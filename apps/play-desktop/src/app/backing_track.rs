use super::*;

const BACKING_WAVEFORM_BINS: usize = 300;
const BACKING_INDEX_WIDTH: f32 = 30.0;
const BACKING_TITLE_WIDTH: f32 = 250.0;
const BACKING_REFERENCE_WIDTH: f32 = 42.0;
const BACKING_SWITCH_WIDTH: f32 = 34.0;
const BACKING_VOLUME_LABEL_WIDTH: f32 = 32.0;
const BACKING_VOLUME_SLIDER_WIDTH: f32 = 108.0;
const BACKING_DB_WIDTH: f32 = 52.0;
const BACKING_PAN_LABEL_WIDTH: f32 = 32.0;
const BACKING_PAN_SLIDER_WIDTH: f32 = 100.0;
const BACKING_PAN_VALUE_WIDTH: f32 = 20.0;
const BACKING_REMOVE_WIDTH: f32 = 32.0;
const BACKING_SYNC_WIDTH: f32 = 54.0;
const BACKING_MIN_CONTENT_WIDTH: f32 = 1100.0;

fn backing_track_cell_widths() -> [f32; 7] {
    [
        BACKING_INDEX_WIDTH,
        BACKING_TITLE_WIDTH,
        BACKING_REFERENCE_WIDTH,
        BACKING_SWITCH_WIDTH,
        BACKING_SWITCH_WIDTH,
        BACKING_REMOVE_WIDTH,
        BACKING_SYNC_WIDTH,
    ]
}

fn backing_mix_cell_widths() -> [f32; 6] {
    [
        BACKING_VOLUME_LABEL_WIDTH,
        BACKING_VOLUME_SLIDER_WIDTH,
        BACKING_DB_WIDTH,
        BACKING_PAN_LABEL_WIDTH,
        BACKING_PAN_SLIDER_WIDTH,
        BACKING_PAN_VALUE_WIDTH,
    ]
}

fn backing_track_name(path: &Path) -> String {
    let stem = path.file_stem().and_then(|name| name.to_str()).unwrap_or("Nouvelle piste").trim();
    stem.rsplit_once(" - ").map(|(_, name)| name).unwrap_or(stem).to_owned()
}

fn backing_file_name(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).unwrap_or("Aucun fichier").to_owned()
}

fn is_default_backing_role(role: &str) -> bool {
    matches!(role, "Nouvelle piste" | "Nouveau rôle")
}

fn load_backing_waveform(path: &Path) -> Option<(Vec<f32>, f32)> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = rodio::Decoder::try_from(file).ok()?;
    let channels = decoder.channels().get() as usize;
    let sample_rate = decoder.sample_rate().get();
    let total_frames = decoder
        .total_duration()
        .map(|duration| (duration.as_secs_f32() * sample_rate as f32) as usize)
        .unwrap_or(BACKING_WAVEFORM_BINS);
    let mut waveform = vec![0.0_f32; BACKING_WAVEFORM_BINS];
    let mut frame_peak = 0.0_f32;
    let mut frame_channels = 0_usize;
    let mut frame_index = 0_usize;
    for sample in decoder {
        frame_peak = frame_peak.max(sample.abs());
        frame_channels += 1;
        if frame_channels == channels.max(1) {
            let bin = (frame_index * BACKING_WAVEFORM_BINS / total_frames.max(1))
                .min(BACKING_WAVEFORM_BINS - 1);
            waveform[bin] = waveform[bin].max(frame_peak);
            frame_peak = 0.0;
            frame_channels = 0;
            frame_index += 1;
        }
    }
    Some((waveform, total_frames as f32 / sample_rate.max(1) as f32))
}

fn draw_backing_waveform(
    ui: &mut Ui,
    waveform: Option<&[f32]>,
    reference: bool,
    width: f32,
    playback_position: Option<f32>,
    duration: Option<f32>,
) -> egui::Response {
    let (response, painter) = ui.allocate_painter(vec2(width.max(180.0), 42.0), Sense::click());
    let rect = response.rect;
    painter.rect_filled(rect, 3.0, Color32::from_rgb(20, 31, 42));
    let center = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), center), egui::pos2(rect.right(), center)],
        egui::Stroke::new(1.0, Color32::from_rgb(65, 78, 91)),
    );
    let Some(waveform) = waveform else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Aucun échantillon",
            FontId::proportional(10.0),
            Color32::from_gray(130),
        );
        return response;
    };
    let color =
        if reference { Color32::from_rgb(239, 188, 74) } else { Color32::from_rgb(67, 183, 213) };
    let width = rect.width() / waveform.len().max(1) as f32;
    for (index, peak) in waveform.iter().enumerate() {
        let half_height = (peak.clamp(0.0, 1.0) * (rect.height() * 0.44)).max(1.0);
        let x = rect.left() + index as f32 * width;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, center - half_height),
                egui::pos2((x + width).min(rect.right()), center + half_height),
            ),
            0.0,
            color,
        );
    }
    if let (Some(position), Some(duration)) = (playback_position, duration)
        && duration > 0.0
    {
        let progress = (position / duration).clamp(0.0, 1.0);
        let x = rect.left() + progress * rect.width();
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2.0, Color32::from_rgb(255, 94, 94)),
        );
    }
    response.on_hover_text("Double-cliquer pour positionner la lecture")
}

impl PlayDesktop {
    fn ensure_backing_waveforms(&mut self) -> bool {
        let mut changed = false;
        while let Ok((path, waveform)) = self.backing_waveform_results.try_recv() {
            self.backing_waveform_loading.retain(|loading| loading != &path);
            if let Some(index) =
                self.backing_stems.iter().position(|stem| stem.path.as_ref() == Some(&path))
            {
                if let Some((waveform, duration)) = waveform {
                    self.backing_waveforms[index] = Some(waveform);
                    self.backing_durations[index] = Some(duration);
                } else {
                    self.backing_waveforms[index] = None;
                    self.backing_durations[index] = None;
                }
                self.backing_waveform_ready[index] = true;
            }
        }
        self.backing_waveforms.resize(self.backing_stems.len(), None);
        self.backing_durations.resize(self.backing_stems.len(), None);
        self.backing_waveform_ready.resize(self.backing_stems.len(), false);
        self.backing_waveform_loading
            .retain(|path| self.backing_stems.iter().any(|stem| stem.path.as_ref() == Some(path)));
        for index in 0..self.backing_stems.len() {
            let Some(path) = self.backing_stems[index].path.clone() else { continue };
            if is_default_backing_role(&self.backing_stems[index].role) {
                self.backing_stems[index].role = backing_file_name(&path);
                changed = true;
            }
            if !self.backing_waveform_ready[index]
                && !self.backing_waveform_loading.iter().any(|loading| loading == &path)
            {
                self.backing_waveform_loading.push(path.clone());
                let sender = self.backing_waveform_sender.clone();
                std::thread::spawn(move || {
                    let waveform = load_backing_waveform(&path);
                    let _ = sender.send((path, waveform));
                });
            }
        }
        changed
    }

    pub(super) fn sync_backing_mix_settings(&mut self) {
        while self.backing_mix_settings.len() < self.backing_stems.len() {
            self.backing_mix_settings.push(Arc::new(Mutex::new(BackingStemMix {
                volume: default_backing_volume(),
                pan: 0.0,
                muted: false,
                solo: false,
            })));
        }
        self.backing_mix_settings.truncate(self.backing_stems.len());
        let solo_active = self.backing_stems.iter().any(|stem| stem.solo);
        self.backing_solo_active.store(solo_active, Ordering::Relaxed);
        for (stem, settings) in self.backing_stems.iter().zip(&self.backing_mix_settings) {
            if let Ok(mut settings) = settings.lock() {
                *settings = stem.mix();
            }
        }
    }

    pub(super) fn add_empty_backing_stem(&mut self) {
        self.backing_stems.push(BackingStem {
            name: "Nouveau musicien".to_owned(),
            role: "Nom du fichier".to_owned(),
            path: None,
            reference: false,
            sync_offset_seconds: 0.0,
            volume: default_backing_volume(),
            pan: 0.0,
            muted: false,
            solo: false,
        });
        self.backing_waveforms.push(None);
        self.backing_durations.push(None);
        self.backing_waveform_ready.push(false);
        self.sync_backing_mix_settings();
        self.backing_project_dirty = true;
        self.save_playlist();
    }

    pub(super) fn import_backing_audio_files(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .set_title("Charger les pistes du BackingTrack")
            .add_filter("Fichiers audio", &["wav", "flac", "mp3", "ogg", "m4a", "aac", "opus"])
            .pick_files()
        else {
            return;
        };

        let mut imported = 0;
        let mut skipped = 0;
        for path in files {
            if self.backing_stems.iter().any(|stem| stem.path.as_ref() == Some(&path)) {
                skipped += 1;
                continue;
            }
            if let Some(index) = self.backing_stems.iter().position(|stem| stem.path.is_none()) {
                let stem = &mut self.backing_stems[index];
                stem.name = backing_track_name(&path);
                stem.role = backing_file_name(&path);
                stem.reference = Self::is_reference_path(Some(&path));
                stem.path = Some(path);
                stem.sync_offset_seconds = 0.0;
                if let Some(waveform) = self.backing_waveforms.get_mut(index) {
                    *waveform = None;
                }
                if let Some(duration) = self.backing_durations.get_mut(index) {
                    *duration = None;
                }
                if let Some(ready) = self.backing_waveform_ready.get_mut(index) {
                    *ready = false;
                }
            } else {
                self.backing_stems.push(BackingStem {
                    name: backing_track_name(&path),
                    role: backing_file_name(&path),
                    reference: false,
                    path: Some(path),
                    sync_offset_seconds: 0.0,
                    volume: default_backing_volume(),
                    pan: 0.0,
                    muted: false,
                    solo: false,
                });
                self.backing_waveforms.push(None);
                self.backing_durations.push(None);
                self.backing_waveform_ready.push(false);
            }
            imported += 1;
        }

        if imported > 0 {
            self.sync_backing_mix_settings();
            self.backing_project_dirty = true;
            self.save_playlist();
        }
        self.audio_error = Some(if imported == 0 {
            format!("Aucune nouvelle piste importée ({skipped} déjà présente(s)).")
        } else if skipped == 0 {
            format!("{imported} piste(s) audio importée(s).")
        } else {
            format!("{imported} piste(s) audio importée(s), {skipped} déjà présente(s).")
        });
    }

    fn saved_backing_project(&self, name: String) -> SavedBackingProject {
        SavedBackingProject {
            name,
            backing_stems: self
                .backing_stems
                .iter()
                .map(|stem| SavedBackingStem {
                    name: stem.name.clone(),
                    role: stem.role.clone(),
                    path: stem.path.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    reference: stem.reference,
                    sync_offset_seconds: stem.sync_offset_seconds,
                    volume: stem.volume,
                    pan: stem.pan,
                    muted: stem.muted,
                    solo: stem.solo,
                })
                .collect(),
            backing_bpm: self.backing_bpm,
            backing_original_bpm: self.backing_original_bpm,
            position_seconds: self.backing_playback_position_seconds().max(0.0),
            stereo_faders: self.stereo_faders,
        }
    }

    fn write_backing_project(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("BackingTrack")
            .to_owned();
        let Some(parent) = path.parent() else {
            self.audio_error = Some("Dossier du projet BackingTrack introuvable.".to_owned());
            return;
        };
        if let Err(error) = std::fs::create_dir_all(parent) {
            self.audio_error = Some(format!("Création du dossier impossible : {error}"));
            return;
        }
        let project = self.saved_backing_project(name.clone());
        let contents = match ron::to_string(&project) {
            Ok(contents) => contents,
            Err(error) => {
                self.audio_error = Some(format!("Encodage du projet impossible : {error}"));
                return;
            }
        };
        if let Err(error) = std::fs::write(&path, contents) {
            self.audio_error = Some(format!("Enregistrement du projet impossible : {error}"));
            return;
        }
        self.backing_project_path = Some(path.clone());
        self.backing_project_name = name.clone();
        self.backing_project_dirty = false;
        self.audio_error = Some(format!("Projet BackingTrack « {name} » enregistré."));
    }

    pub(super) fn save_backing_project(&mut self) {
        if let Some(path) = self.backing_project_path.clone() {
            self.write_backing_project(path);
        } else {
            self.save_backing_project_as();
        }
    }

    pub(super) fn save_backing_project_as(&mut self) {
        let suggested_name = if self.backing_project_name == "Sans nom" {
            "BackingTrack.ron".to_owned()
        } else {
            format!("{}.ron", self.backing_project_name)
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Enregistrer le projet BackingTrack")
            .set_file_name(suggested_name)
            .add_filter("Projet BackingTrack", &["ron"])
            .save_file()
        else {
            return;
        };
        let path = if path.extension().and_then(|extension| extension.to_str()) == Some("ron") {
            path
        } else {
            path.with_extension("ron")
        };
        self.write_backing_project(path);
    }

    pub(super) fn request_backing_action(&mut self, action: PendingBackingAction) {
        if self.backing_project_dirty {
            self.pending_backing_action = Some(action);
        } else {
            self.perform_backing_action(action);
        }
    }

    pub(super) fn perform_backing_action(&mut self, action: PendingBackingAction) {
        match action {
            PendingBackingAction::NewProject => self.new_backing_project(),
            PendingBackingAction::LoadProject => {
                if !self.load_backing_project() {
                    self.backing_project_dirty = true;
                }
            }
            PendingBackingAction::SelectMode(mode) => self.select_mode_without_confirmation(mode),
            PendingBackingAction::CloseApplication => {
                self.save_playlist();
                self.egui_context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(super) fn new_backing_project(&mut self) {
        self.stop_backing_playback();
        self.workspace.stop();
        self.backing_stems.clear();
        self.backing_waveforms.clear();
        self.backing_durations.clear();
        self.backing_waveform_ready.clear();
        self.backing_waveform_loading.clear();
        while self.backing_waveform_results.try_recv().is_ok() {}
        self.backing_mix_settings.clear();
        self.backing_bpm = 120.0;
        self.backing_original_bpm = 120.0;
        self.backing_position_seconds = 0.0;
        self.backing_project_path = None;
        self.backing_project_name = "Sans nom".to_owned();
        self.backing_project_dirty = false;
        self.stereo_faders = [FADER_DEFAULT_VALUE; 2];
        self.sync_backing_mix_settings();
        self.audio_error = Some("Nouveau projet BackingTrack créé.".to_owned());
        self.save_playlist();
    }

    pub(super) fn load_backing_project(&mut self) -> bool {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Ouvrir un projet BackingTrack")
            .add_filter("Projet BackingTrack", &["ron"])
            .pick_file()
        else {
            return false;
        };
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) => {
                self.audio_error = Some(format!("Lecture du projet impossible : {error}"));
                return false;
            }
        };
        let project = match ron::from_str::<SavedBackingProject>(&contents) {
            Ok(project) => project,
            Err(error) => {
                self.audio_error = Some(format!("Projet BackingTrack invalide : {error}"));
                return false;
            }
        };

        self.stop_backing_playback();
        self.workspace.stop();
        self.backing_stems = project
            .backing_stems
            .into_iter()
            .map(|stem| BackingStem {
                name: stem.name,
                role: stem.role,
                path: stem.path.map(PathBuf::from),
                reference: stem.reference,
                sync_offset_seconds: if stem.sync_offset_seconds.is_finite() {
                    stem.sync_offset_seconds.clamp(-10.0, 10.0)
                } else {
                    0.0
                },
                volume: if stem.volume.is_finite() {
                    stem.volume.clamp(0.0, 1.0)
                } else {
                    default_backing_volume()
                },
                pan: if stem.pan.is_finite() { stem.pan.clamp(-1.0, 1.0) } else { 0.0 },
                muted: stem.muted,
                solo: stem.solo,
            })
            .collect();
        self.backing_waveforms = vec![None; self.backing_stems.len()];
        self.backing_durations = vec![None; self.backing_stems.len()];
        self.backing_waveform_ready = vec![false; self.backing_stems.len()];
        self.backing_position_seconds = if project.position_seconds.is_finite() {
            project.position_seconds.max(0.0)
        } else {
            0.0
        };
        self.backing_bpm = if project.backing_bpm.is_finite() {
            project.backing_bpm.clamp(30.0, 240.0)
        } else {
            120.0
        };
        self.backing_original_bpm = if project.backing_original_bpm.is_finite() {
            project.backing_original_bpm.clamp(30.0, 240.0)
        } else {
            120.0
        };
        self.stereo_faders = [
            if project.stereo_faders[0].is_finite() {
                project.stereo_faders[0].clamp(0.0, 1.0)
            } else {
                FADER_DEFAULT_VALUE
            },
            if project.stereo_faders[1].is_finite() {
                project.stereo_faders[1].clamp(0.0, 1.0)
            } else {
                FADER_DEFAULT_VALUE
            },
        ];
        self.backing_project_path = Some(path.clone());
        self.backing_project_name = project.name.trim().to_owned();
        if self.backing_project_name.is_empty() {
            self.backing_project_name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("BackingTrack")
                .to_owned();
        }
        self.backing_project_dirty = false;
        self.sync_backing_mix_settings();
        self.save_playlist();
        self.audio_error =
            Some(format!("Projet BackingTrack « {} » chargé.", self.backing_project_name));
        true
    }

    pub(super) fn remove_backing_stem(&mut self, index: usize) {
        if index >= self.backing_stems.len() {
            return;
        }
        let was_playing = self.workspace.transport == TransportState::Playing;
        self.stop_backing_playback();
        self.backing_stems.remove(index);
        if index < self.backing_waveforms.len() {
            self.backing_waveforms.remove(index);
        }
        if index < self.backing_durations.len() {
            self.backing_durations.remove(index);
        }
        if index < self.backing_waveform_ready.len() {
            self.backing_waveform_ready.remove(index);
        }
        self.backing_mix_settings.remove(index);
        self.backing_project_dirty = true;
        if was_playing {
            self.start_backing_playback();
        }
        self.save_playlist();
    }

    fn backing_end_position_seconds(&self) -> Option<f32> {
        let tempo_ratio = (self.backing_bpm / self.backing_original_bpm).clamp(0.5, 2.0);
        self.backing_stems
            .iter()
            .enumerate()
            .filter_map(|(index, stem)| {
                let path = stem.path.as_ref()?;
                let duration =
                    self.backing_durations.get(index).copied().flatten().or_else(|| {
                        let file = std::fs::File::open(path).ok()?;
                        rodio::Decoder::try_from(file)
                            .ok()?
                            .total_duration()
                            .map(|duration| duration.as_secs_f32())
                    })?;
                Some(((duration - stem.sync_offset_seconds) / tempo_ratio).max(0.0))
            })
            .max_by(f32::total_cmp)
    }

    pub(super) fn set_backing_position(&mut self, position: f32) {
        let was_playing = self.workspace.transport == TransportState::Playing;
        let was_paused = self.workspace.transport == TransportState::Paused;
        self.stop_backing_playback();
        self.backing_position_seconds = position.max(0.0);
        self.save_playlist();
        if was_playing {
            self.start_backing_playback();
        } else if !was_paused {
            self.workspace.stop();
        }
    }

    pub(super) fn seek_backing_to_start(&mut self) {
        self.set_backing_position(0.0);
    }

    pub(super) fn seek_backing_to_end(&mut self) {
        if let Some(position) = self.backing_end_position_seconds() {
            self.set_backing_position(position);
        }
    }

    pub(super) fn stop_backing_playback(&mut self) {
        if let Some(player) = self.backing_player.take() {
            self.backing_position_seconds =
                self.backing_playback_origin_seconds + player.get_pos().as_secs_f32();
            player.stop();
        }
        self.backing_playback_origin_seconds = 0.0;
        self.backing_analysis_buffer = None;
        if let Ok(mut levels) = self.backing_levels.lock() {
            *levels = [0.0; 2];
        }
    }

    pub(super) fn start_backing_playback(&mut self) {
        self.stop_backing_playback();
        let start_position = self.backing_position_seconds.max(0.0);
        self.backing_playback_origin_seconds = start_position;
        self.sync_backing_mix_settings();
        self.audio_error = None;
        if self.audio_output.is_none() {
            self.audio_output_name = Self::default_output_device_name();
            let Ok(mut output) = rodio::DeviceSinkBuilder::open_default_sink() else {
                self.audio_error = Some("Impossible d'ouvrir la sortie audio.".to_owned());
                return;
            };
            output.log_on_drop(false);
            self.audio_output = Some(output);
        }
        let Some(output) = self.audio_output.as_ref() else { return };
        let analysis = Arc::new(Mutex::new(LiveAnalysisBuffer::new(2, 48_000.0)));
        self.backing_analysis_buffer = Some(analysis.clone());
        let output_sample_rate = NonZeroU32::new(48_000).expect("48 kHz est valide");
        let output_channels = NonZeroU16::new(2).expect("2 canaux valides");
        let mut loaded = false;
        let mut mixed_source: Option<Box<dyn rodio::Source<Item = f32> + Send>> = None;
        for (index, stem) in self.backing_stems.iter().enumerate() {
            let Some(path) = stem.path.as_ref() else {
                continue;
            };
            let result = (|| -> Result<Box<dyn rodio::Source<Item = f32> + Send>, String> {
                let file = std::fs::File::open(path)
                    .map_err(|error| format!("{} : {error}", path.display()))?;
                let mut decoder = rodio::Decoder::try_from(file)
                    .map_err(|error| format!("{} : {error}", path.display()))?;
                let tempo_ratio = (self.backing_bpm / self.backing_original_bpm).clamp(0.5, 2.0);
                let track_position_original =
                    start_position * tempo_ratio + stem.sync_offset_seconds;
                let seek_succeeded = decoder
                    .try_seek(Duration::from_secs_f32(track_position_original.max(0.0)))
                    .is_ok();
                let input = rodio::source::UniformSourceIterator::new(
                    decoder,
                    output_channels,
                    output_sample_rate,
                );
                let input = BackingTimeStretchSource::new(
                    input,
                    output_channels,
                    output_sample_rate,
                    tempo_ratio,
                );
                let source = BackingStemSource {
                    input,
                    settings: self.backing_mix_settings[index].clone(),
                    solo_active: self.backing_solo_active.clone(),
                    master_gains: self.channel_gains.clone(),
                    levels: self.backing_levels.clone(),
                    analysis: analysis.clone(),
                    pending_analysis: Vec::with_capacity(2 * 512),
                    channel: 0,
                };
                let offset_seconds = if seek_succeeded {
                    if track_position_original < 0.0 {
                        track_position_original / tempo_ratio
                    } else {
                        0.0
                    }
                } else {
                    start_position + stem.sync_offset_seconds / tempo_ratio
                };
                let source = BackingStemOffsetSource::new(
                    source,
                    offset_seconds,
                    output_channels,
                    output_sample_rate,
                );
                Ok(Box::new(source))
            })();
            match result {
                Ok(source) => {
                    loaded = true;
                    mixed_source = Some(match mixed_source.take() {
                        Some(previous) => Box::new(previous.mix(source)),
                        None => source,
                    });
                }
                Err(error) => {
                    self.audio_error = Some(format!("Stem {index}: {error}"));
                }
            }
        }
        if !loaded {
            self.backing_analysis_buffer = None;
            self.audio_error = Some("Chargez au moins un stem audio avant la lecture.".to_owned());
            return;
        }
        let Some(mixed_source) = mixed_source else {
            self.backing_analysis_buffer = None;
            self.audio_error = Some("Chargez au moins un stem audio avant la lecture.".to_owned());
            return;
        };
        let player = rodio::Player::connect_new(output.mixer());
        player.pause();
        player.append(mixed_source);
        player.play();
        self.backing_player = Some(player);
        self.workspace.transport = TransportState::Playing;
    }

    pub(super) fn toggle_backing_playback(&mut self) {
        match self.workspace.transport {
            TransportState::Playing => {
                if let Some(player) = self.backing_player.as_ref() {
                    player.pause();
                }
                self.workspace.transport = TransportState::Paused;
            }
            TransportState::Paused => {
                if self.backing_player.is_none() {
                    self.start_backing_playback();
                } else {
                    if let Some(player) = self.backing_player.as_ref() {
                        player.play();
                    }
                    self.workspace.transport = TransportState::Playing;
                }
            }
            TransportState::Stopped => self.start_backing_playback(),
        }
    }

    pub(super) fn backing_playback_position_seconds(&self) -> f32 {
        self.backing_player
            .as_ref()
            .map(|player| self.backing_playback_origin_seconds + player.get_pos().as_secs_f32())
            .unwrap_or(self.backing_position_seconds)
    }

    fn backing_playback_position(&self) -> Option<f32> {
        self.backing_stems
            .iter()
            .any(|stem| stem.path.is_some())
            .then_some(self.backing_playback_position_seconds())
    }

    pub(super) fn draw_backing_track(&mut self, ui: &mut Ui) {
        self.sync_backing_mix_settings();
        if self.ensure_backing_waveforms() {
            self.backing_project_dirty = true;
            self.save_playlist();
        }
        let playback_position = self.backing_playback_position();
        let mut tempo_changed = false;
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("BackingTrack");
                    ui.small(format!("Projet : {}", self.backing_project_name));
                    ui.separator();
                    ui.label("BPM original");
                    tempo_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.backing_original_bpm)
                                .range(30.0..=240.0),
                        )
                        .changed();
                    ui.separator();
                    ui.label("BPM cible");
                    if ui
                        .add(egui::DragValue::new(&mut self.backing_bpm).range(30.0..=240.0))
                        .changed()
                    {
                        tempo_changed = true;
                    }
                    ui.separator();
                    let ratio = (self.backing_bpm / self.backing_original_bpm).clamp(0.5, 2.0);
                    ui.small(format!("Tempo ×{ratio:.3} — hauteur conservée"));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.set_min_width(ui.available_width().max(BACKING_MIN_CONTENT_WIDTH));
                    let widths = backing_track_cell_widths();
                    ui.add_sized([widths[0], 24.0], egui::Label::new(RichText::new("#").strong()));
                    ui.add_sized(
                        [widths[1], 24.0],
                        egui::Label::new(RichText::new("Titre / rôle").strong()),
                    );
                    ui.add_sized(
                        [widths[2], 24.0],
                        egui::Label::new(RichText::new("Réf").strong()),
                    );
                    ui.add_sized([widths[3], 24.0], egui::Label::new(RichText::new("M").strong()));
                    ui.add_sized([widths[4], 24.0], egui::Label::new(RichText::new("S").strong()));
                    ui.add_sized([widths[5], 24.0], egui::Label::new(RichText::new("").strong()));
                    ui.add_sized(
                        [widths[6], 24.0],
                        egui::Label::new(RichText::new("Décalage").strong()),
                    );
                    ui.add_sized(
                        [ui.available_width().max(180.0), 24.0],
                        egui::Label::new(RichText::new("Spectre / forme d'onde").strong()),
                    );
                });
                ui.separator();
                let mut remove_index = None;
                let mut changed = false;
                let mut reference_index = None;
                let mut seek_request = None;
                egui::ScrollArea::both()
                    .id_salt("backing_stems_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for index in 0..self.backing_stems.len() {
                            let waveform = self.backing_waveforms.get(index).and_then(Clone::clone);
                            let duration = self.backing_durations.get(index).copied().flatten();
                            let stem = &mut self.backing_stems[index];
                            let tempo_ratio =
                                (self.backing_bpm / self.backing_original_bpm).clamp(0.5, 2.0);
                            let track_playback_position = playback_position
                                .map(|position| position * tempo_ratio + stem.sync_offset_seconds);
                            Frame::new()
                                .fill(if index % 2 == 0 {
                                    Color32::from_rgb(35, 41, 51)
                                } else {
                                    Color32::from_rgb(31, 37, 46)
                                })
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 5))
                                .show(ui, |ui| {
                                    ui.set_min_width(
                                        ui.available_width().max(BACKING_MIN_CONTENT_WIDTH),
                                    );
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.horizontal(|ui| {
                                        let widths = backing_track_cell_widths();
                                        ui.add_sized(
                                            [widths[0], 24.0],
                                            egui::Label::new(format!("{:02}", index + 1)),
                                        );
                                        let role = ui.add_sized(
                                            [widths[1], 24.0],
                                            egui::TextEdit::singleline(&mut stem.role)
                                                .hint_text("Nom du fichier"),
                                        );
                                        changed |= role.changed();
                                        if ui
                                            .add_sized(
                                                [widths[2], 24.0],
                                                egui::Button::new(RichText::new("Réf").strong())
                                                    .selected(stem.reference),
                                            )
                                            .on_hover_text("Utiliser cette piste comme Référence")
                                            .clicked()
                                        {
                                            reference_index = Some(index);
                                        }
                                        if ui
                                            .add_sized(
                                                [widths[3], 24.0],
                                                egui::Button::new(RichText::new("M").strong())
                                                    .selected(stem.muted),
                                            )
                                            .on_hover_text("Mute")
                                            .clicked()
                                        {
                                            stem.muted = !stem.muted;
                                            changed = true;
                                        }
                                        if ui
                                            .add_sized(
                                                [widths[4], 24.0],
                                                egui::Button::new(RichText::new("S").strong())
                                                    .selected(stem.solo),
                                            )
                                            .on_hover_text("Solo")
                                            .clicked()
                                        {
                                            stem.solo = !stem.solo;
                                            changed = true;
                                        }
                                        if ui
                                            .add_sized(
                                                [widths[5], 24.0],
                                                egui::Button::new(RichText::new("\u{e872}").font(
                                                    FontId::new(
                                                        17.0,
                                                        FontFamily::Name("material_icons".into()),
                                                    ),
                                                )),
                                            )
                                            .on_hover_text("Supprimer cette piste")
                                            .clicked()
                                        {
                                            remove_index = Some(index);
                                        }
                                        ui.add_sized(
                                            [widths[6], 24.0],
                                            egui::Label::new(format!(
                                                "{:+.2}s",
                                                stem.sync_offset_seconds
                                            )),
                                        );
                                        let waveform_width = ui.available_width();
                                        let waveform_response = draw_backing_waveform(
                                            ui,
                                            waveform.as_deref(),
                                            stem.reference,
                                            waveform_width,
                                            track_playback_position,
                                            duration,
                                        );
                                        if waveform_response.double_clicked()
                                            && let (Some(pointer), Some(duration)) =
                                                (waveform_response.interact_pointer_pos(), duration)
                                        {
                                            let progress = ((pointer.x
                                                - waveform_response.rect.left())
                                                / waveform_response.rect.width())
                                            .clamp(0.0, 1.0);
                                            let track_position = progress * duration;
                                            seek_request = Some(
                                                ((track_position - stem.sync_offset_seconds)
                                                    / tempo_ratio)
                                                    .max(0.0),
                                            );
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        let mix_widths = backing_mix_cell_widths();
                                        ui.add_sized(
                                            [BACKING_INDEX_WIDTH, 24.0],
                                            egui::Label::new(""),
                                        );
                                        ui.add_sized(
                                            [mix_widths[0], 24.0],
                                            egui::Label::new("Vol"),
                                        );
                                        changed |= ui
                                            .add_sized(
                                                [mix_widths[1], 24.0],
                                                egui::Slider::new(&mut stem.volume, 0.0..=1.0)
                                                    .show_value(false),
                                            )
                                            .changed();
                                        ui.add_sized(
                                            [mix_widths[2], 24.0],
                                            egui::Label::new(format!(
                                                "{} dB",
                                                format_fader_db(Self::fader_to_db(stem.volume))
                                            )),
                                        );
                                        ui.add_sized(
                                            [mix_widths[3], 24.0],
                                            egui::Label::new("Pan"),
                                        );
                                        changed |= ui
                                            .add_sized(
                                                [mix_widths[4], 24.0],
                                                egui::Slider::new(&mut stem.pan, -1.0..=1.0)
                                                    .show_value(false),
                                            )
                                            .changed();
                                        ui.add_sized(
                                            [mix_widths[5], 24.0],
                                            egui::Label::new(if stem.pan < -0.05 {
                                                "G"
                                            } else if stem.pan > 0.05 {
                                                "D"
                                            } else {
                                                "C"
                                            }),
                                        );
                                    });
                                });
                            ui.add_space(2.0);
                        }
                    });
                if let Some(reference_index) = reference_index {
                    for (index, stem) in self.backing_stems.iter_mut().enumerate() {
                        stem.reference = index == reference_index;
                        if stem.reference {
                            stem.sync_offset_seconds = 0.0;
                        }
                    }
                    changed = true;
                }
                if let Some(error) = &self.audio_error {
                    ui.colored_label(Color32::from_rgb(255, 205, 95), error);
                }
                if let Some(index) = remove_index {
                    self.remove_backing_stem(index);
                    changed = false;
                }
                if let Some(position) = seek_request {
                    self.set_backing_position(position);
                }
                if changed {
                    self.backing_project_dirty = true;
                    self.sync_backing_mix_settings();
                    self.save_playlist();
                }
            });
        if tempo_changed {
            self.backing_original_bpm = self.backing_original_bpm.clamp(30.0, 240.0);
            self.backing_bpm = self.backing_bpm.clamp(30.0, 240.0);
            self.backing_project_dirty = true;
            self.save_playlist();
            if self.workspace.transport == TransportState::Playing {
                self.start_backing_playback();
            }
        }
    }
}
