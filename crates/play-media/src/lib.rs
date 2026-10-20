//! Traitement média et mesure indépendant de l'interface.
//!
//! Les décodeurs et les entrées audio pourront être ajoutés ici sans polluer
//! l'application desktop. Le premier jalon porte sur l'analyse spectrale,
//! reprise de `measurement-library` de V1.

use rustfft::{FftPlanner, num_complex::Complex};
use thiserror::Error;

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

    /// Retourne 32 niveaux normalisés en bandes logarithmiques simples.
    /// La conversion dB, le lissage et la calibration ECM8000 suivront le
    /// même découpage que `measurement-library` de V1.
    pub fn analyze(&self, samples: &[f32]) -> Result<[f32; 32], MediaError> {
        if samples.is_empty() {
            return Err(MediaError::EmptySignal);
        }

        // Les premières bandes logarithmiques sont très étroites. Avec une
        // FFT de 8192 points, elles peuvent ne contenir aucun bin lorsque la
        // piste est en 96 ou 192 kHz. Le zéro-padding garde environ 4 Hz de
        // résolution dans les basses, quelle que soit la fréquence du fichier.
        let resolution_fft_size = (self.sample_rate / 4.0).ceil() as usize;
        let fft_size =
            samples.len().next_power_of_two().max(resolution_fft_size.next_power_of_two()).max(64);
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let mut buffer = vec![Complex::new(0.0, 0.0); fft_size];
        for (index, sample) in samples.iter().copied().enumerate() {
            let window =
                0.5 - 0.5 * (2.0 * core::f32::consts::PI * index as f32 / fft_size as f32).cos();
            buffer[index].re = sample * window;
        }
        fft.process(&mut buffer);

        let mut bands = [0.0_f32; 32];
        let mut maximum = 0.0_f32;
        let min_frequency = 20.0_f32;
        let max_frequency = 20_000.0_f32;
        let min_log = min_frequency.log2();
        let log_span = max_frequency.log2() - min_log;
        for (index, value) in buffer.iter().take(fft_size / 2).enumerate() {
            let magnitude = value.norm() / fft_size as f32;
            let frequency = index as f32 * self.sample_rate / fft_size as f32;
            if frequency < min_frequency {
                continue;
            }
            let normalized = (frequency.min(max_frequency).log2() - min_log) / log_span;
            let band = (normalized * 32.0).floor().clamp(0.0, 31.0) as usize;
            bands[band] = bands[band].max(magnitude);
            maximum = maximum.max(magnitude);
        }
        if maximum > 0.0 {
            for band in &mut bands {
                *band = (*band / maximum).clamp(0.0, 1.0);
            }
        }
        Ok(bands)
    }
}

#[cfg(test)]
mod tests {
    use super::SpectrumAnalyzer;

    #[test]
    fn spectrum_has_32_bands() {
        let samples = (0..1024).map(|index| (index as f32 * 0.1).sin()).collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(48_000.0).analyze(&samples).unwrap();
        assert_eq!(bands.len(), 32);
        assert!(bands.iter().any(|value| *value > 0.0));
    }

    #[test]
    fn low_frequency_signal_stays_in_the_first_bands() {
        let sample_rate = 48_000.0;
        let samples = (0..8192)
            .map(|index| (2.0 * core::f32::consts::PI * 60.0 * index as f32 / sample_rate).sin())
            .collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(sample_rate).analyze(&samples).unwrap();
        let strongest_band = bands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        assert!(strongest_band < 8, "60 Hz should be in the low-frequency bands");
    }

    #[test]
    fn high_sample_rate_keeps_the_low_bands_present() {
        let mut samples = vec![0.0; 8192];
        samples[100] = 1.0;
        let bands = SpectrumAnalyzer::new(192_000.0).analyze(&samples).unwrap();
        assert!(bands[..8].iter().all(|value| *value > 0.0));
    }
}
