use super::*;

impl PlayDesktop {
    pub(super) fn draw_spectrum(&self, ui: &mut egui::Ui) {
        self.draw_spectrum_with_height(ui, SPECTRUM_HEIGHT);
    }

    pub(super) fn draw_spectrum_with_height(&self, ui: &mut egui::Ui, height: f32) {
        let desired_size = egui::vec2(ui.available_width(), height.max(1.0));
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

    pub(super) fn draw_room_spectrum(&self, ui: &mut egui::Ui) {
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

    pub(super) fn spectrum_display_level(amplitude: f32) -> f32 {
        const FLOOR_DB: f32 = -60.0;
        if amplitude <= 0.000001 {
            0.0
        } else {
            ((20.0 * amplitude.clamp(0.0, 1.0).log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
        }
    }

    pub(super) fn room_spectrum_display_level(amplitude: f32) -> f32 {
        const ROOM_DB_MAX: f32 = 110.0;
        if amplitude <= 0.000001 {
            0.0
        } else {
            ((20.0 * amplitude.clamp(0.0, 1.0).log10() + ROOM_DB_MAX) / ROOM_DB_MAX).clamp(0.0, 1.0)
        }
    }

    pub(super) fn draw_toolbar(&mut self, ui: &mut Ui) {
        ui.set_min_height(42.0);
        ui.spacing_mut().item_spacing.x = 6.0;
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
            if self.scene.mode == WorkspaceMode::BackingTrack {
                ui.separator();
                if ui.button("Ouvrir un projet BackingTrack…").clicked() {
                    self.request_backing_action(PendingBackingAction::LoadProject);
                    ui.close();
                }
                if ui.button("Nouveau projet BackingTrack").clicked() {
                    self.request_backing_action(PendingBackingAction::NewProject);
                    ui.close();
                }
            }
        });

        toolbar_separator(ui);

        // Les six commandes suivent le même ordre que le Player
        // d'Exec Modernisé : précédent, lecture, arrêt, suivant, boucle,
        // enchaînement.
        let backing_mode = self.scene.mode == WorkspaceMode::BackingTrack;
        if toolbar_icon_button(
            ui,
            &self.toolbar_textures,
            "\u{e045}",
            false,
            if backing_mode { "Revenir au début du BackingTrack" } else { "Piste précédente" },
        )
        .clicked()
        {
            if backing_mode {
                self.seek_backing_to_start();
            } else {
                self.change_track(-1);
            }
        }
        let play_icon = if self.workspace.transport == TransportState::Playing {
            "\u{e034}"
        } else {
            "\u{e037}"
        };
        if toolbar_icon_button(
            ui,
            &self.toolbar_textures,
            play_icon,
            false,
            if backing_mode { "Lecture / pause du BackingTrack" } else { "Lecture / pause" },
        )
        .clicked()
        {
            if backing_mode {
                self.toggle_backing_playback();
            } else {
                self.toggle_playback();
            }
        }
        if toolbar_icon_button(
            ui,
            &self.toolbar_textures,
            "\u{e047}",
            false,
            if backing_mode { "Arrêter le BackingTrack" } else { "Arrêter" },
        )
        .clicked()
        {
            if backing_mode {
                self.stop_backing_playback();
                self.workspace.stop();
            } else {
                self.stop_playback();
            }
        }
        if toolbar_icon_button(
            ui,
            &self.toolbar_textures,
            "\u{e044}",
            false,
            if backing_mode { "Aller à la fin du BackingTrack" } else { "Piste suivante" },
        )
        .clicked()
        {
            if backing_mode {
                self.seek_backing_to_end();
            } else {
                self.change_track(1);
            }
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

        if !backing_mode {
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
        }
    }

    pub(super) fn draw_microphone_profile_menu(&mut self, ui: &mut Ui) {
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

    pub(super) fn draw_mode_toolbar(&mut self, ui: &mut Ui) {
        ui.set_min_height(42.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.set_height(38.0);
        match self.scene.mode {
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
            WorkspaceMode::BackingTrack => {
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e2c7}",
                    false,
                    "Charger une ou plusieurs pistes audio",
                )
                .clicked()
                {
                    self.import_backing_audio_files();
                }
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e161}",
                    false,
                    "Enregistrer le projet BackingTrack",
                )
                .clicked()
                {
                    self.save_backing_project();
                }
                toolbar_separator(ui);
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e627}",
                    false,
                    "Synchroniser les stems avec la Référence",
                )
                .clicked()
                {
                    self.synchronize_backing_stems();
                }
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e145}",
                    false,
                    "Créer une piste vide",
                )
                .clicked()
                {
                    self.add_empty_backing_stem();
                }
            }
            WorkspaceMode::Ripper => {
                self.draw_system_capture_device_menu(ui);
                let capture_active = self.system_capture_active();
                if toolbar_icon_button(
                    ui,
                    &self.toolbar_textures,
                    "\u{e029}",
                    capture_active,
                    if capture_active {
                        "Arrêter et enregistrer le WAV"
                    } else {
                        "Capturer la sortie système en WAV"
                    },
                )
                .clicked()
                {
                    if capture_active {
                        self.stop_system_capture();
                    } else {
                        self.start_system_capture();
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn select_mode(&mut self, mode: WorkspaceMode) {
        if self.scene.mode == WorkspaceMode::BackingTrack
            && mode != WorkspaceMode::BackingTrack
            && self.backing_project_dirty
        {
            self.pending_backing_action = Some(PendingBackingAction::SelectMode(mode));
            return;
        }
        self.select_mode_without_confirmation(mode);
    }

    pub(super) fn select_mode_without_confirmation(&mut self, mode: WorkspaceMode) {
        if mode == WorkspaceMode::BackingTrack && self.scene.mode != WorkspaceMode::BackingTrack {
            self.stop_playback();
        } else if self.scene.mode == WorkspaceMode::BackingTrack
            && mode != WorkspaceMode::BackingTrack
        {
            self.stop_backing_playback();
            self.workspace.stop();
        }
        if self.scene.mode == WorkspaceMode::Ripper && mode != WorkspaceMode::Ripper {
            self.stop_system_capture();
        }
        self.scene.mode = mode;
        self.workspace.document.mode = mode;
        if mode == WorkspaceMode::LiveTablature {
            self.tablature_playing = true;
            self.tablature_recording = true;
            self.tablature_pause_started = None;
        }
        if mode == WorkspaceMode::Ripper {
            self.prefer_ripper_source();
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

    pub(super) fn draw_playback_seek(&mut self, ui: &mut Ui) {
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

    pub(super) fn draw_led_meter(ui: &mut Ui, level: f32, height: f32) {
        ui.vertical(|ui| {
            let led_count = if height >= 720.0 {
                LED_COUNT * 4
            } else if height >= 360.0 {
                LED_COUNT * 2
            } else {
                LED_COUNT
            };
            const LED_WIDTH: f32 = 22.0;
            ui.set_width(LED_WIDTH);
            let (response, painter) =
                ui.allocate_painter(vec2(LED_WIDTH, height), egui::Sense::hover());
            let rect = response.rect;
            let step = rect.height() / led_count as f32;
            let decibels = if level > 0.0001 { 20.0 * level.log10() } else { -60.0 };
            for index in 0..led_count {
                let threshold = FADER_MIN_DB
                    + index as f32 * (FADER_MAX_DB - FADER_MIN_DB)
                        / (led_count.saturating_sub(1).max(1) as f32);
                let active = decibels >= threshold;
                let color = if threshold >= 6.0 {
                    if active { Color32::RED } else { Color32::from_rgb(55, 24, 28) }
                } else if threshold >= 0.0 {
                    if active { Color32::YELLOW } else { Color32::from_rgb(58, 55, 24) }
                } else if active {
                    Color32::from_rgb(72, 210, 166)
                } else {
                    Color32::from_rgb(26, 67, 63)
                };
                let center =
                    egui::pos2(rect.center().x, rect.bottom() - (index as f32 + 0.5) * step);
                let led_height = (step - 2.0).max(1.5);
                let led = egui::Rect::from_center_size(
                    center,
                    vec2((rect.width() - 1.0).max(4.0), led_height),
                );
                painter.rect_filled(led, 1.0, color);
            }
        });
    }

    pub(super) fn draw_decibel_scale(ui: &mut Ui, height: f32) {
        ui.vertical(|ui| {
            ui.set_width(28.0);
            let (response, painter) = ui.allocate_painter(vec2(28.0, height), egui::Sense::hover());
            let rect = response.rect;
            for decibels in [10, 0, -6, -12, -18, -24, -36, -48, -60] {
                let normalized = (decibels as f32 - FADER_MIN_DB) / (FADER_MAX_DB - FADER_MIN_DB);
                let y = rect.bottom() - normalized * rect.height();
                let y = y.clamp(rect.top() + 6.0, rect.bottom() - 6.0);
                painter.text(
                    egui::pos2(rect.left() + 1.0, y),
                    egui::Align2::LEFT_CENTER,
                    format_fader_db(decibels as f32),
                    FontId::proportional(9.0),
                    Color32::from_gray(170),
                );
            }
        });
    }

    pub(super) fn draw_stereo_fader(ui: &mut Ui, height: f32, value: &mut f32) -> egui::Rect {
        let mut fader_rect = egui::Rect::NOTHING;
        ui.vertical(|ui| {
            ui.set_width(28.0);
            let (response, painter) =
                ui.allocate_painter(vec2(28.0, height), Sense::click_and_drag());
            let rect = response.rect;
            fader_rect = rect;
            let track = egui::Rect::from_center_size(
                rect.center(),
                vec2(8.0, (rect.height() - 16.0).max(1.0)),
            );
            if response.is_pointer_button_down_on() || response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    *value = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
                }
            }
            let level_to_y = |decibels: f32| {
                track.bottom()
                    - ((decibels - FADER_MIN_DB) / (FADER_MAX_DB - FADER_MIN_DB)).clamp(0.0, 1.0)
                        * track.height()
            };
            painter.rect_filled(track, 2.0, Color32::from_rgb(10, 15, 20));
            painter.rect_stroke(
                track,
                2.0,
                egui::Stroke::new(1.0, Color32::from_rgb(53, 62, 72)),
                egui::StrokeKind::Inside,
            );
            let zero_y = level_to_y(0.0);
            painter.line_segment(
                [egui::pos2(track.left() - 3.0, zero_y), egui::pos2(track.right() + 3.0, zero_y)],
                egui::Stroke::new(1.0, Color32::from_rgb(102, 110, 118)),
            );
            let knob_y = track.bottom() - *value * track.height();
            let knob = egui::Rect::from_center_size(
                egui::pos2(track.center().x, knob_y),
                vec2(20.0, 10.0),
            );
            painter.rect_filled(knob, 3.0, Color32::from_rgb(180, 184, 190));
            painter.rect_stroke(
                knob,
                3.0,
                egui::Stroke::new(1.0, Color32::from_gray(230)),
                egui::StrokeKind::Inside,
            );
            let decibels = Self::fader_to_db(*value);
            ui.add_sized([28.0, 16.0], egui::Label::new(format_fader_db(decibels)));
        });
        fader_rect
    }

    pub(super) fn fader_to_gain(value: f32) -> f32 {
        super::fader_to_gain(value)
    }

    pub(super) fn fader_to_db(value: f32) -> f32 {
        super::fader_to_db(value)
    }

    pub(super) fn draw_stereo_meter_panel(&mut self, ui: &mut Ui) {
        ui.set_min_width(142.0);
        let faders_before = self.stereo_faders;
        let meter_height = (ui.available_height() - 28.0).max(180.0);
        let levels = if self.scene.mode == WorkspaceMode::BackingTrack {
            let mut levels = [0.0; 2];
            if let Ok(mut stored_levels) = self.backing_levels.lock() {
                levels = *stored_levels;
                *stored_levels = [0.0; 2];
            }
            levels
        } else if self.scene.mode == WorkspaceMode::Ripper {
            self.channel_levels
        } else {
            [
                self.channel_levels[0] * Self::fader_to_gain(self.stereo_faders[0]),
                self.channel_levels[1] * Self::fader_to_gain(self.stereo_faders[1]),
            ]
        };
        for (display, target) in self.meter_display_levels.iter_mut().zip(levels) {
            // Attaque rapide pour suivre les transitoires comme le spectre,
            // relâchement plus lent pour garder une lecture stable.
            let coefficient = if target > *display { 0.88 } else { 0.18 };
            *display += (target - *display) * coefficient;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            Self::draw_decibel_scale(ui, meter_height);
            Self::draw_led_meter(ui, self.meter_display_levels[0], meter_height);
            let left_fader = Self::draw_stereo_fader(ui, meter_height, &mut self.stereo_faders[0]);
            let right_fader = Self::draw_stereo_fader(ui, meter_height, &mut self.stereo_faders[1]);
            Self::draw_led_meter(ui, self.meter_display_levels[1], meter_height);

            let middle = egui::Rect::from_min_max(
                egui::pos2(left_fader.right() - 10.0, left_fader.top()),
                egui::pos2(right_fader.left() + 10.0, right_fader.bottom()),
            );
            let response = ui.interact(
                middle,
                ui.scope_id().with("stereo_fader_link"),
                Sense::click_and_drag(),
            );
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
        if self.scene.mode == WorkspaceMode::BackingTrack && self.stereo_faders != faders_before {
            self.backing_project_dirty = true;
        }
        if let Some(player) = self.pink_noise_player.as_ref() {
            let average_gain = (Self::fader_to_gain(self.stereo_faders[0])
                + Self::fader_to_gain(self.stereo_faders[1]))
                * 0.5;
            player.set_volume(0.8 * average_gain);
        }
    }

    pub(super) fn draw_room_measurement_dialog(&mut self, ctx: &Context) {
        if !self.show_room_measurement_dialog {
            return;
        }

        let mut start_requested = false;
        let response =
            egui::Modal::new(egui::Id::unique("room_measurement_setup")).show(ctx, |ui| {
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

    pub(super) fn draw_music_loader(&mut self, ctx: &Context) {
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

    pub(super) fn draw_backing_unsaved_dialog(&mut self, ctx: &Context) {
        if self.pending_backing_action.is_none() {
            return;
        }

        let mut save_requested = false;
        let mut discard_requested = false;
        let mut cancel_requested = false;
        let response =
            egui::Modal::new(egui::Id::unique("backing_unsaved_project")).show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.heading("Projet BackingTrack non enregistré");
                ui.add_space(8.0);
                ui.label(format!(
                    "Le projet « {} » contient des modifications non enregistrées.",
                    self.backing_project_name
                ));
                ui.label("Voulez-vous enregistrer le projet avant de continuer ?");
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Enregistrer").clicked() {
                        save_requested = true;
                    }
                    if ui.button("Ne pas enregistrer").clicked() {
                        discard_requested = true;
                    }
                    if ui.button("Annuler").clicked() {
                        cancel_requested = true;
                    }
                });
            });

        if save_requested {
            self.save_backing_project();
            if !self.backing_project_dirty {
                if let Some(action) = self.pending_backing_action.take() {
                    self.perform_backing_action(action);
                }
            }
        } else if discard_requested {
            self.backing_project_dirty = false;
            if let Some(action) = self.pending_backing_action.take() {
                self.perform_backing_action(action);
            }
        } else if cancel_requested || response.should_close() {
            self.pending_backing_action = None;
        }
    }

    pub(super) fn draw_backing_sync_dialog(&self, ctx: &Context) {
        let Some((completed, total)) = self.backing_sync_progress else {
            return;
        };
        let fraction = if total == 0 { 0.0 } else { completed as f32 / total as f32 };
        egui::Modal::new(egui::Id::unique("backing_sync_progress")).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.heading("Synchronisation des BackingTracks");
            ui.add_space(8.0);
            ui.label("Analyse et calage des pistes en cours…");
            ui.add_space(8.0);
            ui.add(
                egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .desired_width(320.0)
                    .show_percentage(),
            );
            ui.add_space(4.0);
            ui.small(format!("Piste {completed} sur {total}"));
        });
    }

    pub(super) fn draw_microphone_controls(&mut self, ui: &mut Ui) {
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

    pub(super) fn track_is_playing(&self) -> bool {
        self.workspace.transport == TransportState::Playing
            && self.audio_player.as_ref().is_some_and(|player| !player.empty())
    }

    pub(super) fn draw_current_track_banner(&self, ui: &mut Ui) {
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

    pub(super) fn draw_ui(&mut self, root_ui: &mut egui::Ui) {
        self.poll_metadata_loading();
        self.poll_backing_sync();
        self.poll_system_capture();
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

        match self.scene.mode {
            WorkspaceMode::Player => self.update_spectrum_from_playback(),
            WorkspaceMode::LiveTablature => self.update_live_analysis(),
            WorkspaceMode::RoomAnalyzer => self.update_spectrum_from_microphone(),
            WorkspaceMode::BackingTrack => {
                self.sync_backing_mix_settings();
                let buffer = self.backing_analysis_buffer.clone();
                let _ = self.update_spectrum_from_buffer(buffer.as_ref());
            }
            WorkspaceMode::Ripper => {
                let buffer = self.system_capture_buffer.clone();
                let _ = self.update_spectrum_from_buffer(buffer.as_ref());
            }
        }

        let has_mode_toolbar = matches!(
            self.scene.mode,
            WorkspaceMode::RoomAnalyzer
                | WorkspaceMode::LiveTablature
                | WorkspaceMode::BackingTrack
                | WorkspaceMode::Ripper
        );
        egui::Panel::top("top_toolbar")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(45, 45, 45))
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    self.draw_toolbar(ui);
                    if has_mode_toolbar {
                        toolbar_separator(ui);
                        self.draw_mode_toolbar(ui);
                    }
                });
            });
        if matches!(
            self.scene.mode,
            WorkspaceMode::Player
                | WorkspaceMode::RoomAnalyzer
                | WorkspaceMode::LiveTablature
                | WorkspaceMode::BackingTrack
                | WorkspaceMode::Ripper
        ) {
            egui::Panel::right("stereo_meter").exact_size(150.0).show(root_ui, |ui| {
                self.draw_stereo_meter_panel(ui);
            });
        }
        if self.scene.mode == WorkspaceMode::BackingTrack {
            egui::Panel::bottom("backing_spectrum").exact_size(SPECTRUM_HEIGHT + 16.0).show(
                root_ui,
                |ui| {
                    self.draw_spectrum(ui);
                },
            );
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
            WorkspaceMode::BackingTrack => self.draw_backing_track(ui),
            WorkspaceMode::Ripper => self.draw_ripper(ui),
        });
        self.draw_music_loader(&self.egui_context.clone());
        self.draw_room_measurement_dialog(&self.egui_context.clone());
        self.draw_backing_unsaved_dialog(&self.egui_context.clone());
        self.draw_backing_sync_dialog(&self.egui_context.clone());
        self.egui_context.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
