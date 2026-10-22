//! Modèle indépendant de la plateforme pour Play.
//!
//! Cette crate reprend les contrats utiles des applications V1 : un document
//! de travail, des panneaux indépendants et un état de transport partagé.
//! Elle ne connaît ni egui, ni wgpu, ni les API audio du système.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceMode {
    Player,
    RoomAnalyzer,
    LiveTablature,
    UiLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub year: Option<u16>,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDocument {
    pub name: String,
    pub mode: WorkspaceMode,
    pub tracks: Vec<Track>,
}

impl Default for ProjectDocument {
    fn default() -> Self {
        Self { name: String::new(), mode: WorkspaceMode::Player, tracks: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpectrumSnapshot {
    pub bands: [f32; 32],
    pub peak: f32,
}

impl Default for SpectrumSnapshot {
    fn default() -> Self {
        Self { bands: [0.0; 32], peak: 0.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub document: ProjectDocument,
    pub transport: TransportState,
    pub spectrum: SpectrumSnapshot,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            document: ProjectDocument::default(),
            transport: TransportState::Stopped,
            spectrum: SpectrumSnapshot::default(),
        }
    }
}

impl Workspace {
    pub fn toggle_playback(&mut self) {
        self.transport = match self.transport {
            TransportState::Playing => TransportState::Paused,
            TransportState::Stopped | TransportState::Paused => TransportState::Playing,
        };
    }

    pub fn stop(&mut self) {
        self.transport = TransportState::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::{TransportState, Workspace};

    #[test]
    fn playback_toggle_is_stable() {
        let mut workspace = Workspace::default();
        workspace.toggle_playback();
        assert_eq!(workspace.transport, TransportState::Playing);
        workspace.toggle_playback();
        assert_eq!(workspace.transport, TransportState::Paused);
        workspace.stop();
        assert_eq!(workspace.transport, TransportState::Stopped);
    }
}
