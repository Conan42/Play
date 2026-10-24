use super::*;

const BPM_ANALYSIS_SECONDS: usize = 120;

pub(super) fn basic_audio_metadata(
    path: &Path,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
) -> AudioMetadata {
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_uppercase())
        .unwrap_or_else(|| "AUDIO".to_owned());
    AudioMetadata {
        format,
        title: None,
        artist: None,
        album: None,
        year: None,
        bit_depth: None,
        sample_rate,
        channels,
        duration_seconds,
        bitrate_kbps: None,
        bpm: None,
        bpm_estimated: false,
        genre: None,
    }
}

fn read_audio_metadata(
    path: &Path,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
) -> AudioMetadata {
    let mut metadata = basic_audio_metadata(path, sample_rate, channels, duration_seconds);

    if let Ok(tagged_file) = lofty::read_from_path(path) {
        let properties = tagged_file.properties();
        metadata.title =
            tagged_file.tags().iter().find_map(|tag| tag.title().map(|value| value.into_owned()));
        metadata.artist =
            tagged_file.tags().iter().find_map(|tag| tag.artist().map(|value| value.into_owned()));
        metadata.album =
            tagged_file.tags().iter().find_map(|tag| tag.album().map(|value| value.into_owned()));
        metadata.year = tagged_file.tags().iter().find_map(|tag| tag.date().map(|date| date.year));
        metadata.bit_depth = properties.bit_depth();
        metadata.bitrate_kbps = properties.audio_bitrate();
        metadata.bpm = tagged_file.tags().iter().find_map(|tag| {
            tag.get_string(lofty::tag::ItemKey::Bpm)
                .or_else(|| tag.get_string(lofty::tag::ItemKey::IntegerBpm))
                .and_then(|value| value.replace(',', ".").parse::<f32>().ok())
        });
        metadata.genre = tagged_file
            .tags()
            .iter()
            .find_map(|tag| tag.get_string(lofty::tag::ItemKey::Genre).map(str::to_owned));
    }

    metadata
}

fn estimate_bpm_from_decoder<I>(decoder: I, sample_rate: u32, channels: usize) -> Option<f32>
where
    I: Iterator<Item = f32>,
{
    if sample_rate == 0 || channels == 0 {
        return None;
    }

    let max_samples =
        (sample_rate as usize).saturating_mul(BPM_ANALYSIS_SECONDS).saturating_mul(channels);
    let mut mono_samples =
        Vec::with_capacity((sample_rate as usize).saturating_mul(BPM_ANALYSIS_SECONDS));
    let mut channel_sum = 0.0_f32;
    let mut channel_count = 0_usize;

    for sample in decoder.take(max_samples) {
        channel_sum += sample;
        channel_count += 1;
        if channel_count == channels {
            mono_samples.push(channel_sum / channels as f32);
            channel_sum = 0.0;
            channel_count = 0;
        }
    }

    estimate_bpm_from_samples(&mono_samples, sample_rate as f32)
}

fn load_track_metadata(path: &Path, cached_bpm: Option<(Option<f32>, bool)>) -> AudioMetadata {
    let mut sample_rate = 0;
    let mut channels = 0;
    let mut duration_seconds = 0.0;
    if let Ok(file) = std::fs::File::open(path)
        && let Ok(decoder) = rodio::Decoder::try_from(file)
    {
        sample_rate = decoder.sample_rate().get();
        channels = decoder.channels().get() as usize;
        duration_seconds =
            decoder.total_duration().map(|duration| duration.as_secs_f32()).unwrap_or(0.0);
    }
    let mut metadata = read_audio_metadata(path, sample_rate, channels, duration_seconds);
    if metadata.bpm.is_none() {
        if let Some((bpm, estimated)) = cached_bpm {
            metadata.bpm = bpm;
            metadata.bpm_estimated = estimated && bpm.is_some();
        } else if let Ok(file) = std::fs::File::open(path)
            && let Ok(decoder) = rodio::Decoder::try_from(file)
            && let Some(bpm) = estimate_bpm_from_decoder(decoder, sample_rate, channels)
        {
            metadata.bpm = Some(bpm);
            metadata.bpm_estimated = true;
        }
    }
    metadata
}

impl PlayDesktop {
    pub(super) fn queue_metadata_load(
        &mut self,
        index: usize,
        path: PathBuf,
        cached_bpm: Option<(Option<f32>, bool)>,
    ) {
        if let Some(queued) = self.track_metadata_queued.get_mut(index) {
            if *queued {
                return;
            }
            *queued = true;
        }
        if self.metadata_sender.is_none() {
            let (sender, receiver) = mpsc::channel();
            self.metadata_sender = Some(sender);
            self.metadata_loading = Some(receiver);
        }
        let Some(sender) = self.metadata_sender.as_ref().cloned() else {
            return;
        };
        self.metadata_pending += 1;
        std::thread::spawn(move || {
            let metadata = load_track_metadata(&path, cached_bpm);
            let _ = sender.send((path, metadata));
        });
    }

    pub(super) fn apply_track_metadata(&mut self, index: usize, metadata: AudioMetadata) {
        if let Some(slot) = self.track_metadata.get_mut(index) {
            *slot = Some(metadata.clone());
        }
        if let Some(queued) = self.track_metadata_queued.get_mut(index) {
            *queued = false;
        }
        if let Some(track) = self.workspace.document.tracks.get_mut(index) {
            track.duration_seconds = metadata.duration_seconds;
            if let Some(title) = &metadata.title {
                track.title = title.clone();
            }
            if let Some(artist) = &metadata.artist {
                track.artist = artist.clone();
            }
            if let Some(album) = &metadata.album {
                track.album = album.clone();
            }
            track.year = metadata.year;
        }
        if self.selected_track == Some(index) {
            self.audio_metadata = Some(metadata);
        }
        self.save_playlist();
    }

    pub(super) fn queue_missing_bpm_analysis(&mut self) {
        let pending = self
            .loaded_music
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                let missing_bpm = self
                    .track_metadata
                    .get(*index)
                    .and_then(|metadata| metadata.as_ref())
                    .is_none_or(|metadata| metadata.bpm.is_none());
                missing_bpm && !self.track_metadata_queued.get(*index).copied().unwrap_or(false)
            })
            .map(|(index, path)| (index, path.clone()))
            .collect::<Vec<_>>();
        for (index, path) in pending {
            self.queue_metadata_load(index, path, None);
        }
    }

    pub(super) fn poll_metadata_loading(&mut self) {
        let Some(receiver) = self.metadata_loading.take() else {
            return;
        };
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok((path, metadata)) => {
                    if let Some(index) = self.loaded_music.iter().position(|loaded| loaded == &path)
                    {
                        self.apply_track_metadata(index, metadata);
                    }
                    self.metadata_pending = self.metadata_pending.saturating_sub(1);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected || self.metadata_pending == 0 {
            self.metadata_sender = None;
            self.metadata_loading = None;
        } else {
            self.metadata_loading = Some(receiver);
        }
    }
}
