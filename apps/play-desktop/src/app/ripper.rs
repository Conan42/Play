use super::*;
use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};

impl PlayDesktop {
    pub(super) fn system_capture_active(&self) -> bool {
        self.system_capture_thread.is_some()
    }

    pub(super) fn poll_system_capture(&mut self) {
        let finished =
            self.system_capture_thread.as_ref().is_some_and(std::thread::JoinHandle::is_finished);
        if !finished {
            return;
        }

        let Some(thread) = self.system_capture_thread.take() else { return };
        self.system_capture_stop = None;
        self.system_capture_started_at = None;
        match thread.join() {
            Ok(Ok(sample_count)) => {
                self.audio_error = Some(format!(
                    "Capture terminée : {} ({sample_count} échantillons)",
                    self.system_capture_path
                        .as_deref()
                        .map(Path::display)
                        .map(|path| path.to_string())
                        .unwrap_or_else(|| "fichier WAV".to_owned())
                ));
            }
            Ok(Err(error)) => {
                self.system_capture_error = Some(error.clone());
                self.audio_error = Some(format!("Capture WAV : {error}"));
            }
            Err(_) => {
                let error = "Le thread de capture WAV s'est arrêté brutalement.".to_owned();
                self.system_capture_error = Some(error.clone());
                self.audio_error = Some(error);
            }
        }
    }

    pub(super) fn start_system_capture(&mut self) {
        self.poll_system_capture();
        if self.system_capture_active() {
            return;
        }
        let suggested_name = Self::capture_file_name();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Enregistrer la sortie système en WAV")
            .set_file_name(suggested_name)
            .add_filter("WAV PCM", &["wav"])
            .save_file()
        else {
            return;
        };
        let path = if path.extension().and_then(|extension| extension.to_str()) == Some("wav") {
            path
        } else {
            path.with_extension("wav")
        };

        self.system_capture_error = None;
        #[cfg(target_os = "linux")]
        {
            let Some(output_name) = Self::preferred_ripper_output_name() else {
                self.system_capture_error = Some(
                    "Aucune sortie audio par défaut n'est disponible pour PipeWire.".to_owned(),
                );
                return;
            };
            if let Err(error) = self.start_pipewire_capture(path, output_name) {
                self.system_capture_error = Some(error);
            }
            return;
        }

        #[cfg(not(target_os = "linux"))]
        let (device, device_name) = {
            if self.microphone_devices.is_empty() {
                self.refresh_microphone_devices();
            }
            let Some(device) = self.microphone_devices.get(self.microphone_device_index).cloned()
            else {
                self.system_capture_error = Some("Aucune entrée audio disponible.".to_owned());
                return;
            };
            let device_name = device.to_string();
            (device, device_name)
        };

        #[cfg(not(target_os = "linux"))]
        {
            let builder = match rodio::microphone::MicrophoneBuilder::new().device(device) {
                Ok(builder) => builder,
                Err(error) => {
                    self.system_capture_error = Some(format!("Source {device_name} : {error}"));
                    return;
                }
            };
            let builder = match builder.default_config() {
                Ok(builder) => builder,
                Err(error) => {
                    self.system_capture_error =
                        Some(format!("Configuration de {device_name} : {error}"));
                    return;
                }
            };
            let config = builder.get_config();
            let channels = config.channel_count.get();
            let sample_rate = config.sample_rate.get();
            let analysis_buffer = Arc::new(Mutex::new(LiveAnalysisBuffer::new(
                channels as usize,
                sample_rate as f32,
            )));
            let microphone = match builder.open_stream() {
                Ok(microphone) => microphone,
                Err(error) => {
                    self.system_capture_error =
                        Some(format!("Ouverture de {device_name} : {error}"));
                    return;
                }
            };
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread_path = path.clone();
            let thread_analysis = analysis_buffer.clone();
            let thread_gains = self.channel_gains.clone();
            let thread = std::thread::spawn(move || {
                Self::write_capture_wav(
                    microphone,
                    thread_stop,
                    thread_path,
                    thread_analysis,
                    thread_gains,
                    channels,
                    sample_rate,
                )
            });

            self.system_capture_stop = Some(stop);
            self.system_capture_thread = Some(thread);
            self.system_capture_buffer = Some(analysis_buffer);
            self.system_capture_path = Some(path);
            self.system_capture_source_name = Some(device_name.clone());
            self.system_capture_started_at = Some(Instant::now());
            self.audio_error = Some(format!("Capture en cours depuis {device_name}"));
        }
    }

    #[cfg(target_os = "linux")]
    fn start_pipewire_capture(&mut self, path: PathBuf, output_name: String) -> Result<(), String> {
        let mut child = Command::new("pw-record")
            .args([
                "--properties",
                "stream.capture.sink=true",
                "--target",
                output_name.as_str(),
                "--raw",
                "--format",
                "s16",
                "--channels",
                "2",
                "--rate",
                "48000",
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Lancement de pw-record impossible : {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "pw-record n'a pas fourni de flux de capture.".to_owned())?;
        let channels = 2;
        let sample_rate = 48_000;
        let analysis_buffer =
            Arc::new(Mutex::new(LiveAnalysisBuffer::new(channels, sample_rate as f32)));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_analysis = analysis_buffer.clone();
        let thread_gains = self.channel_gains.clone();
        let thread_path = path.clone();
        let thread = std::thread::spawn(move || {
            Self::write_pipewire_capture_wav(
                child,
                stdout,
                thread_stop,
                thread_path,
                thread_analysis,
                thread_gains,
                channels,
                sample_rate,
            )
        });
        self.system_capture_stop = Some(stop);
        self.system_capture_thread = Some(thread);
        self.system_capture_buffer = Some(analysis_buffer);
        self.system_capture_source_name = Some(format!("Sortie système : {output_name}"));
        self.system_capture_path = Some(path);
        self.system_capture_started_at = Some(Instant::now());
        self.audio_error = Some("Capture de la sortie PipeWire en cours".to_owned());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn write_pipewire_capture_wav(
        mut child: Child,
        mut stdout: ChildStdout,
        stop: Arc<AtomicBool>,
        path: PathBuf,
        analysis: Arc<Mutex<LiveAnalysisBuffer>>,
        gains: Arc<Mutex<[f32; 2]>>,
        channels: usize,
        sample_rate: u32,
    ) -> Result<usize, String> {
        let spec = hound::WavSpec {
            channels: channels as u16,
            sample_rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec)
            .map_err(|error| format!("Création de {} impossible : {error}", path.display()))?;
        let mut raw = [0_u8; 8192];
        let mut pending_bytes = Vec::new();
        let mut pending_frame = Vec::with_capacity(channels);
        let mut pending_analysis = Vec::with_capacity(channels * 512);
        let mut sample_count = 0;
        while !stop.load(Ordering::Relaxed) {
            let byte_count = stdout
                .read(&mut raw)
                .map_err(|error| format!("Lecture de pw-record impossible : {error}"))?;
            if byte_count == 0 {
                break;
            }
            pending_bytes.extend_from_slice(&raw[..byte_count]);
            let complete_bytes = pending_bytes.len() / 2 * 2;
            for bytes in pending_bytes[..complete_bytes].chunks_exact(2) {
                let sample = i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0;
                let channel = pending_frame.len();
                let gain = gains.lock().map(|gains| gains[channel.min(1)]).unwrap_or(1.0);
                pending_frame.push(sample * gain);
                if pending_frame.len() == channels {
                    for sample in &pending_frame {
                        let sample = (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                        writer.write_sample(sample).map_err(|error| {
                            format!("Écriture de {} impossible : {error}", path.display())
                        })?;
                    }
                    pending_analysis.extend_from_slice(&pending_frame);
                    sample_count += channels;
                    pending_frame.clear();
                    if pending_analysis.len() >= channels * 512 {
                        if let Ok(mut buffer) = analysis.lock() {
                            buffer.push_samples(&pending_analysis);
                        }
                        pending_analysis.clear();
                    }
                }
            }
            pending_bytes.drain(..complete_bytes);
        }
        let _ = child.kill();
        let _ = child.wait();
        if !pending_analysis.is_empty()
            && let Ok(mut buffer) = analysis.lock()
        {
            buffer.push_samples(&pending_analysis);
        }
        writer
            .finalize()
            .map_err(|error| format!("Finalisation de {} impossible : {error}", path.display()))?;
        if sample_count == 0 {
            return Err(format!(
                "PipeWire n'a fourni aucun échantillon depuis la sortie sélectionnée : {}",
                path.display()
            ));
        }
        Ok(sample_count)
    }

    pub(super) fn stop_system_capture(&mut self) {
        let Some(thread) = self.system_capture_thread.take() else {
            self.system_capture_stop = None;
            self.system_capture_started_at = None;
            return;
        };
        if let Some(stop) = self.system_capture_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.system_capture_started_at = None;
        match thread.join() {
            Ok(Ok(sample_count)) => {
                self.audio_error = Some(format!(
                    "Capture WAV terminée : {} ({sample_count} échantillons)",
                    self.system_capture_path
                        .as_deref()
                        .map(Path::display)
                        .map(|path| path.to_string())
                        .unwrap_or_else(|| "fichier WAV".to_owned())
                ));
            }
            Ok(Err(error)) => {
                self.system_capture_error = Some(error.clone());
                self.audio_error = Some(format!("Capture WAV : {error}"));
            }
            Err(_) => {
                let error = "Le thread de capture WAV s'est arrêté brutalement.".to_owned();
                self.system_capture_error = Some(error.clone());
                self.audio_error = Some(error);
            }
        }
    }

    fn capture_file_name() -> String {
        let timestamp = chrono_like_timestamp();
        format!("songsterr-capture-{timestamp}.wav")
    }

    #[cfg(not(target_os = "linux"))]
    fn write_capture_wav(
        mut microphone: rodio::microphone::Microphone,
        stop: Arc<AtomicBool>,
        path: PathBuf,
        analysis: Arc<Mutex<LiveAnalysisBuffer>>,
        gains: Arc<Mutex<[f32; 2]>>,
        channels: u16,
        sample_rate: u32,
    ) -> Result<usize, String> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec)
            .map_err(|error| format!("Création de {} impossible : {error}", path.display()))?;
        let mut sample_count = 0;
        let channel_count = channels as usize;
        let mut pending_frame = Vec::with_capacity(channel_count);
        let mut pending_analysis = Vec::with_capacity(channel_count * 512);
        while !stop.load(Ordering::Relaxed) {
            let Some(sample) = microphone.next() else { break };
            let channel = pending_frame.len();
            let gain = gains.lock().map(|gains| gains[channel.min(1)]).unwrap_or(1.0);
            pending_frame.push(sample * gain);
            if pending_frame.len() == channel_count {
                for sample in &pending_frame {
                    let sample = (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                    writer.write_sample(sample).map_err(|error| {
                        format!("Écriture de {} impossible : {error}", path.display())
                    })?;
                }
                pending_analysis.extend_from_slice(&pending_frame);
                sample_count += channel_count;
                pending_frame.clear();
                if pending_analysis.len() >= channel_count * 512 {
                    if let Ok(mut buffer) = analysis.lock() {
                        buffer.push_samples(&pending_analysis);
                    }
                    pending_analysis.clear();
                }
            }
        }
        if !pending_analysis.is_empty()
            && let Ok(mut buffer) = analysis.lock()
        {
            buffer.push_samples(&pending_analysis);
        }
        writer
            .finalize()
            .map_err(|error| format!("Finalisation de {} impossible : {error}", path.display()))?;
        if sample_count == 0 {
            return Err(format!(
                "La source audio n'a fourni aucun échantillon : {}",
                path.display()
            ));
        }
        Ok(sample_count)
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "capture".to_owned())
}

impl PlayDesktop {
    #[cfg(target_os = "linux")]
    fn preferred_ripper_output_name() -> Option<String> {
        let interface_output =
            rodio::cpal::default_host().output_devices().ok().and_then(|devices| {
                devices
                    .filter_map(|device| {
                        device.description().ok().map(|description| {
                            (description.name().to_owned(), description.name().to_ascii_lowercase())
                        })
                    })
                    .find(|(_, name)| name.contains("scarlett") || name.contains("focusrite"))
                    .map(|(name, _)| name)
            });

        interface_output.or_else(Self::default_output_device_name)
    }

    pub(super) fn prefer_ripper_source(&mut self) {
        if self.microphone_devices.is_empty() {
            self.refresh_microphone_devices();
        }
        if let Some(index) = self.microphone_devices.iter().position(|device| {
            let name = device.to_string().to_ascii_lowercase();
            name.contains("monitor")
                || name.contains("loopback")
                || name.contains("stereo mix")
                || name.contains("what u hear")
        }) {
            self.microphone_device_index = index;
        }
    }

    pub(super) fn draw_ripper(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(Color32::from_rgb(29, 34, 43))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.heading("Ripper");
                ui.label("Capture la sortie audio du système dans un fichier WAV pour la réutiliser dans BackingTrack.");
                ui.add_space(8.0);
                self.draw_system_capture_status(ui);
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new("Procédure Songsterr").strong());
                ui.label("1. Envoyez le navigateur vers la sortie Scarlett et vérifiez que le son arrive dans le casque.");
                ui.label("2. Dans la barre d'outils contextuelle, vérifiez que la sortie système Scarlett est affichée.");
                ui.label("3. Démarrez la capture, lancez Songsterr, puis arrêtez la capture à la fin du morceau.");
                ui.label("4. Le WAV peut ensuite être ajouté comme stem dans BackingTrack.");
                ui.add_space(8.0);
                ui.small("La capture enregistre le mix stéréo entendu par le casque. Elle ne sépare pas les instruments.");
                ui.small("Les faders L/R du panneau droit règlent le niveau enregistré dans le WAV.");
                if let Some(error) = &self.system_capture_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if let Some(error) = &self.audio_error {
                    ui.colored_label(Color32::from_rgb(255, 205, 95), error);
                }
        });
        ui.add_space(8.0);
        let spectrum_height = ui.available_height().max(1.0);
        self.draw_spectrum_with_height(ui, spectrum_height);
    }

    pub(super) fn draw_system_capture_device_menu(&mut self, ui: &mut Ui) {
        #[cfg(target_os = "linux")]
        {
            let output_name = Self::preferred_ripper_output_name()
                .unwrap_or_else(|| "Aucune sortie audio par défaut".to_owned());
            let response = toolbar_menu_button(
                ui,
                &self.toolbar_textures,
                "\u{e1b2}",
                "Sortie système capturée",
            );
            egui::Popup::menu(&response).show(|ui| {
                ui.set_min_width(360.0);
                ui.label("Sortie système capturée");
                ui.small("Ripper privilégie automatiquement la sortie Scarlett ou Focusrite.");
                ui.separator();
                ui.label(output_name);
                ui.small(
                    "Si l'interface n'est pas disponible, la sortie audio par défaut est utilisée.",
                );
            });
            return;
        }

        #[cfg(not(target_os = "linux"))]
        {
            if self.microphone_devices.is_empty() {
                self.refresh_microphone_devices();
            }
            let names: Vec<String> =
                self.microphone_devices.iter().map(ToString::to_string).collect();
            let selected_name = names
                .get(self.microphone_device_index)
                .map(String::as_str)
                .unwrap_or("Aucune entrée");
            let response = toolbar_menu_button(
                ui,
                &self.toolbar_textures,
                "\u{e1b2}",
                "Sélectionner la source à capturer",
            );
            let mut refresh = false;
            let mut selected_index = self.microphone_device_index;
            egui::Popup::menu(&response).show(|ui| {
                ui.set_min_width(360.0);
                ui.label("Source de capture WAV");
                ui.small("Choisir « Monitor » de la sortie Scarlett pour enregistrer Songsterr.");
                ui.separator();
                for (index, name) in names.iter().enumerate() {
                    let is_monitor = name.to_ascii_lowercase().contains("monitor");
                    let label = if is_monitor {
                        format!("{name}  • sortie système")
                    } else {
                        name.clone()
                    };
                    if ui.selectable_label(selected_index == index, label).clicked() {
                        selected_index = index;
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Actualiser les sources").clicked() {
                    refresh = true;
                    ui.close();
                }
            });
            if selected_index != self.microphone_device_index {
                if self.system_capture_active() {
                    self.stop_system_capture();
                }
                self.microphone_device_index = selected_index;
            }
            if refresh {
                self.refresh_microphone_devices();
            }
            if response.hovered() {
                response.on_hover_text(format!("Source actuelle : {selected_name}"));
            }
        }
    }

    pub(super) fn draw_system_capture_status(&self, ui: &mut Ui) {
        let source = self
            .system_capture_source_name
            .clone()
            .or_else(|| {
                #[cfg(target_os = "linux")]
                {
                    Self::preferred_ripper_output_name()
                        .map(|name| format!("Sortie système : {name}"))
                }
                #[cfg(not(target_os = "linux"))]
                {
                    self.microphone_devices
                        .get(self.microphone_device_index)
                        .map(ToString::to_string)
                }
            })
            .unwrap_or_else(|| "Aucune source sélectionnée".to_owned());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Capture système").strong());
            ui.small(format!("Source : {source}"));
            if self.system_capture_active() {
                let elapsed = self
                    .system_capture_started_at
                    .map(|started| started.elapsed().as_secs())
                    .unwrap_or(0);
                ui.colored_label(
                    Color32::from_rgb(255, 205, 95),
                    format!("● en cours · {:02}:{:02}", elapsed / 60, elapsed % 60),
                );
                if let Some(path) = &self.system_capture_path {
                    ui.small(path.to_string_lossy());
                }
            } else if let Some(path) = &self.system_capture_path {
                ui.small(format!("Dernier fichier : {}", path.display()));
            }
        });
        let source_is_loopback = source.to_ascii_lowercase().contains("sortie système")
            || source.to_ascii_lowercase().contains("monitor")
            || source.to_ascii_lowercase().contains("loopback")
            || source.to_ascii_lowercase().contains("stereo mix")
            || source.to_ascii_lowercase().contains("what u hear");
        if !source_is_loopback {
            ui.colored_label(
                Color32::from_rgb(255, 205, 95),
                "Cette source est une entrée physique. Pour Songsterr, choisissez une source « Monitor ».",
            );
        }
    }
}
