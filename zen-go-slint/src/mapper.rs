use crate::models::{
    ChoiceSnapshot, GuiConnectionState, GuiPage, GuiSnapshot, HeaderSnapshot, MixerSnapshot,
    MixerStripSnapshot, OutputSnapshot, PreampSnapshot, ProfileSnapshot, RawPacketSnapshot,
    SettingsSnapshot,
};
use antelope_protocol::{
    ClockSource, MixerAssignment, MixerChannelState, MixerSurface, OutputMode, OutputState,
    PreampInputState, SampleRate,
};
use zen_go_tui::app::{AppState, RawPacketTab};

pub fn placeholder_header(page: GuiPage) -> HeaderSnapshot {
    HeaderSnapshot {
        active_page: page,
        connection: GuiConnectionState::Disconnected,
        status_label: "Waiting for Zen Go device".to_string(),
        sample_rate_label: "-- Hz".to_string(),
        clock_source_label: "--".to_string(),
        profile_label: "No profile".to_string(),
        stale: false,
    }
}

pub fn snapshot_from_app_state(state: &AppState, active_page: GuiPage) -> GuiSnapshot {
    let mut snapshot = GuiSnapshot::disconnected(active_page);

    snapshot.header = header_from_app_state(state, active_page);
    snapshot.outputs = state.output.states.iter().map(output_from_state).collect();
    snapshot.mixer = mixer_from_app_state(state);
    snapshot.preamps = preamps_from_app_state(state);
    snapshot.profiles = profiles_from_app_state(state);
    snapshot.raw = raw_from_app_state(state);
    snapshot.settings = settings_from_app_state(state);
    snapshot.sample_rate_choices = sample_rate_choices(state.device.status.sample_rate);
    snapshot.clock_source_choices = clock_source_choices(state.device.status.clock_source);
    snapshot.assignment_choices = assignment_choices(None);
    snapshot.notice = state.ui.last_message.clone();

    snapshot
}

fn header_from_app_state(state: &AppState, active_page: GuiPage) -> HeaderSnapshot {
    let connection = if state.device.connection.connected {
        GuiConnectionState::Connected
    } else {
        GuiConnectionState::Disconnected
    };

    HeaderSnapshot {
        active_page,
        connection,
        status_label: connection.label().to_string(),
        sample_rate_label: state
            .device
            .status
            .sample_rate
            .map(SampleRate::label)
            .unwrap_or_else(|| "-- Hz".to_string()),
        clock_source_label: state
            .device
            .status
            .clock_source
            .map(ClockSource::label)
            .unwrap_or("--")
            .to_string(),
        profile_label: selected_profile_name(state)
            .unwrap_or("No profile")
            .to_string(),
        stale: false,
    }
}

fn selected_profile_name(state: &AppState) -> Option<&str> {
    state
        .popup
        .profile_names
        .get(state.popup.selected_index)
        .map(String::as_str)
}

fn output_from_state(output: &OutputState) -> OutputSnapshot {
    OutputSnapshot {
        index: output.target.index() as usize,
        name: output.target.label().to_string(),
        level_step: output.volume,
        level_db: output.display_db(),
        level_ratio: output.gain_ratio() as f32,
        meter_ratio: 0.0,
        mode_label: output.mode.label().to_string(),
        muted: output.mode == OutputMode::Mute,
        dimmed: output.mode == OutputMode::Dim,
    }
}

fn mixer_from_app_state(state: &AppState) -> MixerSnapshot {
    let active_surface = MixerSurface::from_surface(state.mixer.surface);
    let strips = state.mixer.channels[active_surface.index()]
        .iter()
        .enumerate()
        .map(|(index, channel)| {
            mixer_strip_from_state(channel, index == state.mixer.selected_channel)
        })
        .collect();

    MixerSnapshot {
        active_surface_index: active_surface.index(),
        active_surface_label: match active_surface {
            MixerSurface::Mix1 => "Mix 1".to_string(),
            MixerSurface::Mix2 => "Mix 2".to_string(),
        },
        strips,
    }
}

fn mixer_strip_from_state(channel: &MixerChannelState, selected: bool) -> MixerStripSnapshot {
    let assignment = channel.assignment;
    MixerStripSnapshot {
        channel: channel.channel,
        name: format!("CH {:02}", channel.channel),
        assignment_label: assignment
            .map(MixerAssignment::label)
            .unwrap_or("Unassigned")
            .to_string(),
        assignment_short_label: assignment
            .map(MixerAssignment::short_label)
            .unwrap_or("--")
            .to_string(),
        level: channel.level.unwrap_or(96),
        level_ratio: channel.gain_ratio().unwrap_or(0.0) as f32,
        meter_ratio: channel.meter_ratio().unwrap_or(0.0) as f32,
        pan_raw: channel.pan.raw(),
        pan_ratio: channel.pan.ratio() as f32,
        pan_display: channel.pan.display_percent(),
        muted: channel.muted.unwrap_or(false),
        soloed: channel.soloed.unwrap_or(false),
        linked: channel.linked.unwrap_or(false),
        linkable: channel.channel % 2 == 1,
        selected,
    }
}

fn preamps_from_app_state(state: &AppState) -> Vec<PreampSnapshot> {
    [state.preamp.state.input1, state.preamp.state.input2]
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            preamp_from_state(
                input,
                (index + 1) as u8,
                index == state.preamp.selected_input,
            )
        })
        .collect()
}

fn preamp_from_state(input: PreampInputState, input_number: u8, selected: bool) -> PreampSnapshot {
    PreampSnapshot {
        input: input_number,
        name: format!("A{input_number}"),
        mode_label: input.mode.label().to_string(),
        gain_raw: input.gain_raw,
        gain_ratio: input.gain_ratio() as f32,
        meter_ratio: input.observed_meter_ratio().unwrap_or(0.0) as f32,
        phantom: input.phantom_on,
        phase_inverted: input.mode_raw & 0x40 != 0,
        selected,
    }
}

fn profiles_from_app_state(state: &AppState) -> Vec<ProfileSnapshot> {
    state
        .popup
        .profile_names
        .iter()
        .enumerate()
        .map(|(index, name)| ProfileSnapshot {
            index,
            name: name.clone(),
            selected: index == state.popup.selected_index,
        })
        .collect()
}

fn raw_from_app_state(state: &AppState) -> RawPacketSnapshot {
    RawPacketSnapshot {
        tab_label: raw_tab_label(state.raw_view.selected_tab).to_string(),
        summary: state.device.status.last_refresh_summary.clone(),
        rows: state.raw_view.recent_query_reply_log.clone(),
    }
}

fn raw_tab_label(tab: RawPacketTab) -> &'static str {
    match tab {
        RawPacketTab::Query74 => "Query 74",
        RawPacketTab::State73 => "State 73",
        RawPacketTab::Auxiliary => "Auxiliary",
        RawPacketTab::Query75 => "Query 75",
        RawPacketTab::DeviceNotification => "Notifications",
    }
}

fn settings_from_app_state(state: &AppState) -> SettingsSnapshot {
    SettingsSnapshot {
        refresh_rate_label: state.ui.settings.refresh_rate.label().to_string(),
        peak_enabled: state.ui.settings.peak_enabled,
        peak_threshold_label: format!("{} dB", state.ui.settings.peak_threshold_db()),
        peak_hold_label: state.ui.settings.peak_hold_duration.label().to_string(),
        auto_save: state.ui.settings.auto_save,
    }
}

fn sample_rate_choices(selected: Option<SampleRate>) -> Vec<ChoiceSnapshot> {
    SampleRate::all_confirmed()
        .iter()
        .enumerate()
        .map(|(index, rate)| ChoiceSnapshot {
            index,
            label: rate.label(),
            selected: selected == Some(*rate),
        })
        .collect()
}

fn clock_source_choices(selected: Option<ClockSource>) -> Vec<ChoiceSnapshot> {
    ClockSource::all_confirmed()
        .iter()
        .enumerate()
        .map(|(index, source)| ChoiceSnapshot {
            index,
            label: source.label().to_string(),
            selected: selected == Some(*source),
        })
        .collect()
}

fn assignment_choices(selected: Option<MixerAssignment>) -> Vec<ChoiceSnapshot> {
    MixerAssignment::grounded_choices()
        .iter()
        .enumerate()
        .map(|(index, assignment)| ChoiceSnapshot {
            index,
            label: assignment.label().to_string(),
            selected: selected == Some(*assignment),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::snapshot_from_app_state;
    use crate::models::{GuiConnectionState, GuiPage};
    use antelope_protocol::{
        ClockSource, MixerAssignment, MixerChannelState, OutputMode, OutputState, OutputTarget,
        PanState, PreampState, SampleRate, Surface,
    };
    use zen_go_tui::app::AppState;

    #[test]
    fn maps_header_and_outputs_from_app_state() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.device.status.sample_rate = Some(SampleRate::Hz48000);
        state.device.status.clock_source = Some(ClockSource::Internal);
        state.output.states[1] = OutputState::new(OutputTarget::Hp1, 12, OutputMode::Dim);
        state.output.selected = 1;
        state.ui.last_message = "Synced".to_string();

        let snapshot = snapshot_from_app_state(&state, GuiPage::Mixer);

        assert_eq!(snapshot.header.connection, GuiConnectionState::Connected);
        assert_eq!(snapshot.header.sample_rate_label, "48000 Hz");
        assert_eq!(snapshot.header.clock_source_label, "Internal");
        assert_eq!(snapshot.notice, "Synced");
        assert_eq!(snapshot.outputs[1].name, "HP1");
        assert_eq!(snapshot.outputs[1].level_step, 12);
        assert_eq!(snapshot.outputs[1].level_db, -12);
        assert!(snapshot.outputs[1].dimmed);
        assert!(!snapshot.outputs[1].muted);
    }

    #[test]
    fn maps_active_mixer_surface_and_strips() {
        let mut state = AppState::default();
        state.mixer.surface = Surface::Hp2;
        state.mixer.selected_channel = 2;
        state.mixer.channels[1][2] = MixerChannelState {
            channel: 3,
            level: Some(30),
            meter: Some(15),
            muted: Some(true),
            soloed: Some(false),
            pan: PanState::right(),
            assignment: Some(MixerAssignment::ComputerPlay(3)),
            linked: Some(true),
        };

        let snapshot = snapshot_from_app_state(&state, GuiPage::Mixer);
        let strip = &snapshot.mixer.strips[2];

        assert_eq!(snapshot.mixer.active_surface_index, 1);
        assert_eq!(snapshot.mixer.active_surface_label, "Mix 2");
        assert_eq!(strip.channel, 3);
        assert_eq!(strip.assignment_label, "Computer Play 3");
        assert_eq!(strip.assignment_short_label, "C3");
        assert_eq!(strip.level, 30);
        assert!(strip.muted);
        assert!(!strip.soloed);
        assert!(strip.linked);
        assert!(strip.selected);
        assert!(snapshot.mixer.strips[2].linkable);
        assert!(!snapshot.mixer.strips[1].linkable);
    }

    #[test]
    fn maps_preamps_and_settings() {
        let mut state = AppState::default();
        state.preamp.state = PreampState::from_cluster([0x10, 0x20, 0x10, 0x43]);
        state.preamp.selected_input = 1;
        state.ui.settings.auto_save = true;

        let snapshot = snapshot_from_app_state(&state, GuiPage::Settings);

        assert_eq!(snapshot.preamps[0].gain_raw, 0x10);
        assert_eq!(snapshot.preamps[1].gain_raw, 0x20);
        assert!(snapshot.preamps[0].phantom);
        assert!(snapshot.preamps[1].phase_inverted);
        assert!(snapshot.preamps[1].selected);
        assert!(snapshot.settings.auto_save);
        assert_eq!(snapshot.settings.refresh_rate_label, "30 FPS");
    }
}
