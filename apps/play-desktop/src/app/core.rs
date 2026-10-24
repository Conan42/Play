use super::*;

impl PlayDesktop {
    fn new() -> Self {
        let egui_context = Context::default();
        setup_fonts(&egui_context);
        let toolbar_textures = ToolbarTextures {
            normal: load_toolbar_texture(
                &egui_context,
                "play_toolbar_button_normal",
                include_bytes!("../../../../assets/texture/bouton_normal.png"),
            ),
            pressed: load_toolbar_texture(
                &egui_context,
                "play_toolbar_button_pressed",
                include_bytes!("../../../../assets/texture/bouton_enfoncer.png"),
            ),
            menu_normal: load_toolbar_texture(
                &egui_context,
                "play_toolbar_menu_normal",
                include_bytes!("../../../../assets/texture/menu_normal.png"),
            ),
            menu_pressed: load_toolbar_texture(
                &egui_context,
                "play_toolbar_menu_pressed",
                include_bytes!("../../../../assets/texture/menu_enfoncer.png"),
            ),
        };
        let (backing_waveform_sender, backing_waveform_results) = mpsc::channel();
        let (backing_sync_sender, backing_sync_results) = mpsc::channel();

        let mut app = Self {
            window: None,
            egui_context,
            egui_state: None,
            painter: None,
            workspace: Workspace::default(),
            scene: Scene::default(),
            repeat_mode: None,
            shuffle_enabled: false,
            shuffle_state: 0x9e37_79b9_7f4a_7c15,
            chain_mode: None,
            chain_transition_seconds: 30.0,
            click_chain_enabled: false,
            show_music_loader: false,
            show_room_measurement_dialog: false,
            backing_stems: Vec::new(),
            backing_waveforms: Vec::new(),
            backing_durations: Vec::new(),
            backing_waveform_ready: Vec::new(),
            backing_waveform_sender,
            backing_waveform_results,
            backing_sync_sender,
            backing_sync_results,
            backing_sync_progress: None,
            backing_sync_resume_playing: false,
            backing_waveform_loading: Vec::new(),
            backing_bpm: 120.0,
            backing_original_bpm: 120.0,
            backing_position_seconds: 0.0,
            backing_player: None,
            backing_playback_origin_seconds: 0.0,
            backing_project_path: None,
            backing_project_name: "Sans nom".to_owned(),
            backing_project_dirty: false,
            pending_backing_action: None,
            backing_mix_settings: Vec::new(),
            backing_solo_active: Arc::new(AtomicBool::new(false)),
            backing_analysis_buffer: None,
            backing_levels: Arc::new(Mutex::new([0.0; 2])),
            loaded_music: Vec::new(),
            track_covers: Vec::new(),
            track_cover_checked: Vec::new(),
            track_metadata: Vec::new(),
            track_metadata_queued: Vec::new(),
            track_column_order: TrackColumn::default_order(),
            track_column_visible: TrackColumn::default_visibility(),
            selected_track: None,
            resume_position_seconds: 0.0,
            last_saved_position_seconds: 0.0,
            track_scroll_target: None,
            audio_metadata: None,
            metadata_loading: None,
            metadata_sender: None,
            metadata_pending: 0,
            audio_output: None,
            audio_output_name: None,
            audio_player: None,
            pink_noise_player: None,
            chain_previous_player: None,
            chain_transition: None,
            chain_transition_started: None,
            chain_transition_trigger_position: None,
            chain_tempo_start_speed: None,
            chain_tempo_target_speed: None,
            audio_error: None,
            stereo_faders: [FADER_DEFAULT_VALUE; 2],
            stereo_link_drag_start: None,
            stereo_link_pointer_start: None,
            channel_gains: Arc::new(Mutex::new([1.0; 2])),
            analysis_buffer: None,
            channel_levels: [0.0; 2],
            meter_display_levels: [0.0; 2],
            stereo_spectrum: [vec![0.0; SPECTRUM_BANDS], vec![0.0; SPECTRUM_BANDS]],
            room_spectrum: [0.0; ROOM_SPECTRUM_BANDS],
            microphone_buffer: None,
            microphone_stop: None,
            microphone_thread: None,
            microphone_devices: Vec::new(),
            microphone_device_index: 0,
            microphone_profile: MicrophoneProfile::BehringerEcm8000,
            live_analysis_source: LiveAnalysisSource::Microphone,
            microphone_input_peak: 0.0,
            microphone_error: None,
            microphone_monitor_enabled: true,
            microphone_mono_enabled: false,
            microphone_monitor_output: None,
            microphone_monitor_player: None,
            microphone_monitor_output_name: None,
            system_capture_stop: None,
            system_capture_thread: None,
            system_capture_buffer: None,
            system_capture_source_name: None,
            system_capture_path: None,
            system_capture_started_at: None,
            system_capture_error: None,
            microphone_pitch: None,
            microphone_chord: None,
            live_tab_strumming: false,
            metronome_enabled: false,
            metronome_bpm: 90.0,
            tablature_playing: true,
            tablature_recording: true,
            tablature_pause_started: None,
            tablature_notes: VecDeque::new(),
            tablature_last_position: None,
            tablature_last_recorded_at: None,
            tablature_measure_origin: None,
            tablature_display_mode: TablatureDisplayMode::Tablature,
            toolbar_textures,
        };
        app.restore_playlist();
        app
    }

    fn initialize_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Play")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 760.0))
                        .with_min_inner_size(winit::dpi::LogicalSize::new(900.0, 680.0)),
                )
                .expect("création de la fenêtre Play"),
        );
        let egui_state = State::new(
            self.egui_context.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let configuration = egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::from_display_handle(
                event_loop.owned_display_handle(),
            ),
            ..Default::default()
        };
        let mut painter = pollster::block_on(Painter::new(
            self.egui_context.clone(),
            configuration,
            false,
            RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
            .expect("initialisation du renderer wgpu");

        self.window = Some(window);
        self.egui_state = Some(egui_state);
        self.painter = Some(painter);
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let raw_input = {
            let Some(egui_state) = self.egui_state.as_mut() else { return };
            egui_state.take_egui_input(&window)
        };
        let context = self.egui_context.clone();
        let mut full_output = context.run_ui(raw_input, |root_ui| self.draw_ui(root_ui));
        if let Some(egui_state) = self.egui_state.as_mut() {
            egui_state.handle_platform_output(&window, full_output.platform_output);
        }
        let clipped_primitives =
            self.egui_context.tessellate(full_output.shapes, full_output.pixels_per_point);
        let pixels_per_point = full_output.pixels_per_point;
        let Some(painter) = self.painter.as_mut() else { return };
        painter.paint_and_update_textures(
            ViewportId::ROOT,
            pixels_per_point,
            [0.055, 0.065, 0.09, 1.0],
            &clipped_primitives,
            &mut full_output.textures_delta,
            Vec::new(),
            &window,
        );
    }
}

impl Drop for PlayDesktop {
    fn drop(&mut self) {
        self.stop_system_capture();
        self.stop_microphone();
    }
}

impl ApplicationHandler for PlayDesktop {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.initialize_window(event_loop);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else { return };
        if let Some(state) = self.egui_state.as_mut() {
            let _ = state.on_window_event(&window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.scene.mode == WorkspaceMode::BackingTrack && self.backing_project_dirty {
                    self.pending_backing_action = Some(PendingBackingAction::CloseApplication);
                    window.request_redraw();
                    return;
                }
                self.save_playlist();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(painter) = self.painter.as_mut()
                    && let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    painter.on_window_resized(ViewportId::ROOT, width, height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                window.request_redraw();
            }
            _ => {}
        }
    }
}

pub(crate) fn run() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    let mut app = PlayDesktop::new();
    event_loop.run_app(&mut app)
}
