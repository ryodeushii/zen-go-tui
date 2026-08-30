slint::include_modules!();

use crate::models::{
    ChoiceSnapshot, GuiSnapshot, MixerStripSnapshot, OutputSnapshot, PreampSnapshot,
};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct AppViewModel {
    pub active_page: i32,
    pub status_label: SharedString,
    pub sample_rate_label: SharedString,
    pub clock_source_label: SharedString,
    pub profile_label: SharedString,
    pub mixer_label: SharedString,
    pub footer_notice: SharedString,
    pub controls_available: bool,
    pub outputs: Vec<OutputView>,
    pub mixer_strips: Vec<MixerStripView>,
    pub preamps: Vec<PreampView>,
    pub sample_rate_choices: Vec<ChoiceView>,
    pub clock_source_choices: Vec<ChoiceView>,
    pub assignment_choices: Vec<ChoiceView>,
    pub profiles: Vec<ChoiceView>,
    pub raw_tab_label: SharedString,
    pub raw_summary: SharedString,
    pub raw_rows: Vec<SharedString>,
    pub refresh_rate_label: SharedString,
    pub peak_threshold_label: SharedString,
    pub peak_hold_label: SharedString,
    pub peak_enabled: bool,
    pub auto_save: bool,
}

pub fn snapshot_to_view_model(snapshot: &GuiSnapshot) -> AppViewModel {
    AppViewModel {
        active_page: snapshot.header.active_page.gui_index().unwrap_or(0),
        status_label: snapshot.header.status_label.as_str().into(),
        sample_rate_label: snapshot.header.sample_rate_label.as_str().into(),
        clock_source_label: snapshot.header.clock_source_label.as_str().into(),
        profile_label: snapshot.header.profile_label.as_str().into(),
        mixer_label: snapshot.mixer.active_surface_label.as_str().into(),
        footer_notice: snapshot.notice.as_str().into(),
        controls_available: matches!(
            snapshot.header.connection,
            crate::models::GuiConnectionState::Connected
        ),
        outputs: snapshot.outputs.iter().map(output_to_view).collect(),
        mixer_strips: snapshot.mixer.strips.iter().map(strip_to_view).collect(),
        preamps: snapshot.preamps.iter().map(preamp_to_view).collect(),
        sample_rate_choices: snapshot
            .sample_rate_choices
            .iter()
            .map(choice_to_view)
            .collect(),
        clock_source_choices: snapshot
            .clock_source_choices
            .iter()
            .map(choice_to_view)
            .collect(),
        assignment_choices: snapshot
            .assignment_choices
            .iter()
            .map(choice_to_view)
            .collect(),
        profiles: snapshot
            .profiles
            .iter()
            .map(|profile| ChoiceView {
                index: profile.index as i32,
                label: profile.name.as_str().into(),
                selected: profile.selected,
            })
            .collect(),
        raw_tab_label: snapshot.raw.tab_label.as_str().into(),
        raw_summary: snapshot.raw.summary.as_str().into(),
        raw_rows: snapshot
            .raw
            .rows
            .iter()
            .map(|row| row.as_str().into())
            .collect(),
        refresh_rate_label: snapshot.settings.refresh_rate_label.as_str().into(),
        peak_threshold_label: snapshot.settings.peak_threshold_label.as_str().into(),
        peak_hold_label: snapshot.settings.peak_hold_label.as_str().into(),
        peak_enabled: snapshot.settings.peak_enabled,
        auto_save: snapshot.settings.auto_save,
    }
}

pub fn apply_snapshot(window: &AppWindow, snapshot: &GuiSnapshot) {
    let view_model = snapshot_to_view_model(snapshot);
    window.set_active_page(view_model.active_page);
    window.set_status_label(view_model.status_label);
    window.set_sample_rate_label(view_model.sample_rate_label);
    window.set_clock_source_label(view_model.clock_source_label);
    window.set_profile_label(view_model.profile_label);
    window.set_mixer_label(view_model.mixer_label);
    window.set_footer_notice(view_model.footer_notice);
    window.set_controls_available(view_model.controls_available);
    if !sync_model_rows(&window.get_outputs(), &view_model.outputs) {
        window.set_outputs(model_from_vec(view_model.outputs));
    }
    if !sync_model_rows(&window.get_mixer_strips(), &view_model.mixer_strips) {
        window.set_mixer_strips(model_from_vec(view_model.mixer_strips));
    }
    if !sync_model_rows(&window.get_preamps(), &view_model.preamps) {
        window.set_preamps(model_from_vec(view_model.preamps));
    }
    if !sync_model_rows(
        &window.get_sample_rate_choices(),
        &view_model.sample_rate_choices,
    ) {
        window.set_sample_rate_choices(model_from_vec(view_model.sample_rate_choices));
    }
    if !sync_model_rows(
        &window.get_clock_source_choices(),
        &view_model.clock_source_choices,
    ) {
        window.set_clock_source_choices(model_from_vec(view_model.clock_source_choices));
    }
    if !sync_model_rows(
        &window.get_assignment_choices(),
        &view_model.assignment_choices,
    ) {
        window.set_assignment_choices(model_from_vec(view_model.assignment_choices));
    }
    if !sync_model_rows(&window.get_profiles(), &view_model.profiles) {
        window.set_profiles(model_from_vec(view_model.profiles));
    }
    window.set_raw_tab_label(view_model.raw_tab_label);
    window.set_raw_summary(view_model.raw_summary);
    if !sync_model_rows(&window.get_raw_rows(), &view_model.raw_rows) {
        window.set_raw_rows(model_from_vec(view_model.raw_rows));
    }
    window.set_refresh_rate_label(view_model.refresh_rate_label);
    window.set_peak_threshold_label(view_model.peak_threshold_label);
    window.set_peak_hold_label(view_model.peak_hold_label);
    window.set_peak_enabled(view_model.peak_enabled);
    window.set_auto_save(view_model.auto_save);
}

fn output_to_view(output: &OutputSnapshot) -> OutputView {
    OutputView {
        index: output.index as i32,
        name: output.name.as_str().into(),
        level_step: output.level_step as i32,
        level_db: output.level_db as i32,
        level_percent: percent(output.level_ratio),
        muted: output.muted,
        dimmed: output.dimmed,
    }
}

fn strip_to_view(strip: &MixerStripSnapshot) -> MixerStripView {
    MixerStripView {
        channel: strip.channel as i32,
        name: strip.name.as_str().into(),
        assignment: strip.assignment_short_label.as_str().into(),
        level_step: strip.level as i32,
        level_db: -(strip.level as i32),
        meter_percent: percent(strip.meter_ratio),
        pan_raw: strip.pan_raw as i32,
        pan: strip.pan_display as i32,
        muted: strip.muted,
        soloed: strip.soloed,
        linked: strip.linked,
        linkable: strip.linkable,
        selected: strip.selected,
    }
}

fn preamp_to_view(preamp: &PreampSnapshot) -> PreampView {
    PreampView {
        input: preamp.input as i32,
        name: preamp.name.as_str().into(),
        mode: preamp.mode_label.as_str().into(),
        gain: preamp.gain_raw as i32,
        meter_percent: percent(preamp.meter_ratio),
        phantom: preamp.phantom,
        phase_inverted: preamp.phase_inverted,
        selected: preamp.selected,
    }
}

fn choice_to_view(choice: &ChoiceSnapshot) -> ChoiceView {
    ChoiceView {
        index: choice.index as i32,
        label: choice.label.as_str().into(),
        selected: choice.selected,
    }
}

fn model_from_vec<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(values)).into()
}

fn sync_model_rows<T: Clone + PartialEq + 'static>(model: &ModelRc<T>, values: &[T]) -> bool {
    let Some(model) = model.as_any().downcast_ref::<VecModel<T>>() else {
        return false;
    };
    if model.row_count() != values.len() {
        return false;
    }

    for (row, value) in values.iter().enumerate() {
        if model.row_data(row).as_ref() != Some(value) {
            model.set_row_data(row, value.clone());
        }
    }
    true
}

fn percent(ratio: f32) -> i32 {
    (ratio.clamp(0.0, 1.0) * 100.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::{snapshot_to_view_model, sync_model_rows};
    use crate::models::{GuiConnectionState, GuiPage, GuiSnapshot};
    use slint::{Model, ModelRc, VecModel};
    use std::rc::Rc;

    #[test]
    fn model_sync_updates_changed_rows_without_replacing_the_model() {
        let backing = Rc::new(VecModel::from(vec![1, 2, 3]));
        let model: ModelRc<i32> = backing.clone().into();

        assert!(sync_model_rows(&model, &[1, 4, 3]));
        assert_eq!(backing.row_data(0), Some(1));
        assert_eq!(backing.row_data(1), Some(4));
        assert_eq!(backing.row_data(2), Some(3));
        assert!(model
            .as_any()
            .downcast_ref::<VecModel<i32>>()
            .is_some_and(|current| std::ptr::eq(current, Rc::as_ptr(&backing))));
    }

    #[test]
    fn routing_snapshot_falls_back_to_mixer_for_gui_page() {
        let snapshot = GuiSnapshot::disconnected(GuiPage::Routing);
        assert_eq!(snapshot_to_view_model(&snapshot).active_page, 0);
    }

    #[test]
    fn bridge_marks_device_controls_unavailable_without_connected_snapshot() {
        let snapshot = GuiSnapshot::disconnected(GuiPage::Mixer);

        let view_model = snapshot_to_view_model(&snapshot);

        assert_eq!(view_model.active_page, 0);
        assert_eq!(view_model.status_label.as_str(), "Disconnected");
        assert!(!view_model.controls_available);
        assert_eq!(view_model.sample_rate_label.as_str(), "-- Hz");
        assert_eq!(view_model.outputs.len(), 3);
        assert_eq!(view_model.outputs[0].name.as_str(), "Monitor");
        assert_eq!(view_model.mixer_strips.len(), 16);
        assert!(view_model.mixer_strips[0].linkable);
        assert!(!view_model.mixer_strips[1].linkable);
        assert_eq!(view_model.preamps.len(), 2);
        assert_eq!(view_model.sample_rate_choices.len(), 7);
        assert_eq!(view_model.clock_source_choices.len(), 3);
        assert_eq!(view_model.assignment_choices.len(), 17);
        assert_eq!(view_model.raw_tab_label.as_str(), "State 73");
        assert_eq!(view_model.refresh_rate_label.as_str(), "100 ms");
        assert!(view_model.peak_enabled);
        assert!(view_model.auto_save);
        assert_eq!(
            view_model.footer_notice.as_str(),
            "Waiting for Zen Go device"
        );

        let mut connected = snapshot;
        connected.header.connection = GuiConnectionState::Connected;
        assert!(snapshot_to_view_model(&connected).controls_available);
    }
}
