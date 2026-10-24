#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct SavedBpmEntry {
    path: String,
    #[serde(default)]
    bpm: Option<f32>,
    #[serde(default)]
    estimated: bool,
    #[serde(default)]
    analyzed: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct SavedPlaylist {
    paths: Vec<String>,
    #[serde(default)]
    bpm_cache: Vec<SavedBpmEntry>,
    #[serde(default)]
    column_order: Vec<TrackColumn>,
    #[serde(default)]
    visible_columns: Vec<TrackColumn>,
    #[serde(default)]
    layout_version: u32,
    #[serde(default)]
    selected_track: Option<usize>,
    #[serde(default)]
    position_seconds: f32,
    #[serde(default)]
    repeat_mode: Option<RepeatMode>,
    #[serde(default)]
    shuffle_enabled: bool,
    #[serde(default)]
    chain_mode: Option<ChainMode>,
    #[serde(default = "default_chain_transition_seconds")]
    chain_transition_seconds: f32,
    #[serde(default)]
    click_chain_enabled: bool,
    #[serde(default)]
    backing_stems: Vec<SavedBackingStem>,
    #[serde(default = "default_backing_bpm")]
    backing_bpm: f32,
    #[serde(default = "default_backing_bpm")]
    backing_original_bpm: f32,
    #[serde(default)]
    backing_position_seconds: f32,
    #[serde(default)]
    backing_project_path: Option<String>,
    #[serde(default)]
    backing_project_name: String,
}

fn default_chain_transition_seconds() -> f32 {
    30.0
}

fn default_backing_bpm() -> f32 {
    120.0
}

fn playlist_storage_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    })?;
    Some(base.join("play").join("playlist.ron"))
}

use super::*;

impl PlayDesktop {
    pub(super) fn restore_playlist(&mut self) {
        let Some(path) = playlist_storage_path() else { return };
        let Ok(contents) = std::fs::read_to_string(path) else { return };
        let Ok(saved) = ron::from_str::<SavedPlaylist>(&contents) else { return };
        let saved_selected_track = saved.selected_track;
        let saved_position_seconds = saved.position_seconds.max(0.0);
        let saved_repeat_mode = saved.repeat_mode;
        let saved_shuffle_enabled = saved.shuffle_enabled;
        let saved_chain_mode = saved.chain_mode;
        let saved_chain_transition_seconds = if saved.layout_version < 2 {
            default_chain_transition_seconds()
        } else if saved.chain_transition_seconds.is_finite() {
            saved.chain_transition_seconds.clamp(0.0, 30.0)
        } else {
            default_chain_transition_seconds()
        };
        if !saved.column_order.is_empty() {
            self.track_column_order = Self::normalize_column_order(saved.column_order);
        }
        if saved.layout_version > 0 && !saved.visible_columns.is_empty() {
            self.track_column_visible = [false; TRACK_COLUMN_COUNT];
            for column in saved.visible_columns {
                self.track_column_visible[column.index()] = true;
            }
            // La colonne BPM a été ajoutée après la première version du
            // réglage des colonnes : elle est activée lors de la migration.
            if saved.layout_version < 3 {
                self.track_column_visible[TrackColumn::Bpm.index()] = true;
            }
            if !self.track_column_visible.iter().any(|visible| *visible) {
                self.track_column_visible = TrackColumn::default_visibility();
            }
        }
        let restore_backing_stems = saved.backing_project_path.is_some()
            || saved.backing_stems.iter().any(|stem| stem.path.is_some());
        if restore_backing_stems {
            self.backing_stems = saved
                .backing_stems
                .into_iter()
                .map(|stem| BackingStem {
                    name: stem.name,
                    role: stem.role,
                    path: stem.path.map(PathBuf::from),
                    reference: stem.reference,
                    sync_offset_seconds: if stem.sync_offset_seconds.is_finite() {
                        stem.sync_offset_seconds.clamp(-10.0, 10.0)
                    } else {
                        0.0
                    },
                    volume: stem.volume.clamp(0.0, 1.0),
                    pan: stem.pan.clamp(-1.0, 1.0),
                    muted: stem.muted,
                    solo: stem.solo,
                })
                .collect();
        }
        self.backing_bpm = if saved.backing_bpm.is_finite() {
            saved.backing_bpm.clamp(30.0, 240.0)
        } else {
            default_backing_bpm()
        };
        self.backing_original_bpm = if saved.backing_original_bpm.is_finite() {
            saved.backing_original_bpm.clamp(30.0, 240.0)
        } else {
            default_backing_bpm()
        };
        self.backing_position_seconds = saved.backing_position_seconds.max(0.0);
        self.backing_project_path = saved.backing_project_path.map(PathBuf::from);
        self.backing_project_name = if saved.backing_project_name.trim().is_empty() {
            self.backing_project_path
                .as_ref()
                .and_then(|path| path.file_stem())
                .and_then(|stem| stem.to_str())
                .unwrap_or("Sans nom")
                .to_owned()
        } else {
            saved.backing_project_name
        };
        self.backing_project_dirty = self.backing_project_path.is_none()
            && self.backing_stems.iter().any(|stem| stem.path.is_some());
        let bpm_cache = saved.bpm_cache;
        for path in saved.paths {
            let cached_bpm = bpm_cache
                .iter()
                .find(|entry| entry.path == path)
                .and_then(|entry| entry.analyzed.then_some((entry.bpm, entry.estimated)));
            self.add_music_path_with_cached_bpm(PathBuf::from(path), cached_bpm);
        }
        self.repeat_mode = saved_repeat_mode;
        self.shuffle_enabled = saved_shuffle_enabled;
        self.chain_mode = saved_chain_mode;
        self.chain_transition_seconds = saved_chain_transition_seconds;
        self.click_chain_enabled = saved.click_chain_enabled;
        self.selected_track = saved_selected_track
            .filter(|index| *index < self.loaded_music.len())
            .or(self.selected_track);
        self.resume_position_seconds =
            if self.selected_track.is_some() { saved_position_seconds } else { 0.0 };
        self.last_saved_position_seconds = self.resume_position_seconds;
    }

    pub(super) fn normalize_column_order(order: Vec<TrackColumn>) -> Vec<TrackColumn> {
        let mut normalized = Vec::with_capacity(TrackColumn::default_order().len());
        for column in order.into_iter().chain(TrackColumn::default_order()) {
            if !normalized.contains(&column) {
                normalized.push(column);
            }
        }
        normalized
    }

    pub(super) fn save_playlist(&self) {
        let Some(path) = playlist_storage_path() else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let saved = SavedPlaylist {
            paths: self
                .loaded_music
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            bpm_cache: self
                .loaded_music
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let metadata =
                        self.track_metadata.get(index).and_then(|metadata| metadata.as_ref());
                    SavedBpmEntry {
                        path: path.to_string_lossy().into_owned(),
                        bpm: metadata.and_then(|metadata| metadata.bpm),
                        estimated: metadata.map(|metadata| metadata.bpm_estimated).unwrap_or(false),
                        analyzed: metadata.is_some(),
                    }
                })
                .collect(),
            column_order: self.track_column_order.clone(),
            visible_columns: self
                .track_column_order
                .iter()
                .copied()
                .filter(|column| self.track_column_visible[column.index()])
                .collect(),
            layout_version: 5,
            selected_track: self.selected_track,
            position_seconds: self
                .audio_player
                .as_ref()
                .map(|player| player.get_pos().as_secs_f32())
                .unwrap_or(self.resume_position_seconds)
                .max(0.0),
            repeat_mode: self.repeat_mode,
            shuffle_enabled: self.shuffle_enabled,
            chain_mode: self.chain_mode,
            chain_transition_seconds: self.chain_transition_seconds,
            click_chain_enabled: self.click_chain_enabled,
            backing_stems: self
                .backing_stems
                .iter()
                .map(|stem| SavedBackingStem {
                    name: stem.name.clone(),
                    role: stem.role.clone(),
                    path: stem.path.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    reference: stem.reference,
                    sync_offset_seconds: stem.sync_offset_seconds,
                    volume: stem.volume,
                    pan: stem.pan,
                    muted: stem.muted,
                    solo: stem.solo,
                })
                .collect(),
            backing_bpm: self.backing_bpm,
            backing_original_bpm: self.backing_original_bpm,
            backing_position_seconds: self.backing_playback_position_seconds(),
            backing_project_path: self
                .backing_project_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            backing_project_name: self.backing_project_name.clone(),
        };
        let Ok(contents) = ron::to_string(&saved) else { return };
        let _ = std::fs::write(path, contents);
    }
}
