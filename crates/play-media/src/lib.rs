//! Traitement média et mesure indépendant de l'interface.
//!
//! Les décodeurs et les entrées audio pourront être ajoutés ici sans polluer
//! l'application desktop. Le premier jalon porte sur l'analyse spectrale,
//! reprise de `measurement-library` de V1.

use thiserror::Error;

pub const SPECTRUM_BANDS: usize = 64;
pub const SPECTRUM_MIN_FREQUENCY: f32 = 16.0;
pub const SPECTRUM_MAX_FREQUENCY: f32 = 23_000.0;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("image invalide: {0}")]
    Image(#[from] image::ImageError),
    #[error("le signal ne contient aucun échantillon")]
    EmptySignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbaImageInfo {
    pub width: u32,
    pub height: u32,
}

pub fn inspect_image(bytes: &[u8]) -> Result<RgbaImageInfo, MediaError> {
    let image = image::load_from_memory(bytes)?.to_rgba8();
    Ok(RgbaImageInfo { width: image.width(), height: image.height() })
}

#[derive(Debug, Clone, Copy)]
pub struct SpectrumAnalyzer {
    sample_rate: f32,
}

impl SpectrumAnalyzer {
    pub const fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub const fn sample_rate(self) -> f32 {
        self.sample_rate
    }

    /// Retourne 64 niveaux d'amplitude normalisés répartis logarithmiquement
    /// entre 16 Hz et 23 kHz (ou la limite de Nyquist).
    pub fn analyze(&self, samples: &[f32]) -> Result<[f32; SPECTRUM_BANDS], MediaError> {
        let values = self.analyze_with_band_count(samples, SPECTRUM_BANDS)?;
        let mut bands = [0.0_f32; SPECTRUM_BANDS];
        bands.copy_from_slice(&values);
        Ok(bands)
    }

    /// Analyse le signal avec un nombre de bandes choisi par l'interface.
    ///
    /// Cette version sert notamment aux analyseurs haute résolution qui ont
    /// besoin de plus de bandes que l'affichage standard du lecteur.
    pub fn analyze_with_band_count(
        &self,
        samples: &[f32],
        band_count: usize,
    ) -> Result<Vec<f32>, MediaError> {
        if samples.is_empty() {
            return Err(MediaError::EmptySignal);
        }

        let band_count = band_count.max(1);
        let mut bands = vec![0.0_f32; band_count];
        let max_frequency = (self.sample_rate * 0.5).min(SPECTRUM_MAX_FREQUENCY);
        let min_frequency = SPECTRUM_MIN_FREQUENCY.min(max_frequency * 0.5);
        if !(max_frequency > min_frequency && self.sample_rate > 0.0) {
            return Ok(bands);
        }

        let fft_len = samples.len().next_power_of_two();
        let mut fft = vec![(0.0_f32, 0.0_f32); fft_len];
        let mean = samples.iter().copied().sum::<f32>() / samples.len() as f32;
        let window_denominator = (samples.len().saturating_sub(1)).max(1) as f32;
        for (index, sample) in samples.iter().enumerate() {
            let window =
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / window_denominator).cos();
            fft[index].0 = (*sample - mean) * window;
        }
        fft_in_place(&mut fft);

        let ratio = (max_frequency / min_frequency).powf(1.0 / band_count as f32);
        for (band, level) in bands.iter_mut().enumerate() {
            let lower = min_frequency * ratio.powf(band as f32);
            let upper = (min_frequency * ratio.powf(band as f32 + 1.0)).min(max_frequency);
            let center = (lower * upper).sqrt();
            let first_bin = (lower * fft_len as f32 / self.sample_rate).ceil() as usize;
            let last_bin = (upper * fft_len as f32 / self.sample_rate).floor() as usize;
            let mut peak_power = 0.0_f32;

            if last_bin <= first_bin {
                // Les premières bandes sont plus étroites que la résolution
                // FFT. Une réutilisation du bin voisin faisait bouger deux
                // bandes ensemble ; on mesure ici la fréquence centrale.
                peak_power = goertzel_windowed_power(samples, self.sample_rate, center, mean);
            } else {
                for bin in first_bin..=last_bin.min(fft_len / 2) {
                    peak_power = peak_power.max(fft[bin].0 * fft[bin].0 + fft[bin].1 * fft[bin].1);
                }
            }
            *level = (peak_power * 16.0 / fft_len as f32 / fft_len as f32).sqrt().clamp(0.0, 1.0);
        }
        Ok(bands)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchEstimate {
    pub frequency: f32,
    pub midi_note: u8,
    pub cents: f32,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordQuality {
    Major,
    Minor,
    Power,
    Sus2,
    Sus4,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordEstimate {
    pub root: u8,
    pub quality: ChordQuality,
    pub confidence: f32,
}

fn centered_samples(samples: &[f32]) -> Option<(f32, f32)> {
    if samples.is_empty() {
        return None;
    }
    let mean = samples.iter().copied().sum::<f32>() / samples.len() as f32;
    let energy = samples
        .iter()
        .map(|sample| {
            let centered = *sample - mean;
            centered * centered
        })
        .sum::<f32>();
    let rms = (energy / samples.len() as f32).sqrt();
    (rms > 0.003).then_some((mean, energy))
}

fn normalized_correlation(samples: &[f32], mean: f32, lag: usize) -> f32 {
    let count = samples.len().saturating_sub(lag);
    if count < 2 {
        return 0.0;
    }
    let mut product = 0.0;
    let mut left_energy = 0.0;
    let mut right_energy = 0.0;
    for index in 0..count {
        let left = samples[index] - mean;
        let right = samples[index + lag] - mean;
        product += left * right;
        left_energy += left * left;
        right_energy += right * right;
    }
    product / (left_energy * right_energy).sqrt().max(f32::EPSILON)
}

fn fft_in_place(values: &mut [(f32, f32)]) {
    let length = values.len();
    if length < 2 {
        return;
    }

    let mut reversed = 0;
    for index in 1..length {
        let mut bit = length >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            values.swap(index, reversed);
        }
    }

    let mut half_size = 1;
    while half_size < length {
        let angle = -std::f32::consts::PI / half_size as f32;
        let (step_real, step_imag) = (angle.cos(), angle.sin());
        let mut block_start = 0;
        while block_start < length {
            let mut factor = (1.0_f32, 0.0_f32);
            for offset in 0..half_size {
                let even_index = block_start + offset;
                let odd_index = even_index + half_size;
                let odd = (
                    values[odd_index].0 * factor.0 - values[odd_index].1 * factor.1,
                    values[odd_index].0 * factor.1 + values[odd_index].1 * factor.0,
                );
                let even = values[even_index];
                values[even_index] = (even.0 + odd.0, even.1 + odd.1);
                values[odd_index] = (even.0 - odd.0, even.1 - odd.1);
                factor = (
                    factor.0 * step_real - factor.1 * step_imag,
                    factor.0 * step_imag + factor.1 * step_real,
                );
            }
            block_start += half_size * 2;
        }
        half_size <<= 1;
    }
}

fn goertzel_windowed_power(samples: &[f32], sample_rate: f32, frequency: f32, mean: f32) -> f32 {
    let denominator = (samples.len().saturating_sub(1)).max(1) as f32;
    let omega = 2.0 * std::f32::consts::PI * frequency / sample_rate;
    let coefficient = 2.0 * omega.cos();
    let mut previous = 0.0;
    let mut previous_previous = 0.0;
    for (index, sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / denominator).cos();
        let current = (*sample - mean) * window + coefficient * previous - previous_previous;
        previous_previous = previous;
        previous = current;
    }
    previous_previous * previous_previous + previous * previous
        - coefficient * previous * previous_previous
}

/// Estimates the dominant fundamental frequency using normalized autocorrelation.
pub fn detect_pitch(samples: &[f32], sample_rate: f32) -> Option<PitchEstimate> {
    let (mean, _) = centered_samples(samples)?;
    if !(sample_rate > 0.0) || samples.len() < 512 {
        return None;
    }
    let min_lag = (sample_rate / 1_000.0).floor().max(2.0) as usize;
    let max_lag = (sample_rate / 55.0).ceil() as usize;
    let max_lag = max_lag.min(samples.len() / 2);
    if min_lag >= max_lag {
        return None;
    }

    let mut best_lag = min_lag;
    let mut best_correlation = 0.0;
    for lag in min_lag..=max_lag {
        let correlation = normalized_correlation(samples, mean, lag);
        if correlation > best_correlation {
            best_lag = lag;
            best_correlation = correlation;
        }
    }
    if best_correlation < 0.35 {
        return None;
    }

    let previous = normalized_correlation(samples, mean, best_lag.saturating_sub(1));
    let next = normalized_correlation(samples, mean, best_lag + 1);
    let denominator = 2.0 * (2.0 * best_correlation - previous - next);
    let correction =
        if denominator.abs() > f32::EPSILON { (previous - next) / denominator } else { 0.0 };
    let period = (best_lag as f32 + correction.clamp(-0.5, 0.5)).max(1.0);
    let frequency = sample_rate / period;
    if !(55.0..=1_000.0).contains(&frequency) {
        return None;
    }

    let midi_float = 69.0 + 12.0 * (frequency / 440.0).log2();
    let midi_note = midi_float.round().clamp(0.0, 127.0) as u8;
    let note_frequency = 440.0 * 2.0_f32.powf((midi_note as f32 - 69.0) / 12.0);
    Some(PitchEstimate {
        frequency,
        midi_note,
        cents: 1_200.0 * (frequency / note_frequency).log2(),
        confidence: best_correlation.clamp(0.0, 1.0),
    })
}

fn goertzel_power(samples: &[f32], sample_rate: f32, frequency: f32) -> f32 {
    let n = samples.len() as f32;
    let bin = (0.5 + n * frequency / sample_rate).floor();
    let omega = 2.0 * std::f32::consts::PI * bin / n;
    let coefficient = 2.0 * omega.cos();
    let mut previous = 0.0;
    let mut previous_previous = 0.0;
    for sample in samples {
        let current = *sample + coefficient * previous - previous_previous;
        previous_previous = previous;
        previous = current;
    }
    (previous_previous * previous_previous + previous * previous
        - coefficient * previous * previous_previous)
        / n.max(1.0)
}

/// Estimates a common guitar chord from the chroma energy of the microphone signal.
pub fn detect_chord(samples: &[f32], sample_rate: f32) -> Option<ChordEstimate> {
    centered_samples(samples)?;
    if !(sample_rate > 0.0) || samples.len() < 1_024 {
        return None;
    }
    let mut chroma = [0.0_f32; 12];
    for midi in 40..=84 {
        let fundamental = 440.0 * 2.0_f32.powf((midi as f32 - 69.0) / 12.0);
        let mut energy = 0.0;
        for (harmonic, weight) in [(1.0, 1.0), (2.0, 0.35), (3.0, 0.2)] {
            let frequency = fundamental * harmonic;
            if frequency < sample_rate * 0.48 {
                energy += goertzel_power(samples, sample_rate, frequency) * weight;
            }
        }
        chroma[midi as usize % 12] += energy.max(0.0);
    }
    let maximum = chroma.iter().copied().fold(0.0, f32::max);
    if maximum <= f32::EPSILON {
        return None;
    }
    for value in &mut chroma {
        *value /= maximum;
    }
    let active_notes = chroma.iter().filter(|value| **value > 0.28).count();
    if active_notes < 2 {
        return None;
    }

    let templates: &[(ChordQuality, &[usize])] = &[
        (ChordQuality::Major, &[0, 4, 7]),
        (ChordQuality::Minor, &[0, 3, 7]),
        (ChordQuality::Power, &[0, 7]),
        (ChordQuality::Sus2, &[0, 2, 7]),
        (ChordQuality::Sus4, &[0, 5, 7]),
    ];
    let mut best = None;
    for root in 0..12 {
        for (quality, intervals) in templates {
            let mut score = 0.0;
            let mut covered = [false; 12];
            for interval in *intervals {
                let index = (root + interval) % 12;
                score += chroma[index];
                covered[index] = true;
            }
            let outside = chroma
                .iter()
                .enumerate()
                .filter(|(index, _)| !covered[*index])
                .map(|(_, value)| *value)
                .sum::<f32>();
            score -= outside * 0.18;
            if best.is_none_or(|(_, _, best_score)| score > best_score) {
                best = Some((root as u8, *quality, score));
            }
        }
    }
    let (root, quality, score) = best?;
    Some(ChordEstimate { root, quality, confidence: (score / 3.0).clamp(0.0, 1.0) })
}

#[cfg(test)]
mod tests {
    use super::{
        SPECTRUM_BANDS, SPECTRUM_MAX_FREQUENCY, SPECTRUM_MIN_FREQUENCY, SpectrumAnalyzer,
        detect_chord, detect_pitch,
    };

    #[test]
    fn spectrum_has_64_bands() {
        let samples = (0..1024).map(|index| (index as f32 * 0.1).sin()).collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(48_000.0).analyze(&samples).unwrap();
        assert_eq!(bands.len(), SPECTRUM_BANDS);
        assert!(bands.iter().any(|value| *value > 0.0));
    }

    #[test]
    fn spectrum_follows_frequency_instead_of_time_position() {
        let sample_rate = 48_000.0;
        let frequency = 1_000.0;
        let samples = (0..4_096)
            .map(|index| {
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
            })
            .collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(48_000.0).analyze(&samples).unwrap();
        let peak_band = bands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        let expected_band = ((frequency / SPECTRUM_MIN_FREQUENCY).ln()
            / (SPECTRUM_MAX_FREQUENCY / SPECTRUM_MIN_FREQUENCY).ln()
            * SPECTRUM_BANDS as f32) as usize;
        assert!((peak_band as isize - expected_band as isize).abs() <= 2);
        assert!(bands[peak_band] > 0.5);
    }

    #[test]
    fn spectrum_resolves_low_frequency_content() {
        let sample_rate = 48_000.0;
        let frequency = 55.0;
        let samples = (0..4_096)
            .map(|index| {
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
            })
            .collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(sample_rate).analyze(&samples).unwrap();
        let peak_band = bands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        assert!(peak_band < 16);
        assert!(bands[peak_band] > 0.4);
    }

    #[test]
    fn spectrum_preserves_full_scale_amplitude() {
        let sample_rate = 48_000.0;
        let amplitude = 0.25;
        let frequency = 1_000.0;
        let samples = (0..4_096)
            .map(|index| {
                amplitude
                    * (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
            })
            .collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(sample_rate).analyze(&samples).unwrap();
        let peak = bands.iter().copied().fold(0.0, f32::max);
        assert!((peak - amplitude).abs() < 0.03, "expected {amplitude}, got {peak}");
    }

    #[test]
    fn spectrum_separates_low_frequency_content() {
        let sample_rate = 48_000.0;
        let make_signal = |frequency: f32| {
            (0..16_384)
                .map(|index| {
                    (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
                })
                .collect::<Vec<_>>()
        };
        let analyzer = SpectrumAnalyzer::new(sample_rate);
        let low_peak = analyzer
            .analyze(&make_signal(32.0))
            .unwrap()
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        let high_peak = analyzer
            .analyze(&make_signal(55.0))
            .unwrap()
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        assert!(high_peak >= low_peak + 3, "low bands collapsed: {low_peak} and {high_peak}");
    }

    #[test]
    fn spectrum_does_not_reuse_the_same_bin_for_low_bands() {
        let sample_rate = 48_000.0;
        let frequency = 18.0;
        let samples = (0..16_384)
            .map(|index| {
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
            })
            .collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(sample_rate).analyze(&samples).unwrap();
        let peak_band = bands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        assert_eq!(peak_band, 1, "18 Hz was assigned to band {peak_band}");
    }

    #[test]
    fn pitch_detector_finds_a440() {
        let sample_rate = 48_000.0;
        let samples = (0..4_096)
            .map(|index| (2.0 * std::f32::consts::PI * 440.0 * index as f32 / sample_rate).sin())
            .collect::<Vec<_>>();
        let pitch = detect_pitch(&samples, sample_rate).unwrap();
        assert!((pitch.frequency - 440.0).abs() < 3.0);
        assert_eq!(pitch.midi_note, 69);
    }

    #[test]
    fn chord_detector_finds_c_major_tones() {
        let sample_rate = 48_000.0;
        let frequencies = [261.63, 329.63, 392.0];
        let samples = (0..4_096)
            .map(|index| {
                frequencies
                    .iter()
                    .map(|frequency| {
                        (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate).sin()
                    })
                    .sum::<f32>()
                    / frequencies.len() as f32
            })
            .collect::<Vec<_>>();
        let chord = detect_chord(&samples, sample_rate).unwrap();
        assert_eq!(chord.root, 0);
    }
}
