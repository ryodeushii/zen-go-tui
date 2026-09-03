use std::time::Instant;

use antelope_protocol::{
    Action, ClockSource, ControlValue, DeviceEvent, DeviceSnapshot, DeviceStateSnapshot,
    DynamicDeviceState, DynamicGlobalState, DynamicInputState, DynamicMixerStrip,
    DynamicMixerSurface, DynamicOutputState, DynamicRoutingGroup, DynamicStatePatch, GlobalControl,
    MixerAddress, MixerChannelState, MixerPassiveStripState, MixerSurface, OutputMode, OutputState,
    OutputTarget, PreampState, QueryResponse, RuntimeEntry, RuntimeProfile, SampleRate, Surface,
};

mod types;
pub use types::*;

mod state;
pub use state::*;

mod picker;
pub use picker::*;

mod profile_editor;
pub use profile_editor::*;

mod controller;
pub use controller::Controller;

#[cfg(test)]
mod dynamic_state_tests;
#[cfg(test)]
mod picker_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructuralSnapshot {
    sample_rate: SampleRate,
    sample_rate_hz: u32,
    clock_source: ClockSource,
    status_flags: [u8; 2],
    front_panel_bytes: [u8; 3],
    outputs: [OutputState; 3],
    dsp_cluster: [u8; 4],
    surface: Surface,
    mixer_surfaces: [[MixerPassiveStripState; 16]; 2],
}

impl StructuralSnapshot {
    fn from_snapshot(snapshot: &DeviceStateSnapshot) -> Self {
        Self {
            sample_rate: snapshot.sample_rate,
            sample_rate_hz: snapshot.sample_rate_hz,
            clock_source: snapshot.clock_source,
            status_flags: snapshot.status_flags,
            front_panel_bytes: snapshot.front_panel_bytes,
            outputs: snapshot.outputs,
            dsp_cluster: snapshot.dsp_cluster,
            surface: snapshot.surface,
            mixer_surfaces: snapshot.mixer_decode.surfaces,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub ui_profile: UiProfileState,
    pub device: DeviceState,
    pub mixer: MixerState,
    pub mixer_send_surfaces: Vec<u8>,
    pub output: OutputData,
    pub preamp: PreampData,
    pub input_spaces: Vec<InputSpaceState>,
    pub globals: Vec<DynamicGlobalState>,
    pub routing_capabilities: Vec<RoutingGroupCapability>,
    pub routing: Vec<DynamicRoutingGroup>,
    pub ui: UiState,
    pub popup: PopupState,
    pub raw_view: RawViewState,
    latest_structural_snapshot: Option<StructuralSnapshot>,
}

impl AppState {
    /// Allocate UI metadata and addressable storage from one catalog entry.
    pub fn from_entry(entry: &RuntimeEntry) -> Self {
        let mut state = Self::from_profile(&entry.profile);
        state.ui_profile = UiProfileState::from_entry(entry);
        state
    }

    /// Compatibility helper for geometry-only callers. It deliberately carries no readiness
    /// decision and therefore produces read-only controls.
    pub fn from_profile(profile: &RuntimeProfile) -> Self {
        let input_spaces: Vec<InputSpaceState> = profile
            .address_spaces
            .iter()
            .map(|space| {
                let mut inputs: Vec<_> = profile
                    .inputs
                    .iter()
                    .filter(|input| input.space_id == space.space_id)
                    .map(|input| DynamicInputState {
                        address: antelope_protocol::InputAddress {
                            space: input.space_id,
                            index: input.index,
                        },
                        name: input.name.clone(),
                        mode: None,
                        gain: None,
                        phantom: None,
                        phase: None,
                        meter: None,
                        parameters: Vec::new(),
                    })
                    .collect();
                inputs.sort_by_key(|input| input.address.index);
                InputSpaceState {
                    id: space.id.clone(),
                    space_id: space.space_id,
                    name: space.name.clone(),
                    inputs,
                }
            })
            .collect();
        let outputs: Vec<_> = profile
            .outputs
            .iter()
            .map(|output| DynamicOutputState {
                address: antelope_protocol::OutputAddress { id: output.id },
                name: if profile.identity.pid == 0xa015 {
                    match output.id {
                        0 => "Monitor".into(),
                        1 => "HP 1".into(),
                        2 => "HP 2".into(),
                        _ => output.name.clone(),
                    }
                } else {
                    output.name.clone()
                },
                level: None,
                muted: None,
                dimmed: None,
                parameters: Vec::new(),
            })
            .collect();
        let surfaces: Vec<_> = profile
            .mixers
            .iter()
            .map(|mixer| DynamicMixerSurface {
                surface: mixer.mix_index,
                name: mixer.name.clone(),
                master: mixer
                    .has_master
                    .then(|| Self::empty_dynamic_strip(0, "Master".into())),
                strips: (1..=mixer.strip_count)
                    .map(|strip| Self::empty_dynamic_strip(strip, format!("CH {strip:02}")))
                    .collect(),
            })
            .collect();
        let channels = surfaces
            .iter()
            .map(|surface| {
                surface
                    .strips
                    .iter()
                    .filter_map(Self::compatibility_channel)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let peaks = channels
            .iter()
            .map(|surface| vec![None; surface.len()])
            .collect();
        let compatibility_outputs = profile
            .outputs
            .iter()
            .take(3)
            .enumerate()
            .filter_map(|(index, _)| {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return None,
                };
                Some(OutputState::new(target, 0, OutputMode::Normal))
            })
            .collect();
        let physical_input_count = input_spaces
            .iter()
            .find(|space| space.space_id == 0)
            .map_or(0, |space| space.inputs.len());

        let globals = profile
            .params
            .iter()
            .filter(|parameter| parameter.applies_to == "globals")
            .filter_map(|parameter| {
                let control = match parameter.name.as_str() {
                    "sample_rate" => GlobalControl::SampleRate,
                    "clock_source" => GlobalControl::ClockSource,
                    "surface" => GlobalControl::Surface,
                    _ => GlobalControl::Parameter(parameter.id?),
                };
                Some(DynamicGlobalState {
                    control,
                    value: if parameter.value_type == "enum" {
                        ControlValue::Enum(0)
                    } else {
                        ControlValue::Int(0)
                    },
                })
            })
            .collect();

        Self {
            ui_profile: UiProfileState::compatibility(profile),
            input_spaces,
            globals,
            routing_capabilities: profile
                .routing_groups
                .iter()
                .map(|group| RoutingGroupCapability {
                    destination: group.destination,
                    name: group.name.clone(),
                    channel_count: group.channel_count,
                })
                .collect(),
            routing: Vec::new(),
            mixer_send_surfaces: profile
                .mixers
                .iter()
                .filter(|mixer| mixer.send_range.is_some())
                .map(|mixer| mixer.mix_index)
                .collect(),
            mixer: MixerState {
                surface: Surface::MonitorHp1,
                surface_index: 0,
                surfaces,
                channels,
                selected_channel: 0,
                strip_scroll: 0,
                visible_strip_count: MIXER_STRIP_PAGE_SIZE,
                peaks,
            },
            output: OutputData {
                dynamic: outputs,
                states: compatibility_outputs,
                selected: 0,
            },
            preamp: PreampData {
                state: PreampState::default(),
                selected_input: 0,
                peaks: vec![None; physical_input_count],
            },
            ..Self::default()
        }
    }

    fn empty_dynamic_strip(strip: u16, name: String) -> DynamicMixerStrip {
        DynamicMixerStrip {
            strip,
            name,
            fader: None,
            pan: None,
            send: None,
            muted: None,
            soloed: None,
            linked: None,
            meter: None,
            parameters: Vec::new(),
        }
    }

    fn compatibility_channel(strip: &DynamicMixerStrip) -> Option<MixerChannelState> {
        let channel = u8::try_from(strip.strip).ok()?;
        if channel == 0 {
            return None;
        }
        let mut state = MixerChannelState::unknown(channel);
        state.level = strip.fader.and_then(|value| u8::try_from(value).ok());
        state.pan = strip
            .pan
            .and_then(|value| u8::try_from(value).ok())
            .map(antelope_protocol::PanState::from_raw)
            .unwrap_or_default();
        state.muted = strip.muted;
        state.soloed = strip.soloed;
        state.linked = strip.linked;
        state.meter = strip.meter;
        Some(state)
    }

    pub fn inputs_for_space(&self, id: &str) -> &[DynamicInputState] {
        self.input_spaces
            .iter()
            .find(|space| space.id == id)
            .map_or(&[], |space| space.inputs.as_slice())
    }

    pub fn outputs(&self) -> &[DynamicOutputState] {
        &self.output.dynamic
    }

    pub fn mixers(&self) -> &[DynamicMixerSurface] {
        &self.mixer.surfaces
    }

    pub fn mixers_mut(&mut self) -> &mut [DynamicMixerSurface] {
        &mut self.mixer.surfaces
    }

    pub fn routing_group(&self, destination: u16) -> Option<&DynamicRoutingGroup> {
        self.routing
            .iter()
            .find(|group| group.destination == destination)
    }

    pub fn reconfigure_for_profile(&mut self, profile: &RuntimeProfile) {
        let selected_output = self.output.selected;
        let selected_input = self.preamp.selected_input;
        let selected_surface = self.mixer.surface_index;
        let selected_strip = self.mixer.selected_channel;
        let strip_scroll = self.mixer.strip_scroll;
        let visible = self.mixer.visible_strip_count;
        let mut configured = Self::from_profile(profile);
        configured.output.selected =
            selected_output.min(configured.outputs().len().saturating_sub(1));
        let input_count = configured
            .input_spaces
            .first()
            .map_or(0, |space| space.inputs.len());
        configured.preamp.selected_input = selected_input.min(input_count.saturating_sub(1));
        configured.mixer.surface_index =
            selected_surface.min(configured.mixers().len().saturating_sub(1));
        let strip_count = configured
            .mixers()
            .get(configured.mixer.surface_index)
            .map_or(0, |surface| surface.strips.len());
        configured.mixer.selected_channel = selected_strip.min(strip_count.saturating_sub(1));
        configured.mixer.strip_scroll = strip_scroll.min(strip_count.saturating_sub(1));
        configured.mixer.visible_strip_count = visible.max(1);
        configured.device = self.device.clone();
        configured.ui = self.ui.clone();
        configured.popup = self.popup.clone();
        configured.raw_view = self.raw_view.clone();
        *self = configured;
    }

    pub fn active_mixer_surface(&self) -> Option<usize> {
        (!self.mixer.surfaces.is_empty())
            .then(|| self.mixer.surface_index.min(self.mixer.surfaces.len() - 1))
    }

    pub(crate) fn active_legacy_mixer_surface(&self) -> MixerSurface {
        if self.mixer.surface_index == 0 {
            MixerSurface::Mix1
        } else {
            MixerSurface::Mix2
        }
    }

    pub fn visible_mixer_strip_bounds(&self) -> std::ops::Range<usize> {
        let Some(surface) = self.active_mixer_surface() else {
            return 0..0;
        };
        let Some(total) = self
            .mixer
            .surfaces
            .get(surface)
            .map(|surface| surface.strips.len())
        else {
            return 0..0;
        };
        if total == 0 {
            return 0..0;
        }
        let start = self.mixer.strip_scroll.min(total - 1);
        let end = start
            .saturating_add(self.mixer.visible_strip_count.max(1))
            .min(total);
        start..end
    }

    pub fn complete_mixer_action<F>(&self, address: MixerAddress, mutate: F) -> Option<Action>
    where
        F: FnOnce(&mut DynamicMixerStrip),
    {
        let surface = self
            .mixer
            .surfaces
            .iter()
            .find(|surface| surface.surface == address.surface)?;
        let mut strip = if address.strip == 0 {
            surface.master.as_ref()?.clone()
        } else {
            surface
                .strips
                .iter()
                .find(|strip| strip.strip == address.strip)?
                .clone()
        };
        mutate(&mut strip);
        Some(Action::SetMixerStripState {
            address,
            fader: strip.fader?,
            pan: strip.pan?,
            muted: strip.muted?,
            soloed: strip.soloed?,
            send: strip.send,
        })
    }

    pub fn apply_pending_mutation(&mut self, pending: PendingMutation) -> bool {
        match pending {
            PendingMutation::Mixer(strips) => {
                let mut changed = false;
                for pending in strips {
                    let Some(surface) = self
                        .mixer
                        .surfaces
                        .iter_mut()
                        .find(|surface| surface.surface == pending.address.surface)
                    else {
                        continue;
                    };
                    let slot = if pending.address.strip == 0 {
                        surface.master.as_mut()
                    } else {
                        surface
                            .strips
                            .iter_mut()
                            .find(|strip| strip.strip == pending.address.strip)
                    };
                    if let Some(slot) = slot {
                        *slot = pending.strip;
                        changed = true;
                    }
                }
                if changed {
                    self.sync_compatibility_views();
                }
                changed
            }
            PendingMutation::Output(output) => {
                let exists = self
                    .output
                    .dynamic
                    .iter()
                    .any(|slot| slot.address == output.address);
                if exists {
                    self.apply_output_patch(vec![output]);
                }
                exists
            }
            PendingMutation::Input(input) => {
                let exists = self.input_spaces.iter().any(|space| {
                    space.space_id == input.address.space
                        && space
                            .inputs
                            .iter()
                            .any(|slot| slot.address == input.address)
                });
                if exists {
                    self.apply_input_patch(vec![input]);
                }
                exists
            }
            PendingMutation::Routing(group) => {
                let Some(slot) = self
                    .routing
                    .iter_mut()
                    .find(|slot| slot.destination == group.destination)
                else {
                    return false;
                };
                if slot.sources.len() != group.sources.len() {
                    return false;
                }
                *slot = group;
                true
            }
            #[cfg(test)]
            PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan,
                muted,
            } => self
                .mixer
                .channels
                .get_mut(mixer.index())
                .and_then(|surface| {
                    channel
                        .checked_sub(1)
                        .and_then(|index| surface.get_mut(usize::from(index)))
                })
                .is_some_and(|slot| {
                    slot.level = Some(level);
                    slot.pan = pan;
                    slot.muted = Some(muted);
                    true
                }),
            #[cfg(test)]
            PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            } => self
                .mixer
                .channels
                .get_mut(mixer.index())
                .and_then(|surface| {
                    channel
                        .checked_sub(1)
                        .and_then(|index| surface.get_mut(usize::from(index)))
                })
                .is_some_and(|slot| {
                    slot.muted = Some(muted);
                    true
                }),
            #[cfg(test)]
            PendingMutation::MixerPan {
                mixer,
                channel,
                pan,
            } => self
                .mixer
                .channels
                .get_mut(mixer.index())
                .and_then(|surface| {
                    channel
                        .checked_sub(1)
                        .and_then(|index| surface.get_mut(usize::from(index)))
                })
                .is_some_and(|slot| {
                    slot.pan = pan;
                    true
                }),
            #[cfg(test)]
            PendingMutation::MixerAssignment { strip, assignment } => {
                strip.checked_sub(1).is_some_and(|index| {
                    let mut changed = false;
                    for surface in &mut self.mixer.channels {
                        if let Some(slot) = surface.get_mut(usize::from(index)) {
                            slot.assignment = Some(assignment);
                            changed = true;
                        }
                    }
                    changed
                })
            }
            #[cfg(test)]
            PendingMutation::MixerLink {
                mixer,
                selector,
                enabled,
            } => antelope_protocol::MixerLinkTarget::from_selector(mixer, selector).is_some_and(
                |target| {
                    let (left, right) = (target.left_channel, target.right_channel);
                    let Some(surface) = self.mixer.channels.get_mut(mixer.index()) else {
                        return false;
                    };
                    let mut changed = false;
                    for channel in [left, right] {
                        if let Some(slot) = channel
                            .checked_sub(1)
                            .and_then(|index| surface.get_mut(usize::from(index)))
                        {
                            slot.linked = Some(enabled);
                            changed = true;
                        }
                    }
                    changed
                },
            ),
            #[cfg(test)]
            PendingMutation::OutputVolume { target, step } => self
                .output
                .states
                .get_mut(usize::from(target.index()))
                .is_some_and(|slot| {
                    slot.volume = step;
                    true
                }),
            #[cfg(test)]
            PendingMutation::OutputMode { target, mode } => self
                .output
                .states
                .get_mut(usize::from(target.index()))
                .is_some_and(|slot| {
                    slot.mode = mode;
                    true
                }),
            #[cfg(test)]
            PendingMutation::PreampGain { input, raw } => {
                let changed = self
                    .device
                    .dsp_cluster
                    .get_mut(usize::from(input))
                    .is_some_and(|slot| {
                        *slot = raw;
                        true
                    });
                if changed {
                    self.refresh_preamp_from_cluster_preserving_observed_meter();
                }
                changed
            }
            #[cfg(test)]
            PendingMutation::PreampMode { input, mode } => {
                let changed = input
                    .checked_add(2)
                    .and_then(|offset| self.device.dsp_cluster.get_mut(usize::from(offset)))
                    .is_some_and(|slot| {
                        *slot = (*slot & 0xf0) | mode.code();
                        true
                    });
                if changed {
                    self.refresh_preamp_from_cluster_preserving_observed_meter();
                }
                changed
            }
            #[cfg(test)]
            PendingMutation::PreampPhantom { input, enabled } => {
                let changed = input
                    .checked_add(2)
                    .and_then(|offset| self.device.dsp_cluster.get_mut(usize::from(offset)))
                    .is_some_and(|slot| {
                        *slot = (*slot & 0x0f) | if enabled { 0x10 } else { 0 };
                        true
                    });
                if changed {
                    self.refresh_preamp_from_cluster_preserving_observed_meter();
                }
                changed
            }
            #[cfg(test)]
            PendingMutation::PreampPhase { input, enabled } => {
                let changed = input
                    .checked_add(2)
                    .and_then(|offset| self.device.dsp_cluster.get_mut(usize::from(offset)))
                    .is_some_and(|slot| {
                        *slot = (*slot & 0x1f) | if enabled { 0x40 } else { 0 };
                        true
                    });
                if changed {
                    self.refresh_preamp_from_cluster_preserving_observed_meter();
                }
                changed
            }
        }
    }

    pub fn prune_expired_peaks(&mut self) {
        let hold = self.ui.settings.peak_hold_duration.duration();
        for surface in &mut self.mixer.peaks {
            for peak in surface {
                if peak.is_some_and(|peak| peak.detected_at.elapsed() >= hold) {
                    *peak = None;
                }
            }
        }
        for peak in &mut self.preamp.peaks {
            if peak.is_some_and(|peak| peak.detected_at.elapsed() >= hold) {
                *peak = None;
            }
        }
    }

    pub fn startup_query_summary(&self, query_id: u8) -> Option<&str> {
        startup_query_slot(query_id)
            .and_then(|index| self.device.status.startup_query_summaries[index].as_deref())
    }

    pub fn selected_query_reply_entry(&self) -> Option<&QueryReplyLogEntry> {
        self.raw_view
            .selected_query_reply_entry
            .and_then(|index| self.raw_view.recent_query_reply_entries.get(index))
    }

    pub fn active_mixer_channels(&self) -> &[MixerChannelState] {
        self.active_mixer_surface()
            .and_then(|index| self.mixer.channels.get(index))
            .map_or(&[], Vec::as_slice)
    }

    pub fn clamp_mixer_strip_scroll(&mut self, visible_count: usize) {
        let visible_count = visible_count.max(1);
        let total = self.active_mixer_channels().len();
        let max_scroll = total.saturating_sub(visible_count);
        self.mixer.strip_scroll = self.mixer.strip_scroll.min(max_scroll);
    }

    pub fn ensure_selected_mixer_channel_visible(&mut self, visible_count: usize) {
        let visible_count = visible_count.max(1);
        self.clamp_mixer_strip_scroll(visible_count);

        if self.mixer.selected_channel < self.mixer.strip_scroll {
            self.mixer.strip_scroll = self.mixer.selected_channel;
        } else if self.mixer.selected_channel >= self.mixer.strip_scroll + visible_count {
            self.mixer.strip_scroll = self.mixer.selected_channel + 1 - visible_count;
        }

        self.clamp_mixer_strip_scroll(visible_count);
    }

    pub fn scroll_mixer_strip_viewport(&mut self, delta: isize, visible_count: usize) {
        let visible_count = visible_count.max(1);
        let total = self.active_mixer_channels().len();
        let max_scroll = total.saturating_sub(visible_count);

        self.mixer.strip_scroll = if delta >= 0 {
            self.mixer
                .strip_scroll
                .saturating_add(delta as usize)
                .min(max_scroll)
        } else {
            self.mixer
                .strip_scroll
                .saturating_sub(delta.saturating_abs() as usize)
        };
    }

    pub fn page_mixer_strip_viewport(&mut self, right: bool, page_size: usize) {
        let total = self.active_mixer_channels().len();
        let page_size = page_size.max(1);
        let max_page_start =
            total.saturating_sub(1).checked_div(page_size).unwrap_or(0) * page_size;

        self.mixer.strip_scroll = if right {
            self.mixer
                .strip_scroll
                .saturating_add(page_size)
                .min(max_page_start)
        } else {
            self.mixer.strip_scroll.saturating_sub(page_size)
        };

        if self.mixer.selected_channel < self.mixer.strip_scroll
            || self.mixer.selected_channel >= self.mixer.strip_scroll + page_size
        {
            self.mixer.selected_channel = self.mixer.strip_scroll.min(total.saturating_sub(1));
        }
    }

    fn snapshot_structurally_differs(&self, snapshot: &DeviceStateSnapshot) -> bool {
        let Some(prev) = &self.latest_structural_snapshot else {
            return true;
        };
        prev.sample_rate != snapshot.sample_rate
            || prev.sample_rate_hz != snapshot.sample_rate_hz
            || prev.clock_source != snapshot.clock_source
            || prev.status_flags != snapshot.status_flags
            || prev.front_panel_bytes != snapshot.front_panel_bytes
            || prev.outputs != snapshot.outputs
            || prev.dsp_cluster != snapshot.dsp_cluster
            || prev.surface != snapshot.surface
            || Self::mixer_surfaces_structurally_differ(
                &prev.mixer_surfaces,
                &snapshot.mixer_decode.surfaces,
            )
    }

    /// Compares passive mixer state after the dynamic driver model normalizes missing pan to center.
    fn mixer_surfaces_structurally_differ(
        previous: &[[MixerPassiveStripState; 16]; 2],
        current: &[[MixerPassiveStripState; 16]; 2],
    ) -> bool {
        previous
            .iter()
            .zip(current.iter())
            .any(|(previous_surface, current_surface)| {
                previous_surface.iter().zip(current_surface.iter()).any(
                    |(previous_strip, current_strip)| {
                        previous_strip.meter != current_strip.meter
                            || previous_strip.muted != current_strip.muted
                            || previous_strip.pan.unwrap_or_default()
                                != current_strip.pan.unwrap_or_default()
                            || previous_strip.linked != current_strip.linked
                    },
                )
            })
    }

    fn apply_meters_only(&mut self, snapshot: &DeviceStateSnapshot) {
        let mixer = self.active_legacy_mixer_surface();
        let mut meter_updates: Vec<(usize, usize, u8)> = Vec::new();
        for channel in 1..=16 {
            let Some(decoded) = snapshot.mixer_decode.strip(mixer, channel) else {
                continue;
            };
            let Some(slot) = self.state_slot_mut(mixer, channel) else {
                continue;
            };
            if let Some(meter) = decoded.meter {
                slot.meter = Some(meter);
                meter_updates.push((mixer.index(), channel as usize - 1, meter));
            }
        }
        for (mix_idx, ch_idx, meter) in meter_updates {
            self.track_mixer_peak(mix_idx, ch_idx, meter);
        }
        if let Some(meter) = snapshot.mixer_decode.observed_preamp1_meter {
            self.track_preamp_peak(0, meter);
        }
        self.preamp.state.input1.observed_meter = snapshot.mixer_decode.observed_preamp1_meter;
        if let Some(meter) = snapshot.mixer_decode.observed_preamp2_meter {
            self.track_preamp_peak(1, meter);
        }
        self.preamp.state.input2.observed_meter = snapshot.mixer_decode.observed_preamp2_meter;
    }

    fn track_peak(slot: &mut Option<MeterPeak>, meter: u8, threshold: u8, enabled: bool) {
        if !enabled || meter > threshold {
            return;
        }
        match slot {
            Some(peak) if meter < peak.raw => {
                *slot = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            None => {
                *slot = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            _ => {}
        }
    }

    fn track_mixer_peak(&mut self, mix_idx: usize, channel_idx: usize, meter: u8) {
        Self::track_peak(
            &mut self.mixer.peaks[mix_idx][channel_idx],
            meter,
            self.ui.settings.peak_threshold_raw,
            self.ui.settings.peak_enabled,
        );
    }

    fn track_preamp_peak(&mut self, input_idx: usize, meter: u8) {
        Self::track_peak(
            &mut self.preamp.peaks[input_idx],
            meter,
            self.ui.settings.peak_threshold_raw,
            self.ui.settings.peak_enabled,
        );
    }

    pub fn apply_snapshot(&mut self, snapshot: &DeviceStateSnapshot) {
        self.device.status.sample_rate = Some(snapshot.sample_rate);
        self.device.status.sample_rate_hz = Some(snapshot.sample_rate_hz);
        self.device.status.clock_source = Some(snapshot.clock_source);
        self.device.status.last_refresh_summary = format!(
            "snapshot {} / {} / surface {}",
            snapshot.sample_rate.label(),
            snapshot.clock_source.label(),
            snapshot.surface.label()
        );
        self.output.states = snapshot.outputs.to_vec();
        self.device.dsp_cluster = snapshot.dsp_cluster;
        self.preamp.state = PreampState::from_cluster(snapshot.dsp_cluster);
        if let Some(meter) = snapshot.mixer_decode.observed_preamp1_meter {
            self.track_preamp_peak(0, meter);
        }
        self.preamp.state.input1.observed_meter = snapshot.mixer_decode.observed_preamp1_meter;
        if let Some(meter) = snapshot.mixer_decode.observed_preamp2_meter {
            self.track_preamp_peak(1, meter);
        }
        self.preamp.state.input2.observed_meter = snapshot.mixer_decode.observed_preamp2_meter;
        self.mixer.surface = snapshot.surface;
        self.apply_passive_mixer_decode(snapshot);
    }

    fn apply_passive_mixer_decode(&mut self, snapshot: &DeviceStateSnapshot) {
        let mut meter_updates: Vec<(usize, usize, u8)> = Vec::new();
        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            for channel in 1..=16 {
                let Some(decoded) = snapshot.mixer_decode.strip(mixer, channel) else {
                    continue;
                };
                let Some(slot) = self.state_slot_mut(mixer, channel) else {
                    continue;
                };

                if let Some(meter) = decoded.meter {
                    slot.meter = Some(meter);
                    meter_updates.push((mixer.index(), channel as usize - 1, meter));
                }
                if let Some(muted) = decoded.muted {
                    slot.muted = Some(muted);
                }
                if let Some(linked) = decoded.linked {
                    slot.linked = Some(linked);
                }
            }
        }
        for (mix_idx, ch_idx, meter) in meter_updates {
            self.track_mixer_peak(mix_idx, ch_idx, meter);
        }
    }

    fn state_slot_mut(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
    ) -> Option<&mut MixerChannelState> {
        self.mixer.channels[mixer.index()].get_mut(channel.checked_sub(1)? as usize)
    }

    pub fn refresh_preamp_from_cluster_preserving_observed_meter(&mut self) {
        let observed_meter_input1 = self.preamp.state.input1.observed_meter;
        let observed_meter_input2 = self.preamp.state.input2.observed_meter;
        self.preamp.state = PreampState::from_cluster(self.device.dsp_cluster);
        self.preamp.state.input1.observed_meter = observed_meter_input1;
        self.preamp.state.input2.observed_meter = observed_meter_input2;
    }

    /// Apply a complete normalized snapshot without fixed-report conversion.
    pub fn apply_dynamic_state(&mut self, state: DynamicDeviceState, raw: Vec<u8>) -> bool {
        let was_connected = self.device.connection.connected;
        let compatibility_changed = state
            .zen_go_compatibility
            .as_deref()
            .map(|compatibility| self.zen_go_dynamic_structurally_differs(compatibility));
        let raw_changed =
            self.popup.raw_view_open && self.raw_view.latest_raw_73.as_ref() != Some(&raw);
        let changed = compatibility_changed.map_or_else(
            || {
                !was_connected
                    || self.globals != state.globals
                    || self.output.dynamic != state.outputs
                    || self.mixer.surfaces != state.mixers
                    || self.routing != state.routing
                    || raw_changed
                    || self
                        .input_spaces
                        .iter()
                        .flat_map(|space| &space.inputs)
                        .ne(state.inputs.iter())
            },
            |structural_changed| !was_connected || structural_changed || raw_changed,
        );
        self.device.connection.connected = true;
        self.device.connection.last_snapshot_at = Some(Instant::now());
        self.device.connection.last_frame_type = Some("0x73 snapshot");
        self.globals = state.globals;
        self.apply_input_patch(state.inputs);
        self.apply_output_patch(state.outputs);
        self.apply_mixer_snapshot(state.mixers);
        self.apply_routing_snapshot(state.routing);
        self.apply_dynamic_globals_to_status();
        self.raw_view.latest_raw_73 = Some(raw);
        changed
    }

    fn zen_go_dynamic_structurally_differs(
        &self,
        state: &antelope_protocol::driver::ZenGoCompatibilityState,
    ) -> bool {
        let Some(previous) = &self.latest_structural_snapshot else {
            return true;
        };
        if previous.sample_rate != state.sample_rate
            || previous.sample_rate_hz != state.sample_rate_hz
            || previous.clock_source != state.clock_source
            || previous.status_flags.as_slice() != state.status_flags
            || previous.front_panel_bytes.as_slice() != state.front_panel_bytes
            || previous.outputs.as_slice() != state.outputs
            || previous.dsp_cluster.as_slice() != state.dsp_cluster
            || previous.surface != state.surface
        {
            return true;
        }
        for (surface, strips) in &state.mixer_surfaces {
            let Some(previous_surface) = previous.mixer_surfaces.get(surface.index()) else {
                return true;
            };
            if previous_surface.len() != strips.len() {
                return true;
            }
            if previous_surface
                .iter()
                .zip(strips)
                .any(|(previous, current)| {
                    previous.meter != current.meter
                        || previous.muted != current.muted
                        || previous.pan.unwrap_or_default() != current.pan
                        || previous.linked != current.linked
                })
            {
                return true;
            }
        }
        false
    }

    fn apply_input_patch(&mut self, inputs: Vec<DynamicInputState>) {
        for input in inputs {
            let Some(space) = self
                .input_spaces
                .iter_mut()
                .find(|space| space.space_id == input.address.space)
            else {
                continue;
            };
            let Some(slot) = space
                .inputs
                .iter_mut()
                .find(|slot| slot.address == input.address)
            else {
                continue;
            };
            *slot = input;
        }
        if let Some(space) = self.input_spaces.iter().find(|space| space.space_id == 0) {
            for input in &space.inputs {
                let index = usize::from(input.address.index);
                let Some(cluster_gain) = self.device.dsp_cluster.get_mut(index) else {
                    continue;
                };
                if let Some(gain) = input.gain.and_then(|gain| u8::try_from(gain).ok()) {
                    *cluster_gain = gain;
                }
                let Some(mode_slot) = index
                    .checked_add(2)
                    .and_then(|offset| self.device.dsp_cluster.get_mut(offset))
                else {
                    continue;
                };
                if let Some(mode) = input.mode.and_then(|mode| u8::try_from(mode).ok()) {
                    *mode_slot = mode & 0x0f;
                }
                if input.phantom == Some(true) {
                    *mode_slot |= 0x10;
                }
                if input.phase == Some(true) {
                    *mode_slot |= 0x40;
                }
            }
            self.refresh_preamp_from_cluster_preserving_observed_meter();
        }
    }

    fn apply_meter_patch(&mut self, inputs: Vec<DynamicInputState>) {
        for input in inputs {
            let Some(space) = self
                .input_spaces
                .iter_mut()
                .find(|space| space.space_id == input.address.space)
            else {
                continue;
            };
            let Some(slot) = space
                .inputs
                .iter_mut()
                .find(|slot| slot.address == input.address)
            else {
                continue;
            };
            slot.meter = input.meter;
        }
    }

    fn apply_output_patch(&mut self, outputs: Vec<DynamicOutputState>) {
        for output in outputs {
            let Some(slot) = self
                .output
                .dynamic
                .iter_mut()
                .find(|slot| slot.address == output.address)
            else {
                continue;
            };
            *slot = output;
        }
        for output in &self.output.dynamic {
            let index = usize::from(output.address.id);
            let Some(slot) = self.output.states.get_mut(index) else {
                continue;
            };
            if let Some(level) = output.level.and_then(|level| u8::try_from(level).ok()) {
                slot.volume = level;
            }
            slot.mode = if output.muted == Some(true) {
                OutputMode::Mute
            } else if output.dimmed == Some(true) {
                OutputMode::Dim
            } else {
                OutputMode::Normal
            };
        }
    }

    fn apply_mixer_snapshot(&mut self, mixers: Vec<DynamicMixerSurface>) {
        for mixer in mixers {
            let Some(slot) = self
                .mixer
                .surfaces
                .iter_mut()
                .find(|slot| slot.surface == mixer.surface)
            else {
                continue;
            };
            let topology_matches = slot.master.is_some() == mixer.master.is_some()
                && slot.strips.len() == mixer.strips.len()
                && slot
                    .strips
                    .iter()
                    .zip(&mixer.strips)
                    .all(|(declared, incoming)| declared.strip == incoming.strip)
                && match (&slot.master, &mixer.master) {
                    (Some(declared), Some(incoming)) => declared.strip == incoming.strip,
                    (None, None) => true,
                    _ => false,
                };
            if !topology_matches {
                continue;
            }
            *slot = mixer;
        }
        self.sync_compatibility_views();
        self.clamp_dynamic_selection();
    }

    pub(crate) fn sync_compatibility_views(&mut self) {
        self.mixer.channels = self
            .mixer
            .surfaces
            .iter()
            .map(|surface| {
                surface
                    .strips
                    .iter()
                    .filter_map(Self::compatibility_channel)
                    .collect()
            })
            .collect();
        self.mixer
            .peaks
            .resize_with(self.mixer.channels.len(), Vec::new);
        for (peaks, channels) in self.mixer.peaks.iter_mut().zip(&self.mixer.channels) {
            peaks.resize(channels.len(), None);
        }
    }

    fn apply_routing_snapshot(&mut self, groups: Vec<DynamicRoutingGroup>) {
        let mut observed = Vec::new();
        for group in groups {
            let Some(capability) = self
                .routing_capabilities
                .iter()
                .find(|capability| capability.destination == group.destination)
            else {
                continue;
            };
            if group.sources.len() == usize::from(capability.channel_count) {
                observed.push(group);
            }
        }
        self.routing = observed;
    }

    fn apply_routing_patch(&mut self, group: DynamicRoutingGroup) {
        let Some(capability) = self
            .routing_capabilities
            .iter()
            .find(|capability| capability.destination == group.destination)
        else {
            return;
        };
        if group.sources.len() != usize::from(capability.channel_count) {
            return;
        }
        if let Some(slot) = self
            .routing
            .iter_mut()
            .find(|slot| slot.destination == group.destination)
        {
            *slot = group;
        } else {
            self.routing.push(group);
            self.routing.sort_by_key(|group| group.destination);
        }
    }

    fn apply_patch(&mut self, patch: DynamicStatePatch) {
        match patch {
            DynamicStatePatch::Inputs(inputs) => self.apply_input_patch(inputs),
            DynamicStatePatch::Outputs(outputs) => self.apply_output_patch(outputs),
            DynamicStatePatch::Mixer(mixer) => self.apply_mixer_snapshot(vec![mixer]),
            DynamicStatePatch::Routing(group) => self.apply_routing_patch(group),
            DynamicStatePatch::Globals(globals) => {
                for global in globals {
                    if let Some(slot) = self
                        .globals
                        .iter_mut()
                        .find(|slot| slot.control == global.control)
                    {
                        *slot = global;
                    }
                }
                self.apply_dynamic_globals_to_status();
            }
        }
    }

    fn apply_dynamic_globals_to_status(&mut self) {
        for global in &self.globals {
            match global {
                DynamicGlobalState {
                    control: GlobalControl::SampleRate,
                    value: ControlValue::Enum(value),
                } => {
                    let rate = SampleRate::from_code(*value as u8);
                    self.device.status.sample_rate = Some(rate);
                    self.device.status.sample_rate_hz = rate.hz();
                }
                DynamicGlobalState {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(value),
                } => {
                    self.device.status.clock_source = Some(ClockSource::from_code(*value as u8));
                }
                DynamicGlobalState {
                    control: GlobalControl::Surface,
                    value: ControlValue::Enum(value),
                } => {
                    if let Ok(index) = usize::try_from(*value) {
                        self.mixer.surface_index =
                            index.min(self.mixer.surfaces.len().saturating_sub(1));
                    }
                }
                _ => {}
            }
        }
    }

    fn clamp_dynamic_selection(&mut self) {
        let strip_count = self
            .active_mixer_surface()
            .and_then(|index| self.mixer.surfaces.get(index))
            .map_or(0, |surface| surface.strips.len());
        self.mixer.selected_channel = self
            .mixer
            .selected_channel
            .min(strip_count.saturating_sub(1));
        self.mixer.strip_scroll = self.mixer.strip_scroll.min(strip_count.saturating_sub(1));
    }

    pub fn observe_event(&mut self, event: DeviceEvent) -> bool {
        match event {
            DeviceEvent::Snapshot { state, raw } => self.apply_dynamic_state(state, raw),
            DeviceEvent::QueryReply {
                query_id,
                sub_id,
                body,
                patch,
                raw,
            } => {
                let was_connected = self.device.connection.connected;
                self.device.connection.connected = true;
                self.device.connection.last_snapshot_at = Some(Instant::now());
                self.device.connection.last_frame_type = Some("0x75 query reply");
                self.raw_view.latest_raw_75 = Some(raw.clone());
                if let Some(patch) = patch {
                    self.apply_patch(patch);
                }
                let reply = QueryResponse {
                    query_id,
                    sub_id,
                    body,
                };
                self.store_startup_query_summary(&reply);
                self.push_query_reply_log(&reply, raw);
                !was_connected || true
            }
            DeviceEvent::Meter { inputs, raw } => {
                let was_connected = self.device.connection.connected;
                self.device.connection.connected = true;
                self.device.connection.last_snapshot_at = Some(Instant::now());
                self.device.connection.last_frame_type = Some("0x75 meter");
                self.apply_meter_patch(inputs);
                self.raw_view.latest_raw_75 = Some(raw);
                !was_connected || true
            }
            DeviceEvent::Auxiliary { bytes, raw } => {
                let changed = !self.device.connection.connected
                    || self.raw_view.latest_raw_83.as_ref() != Some(&raw);
                self.device.connection.connected = true;
                self.device.connection.last_snapshot_at = Some(Instant::now());
                self.device.connection.last_frame_type = Some("0x83 auxiliary");
                self.raw_view.last_auxiliary_len = Some(bytes.len());
                self.raw_view.latest_raw_83 = Some(raw);
                changed
            }
            DeviceEvent::Notification { raw, .. } => {
                let changed = !self.device.connection.connected
                    || self.raw_view.latest_raw_81.as_ref() != Some(&raw);
                self.device.connection.connected = true;
                self.device.connection.last_snapshot_at = Some(Instant::now());
                self.device.connection.last_frame_type = Some("0x81 notification");
                self.raw_view.latest_raw_81 = Some(raw);
                changed
            }
        }
    }

    pub fn observe_frame<R: AsRef<[u8]>>(&mut self, frame: DeviceSnapshot, raw: R) -> bool {
        let raw = raw.as_ref().to_vec();
        let was_connected = self.device.connection.connected;
        self.device.connection.connected = true;
        self.device.connection.last_snapshot_at = Some(Instant::now());
        match frame {
            DeviceSnapshot::Snapshot(snapshot) => {
                let structural_changed =
                    !was_connected || self.snapshot_structurally_differs(&snapshot);
                let raw_changed =
                    self.popup.raw_view_open && self.raw_view.latest_raw_73.as_ref() != Some(&raw);
                let changed = structural_changed || raw_changed;
                self.device.connection.last_frame_type = Some("0x73 snapshot");
                if structural_changed {
                    self.apply_snapshot(&snapshot);
                } else {
                    self.apply_meters_only(&snapshot);
                }
                self.latest_structural_snapshot =
                    Some(StructuralSnapshot::from_snapshot(&snapshot));
                self.raw_view.latest_raw_73 = Some(raw);
                changed
            }
            DeviceSnapshot::Auxiliary(bytes) => {
                let changed = !was_connected
                    || (self.popup.raw_view_open
                        && self.raw_view.latest_raw_83.as_ref() != Some(&raw));
                self.device.connection.last_frame_type = Some("0x83 auxiliary");
                self.raw_view.last_auxiliary_len = Some(bytes.len());
                self.raw_view.latest_raw_83 = Some(raw);
                changed
            }
            DeviceSnapshot::QueryReply(reply) => {
                self.device.connection.last_frame_type = Some("0x75 query reply");
                self.raw_view.latest_raw_75 = Some(raw.clone());
                self.store_startup_query_summary(&reply);
                self.apply_query_reply_readback(&reply);
                self.push_query_reply_log(&reply, raw);
                if let Some(metadata) = reply.metadata() {
                    self.ui.last_message = format!(
                        "Connected to {} (hw {}, serial {})",
                        metadata.product_name, metadata.hardware_version, metadata.serial
                    );
                    self.device.status.metadata = Some(metadata);
                }
                true
            }
            DeviceSnapshot::Notification(_) => {
                let changed = !was_connected || self.popup.raw_view_open;
                self.device.connection.last_frame_type = Some("0x81 notification");
                self.raw_view.latest_raw_81 = Some(raw);
                changed
            }
        }
    }

    fn store_startup_query_summary(&mut self, reply: &QueryResponse) {
        if let Some(index) = startup_query_slot(reply.query_id) {
            self.device.status.startup_query_summaries[index] = Some(reply.summary_label());
        }
    }

    fn push_query_reply_log(&mut self, reply: &QueryResponse, raw: Vec<u8>) {
        let detail = if reply.selector_bitmap().is_some() || reply.selector_pair_bank().is_some() {
            reply.summary_label()
        } else {
            let preview = reply
                .body
                .iter()
                .take(8)
                .map(|byte| format!("{:02x}", byte))
                .collect::<Vec<_>>()
                .join(" ");
            format!("[{} bytes] {}", reply.body.len(), preview)
        };
        let summary = format!(
            "0x75 {:02x}/{:02x} {}",
            reply.query_id, reply.sub_id, detail
        );
        self.raw_view.recent_query_reply_log.push(summary.clone());
        self.raw_view
            .recent_query_reply_entries
            .push(QueryReplyLogEntry { summary, raw });
        if self.raw_view.recent_query_reply_log.len() > 16 {
            let drop_count = self.raw_view.recent_query_reply_log.len() - 16;
            self.raw_view.recent_query_reply_log.drain(0..drop_count);
            self.raw_view
                .recent_query_reply_entries
                .drain(0..drop_count);
        }
        self.raw_view.selected_query_reply_entry =
            Some(self.raw_view.recent_query_reply_entries.len() - 1);
    }

    fn apply_query_reply_readback(&mut self, reply: &QueryResponse) {
        if let Some(assignments) = reply.assignment_readback() {
            for (index, assignment) in assignments.into_iter().enumerate() {
                let Some(assignment) = assignment else {
                    continue;
                };
                for channels in &mut self.mixer.channels {
                    channels[index].assignment = Some(assignment);
                }
            }
        }

        if let Some(startup_links) = reply.startup_link_readback_from_bitmap() {
            for (mixer, links) in startup_links {
                for (index, linked) in links.into_iter().enumerate() {
                    let Some(linked) = linked else {
                        continue;
                    };
                    let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.linked = Some(linked);
                }
            }
        }

        if let Some((mixer, states)) = reply.startup_pan_state_readback() {
            for (index, state) in states.into_iter().enumerate() {
                let Some(state) = state else {
                    continue;
                };
                let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                    continue;
                };
                slot.level = Some(state.level);
                slot.pan = state.pan;
                slot.muted = Some(state.muted);
                slot.soloed = Some(state.soloed);
            }
        }

        if let Some(readback) = reply.mixer_strip_readback() {
            for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
                for (index, state) in readback.surfaces[mixer.index()].into_iter().enumerate() {
                    let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.soloed = Some(state.soloed);
                }
            }
        }
    }

    pub fn mark_disconnected(&mut self) {
        self.device.connection.connected = false;
        self.device.connection.last_frame_type = Some("disconnected");
    }

    pub fn cycle_focus(&mut self) {
        self.ui.focus = match self.ui.focus {
            FocusArea::Status => FocusArea::Outputs,
            FocusArea::Outputs => FocusArea::Mixer,
            FocusArea::Mixer => FocusArea::Preamp,
            FocusArea::Preamp => FocusArea::Outputs,
        };
    }

    pub fn toggle_raw_view(&mut self) {
        self.popup.raw_view_open = !self.popup.raw_view_open;
    }

    pub fn toggle_hotkeys_popup(&mut self) {
        self.popup.hotkeys_open = !self.popup.hotkeys_open;
    }

    pub fn toggle_options_popup(&mut self) {
        self.popup.options_open = !self.popup.options_open;
    }

    pub fn selected_profile_name(&self) -> Option<&str> {
        self.popup
            .profile_names
            .get(self.popup.selected_index)
            .map(String::as_str)
    }

    pub fn clamp_profile_selection(&mut self) {
        if self.popup.profile_names.is_empty() {
            self.popup.selected_index = 0;
        } else {
            self.popup.selected_index = self
                .popup
                .selected_index
                .min(self.popup.profile_names.len().saturating_sub(1));
        }
    }

    pub fn cycle_raw_packet(&mut self, forward: bool) {
        let tabs = [
            RawPacketTab::Query74,
            RawPacketTab::State73,
            RawPacketTab::Auxiliary,
            RawPacketTab::Query75,
            RawPacketTab::DeviceNotification,
        ];
        let index = tabs
            .iter()
            .position(|tab| *tab == self.raw_view.selected_tab)
            .unwrap_or(0);
        let tab = if forward {
            tabs[(index + 1) % tabs.len()]
        } else {
            tabs[index.checked_sub(1).unwrap_or(tabs.len() - 1)]
        };
        self.raw_view.select_tab(tab);
    }

    pub fn cycle_query_reply_entry(&mut self, forward: bool) {
        if self.raw_view.recent_query_reply_entries.is_empty() {
            self.raw_view.selected_query_reply_entry = None;
            self.raw_view.reset_raw_view_scroll();
            return;
        }
        let current = self
            .raw_view
            .selected_query_reply_entry
            .unwrap_or(self.raw_view.recent_query_reply_entries.len() - 1);
        self.raw_view.selected_query_reply_entry = Some(if forward {
            (current + 1) % self.raw_view.recent_query_reply_entries.len()
        } else {
            current
                .checked_sub(1)
                .unwrap_or(self.raw_view.recent_query_reply_entries.len() - 1)
        });
        self.raw_view.reset_raw_view_scroll();
        self.ensure_query_reply_visible();
    }

    fn ensure_query_reply_visible(&mut self) {
        let Some(selected) = self.raw_view.selected_query_reply_entry else {
            return;
        };
        let total = self.raw_view.recent_query_reply_entries.len();
        let visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
        let reversed_index = total - 1 - selected;
        if reversed_index < self.raw_view.query_reply_scroll {
            self.raw_view.query_reply_scroll = reversed_index;
        } else if reversed_index >= self.raw_view.query_reply_scroll + visible {
            self.raw_view.query_reply_scroll = reversed_index - visible + 1;
        }
    }

    pub fn capture_raw_baseline(&mut self) {
        self.raw_view.baseline_raw_73 = self.raw_view.latest_raw_73.clone();
        self.raw_view.baseline_raw_83 = self.raw_view.latest_raw_83.clone();
        self.raw_view.baseline_raw_74 = self.raw_view.latest_raw_74.clone();
        self.raw_view.baseline_raw_75 = self.raw_view.latest_raw_75.clone();
        self.raw_view.baseline_raw_81 = self.raw_view.latest_raw_81.clone();
    }

    pub fn clear_raw_baseline(&mut self) {
        self.raw_view.baseline_raw_73 = None;
        self.raw_view.baseline_raw_83 = None;
        self.raw_view.baseline_raw_74 = None;
        self.raw_view.baseline_raw_75 = None;
        self.raw_view.baseline_raw_81 = None;
    }

    pub fn observe_query_request<R: AsRef<[u8]>>(&mut self, raw: R) {
        let raw = raw.as_ref().to_vec();
        let query_id = raw.get(0x08).copied().unwrap_or(0);
        let sub_id = raw.get(0x0c).copied().unwrap_or(0);
        self.raw_view.latest_raw_74 = Some(raw);
        self.raw_view
            .recent_query_request_log
            .push(format!("0x74 {:02x}/{:02x}", query_id, sub_id));
        if self.raw_view.recent_query_request_log.len() > 16 {
            let drop_count = self.raw_view.recent_query_request_log.len() - 16;
            self.raw_view.recent_query_request_log.drain(0..drop_count);
        }
    }
}

fn startup_query_slot(query_id: u8) -> Option<usize> {
    match query_id {
        0x01 => Some(0),
        0x00 => Some(1),
        0x11 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::profile::{
        DeviceProfile, MixerAssignmentEntry, MixerAssignmentProfile, MixerProfiles,
        MixerStripProfile, OutputModeProfile, OutputProfile, OutputProfiles, PreampInputProfile,
        PreampModeProfile, PreampProfiles,
    };
    use crate::transport::{MockTransport, Transport};
    use antelope_protocol::{
        Action, ClockSource, CommandBatch, ControlValue, DeviceDefinition, DeviceDriver,
        DeviceEvent, DeviceSnapshot, DeviceStateSnapshot, DriverError, Frame, GlobalControl,
        InputAddress, InputControl, MixerAddress, MixerAssignment, MixerChannelState,
        MixerLinkTarget, MixerStrip, MixerSurface, OutputMode, OutputState, OutputTarget, PanState,
        PreampMode, PreampState, QueryRequest, QueryResponse, SampleRate, Surface,
    };

    use super::controller::MAX_FRAMES_PER_POLL;
    use super::*;

    fn raw_frame(bytes: &[u8]) -> [u8; 320] {
        let mut raw = [0_u8; 320];
        raw[..bytes.len()].copy_from_slice(bytes);
        raw
    }

    fn zen_go_controller(transport: Box<dyn Transport>) -> Controller {
        Controller::new(
            transport,
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller")
    }

    fn seed_complete_dynamic_mixer(
        controller: &mut Controller,
        mixer: MixerSurface,
        channels: &[u8],
    ) {
        for &channel in channels {
            let legacy = controller.state.mixer.channels[mixer.index()][usize::from(channel - 1)];
            let strip = controller.state.mixer.surfaces[mixer.index()]
                .strips
                .iter_mut()
                .find(|strip| strip.strip == u16::from(channel))
                .expect("dynamic mixer strip");
            strip.fader = Some(i32::from(legacy.level.unwrap_or(0)));
            strip.pan = Some(i32::from(legacy.pan.raw()));
            strip.muted = Some(legacy.muted.unwrap_or(false));
            strip.soloed = Some(legacy.soloed.unwrap_or(false));
            strip.linked = legacy.linked;
        }
    }

    struct UnsupportedDriver {
        definition: DeviceDefinition,
    }

    impl UnsupportedDriver {
        fn new() -> Self {
            Self {
                definition: DeviceDefinition {
                    id: "unsupported-test-driver".into(),
                    name: "Unsupported test driver".into(),
                    vid: 0x23e5,
                    pid: 0xffff,
                    supported: false,
                },
            }
        }
    }

    impl DeviceDriver for UnsupportedDriver {
        fn definition(&self) -> &DeviceDefinition {
            &self.definition
        }

        fn startup_requests(&self) -> &[QueryRequest] {
            &[]
        }

        fn encode(&self, _action: Action) -> Result<CommandBatch, DriverError> {
            Err(DriverError::UnsupportedAction("test driver".into()))
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError> {
            Ok(None)
        }
    }

    fn routing_group_action(
        strip: u8,
        assignment: MixerAssignment,
        mut assignments: [MixerAssignment; 16],
    ) -> Action {
        assignments[usize::from(strip - 1)] = assignment;
        Action::SetRoutingGroup {
            destination: 0,
            changed_channel: Some(u16::from(strip - 1)),
            sources: assignments
                .into_iter()
                .map(super::controller::routing_source_from_assignment)
                .collect(),
        }
    }

    struct FailingDriver {
        definition: DeviceDefinition,
    }

    impl FailingDriver {
        fn new() -> Self {
            Self {
                definition: DeviceDefinition {
                    id: "failing-test-driver".into(),
                    name: "Failing test driver".into(),
                    vid: 0x23e5,
                    pid: 0xfffe,
                    supported: true,
                },
            }
        }
    }

    impl DeviceDriver for FailingDriver {
        fn definition(&self) -> &DeviceDefinition {
            &self.definition
        }
        fn startup_requests(&self) -> &[QueryRequest] {
            &[]
        }
        fn encode(&self, _action: Action) -> Result<CommandBatch, DriverError> {
            Err(DriverError::InvalidAction("injected encode failure".into()))
        }
        fn decode(&self, _bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError> {
            Err(DriverError::InvalidAction("injected decode failure".into()))
        }
    }

    #[test]
    fn controller_rejects_unsupported_driver_before_writes() {
        let transport = MockTransport::default();
        let result = Controller::new(
            Box::new(transport.clone()),
            Box::new(UnsupportedDriver::new()),
        );
        let error = match result {
            Ok(_) => panic!("unsupported driver must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Unsupported test driver"));
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn controller_propagates_driver_encode_and_decode_failures() {
        let transport = MockTransport::default();
        let mut controller =
            Controller::new(Box::new(transport.clone()), Box::new(FailingDriver::new()))
                .expect("supported failing driver");
        let action = Action::SetGlobal {
            control: GlobalControl::SampleRate,
            value: ControlValue::Enum(2),
        };
        assert!(controller.send(action, None).is_err());
        assert!(transport.take_writes().is_empty());
        transport.push_read(vec![0; 320]);
        assert!(controller.poll_device(Duration::ZERO).is_err());
    }

    #[test]
    fn normalized_query_event_preserves_owned_raw_bytes() {
        let mut state = AppState::default();
        let raw = vec![0x75; 320];

        assert!(state.observe_event(DeviceEvent::QueryReply {
            query_id: 0x00,
            sub_id: 0x00,
            body: vec![0x01, 0x02],
            patch: None,
            raw: raw.clone(),
        }));
        assert_eq!(state.raw_view.latest_raw_75, Some(raw));
    }

    fn aux_frame(bytes: &[u8]) -> [u8; 320] {
        raw_frame(bytes)
    }

    fn snapshot() -> DeviceStateSnapshot {
        DeviceStateSnapshot {
            sample_rate: SampleRate::Hz48000,
            clock_source: ClockSource::Internal,
            sample_rate_hz: 48_000,
            status_flags: [0x08, 0x00],
            front_panel_bytes: [0, 0, 0],
            outputs: [
                OutputState::new(OutputTarget::Monitor, 0x50, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0x40, OutputMode::Mute),
                OutputState::new(OutputTarget::Hp2, 0x30, OutputMode::Dim),
            ],
            dsp_cluster: [0x2f, 0x34, 0x50, 0x10],
            preamp: PreampState::from_cluster([0x2f, 0x34, 0x50, 0x10]),
            surface: Surface::MonitorHp1,
            mixer_decode: Default::default(),
            late_shadow: [0; 12],
        }
    }

    fn seed_shared_assignments(state: &mut AppState) -> [MixerAssignment; 16] {
        let assignments = [
            MixerAssignment::Preamp(1),
            MixerAssignment::Preamp(2),
            MixerAssignment::ComputerPlay(1),
            MixerAssignment::ComputerPlay(2),
            MixerAssignment::ComputerPlay(3),
            MixerAssignment::ComputerPlay(4),
            MixerAssignment::ComputerPlay(5),
            MixerAssignment::ComputerPlay(6),
            MixerAssignment::ComputerPlay(7),
            MixerAssignment::ComputerPlay(8),
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
        ];

        for surface in &mut state.mixer.channels {
            for (channel, assignment) in surface.iter_mut().zip(assignments) {
                channel.assignment = Some(assignment);
            }
        }

        assignments
    }

    fn assignment_pairs(frame: &[u8], count: usize) -> Vec<[u8; 2]> {
        let payload = &frame[0x10 + 0x03..];
        payload
            .as_chunks::<2>()
            .0
            .iter()
            .take(count)
            .copied()
            .collect()
    }

    fn snapshot_frame_bytes(meter: u8) -> [u8; 320] {
        let mut frame = [0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x00] = 0x08;
        payload[0x02] = 0x02;
        payload[0x03] = 0x00;
        payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());
        payload[0x0c] = 0x50;
        payload[0x0d] = 0x00;
        payload[0x0e] = 0x40;
        payload[0x0f] = 0x01;
        payload[0x10] = 0x30;
        payload[0x11] = 0x02;
        payload[0x18..0x1c].copy_from_slice(&[0x2f, 0x34, 0x50, 0x10]);
        payload[0x6a] = 0x0f;
        payload[0xcf] = meter;
        frame
    }

    #[test]
    fn intent_enum_exists_and_can_be_created() {
        // Test that Intent enum exists and can be constructed
        let intent = Intent::Quit;
        assert!(matches!(intent, Intent::Quit));
    }

    #[test]
    fn intent_enum_covers_output_actions() {
        // Test output-related intents
        let adjust = Intent::AdjustOutputLevel {
            index: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustOutputLevel { .. }));

        let set = Intent::SetOutputLevel {
            index: 0,
            step: 0x30,
        };
        assert!(matches!(set, Intent::SetOutputLevel { .. }));

        let mute = Intent::ToggleOutputMute(0);
        assert!(matches!(mute, Intent::ToggleOutputMute(0)));

        let dim = Intent::ToggleOutputDim(0);
        assert!(matches!(dim, Intent::ToggleOutputDim(0)));
    }

    #[test]
    fn intent_enum_covers_mixer_actions() {
        // Test mixer-related intents
        let adjust = Intent::AdjustMixerLevel {
            index: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustMixerLevel { .. }));

        let set = Intent::SetMixerLevel {
            index: 0,
            level: 0x50,
        };
        assert!(matches!(set, Intent::SetMixerLevel { .. }));

        let pan = Intent::AdjustMixerPan {
            index: 0,
            right: true,
        };
        assert!(matches!(pan, Intent::AdjustMixerPan { .. }));

        let set_pan = Intent::SetMixerPan {
            index: 0,
            pan: PanState::center(),
        };
        assert!(matches!(set_pan, Intent::SetMixerPan { .. }));

        let mute = Intent::ToggleMixerMute(1);
        assert!(matches!(mute, Intent::ToggleMixerMute(1)));

        let solo = Intent::ToggleMixerSolo(1);
        assert!(matches!(solo, Intent::ToggleMixerSolo(1)));
    }

    #[test]
    fn intent_enum_covers_preamp_actions() {
        // Test preamp-related intents
        let adjust = Intent::AdjustPreampGain {
            input: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustPreampGain { .. }));

        let set = Intent::SetPreampGain {
            input: 0,
            raw: 0x30,
        };
        assert!(matches!(set, Intent::SetPreampGain { .. }));

        let mode = Intent::PickPreampMode {
            input: 0,
            mode: PreampMode::Mic,
        };
        assert!(matches!(mode, Intent::PickPreampMode { .. }));

        let phase = Intent::TogglePreampPhase(0);
        assert!(matches!(phase, Intent::TogglePreampPhase(0)));

        let phantom = Intent::TogglePreampPhantom(0);
        assert!(matches!(phantom, Intent::TogglePreampPhantom(0)));
    }

    #[test]
    fn intent_enum_covers_navigation_actions() {
        // Test navigation intents
        let quit = Intent::Quit;
        assert!(matches!(quit, Intent::Quit));

        let raw = Intent::ToggleRawView;
        assert!(matches!(raw, Intent::ToggleRawView));

        let surface = Intent::SelectSurface(Surface::MonitorHp1);
        assert!(matches!(surface, Intent::SelectSurface(_)));
    }

    #[test]
    fn intent_enum_covers_selector_actions() {
        // Test selector popup intents
        let sample = Intent::PickSampleRate(SampleRate::Hz48000);
        assert!(matches!(sample, Intent::PickSampleRate(_)));

        let clock = Intent::PickClockSource(ClockSource::Internal);
        assert!(matches!(clock, Intent::PickClockSource(_)));
    }

    #[test]
    fn reducer_prefers_device_snapshot_state() {
        let mut state = AppState::default();
        state.output.states[0].volume = 0x10;

        state.apply_snapshot(&snapshot());

        assert_eq!(state.device.status.sample_rate, Some(SampleRate::Hz48000));
        assert_eq!(state.output.states[0].volume, 0x50);
        assert_eq!(state.output.states[1].mode, OutputMode::Mute);
        assert_eq!(state.mixer.surface, Surface::MonitorHp1);
    }

    #[test]
    fn reducer_updates_preamp_state_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.dsp_cluster = [0x14, 0x2a, 0x11, 0x00];

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input1.mode, PreampMode::Line);
        assert_eq!(state.preamp.state.input1.gain_raw, 0x14);
        assert_eq!(state.preamp.state.input2.mode, PreampMode::Mic);
        assert_eq!(state.preamp.state.input2.gain_raw, 0x2a);
        assert!(!state.preamp.state.input2.phantom_on);
    }

    #[test]
    fn reducer_applies_grounded_passive_mixer_decode_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp1_meter = Some(0x28);
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].muted = Some(false);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].linked = Some(true);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][1].linked = Some(true);

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input1.observed_meter, Some(0x28));
        assert_eq!(state.preamp.state.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].level,
            None
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].muted,
            Some(false)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn passive_snapshot_pan_decode_does_not_override_channel_pan() {
        let mut state = AppState::default();
        state.mixer.channels[MixerSurface::Mix1.index()][0].pan = PanState::center();

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].pan =
            Some(PanState::from_raw(0x1e));

        state.apply_snapshot(&device_snapshot);

        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
    }

    #[test]
    fn query_reply_assignment_readback_updates_shared_strip_assignments() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x03,
                sub_id: 0x05,
                body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x03,
                sub_id: 0x06,
                body: vec![
                    0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03,
                    0x01, 0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08,
                    0x00, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer.channels[mixer.index()];
            assert_eq!(channels[0].assignment, Some(MixerAssignment::Preamp(1)));
            assert_eq!(channels[1].assignment, Some(MixerAssignment::Preamp(2)));
            assert_eq!(
                channels[2].assignment,
                Some(MixerAssignment::ComputerPlay(1))
            );
            assert_eq!(
                channels[3].assignment,
                Some(MixerAssignment::ComputerPlay(2))
            );
            assert_eq!(
                channels[4].assignment,
                Some(MixerAssignment::ComputerPlay(3))
            );
            assert_eq!(
                channels[9].assignment,
                Some(MixerAssignment::ComputerPlay(8))
            );
            assert!(channels[10..]
                .iter()
                .all(|slot| slot.assignment == Some(MixerAssignment::Mute)));
        }
    }

    #[test]
    fn query_reply_startup_link_readback_updates_visible_pairs_from_bitmap() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer.channels[mixer.index()];
            let expected_primary = if mixer == MixerSurface::Mix1 {
                Some(true)
            } else {
                Some(false)
            };
            assert_eq!(channels[0].linked, expected_primary);
            assert_eq!(channels[1].linked, expected_primary);
            for index in (2..10).step_by(2) {
                assert_eq!(channels[index].linked, Some(true));
                assert_eq!(channels[index + 1].linked, Some(true));
            }
            assert!(channels[10..].iter().all(|slot| slot.linked == Some(false)));
        }

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert!(mix1[10..].iter().all(|slot| slot.linked == Some(true)));
        assert!(mix2[10..].iter().all(|slot| slot.linked == Some(true)));
    }

    #[test]
    fn query_reply_startup_pan_state_readback_updates_mix_pan_and_mute() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x00, 0x5e, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x00));
        assert_eq!(mix1[0].pan, PanState::from_raw(0x1e));
        assert_eq!(mix1[0].muted, Some(true));
        assert_eq!(mix1[1].level, Some(0x00));
        assert_eq!(mix1[1].pan, PanState::center());
        assert_eq!(mix1[1].muted, Some(true));
        assert_eq!(mix1[2].level, Some(0x00));
        assert_eq!(mix1[2].pan, PanState::left());
        assert_eq!(mix1[2].muted, Some(false));
        assert_eq!(mix1[3].level, Some(0x00));
        assert_eq!(mix1[3].pan, PanState::right());
        assert_eq!(mix1[3].muted, Some(false));
        assert_eq!(mix2[10].level, Some(0x00));
        assert_eq!(mix2[10].pan, PanState::left());
        assert_eq!(mix2[10].muted, Some(false));
        assert_eq!(mix2[11].level, Some(0x00));
        assert_eq!(mix2[11].pan, PanState::right());
        assert_eq!(mix2[11].muted, Some(false));
    }

    #[test]
    fn query_reply_startup_level_readback_updates_mix_levels() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x12, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x1e, 0x02, 0x1e, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x12));
        assert_eq!(mix2[10].level, Some(0x1e));
        assert_eq!(mix2[11].level, Some(0x1e));
    }

    #[test]
    fn query_reply_strip_readback_does_not_seed_unstable_startup_state() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60,
                    0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e,
                    0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60,
                    0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                    0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert_eq!(mix1[0].muted, None);
        assert!(mix1.iter().all(|slot| slot.level.is_none()));
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
        assert!(mix2.iter().all(|slot| slot.level.is_none()));
        assert!(mix2.iter().all(|slot| slot.muted.is_none()));
        assert!(mix2.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn query_reply_strip_readback_does_not_apply_pan_or_mute_overlay() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x12, 0x3e, 0x60, 0x60, 0x60, 0x60, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        assert_eq!(mix1[0].muted, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn passive_meter_does_not_override_known_level_value() {
        let mut state = AppState::default();
        state.mixer.channels[MixerSurface::Mix1.index()][0].level = Some(0x00);

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].level,
            Some(0x00)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
    }

    #[test]
    fn preamp_pending_gain_updates_authoritative_cluster() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);

        controller
            .send(
                Action::SetInput {
                    address: InputAddress { space: 0, index: 1 },
                    control: InputControl::Gain,
                    value: ControlValue::Int(0x2d),
                },
                Some(PendingMutation::PreampGain {
                    input: 1,
                    raw: 0x2d,
                }),
            )
            .expect("send preamp gain");
        controller.confirm_pending_write();

        assert_eq!(controller.state.preamp.state.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.device.dsp_cluster[1], 0x2d);
    }

    #[test]
    fn preamp_pending_updates_preserve_observed_input_meters() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller.state.preamp.state.input2.observed_meter = Some(0x30);

        controller
            .send(
                Action::SetInput {
                    address: InputAddress { space: 0, index: 1 },
                    control: InputControl::Gain,
                    value: ControlValue::Int(0x2d),
                },
                Some(PendingMutation::PreampGain {
                    input: 1,
                    raw: 0x2d,
                }),
            )
            .expect("send preamp gain");
        controller.confirm_pending_write();

        assert_eq!(controller.state.preamp.state.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.preamp.state.input1.observed_meter, None);
        assert_eq!(
            controller.state.preamp.state.input2.observed_meter,
            Some(0x30)
        );
    }

    #[test]
    fn preamp_pending_mode_phantom_and_phase_update_state() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);

        controller
            .send(
                Action::SetInput {
                    address: InputAddress {
                        space: 0,
                        index: 0_u16,
                    },
                    control: InputControl::Mode,
                    value: ControlValue::Enum(i32::from(PreampMode::Line.code())),
                },
                Some(PendingMutation::PreampMode {
                    input: 0,
                    mode: PreampMode::Line,
                }),
            )
            .expect("send preamp mode");
        controller.confirm_pending_write();
        assert_eq!(controller.state.preamp.state.input1.mode, PreampMode::Line);

        controller.state.device.dsp_cluster[3] = 0x00;
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller
            .send(
                Action::SetInput {
                    address: InputAddress { space: 0, index: 1 },
                    control: InputControl::Phantom,
                    value: ControlValue::Bool(true),
                },
                Some(PendingMutation::PreampPhantom {
                    input: 1,
                    enabled: true,
                }),
            )
            .expect("send preamp phantom");
        controller.confirm_pending_write();
        assert!(controller.state.preamp.state.input2.phantom_on);

        controller.state.device.dsp_cluster[3] = 0x00;
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller
            .send(
                Action::SetInput {
                    address: InputAddress { space: 0, index: 1 },
                    control: InputControl::Phase,
                    value: ControlValue::Bool(true),
                },
                Some(PendingMutation::PreampPhase {
                    input: 1,
                    enabled: true,
                }),
            )
            .expect("send preamp phase");
        controller.confirm_pending_write();
        assert_eq!(controller.state.device.dsp_cluster[3], 0x40);
    }

    #[test]
    fn apply_profile_updates_known_controls_and_writes_commands() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        let profile = DeviceProfile {
            outputs: OutputProfiles {
                monitor: OutputProfile {
                    volume_step: 0x12,
                    mode: OutputModeProfile::Dim,
                },
                hp1: OutputProfile {
                    volume_step: 0x24,
                    mode: OutputModeProfile::Mute,
                },
                hp2: OutputProfile {
                    volume_step: 0x08,
                    mode: OutputModeProfile::Normal,
                },
            },
            preamps: PreampProfiles {
                input1: PreampInputProfile {
                    gain_raw: 0x20,
                    mode: PreampModeProfile::Mic,
                    phantom_on: true,
                    phase_inverted: true,
                },
                input2: PreampInputProfile {
                    gain_raw: 0x10,
                    mode: PreampModeProfile::Line,
                    phantom_on: false,
                    phase_inverted: false,
                },
            },
            assignments: (1..=16)
                .map(|channel| MixerAssignmentEntry {
                    channel,
                    source: if channel == 1 {
                        MixerAssignmentProfile::Preamp(1)
                    } else {
                        MixerAssignmentProfile::Mute
                    },
                })
                .collect(),
            mixers: MixerProfiles {
                mix1: (1..=16)
                    .map(|channel| MixerStripProfile {
                        channel,
                        level_raw: channel - 1,
                        pan_raw: if channel == 1 {
                            PanState::right().raw()
                        } else {
                            PanState::center().raw()
                        },
                        muted: channel % 2 == 0,
                        soloed: channel == 2,
                        linked: channel <= 2,
                    })
                    .collect(),
                mix2: (1..=16)
                    .map(|channel| MixerStripProfile {
                        channel,
                        level_raw: 0x30,
                        pan_raw: PanState::left().raw(),
                        muted: false,
                        soloed: false,
                        linked: false,
                    })
                    .collect(),
            },
        };

        controller.apply_profile(&profile).expect("apply profile");

        assert_eq!(controller.state.output.states[0].volume, 0x12);
        assert_eq!(controller.state.output.states[0].mode, OutputMode::Dim);
        assert_eq!(controller.state.output.states[1].mode, OutputMode::Mute);
        assert_eq!(controller.state.preamp.state.input1.mode, PreampMode::Mic);
        assert!(controller.state.preamp.state.input1.phantom_on);
        assert_eq!(controller.state.preamp.state.input1.mode_raw & 0x40, 0x40);
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].assignment,
            Some(MixerAssignment::Preamp(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::right()
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].pan,
            PanState::left()
        );
        assert!(!transport.take_writes().is_empty());
    }

    #[test]
    fn bootstrap_sends_queries_and_mutations_use_transport() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(i32::from(ClockSource::Usb.code())),
                },
                None,
            )
            .expect("write command");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[1][0x08..0x10], &[0x11, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[2][0x08..0x10], &[0x0a, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[46][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x10..0x12], &[0x04, 0x02]);
    }

    #[test]
    fn clock_source_command_updates_visible_state_immediately() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));
        controller.state.device.status.clock_source = Some(ClockSource::Usb);

        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(i32::from(ClockSource::Internal.code())),
                },
                None,
            )
            .expect("set clock source");

        assert_eq!(
            controller.state.device.status.clock_source,
            Some(ClockSource::Internal)
        );
    }

    #[test]
    fn bootstrap_queries_include_metadata_request() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");

        let writes = transport.take_writes();
        assert!(writes
            .iter()
            .any(|frame| frame[0x08..0x10] == [0x01, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn surface_select_refreshes_query_readback() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::Surface,
                    value: ControlValue::Enum(i32::from(Surface::Hp2.code())),
                },
                None,
            )
            .expect("select surface");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x10..0x13], &[0x49, 0x00, Surface::Hp2.code()]);
        assert_eq!(&writes[1][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn clock_source_change_does_not_force_refresh_query_readback() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(i32::from(ClockSource::Usb.code())),
                },
                None,
            )
            .expect("set clock source");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x04, 0x02]);
    }

    #[test]
    fn sample_rate_change_does_not_force_refresh_query_readback() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::SampleRate,
                    value: ControlValue::Enum(i32::from(SampleRate::Hz96000.code())),
                },
                None,
            )
            .expect("set sample rate");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x03, 0x04]);
    }

    #[test]
    fn mixer_overlay_is_tracked_only_after_command_round_trip() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (antelope_protocol::MixerSurface::Mix1).code(),
                        strip: 3,
                    },
                    fader: 0x2c,
                    pan: i32::from((antelope_protocol::PanState::left()).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan: antelope_protocol::PanState::left(),
                    muted: false,
                }),
            )
            .expect("send mixer");

        assert!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2]
                .level
                .is_none()
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2],
            MixerChannelState::known(3, Some(0x2c), Some(false), PanState::left(), None, None)
        );
    }

    #[test]
    fn linked_mixer_level_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();
        seed_complete_dynamic_mixer(&mut controller, MixerSurface::Mix1, &[3, 4]);

        controller
            .send_mixer_level_change(MixerSurface::Mix1, 4, 0x2c)
            .expect("send linked mixer level");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x2c, 0x02]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x2c, 0x3e]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan,
            PanState::left()
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan,
            PanState::right()
        );
    }

    #[test]
    fn linked_mixer_mute_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();
        seed_complete_dynamic_mixer(&mut controller, MixerSurface::Mix1, &[3, 4]);

        controller
            .send_mixer_mute_change(MixerSurface::Mix1, 3, true)
            .expect("send linked mixer mute");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x42]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0x7e]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].muted,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].muted,
            Some(true)
        );
    }

    #[test]
    fn linked_mixer_solo_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].muted = Some(false);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].muted = Some(false);
        seed_complete_dynamic_mixer(&mut controller, MixerSurface::Mix1, &[3, 4]);

        controller
            .send_mixer_solo_change(MixerSurface::Mix1, 4, true)
            .expect("send linked mixer solo");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x82]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0xbe]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].soloed,
            Some(true)
        );
    }

    #[test]
    fn queried_mixer_strip_readback_updates_solo_state() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));
        let mut body = [0x5a, 0x20].repeat(32);
        body[0] = 0x10;
        body[1] = 0xa0;
        body[32] = 0x10;
        body[33] = 0x20;

        controller.state.observe_frame(
            DeviceSnapshot::QueryReply(QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body,
            }),
            raw_frame(&[0x75, 0x18, 0x00]),
        );

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].soloed,
            Some(false)
        );
    }

    #[test]
    fn grounded_link_target_maps_extended_pair_selectors() {
        let mix1 = MixerLinkTarget::from_channel(MixerSurface::Mix1, 11).expect("mix1 target");
        assert_eq!(
            (mix1.left_channel, mix1.right_channel, mix1.selector),
            (11, 12, 0x05)
        );
        assert_eq!(mix1.companion_bank(), None);

        let mix2 = MixerLinkTarget::from_channel(MixerSurface::Mix2, 15).expect("mix2 target");
        assert_eq!(
            (mix2.left_channel, mix2.right_channel, mix2.selector),
            (15, 16, 0x17)
        );
        assert_eq!(mix2.companion_bank(), None);
    }

    #[test]
    fn mixer_link_change_updates_pair_before_device_confirmation() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller
            .send_mixer_link_change(MixerSurface::Mix1, 1, true)
            .expect("send mixer link");

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn mixer_link_change_writes_selector_and_updates_pair() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        controller
            .send_mixer_link_change(MixerSurface::Mix1, 11, true)
            .expect("send mix1 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x05, 0x01]);
        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][11].linked,
            Some(true)
        );

        controller
            .send_mixer_link_change(MixerSurface::Mix2, 15, true)
            .expect("send mix2 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x17, 0x01]);
        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][14].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][15].linked,
            Some(true)
        );
    }

    #[test]
    fn app_state_starts_with_16_strips_per_surface() {
        let state = AppState::default();

        assert_eq!(state.mixer.channels[MixerSurface::Mix1.index()].len(), 16);
        assert_eq!(state.mixer.channels[MixerSurface::Mix2.index()].len(), 16);
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][15].channel,
            16
        );
    }

    #[test]
    fn mixer_assignment_is_shared_across_surfaces_but_link_is_not() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Oscillator(2),
        });
        controller.confirm_pending_write();

        let target = MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2");
        controller
            .send(
                Action::SetLink {
                    surface: target.mixer.code(),
                    pair: u16::from((target.left_channel - 1) / 2),
                    enabled: true,
                },
                Some(PendingMutation::MixerLink {
                    mixer: MixerSurface::Mix2,
                    selector: target.selector,
                    enabled: true,
                }),
            )
            .expect("send link");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][1].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            None
        );
    }

    #[test]
    fn mixer_assignment_overlay_updates_both_surfaces_for_strip_11() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Mute,
        });
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
    }

    #[test]
    fn normalized_zen_assignment_write_updates_legacy_state_without_route_group() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        for channels in &mut controller.state.mixer.channels {
            for channel in channels {
                channel.assignment = Some(MixerAssignment::Mute);
            }
        }
        controller
            .state
            .routing
            .retain(|group| group.destination != 0);
        assert!(controller.state.routing_group(0).is_none());

        let mut assignments = [MixerAssignment::Mute; 16];
        assignments[4] = MixerAssignment::Oscillator(1);
        controller
            .send(
                Action::SetRoutingGroup {
                    destination: 0,
                    changed_channel: Some(4),
                    sources: assignments
                        .into_iter()
                        .map(super::controller::routing_source_from_assignment)
                        .collect(),
                },
                None,
            )
            .expect("normalized Zen assignment write should succeed");

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(transport.take_writes().len(), 5);
    }

    #[test]
    fn mixer_assignment_write_sends_ordinary_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(
                routing_group_action(
                    5,
                    MixerAssignment::Oscillator(1),
                    [MixerAssignment::Mute; 16],
                ),
                Some(PendingMutation::MixerAssignment {
                    strip: 5,
                    assignment: MixerAssignment::Oscillator(1),
                }),
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn mixer_assignment_write_sends_early_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(
                routing_group_action(
                    1,
                    MixerAssignment::Oscillator(1),
                    [MixerAssignment::Mute; 16],
                ),
                Some(PendingMutation::MixerAssignment {
                    strip: 1,
                    assignment: MixerAssignment::Oscillator(1),
                }),
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&writes[0][0x10 + 0x03..0x10 + 0x05], &[0x09, 0x00]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn late_strip_assignment_write_preserves_existing_assignment_table_entries() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        let assignments = seed_shared_assignments(&mut controller.state);

        controller
            .send(
                routing_group_action(11, MixerAssignment::ComputerPlay(1), assignments),
                None,
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        let bank06 = writes
            .iter()
            .find(|frame| frame[0x10..0x13] == [0xd3, 0x41, 0x06])
            .expect("bank 06 frame");

        assert_eq!(
            assignment_pairs(bank06, 16),
            vec![
                [0x03, 0x00],
                [0x03, 0x01],
                [0x03, 0x02],
                [0x03, 0x03],
                [0x01, 0x02],
                [0x01, 0x03],
                [0x01, 0x04],
                [0x01, 0x05],
                [0x01, 0x06],
                [0x01, 0x07],
                [0x01, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
            ]
        );
    }

    #[test]
    fn link_overlay_respects_full_visible_pair_mapping() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        for target in [
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 5).expect("mix1 5-6"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("mix1 7-8"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 7).expect("mix2 7-8"),
        ] {
            controller
                .send(
                    Action::SetLink {
                        surface: target.mixer.code(),
                        pair: u16::from((target.left_channel - 1) / 2),
                        enabled: true,
                    },
                    Some(PendingMutation::MixerLink {
                        mixer: target.mixer,
                        selector: target.selector,
                        enabled: true,
                    }),
                )
                .expect("send grounded link");
            controller.confirm_pending_write();

            assert_eq!(
                controller.state.mixer.channels[target.mixer.index()]
                    [target.left_channel as usize - 1]
                    .linked,
                Some(true)
            );
            assert_eq!(
                controller.state.mixer.channels[target.mixer.index()]
                    [target.right_channel as usize - 1]
                    .linked,
                Some(true)
            );
        }
        assert!(MixerStrip::ordinary(4).is_none());
    }

    #[test]
    fn grounded_link_with_companion_writes_helper_before_selector_write() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2");

        controller
            .send(
                Action::SetLink {
                    surface: target.mixer.code(),
                    pair: u16::from((target.left_channel - 1) / 2),
                    enabled: true,
                },
                Some(PendingMutation::MixerLink {
                    mixer: MixerSurface::Mix1,
                    selector: target.selector,
                    enabled: true,
                }),
            )
            .expect("send link with companion");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x04, 0x00, 0x01]);
        assert_eq!(&writes[1][0x10..0x14], &[0xa2, 0x03, 0x00, 0x01]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn mixer_pan_updates_are_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (MixerSurface::Mix1).code(),
                        strip: 4,
                    },
                    fader: 0,
                    pan: i32::from((PanState::from_raw(0x08)).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerPan {
                    mixer: MixerSurface::Mix1,
                    channel: 4,
                    pan: PanState::from_raw(0x08),
                }),
            )
            .expect("mix1 pan");
        controller.confirm_pending_write();

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (MixerSurface::Mix2).code(),
                        strip: 4,
                    },
                    fader: 0,
                    pan: i32::from((PanState::from_raw(0x36)).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerPan {
                    mixer: MixerSurface::Mix2,
                    channel: 4,
                    pan: PanState::from_raw(0x36),
                }),
            )
            .expect("mix2 pan");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3]
                .pan
                .raw(),
            0x08
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][3]
                .pan
                .raw(),
            0x36
        );
    }

    #[test]
    fn mixer_mute_does_not_invent_zero_level_for_undecoded_channel() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (antelope_protocol::MixerSurface::Mix1).code(),
                        strip: 7,
                    },
                    fader: 0,
                    pan: i32::from((antelope_protocol::PanState::center()).raw()),
                    muted: true,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerMute {
                    mixer: MixerSurface::Mix1,
                    channel: 7,
                    muted: true,
                }),
            )
            .expect("send mute");

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].muted,
            Some(true)
        );

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (antelope_protocol::MixerSurface::Mix1).code(),
                        strip: 7,
                    },
                    fader: 0,
                    pan: i32::from((antelope_protocol::PanState::center()).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerMute {
                    mixer: MixerSurface::Mix1,
                    channel: 7,
                    muted: false,
                }),
            )
            .expect("send unmute");

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].muted,
            Some(false)
        );
    }

    #[test]
    fn mixer_state_is_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (MixerSurface::Mix1).code(),
                        strip: 3,
                    },
                    fader: 0x2c,
                    pan: i32::from((antelope_protocol::PanState::center()).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan: antelope_protocol::PanState::center(),
                    muted: false,
                }),
            )
            .expect("mix1 send");
        controller.confirm_pending_write();

        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: (MixerSurface::Mix2).code(),
                        strip: 3,
                    },
                    fader: 0x10,
                    pan: i32::from((antelope_protocol::PanState::center()).raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix2,
                    channel: 3,
                    level: 0x10,
                    pan: antelope_protocol::PanState::center(),
                    muted: false,
                }),
            )
            .expect("mix2 send");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][2].level,
            Some(0x10)
        );
    }

    #[test]
    fn mixer_first_adjustment_starts_from_safe_midpoint_not_minimum() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Mixer;
        controller.state.mixer.selected_channel = 0;

        let channel = controller.state.active_mixer_channels()[0].channel;
        controller
            .send(
                Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: MixerSurface::from_surface(controller.state.mixer.surface).code(),
                        strip: u16::from(channel),
                    },
                    fader: 0x1f,
                    pan: i32::from(antelope_protocol::PanState::center().raw()),
                    muted: false,
                    soloed: false,
                    send: None,
                },
                None,
            )
            .expect("send first adjustment");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        let mixer_write = writes.last().expect("mixer write");
        assert_eq!(
            &mixer_write[0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x1f, 0x20]
        );
    }

    #[test]
    fn connection_status_changes_when_frames_arrive() {
        let mut state = AppState::default();
        state.mark_disconnected();
        assert!(!state.device.connection.connected);

        state.observe_frame(
            DeviceSnapshot::Snapshot(snapshot()),
            raw_frame(&[0x73, 0, 0, 0]),
        );

        assert!(state.device.connection.connected);
        assert!(state.device.connection.last_snapshot_at.is_some());
    }

    #[test]
    fn identical_snapshot_does_not_report_visible_change_twice() {
        let mut state = AppState::default();
        let raw = raw_frame(&[0x73, 0, 0, 0]);

        assert!(state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw));
        assert!(!state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw));
    }

    #[test]
    fn raw_only_snapshot_difference_is_not_visible_when_raw_view_is_closed() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.latest_structural_snapshot = Some(StructuralSnapshot::from_snapshot(&snapshot()));
        state.raw_view.latest_raw_73 = Some(raw_frame(&[0x73, 0, 0, 0]).to_vec());

        assert!(!state.observe_frame(
            DeviceSnapshot::Snapshot(snapshot()),
            raw_frame(&[0x73, 0, 0, 1])
        ));
    }

    #[test]
    fn raw_only_snapshot_difference_is_visible_when_raw_view_is_open() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.popup.raw_view_open = true;
        state.latest_structural_snapshot = Some(StructuralSnapshot::from_snapshot(&snapshot()));
        state.raw_view.latest_raw_73 = Some(raw_frame(&[0x73, 0, 0, 0]).to_vec());

        assert!(state.observe_frame(
            DeviceSnapshot::Snapshot(snapshot()),
            raw_frame(&[0x73, 0, 0, 1])
        ));
    }

    #[test]
    fn auxiliary_frame_is_not_visible_when_raw_view_is_closed() {
        let mut state = AppState::default();
        state.device.connection.connected = true;

        assert!(!state.observe_frame(
            DeviceSnapshot::Auxiliary(aux_frame(&[0x60, 0xc0, 0x60, 0x00])),
            raw_frame(&[0x83, 0, 0, 0]),
        ));
    }

    #[test]
    fn auxiliary_frame_is_visible_when_raw_view_is_open() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.popup.raw_view_open = true;

        assert!(state.observe_frame(
            DeviceSnapshot::Auxiliary(aux_frame(&[0x60, 0xc0, 0x60, 0x00])),
            raw_frame(&[0x83, 0, 0, 0]),
        ));
    }

    #[test]
    fn poll_device_does_not_mark_identical_snapshot_dirty_when_view_is_unchanged() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));
        let raw = snapshot_frame_bytes(0x12);
        let snapshot = Frame::parse(&raw)
            .expect("snapshot frame")
            .as_snapshot()
            .expect("snapshot")
            .clone();
        controller.state.device.connection.connected = true;
        controller.state.latest_structural_snapshot =
            Some(StructuralSnapshot::from_snapshot(&snapshot));
        controller.state.raw_view.latest_raw_73 = Some(raw.to_vec());
        controller.state.apply_snapshot(&snapshot);

        transport.push_read(raw.to_vec());

        assert!(!controller.poll_device(Duration::ZERO).expect("poll"));
    }

    #[test]
    #[ignore = "benchmark"]
    fn perf_poll_device_snapshot_backlog() {
        const FRAMES: usize = 20_000;
        let polls = FRAMES.div_ceil(MAX_FRAMES_PER_POLL) + 1;

        let transport = MockTransport::default();
        for meter in 0..FRAMES {
            transport.push_read(snapshot_frame_bytes((meter % 0x3d) as u8).to_vec());
        }

        let mut controller = zen_go_controller(Box::new(transport));
        let started = Instant::now();
        let mut dirty_polls = 0_usize;
        for _ in 0..polls {
            dirty_polls += usize::from(controller.poll_device(Duration::ZERO).expect("poll"));
        }
        let elapsed = started.elapsed();

        println!(
            "poll_device backlog: frames={FRAMES} polls={polls} dirty_polls={dirty_polls} elapsed_ms={} ns_per_frame={}",
            elapsed.as_millis(),
            elapsed.as_nanos() / FRAMES as u128
        );
    }

    #[test]
    fn poll_device_drains_backlog_to_latest_snapshot() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport.clone()));

        let mut first = raw_frame(&[]);
        first[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        first[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let first_payload = &mut first[0x10..];
        first_payload[0x02] = SampleRate::Hz44100.code();
        first_payload[0x03] = ClockSource::Internal.code();
        first_payload[0x04..0x08].copy_from_slice(&44_100_u32.to_be_bytes());

        let mut second = raw_frame(&[]);
        second[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        second[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let second_payload = &mut second[0x10..];
        second_payload[0x02] = SampleRate::Hz48000.code();
        second_payload[0x03] = ClockSource::Usb.code();
        second_payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());

        transport.push_read(first.to_vec());
        transport.push_read(second.to_vec());

        let observed_frame = controller.poll_device(Duration::ZERO).expect("poll");

        assert!(observed_frame);
        assert_eq!(
            controller.state.device.status.sample_rate,
            Some(SampleRate::Hz48000)
        );
        assert_eq!(
            controller.state.device.status.clock_source,
            Some(ClockSource::Usb)
        );
    }

    #[test]
    fn poll_device_reports_idle_reads_without_marking_state_dirty() {
        let transport = MockTransport::default();
        let mut controller = zen_go_controller(Box::new(transport));

        let observed_frame = controller.poll_device(Duration::ZERO).expect("idle poll");

        assert!(!observed_frame);
    }

    #[test]
    fn raw_state_tracks_latest_snapshot_and_auxiliary_frames() {
        let mut state = AppState::default();
        let mut raw73 = raw_frame(&[0x73]);
        raw73[0x10 + 0xcf] = 0x4c;
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw73);

        let raw83 = raw_frame(&[0x83, 0, 0, 0, 0x60, 0xc0, 0x60, 0x00]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary(aux_frame(&[0x60, 0xc0, 0x60, 0x00])),
            raw83,
        );

        assert!(state.raw_view.latest_raw_73.is_some());
        assert!(state.raw_view.latest_raw_83.is_some());
        assert_eq!(
            state.raw_view.latest_raw_73.as_ref().expect("0x73")[0x10 + 0xcf],
            0x4c
        );
        assert_eq!(
            &state.raw_view.latest_raw_83.as_ref().expect("0x83")[0..4],
            &raw83[0..4]
        );

        let raw75 = raw_frame(&[
            0x75, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, b'Z',
        ]);
        let raw74 = raw_frame(&[0x74, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x11, 0, 0, 0, 0x03]);
        state.observe_query_request(raw74);
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x01,
                sub_id: 0x00,
                body: vec![b'Z'],
            }),
            raw75,
        );

        let raw81 = raw_frame(&[0x81, 0x10, 0x20, 0x30, 0x40, 0x50]);
        state.observe_frame(
            DeviceSnapshot::Notification(antelope_protocol::DeviceNotification {
                bytes: [0x81, 0x10, 0x20, 0x30, 0x40, 0x50],
            }),
            raw81,
        );

        assert_eq!(state.raw_view.latest_raw_75, Some(raw75.to_vec()));
        assert_eq!(state.raw_view.latest_raw_81, Some(raw81.to_vec()));
        assert_eq!(state.raw_view.latest_raw_74, Some(raw74.to_vec()));
        assert_eq!(state.raw_view.recent_query_request_log.len(), 1);
        assert!(state.raw_view.recent_query_request_log[0].contains("0x74 11/03"));
        assert_eq!(state.raw_view.recent_query_reply_log.len(), 1);
        assert!(state.raw_view.recent_query_reply_log[0].contains("0x75 01/00"));
        assert_eq!(
            state.startup_query_summary(0x01),
            Some("Metadata: undecoded")
        );
    }

    #[test]
    fn raw_baseline_captures_latest_packets() {
        let mut state = AppState::default();
        state.observe_frame(
            DeviceSnapshot::Snapshot(snapshot()),
            raw_frame(&[0x73, 0, 0, 0]),
        );
        let mut aux_bytes = [0_u8; 320];
        aux_bytes[..4].copy_from_slice(&[0x60, 0xc0, 0x60, 0x00]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary(aux_bytes),
            raw_frame(&[0x83, 0, 0, 0]),
        );
        state.observe_query_request(raw_frame(&[0x74, 0, 0, 0]));
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::Notification(antelope_protocol::DeviceNotification {
                bytes: [1, 2, 3, 4, 5, 6],
            }),
            raw_frame(&[1, 2, 3, 4, 5, 6]),
        );

        state.capture_raw_baseline();
        assert_eq!(state.raw_view.baseline_raw_73, state.raw_view.latest_raw_73);
        assert_eq!(state.raw_view.baseline_raw_83, state.raw_view.latest_raw_83);
        assert_eq!(state.raw_view.baseline_raw_74, state.raw_view.latest_raw_74);
        assert_eq!(state.raw_view.baseline_raw_75, state.raw_view.latest_raw_75);
        assert_eq!(state.raw_view.baseline_raw_81, state.raw_view.latest_raw_81);

        state.clear_raw_baseline();
        assert!(state.raw_view.baseline_raw_73.is_none());
        assert!(state.raw_view.baseline_raw_83.is_none());
        assert!(state.raw_view.baseline_raw_74.is_none());
        assert!(state.raw_view.baseline_raw_75.is_none());
        assert!(state.raw_view.baseline_raw_81.is_none());
    }

    #[test]
    fn stores_grounded_startup_query_summaries_for_all_bootstrap_replies() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x00,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb, 0xcc],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0x12],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        assert_eq!(
            state.startup_query_summary(0x00),
            Some("Capability/default block: 3 bytes [aa bb cc]")
        );
        assert_eq!(
            state.startup_query_summary(0x11),
            Some("Status/capability value: 1 bytes [12]")
        );
    }

    #[test]
    fn metadata_reply_updates_serial_and_hardware_version() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x01,
                sub_id: 0x00,
                body: [
                    b"Zen Go Synergy Core".as_slice(),
                    b"\0".as_slice(),
                    b"4502721001300".as_slice(),
                    b"\0".as_slice(),
                    b"6.6".as_slice(),
                    b"\0".as_slice(),
                ]
                .concat(),
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        let metadata = state.device.status.metadata.expect("metadata");
        assert_eq!(metadata.product_name, "Zen Go Synergy Core");
        assert_eq!(metadata.serial, "4502721001300");
        assert_eq!(metadata.hardware_version, "6.6");
        assert_eq!(
            state.ui.last_message,
            "Connected to Zen Go Synergy Core (hw 6.6, serial 4502721001300)"
        );
    }

    #[test]
    fn query_reply_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id, 0xaa],
                }),
                raw_frame(&[0x75, 0, 0, 0]),
            );
        }

        assert_eq!(state.raw_view.recent_query_reply_log.len(), 16);
        assert!(state
            .raw_view
            .recent_query_reply_log
            .first()
            .unwrap()
            .contains("0x75 03/04"));
        assert!(state
            .raw_view
            .recent_query_reply_log
            .last()
            .unwrap()
            .contains("0x75 03/13"));
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(15));
    }

    #[test]
    fn query_reply_log_surfaces_selector_family_summaries() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00,
                    0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00,
                    0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                    0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                ],
            }),
            raw_frame(&[0x75, 0, 0, 0]),
        );

        assert!(state.raw_view.recent_query_reply_log[0].contains("Selector bitmap"));
        assert!(state.raw_view.recent_query_reply_log[1].contains("Startup Mix2 pan categories"));
    }

    #[test]
    fn selected_query_reply_entry_tracks_latest_reply_and_cycles() {
        let mut state = AppState::default();
        for sub_id in 0..3_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id],
                }),
                raw_frame(&[0x75, sub_id]),
            );
        }

        assert_eq!(state.raw_view.selected_query_reply_entry, Some(2));
        assert_eq!(
            state
                .selected_query_reply_entry()
                .map(|entry| entry.raw.clone()),
            Some(raw_frame(&[0x75, 0x02]).to_vec())
        );

        state.cycle_query_reply_entry(false);
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(1));
        state.cycle_query_reply_entry(true);
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(2));
    }

    #[test]
    fn query_request_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_query_request(raw_frame(&[
                0x74, 0, 0, 0, 0, 0, 0, 0, 0x03, 0, 0, 0, sub_id,
            ]));
        }

        assert_eq!(state.raw_view.recent_query_request_log.len(), 16);
        assert!(state
            .raw_view
            .recent_query_request_log
            .first()
            .unwrap()
            .contains("0x74 03/04"));
        assert!(state
            .raw_view
            .recent_query_request_log
            .last()
            .unwrap()
            .contains("0x74 03/13"));
    }

    #[test]
    fn focus_cycle_skips_raw_view_state() {
        let mut state = AppState::default();
        state.ui.focus = FocusArea::Status;

        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Outputs);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Mixer);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Preamp);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Outputs);
    }

    #[test]
    fn raw_view_toggle_and_packet_tab_cycle_are_independent_of_focus() {
        let mut state = AppState::default();

        state.toggle_raw_view();
        assert!(state.popup.raw_view_open);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::State73);

        state.cycle_raw_packet(true);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::Auxiliary);
        state.cycle_raw_packet(false);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::State73);

        state.toggle_raw_view();
        assert!(!state.popup.raw_view_open);
    }

    #[test]
    fn ensure_selected_mixer_channel_visible_advances_scroll_window() {
        let mut state = AppState::default();
        state.mixer.selected_channel = 6;

        state.ensure_selected_mixer_channel_visible(4);

        assert_eq!(state.mixer.strip_scroll, 3);
    }

    #[test]
    fn mixer_strip_viewport_scroll_clamps_to_available_channels() {
        let mut state = AppState::default();

        state.scroll_mixer_strip_viewport(99, 5);
        assert_eq!(state.mixer.strip_scroll, 11);

        state.scroll_mixer_strip_viewport(-99, 5);
        assert_eq!(state.mixer.strip_scroll, 0);
    }

    #[test]
    fn mixer_strip_viewport_paging_moves_between_banks() {
        let mut state = AppState::default();
        let page = 8;

        state.page_mixer_strip_viewport(true, page);
        assert_eq!(state.mixer.strip_scroll, 8);

        state.page_mixer_strip_viewport(true, page);
        assert_eq!(state.mixer.strip_scroll, 8);

        state.page_mixer_strip_viewport(false, page);
        assert_eq!(state.mixer.strip_scroll, 0);
    }
}
