use super::*;

const SYNC_BIN_SECONDS: f32 = 0.02;
const SYNC_MAX_LAG_SECONDS: f32 = 5.0;

fn decode_onset_envelope(path: &Path) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Ouverture de {} impossible : {error}", path.display()))?;
    let decoder = rodio::Decoder::try_from(file)
        .map_err(|error| format!("Décodage de {} impossible : {error}", path.display()))?;
    let channels = decoder.channels().get() as usize;
    let sample_rate = decoder.sample_rate().get() as usize;
    if channels == 0 || sample_rate == 0 {
        return Err(format!("Format audio invalide pour {}.", path.display()));
    }
    let frames_per_bin = ((sample_rate as f32 * SYNC_BIN_SECONDS).round() as usize).max(1);
    let samples_per_bin = frames_per_bin * channels;
    let mut energy = Vec::new();
    let mut sum_squares = 0.0_f64;
    let mut sample_count = 0_usize;
    for sample in decoder {
        sum_squares += f64::from(sample) * f64::from(sample);
        sample_count += 1;
        if sample_count == samples_per_bin {
            energy.push((sum_squares / sample_count as f64).sqrt() as f32);
            sum_squares = 0.0;
            sample_count = 0;
        }
    }
    if sample_count > 0 {
        energy.push((sum_squares / sample_count as f64).sqrt() as f32);
    }
    if energy.len() < 10 {
        return Err(format!("Pas assez de signal exploitable dans {}.", path.display()));
    }

    let mut decibels = Vec::with_capacity(energy.len());
    for amplitude in energy {
        decibels.push(20.0 * amplitude.max(0.00001).log10());
    }
    let mut onset = vec![0.0; decibels.len()];
    for index in 1..decibels.len() {
        onset[index] = (decibels[index] - decibels[index - 1]).max(0.0);
    }
    Ok(onset)
}

fn correlation_at_lag(reference: &[f32], stem: &[f32], lag: isize) -> f32 {
    let stem_start = lag.max(0) as usize;
    let reference_start = (-lag).max(0) as usize;
    let length = (stem.len().saturating_sub(stem_start))
        .min(reference.len().saturating_sub(reference_start));
    if length < 20 {
        return -1.0;
    }

    let mut stem_sum = 0.0_f64;
    let mut reference_sum = 0.0_f64;
    for index in 0..length {
        stem_sum += f64::from(stem[stem_start + index]);
        reference_sum += f64::from(reference[reference_start + index]);
    }
    let stem_mean = stem_sum / length as f64;
    let reference_mean = reference_sum / length as f64;
    let mut dot = 0.0_f64;
    let mut stem_energy = 0.0_f64;
    let mut reference_energy = 0.0_f64;
    for index in 0..length {
        let stem_value = f64::from(stem[stem_start + index]) - stem_mean;
        let reference_value = f64::from(reference[reference_start + index]) - reference_mean;
        dot += stem_value * reference_value;
        stem_energy += stem_value * stem_value;
        reference_energy += reference_value * reference_value;
    }
    if stem_energy <= f64::EPSILON || reference_energy <= f64::EPSILON {
        -1.0
    } else {
        (dot / (stem_energy * reference_energy).sqrt()) as f32
    }
}

fn estimate_sync_offset(reference: &[f32], stem_path: &Path) -> Result<f32, String> {
    let stem = decode_onset_envelope(stem_path)?;
    let max_lag = (SYNC_MAX_LAG_SECONDS / SYNC_BIN_SECONDS) as isize;
    let mut best_score = -1.0_f32;
    let mut best_lag = 0_isize;
    for lag in -max_lag..=max_lag {
        let score = correlation_at_lag(reference, &stem, lag);
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }
    if best_score < 0.02 {
        return Err(format!(
            "Alignement incertain pour {} (corrélation {:.2}).",
            stem_path.display(),
            best_score
        ));
    }
    Ok(best_lag as f32 * SYNC_BIN_SECONDS)
}

impl PlayDesktop {
    pub(super) fn is_reference_path(path: Option<&Path>) -> bool {
        path.and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .map(|stem| {
                let stem = stem.to_ascii_lowercase();
                stem.contains("reference") || stem.contains("référence") || stem.contains("guide")
            })
            .unwrap_or(false)
    }

    pub(super) fn synchronize_backing_stems(&mut self) {
        if self.backing_sync_progress.is_some() {
            return;
        }
        let reference_index = self
            .backing_stems
            .iter()
            .position(|stem| stem.reference && stem.path.is_some())
            .or_else(|| {
                self.backing_stems.iter().position(|stem| {
                    stem.path.as_deref().is_some_and(|path| Self::is_reference_path(Some(path)))
                })
            });
        let Some(reference_index) = reference_index else {
            self.audio_error = Some(
                "Choisissez d'abord une piste Référence contenant la capture complète.".to_owned(),
            );
            return;
        };
        for stem in &mut self.backing_stems {
            stem.reference = false;
        }
        self.backing_stems[reference_index].reference = true;
        let Some(reference_path) = self.backing_stems[reference_index].path.clone() else {
            self.audio_error = Some("La piste Référence n'a pas de fichier audio.".to_owned());
            return;
        };
        for stem in &mut self.backing_stems {
            stem.reference = false;
        }
        self.backing_stems[reference_index].reference = true;
        let stem_paths: Vec<Option<PathBuf>> =
            self.backing_stems.iter().map(|stem| stem.path.clone()).collect();
        let total = stem_paths
            .iter()
            .enumerate()
            .filter(|(index, path)| *index != reference_index && path.is_some())
            .count();
        if total == 0 {
            self.audio_error =
                Some("Chargez au moins une piste à synchroniser avec la Référence.".to_owned());
            return;
        }
        let was_playing = self.workspace.transport == TransportState::Playing;
        if was_playing {
            self.stop_backing_playback();
        }
        self.backing_sync_resume_playing = was_playing;
        self.backing_sync_progress = Some((0, total));
        let sender = self.backing_sync_sender.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<Option<f32>>, String> {
                let reference_envelope = decode_onset_envelope(&reference_path)?;
                let mut offsets = vec![None; stem_paths.len()];
                offsets[reference_index] = Some(0.0);
                let mut completed = 0;
                for (index, path) in stem_paths.iter().enumerate() {
                    if index == reference_index {
                        continue;
                    }
                    let Some(path) = path.as_ref() else { continue };
                    let offset = estimate_sync_offset(&reference_envelope, path)?;
                    offsets[index] = Some(offset);
                    completed += 1;
                    let _ = sender.send(BackingSyncMessage::Progress { completed, total });
                }
                Ok(offsets)
            })();
            let _ = sender.send(BackingSyncMessage::Finished(result));
        });
    }

    pub(super) fn poll_backing_sync(&mut self) {
        while let Ok(message) = self.backing_sync_results.try_recv() {
            match message {
                BackingSyncMessage::Progress { completed, total } => {
                    if self.backing_sync_progress.is_some() {
                        self.backing_sync_progress = Some((completed, total));
                    }
                }
                BackingSyncMessage::Finished(result) => {
                    self.backing_sync_progress = None;
                    match result {
                        Ok(offsets) => {
                            let mut synchronized = 0;
                            for (stem, offset) in self.backing_stems.iter_mut().zip(offsets) {
                                if let Some(offset) = offset {
                                    stem.sync_offset_seconds = offset.clamp(-10.0, 10.0);
                                    synchronized += 1;
                                }
                            }
                            self.backing_project_dirty = true;
                            self.save_playlist();
                            self.audio_error = Some(format!(
                                "Synchronisation terminée : {synchronized} piste(s) alignée(s) sur Référence."
                            ));
                        }
                        Err(error) => {
                            self.audio_error = Some(error);
                        }
                    }
                    if self.backing_sync_resume_playing {
                        self.backing_sync_resume_playing = false;
                        self.start_backing_playback();
                    }
                }
            }
        }
    }
}
