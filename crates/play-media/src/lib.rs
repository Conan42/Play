//! Traitement média et mesure indépendant de l'interface.
//!
//! Les décodeurs et les entrées audio pourront être ajoutés ici sans polluer
//! l'application desktop. Le premier jalon porte sur l'analyse spectrale,
//! reprise de `measurement-library` de V1.

use thiserror::Error;

pub const SPECTRUM_BANDS: usize = 64;

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

    /// Retourne 64 niveaux d'amplitude normalisés.
    ///
    /// Comme l'analyseur de Player d'Exec Modernisé, chaque bande correspond à
    /// une portion temporelle du bloc audio. Ce sont donc 64 colonnes stables,
    /// adaptées à l'affichage stéréo superposé, et non des bins FFT.
    pub fn analyze(&self, samples: &[f32]) -> Result<[f32; SPECTRUM_BANDS], MediaError> {
        if samples.is_empty() {
            return Err(MediaError::EmptySignal);
        }

        let mut bands = [0.0_f32; SPECTRUM_BANDS];
        for (band, level) in bands.iter_mut().enumerate() {
            let start = band * samples.len() / SPECTRUM_BANDS;
            let end = ((band + 1) * samples.len() / SPECTRUM_BANDS).max(start + 1);
            let sum = samples[start..end].iter().map(|sample| sample.abs()).sum::<f32>();
            *level = (sum / (end - start) as f32).clamp(0.0, 1.0);
        }
        Ok(bands)
    }
}

#[cfg(test)]
mod tests {
    use super::{SPECTRUM_BANDS, SpectrumAnalyzer};

    #[test]
    fn spectrum_has_64_bands() {
        let samples = (0..1024).map(|index| (index as f32 * 0.1).sin()).collect::<Vec<_>>();
        let bands = SpectrumAnalyzer::new(48_000.0).analyze(&samples).unwrap();
        assert_eq!(bands.len(), SPECTRUM_BANDS);
        assert!(bands.iter().any(|value| *value > 0.0));
    }

    #[test]
    fn spectrum_uses_absolute_amplitude_per_time_slice() {
        let mut samples = vec![0.0; SPECTRUM_BANDS * 4];
        let midpoint = samples.len() / 2;
        for sample in &mut samples[..midpoint] {
            *sample = 0.5;
        }
        let bands = SpectrumAnalyzer::new(48_000.0).analyze(&samples).unwrap();
        assert!(bands[..SPECTRUM_BANDS / 2].iter().all(|value| (*value - 0.5).abs() < 0.001));
        assert!(bands[SPECTRUM_BANDS / 2..].iter().all(|value| *value == 0.0));
    }
}
