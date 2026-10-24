use super::*;

impl PlayDesktop {
    pub(super) fn start_room_measurement(&mut self) {
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

    pub(super) fn start_microphone(&mut self) {
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

    pub(super) fn stop_microphone_monitor(&mut self) {
        if let Some(player) = self.microphone_monitor_player.take() {
            player.stop();
        }
        self.microphone_monitor_output = None;
        self.microphone_monitor_output_name = None;
    }

    pub(super) fn set_microphone_monitoring(&mut self, enabled: bool) {
        self.microphone_monitor_enabled = enabled;
        if self.microphone_buffer.is_some()
            && self.scene.mode == WorkspaceMode::LiveTablature
            && self.live_analysis_source == LiveAnalysisSource::Microphone
        {
            self.stop_microphone();
            self.start_microphone();
        }
    }

    pub(super) fn stop_microphone(&mut self) {
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

    pub(super) fn refresh_microphone_devices(&mut self) {
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

    pub(super) fn update_room_spectrum_from_buffer(
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

    pub(super) fn draw_room_analyzer(&mut self, ui: &mut Ui) {
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
}
