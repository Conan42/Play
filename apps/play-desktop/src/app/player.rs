use super::*;

impl PlayDesktop {
    pub(super) fn change_track(&mut self, offset: isize) {
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

    pub(super) fn chain_transition_mode(&self) -> Option<ChainMode> {
        self.chain_mode
            .filter(|mode| *mode != ChainMode::Normal && self.chain_transition_seconds > 0.0)
    }

    pub(super) fn random_track_index(&mut self, current: usize, track_count: usize) -> usize {
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

    pub(super) fn add_music_path(&mut self, path: PathBuf) {
        self.add_music_path_with_cached_bpm(path, None);
    }

    pub(super) fn add_music_path_with_cached_bpm(
        &mut self,
        path: PathBuf,
        cached_bpm: Option<(Option<f32>, bool)>,
    ) {
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
        let cached_metadata = cached_bpm.map(|(bpm, estimated)| {
            let mut metadata = super::bpm::basic_audio_metadata(&path, 0, 0, 0.0);
            metadata.bpm = bpm;
            metadata.bpm_estimated = estimated && bpm.is_some();
            metadata
        });
        self.track_metadata.push(cached_metadata);
        self.track_metadata_queued.push(false);
        let index = self.loaded_music.len();
        self.loaded_music.push(path.clone());
        if self.selected_track.is_none() {
            self.selected_track = Some(index);
        }
        self.queue_metadata_load(index, path, cached_bpm);
    }

    pub(super) fn remove_track(&mut self, index: usize) {
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

    pub(super) fn draw_track_column_menu_contents(&mut self, ui: &mut Ui) {
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

    pub(super) fn track_column_widths(
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

    pub(super) fn track_cell_text(
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

    pub(super) fn draw_track_cell(
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

    pub(super) fn track_info_height(&self) -> f32 {
        TRACK_INFO_HEIGHT
    }

    pub(super) fn draw_track_list(&mut self, ui: &mut Ui) {
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
                                        ui.scope_id().with("track_column_headers"),
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
                                            ui.scope_id().with(("track_row", index)),
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
                                        egui::Popup::context_menu(&row_response).show(|ui| {
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

    pub(super) fn draw_track_info(&mut self, ui: &mut Ui) {
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

    pub(super) fn format_time(seconds: f32) -> String {
        let seconds = seconds.max(0.0) as u64;
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }

    pub(super) fn fader_status_text(&self) -> Option<String> {
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

    pub(super) fn format_sample_rate(sample_rate: u32) -> String {
        if sample_rate % 1000 == 0 {
            format!("{} kHz", sample_rate / 1000)
        } else {
            format!("{:.1} kHz", sample_rate as f32 / 1000.0)
        }
    }

    pub(super) fn toggle_playback(&mut self) {
        if self.scene.mode == WorkspaceMode::BackingTrack {
            self.toggle_backing_playback();
            return;
        }
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

    pub(super) fn stop_playback(&mut self) {
        self.stop_backing_playback();
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

    pub(super) fn default_output_device_name() -> Option<String> {
        rodio::cpal::default_host().default_output_device().and_then(|device| {
            device.description().ok().map(|description| description.name().to_owned())
        })
    }

    pub(super) fn preferred_room_output_name(&self) -> Option<String> {
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

    pub(super) fn open_room_audio_output(&self) -> Result<rodio::MixerDeviceSink, String> {
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

    pub(super) fn start_pink_noise(&mut self) {
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
                "../../../../assets/son/bruit_rose.wav"
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

    pub(super) fn stop_pink_noise(&mut self) {
        if let Some(player) = self.pink_noise_player.take() {
            player.stop();
        }
    }

    pub(super) fn play_track(&mut self, index: usize) {
        let transition = (self.audio_player.is_some() && self.selected_track != Some(index))
            .then(|| {
                self.chain_transition_mode()
                    .or_else(|| self.click_chain_enabled.then_some(ChainMode::VolumeFader))
            })
            .flatten();
        self.play_track_with_transition(index, transition);
    }

    pub(super) fn play_track_with_transition(
        &mut self,
        index: usize,
        transition: Option<ChainMode>,
    ) {
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
            self.queue_metadata_load(index, path.clone(), None);
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
                    super::bpm::basic_audio_metadata(
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

    pub(super) fn finish_track(&mut self) {
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

    pub(super) fn update_chain_transition(&mut self) {
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

    pub(super) fn import_music(&mut self) {
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
}
