use super::*;

impl PlayDesktop {
    pub(super) fn spectrum_band_count_for_graph_width(graph_width: f32) -> usize {
        if graph_width >= SPECTRUM_QUADRUPLE_GRAPH_WIDTH {
            SPECTRUM_BANDS * 4
        } else if graph_width >= SPECTRUM_DOUBLE_GRAPH_WIDTH {
            SPECTRUM_BANDS * 2
        } else {
            SPECTRUM_BANDS
        }
    }

    pub(super) fn current_spectrum_band_count(&self) -> usize {
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

    pub(super) fn analyze_buffer(
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
        let level_start_frame = total_frames.saturating_sub(METER_WINDOW_FRAMES);
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
            if frame >= level_start_frame {
                levels[0] = levels[0].max(left.abs());
                levels[1] = levels[1].max(right.abs());
            }
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

    pub(super) fn update_spectrum_from_buffer(
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

    pub(super) fn update_spectrum_from_playback(&mut self) {
        let buffer = self.analysis_buffer.clone();
        let _ = self.update_spectrum_from_buffer(buffer.as_ref());
    }

    pub(super) fn update_spectrum_from_microphone(&mut self) {
        self.update_live_analysis_from_buffer(self.microphone_buffer.clone(), true);
    }

    pub(super) fn update_live_analysis_from_buffer(
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

    pub(super) fn update_live_analysis(&mut self) {
        match self.live_analysis_source {
            LiveAnalysisSource::Microphone => self.update_spectrum_from_microphone(),
            LiveAnalysisSource::PlaybackOutput => {
                self.update_live_analysis_from_buffer(self.analysis_buffer.clone(), false);
            }
        }
    }

    pub(super) fn update_tablature_history(&mut self) {
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

    pub(super) fn toggle_tablature_playback(&mut self) {
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

    pub(super) fn toggle_tablature_recording(&mut self) {
        self.tablature_recording = !self.tablature_recording;
        if self.tablature_recording {
            self.tablature_last_position = None;
            self.tablature_last_recorded_at = None;
        }
    }

    pub(super) fn clear_tablature(&mut self) {
        self.tablature_notes.clear();
        self.tablature_last_position = None;
        self.tablature_last_recorded_at = None;
        self.tablature_measure_origin = None;
    }

    pub(super) fn microphone_compensation_db(profile: MicrophoneProfile, frequency: f32) -> f32 {
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

    pub(super) fn note_name(midi_note: u8) -> &'static str {
        ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"][midi_note as usize % 12]
    }

    pub(super) fn chord_name(chord: ChordEstimate) -> String {
        let suffix = match chord.quality {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Power => "5",
            ChordQuality::Sus2 => "sus2",
            ChordQuality::Sus4 => "sus4",
        };
        format!("{}{suffix}", Self::note_name(chord.root))
    }

    pub(super) fn guitar_position(midi_note: u8) -> Option<(&'static str, u8)> {
        let strings =
            [(40_u8, "E grave"), (45, "A"), (50, "D"), (55, "G"), (59, "B"), (64, "E aiguë")];
        strings
            .into_iter()
            .filter_map(|(open, name)| {
                midi_note.checked_sub(open).filter(|fret| *fret <= 20).map(|fret| (name, fret))
            })
            .min_by_key(|(_, fret)| *fret)
    }

    pub(super) fn tablature_line(string: &str) -> usize {
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

    pub(super) fn draw_tablature_staff(
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

    pub(super) fn diatonic_staff_position(midi_note: u8) -> i32 {
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

    pub(super) fn draw_standard_staff(
        painter: &egui::Painter,
        rect: egui::Rect,
        note_markers: &[(f32, u8)],
    ) {
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

    pub(super) fn draw_measure_lines(
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

    pub(super) fn draw_live_tablature_panel(&self, ui: &mut Ui) {
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

    pub(super) fn draw_live_tablature(&mut self, ui: &mut Ui) {
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
}
