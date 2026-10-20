//! Contrats UI portables inspirés de `layout.library` et `gadtools.library`.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use play_core::WorkspaceMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelId {
    Browser,
    Main,
    Inspector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub mode: WorkspaceMode,
    pub visible_panels: Vec<PanelId>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            mode: WorkspaceMode::Player,
            visible_panels: vec![PanelId::Browser, PanelId::Main, PanelId::Inspector],
        }
    }
}
