use std::time::Duration;

use anyhow::{bail, Result};
use ratatui::layout::Rect;

use crate::command_queue::CommandQueue;
use crate::profile::DeviceProfile;
use crate::transport::Transport;
use antelope_protocol::{
    Action, ClockSource, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, GlobalControl,
    InputAddress, InputControl, MixerAddress, MixerAssignment, MixerControl, MixerSurface,
    OutputAddress, OutputControl, OutputMode, PanState, PreampMode, QueryRequest, RoutingSource,
    RuntimeEntry, SampleRate, Surface,
};

use super::picker::{AssignmentPickerState, SelectorPopupKind, SelectorPopupState};
use super::profile_editor::{ProfileEditorMode, ProfileEditorState};
use super::types::{
    FocusArea, Intent, PeakHoldDuration, PendingMutation, RawMapScope, RawPacketTab, RefreshRate,
};
use super::AppState;

pub(crate) const MAX_FRAMES_PER_POLL: usize = 32;

pub struct Controller {
    transport: Box<dyn Transport>,
    driver: Box<dyn DeviceDriver>,
    pub state: AppState,
    pub(crate) pending_mutation: Option<PendingMutation>,
    command_queue: CommandQueue,
}

impl Controller {
    pub fn new(transport: Box<dyn Transport>, driver: Box<dyn DeviceDriver>) -> Result<Self> {
        if !driver.definition().supported {
            bail!("driver {} is unsupported", driver.definition().name);
        }

        let catalog = crate::device::ProfileCatalog::builtin();
        let Some(entry) = catalog.find(driver.definition().vid, driver.definition().pid) else {
            // Unknown drivers remain available for protocol-fixture tests; runtime sessions must
            // use `new_for_entry` so selected profile topology cannot be replaced by this state.
            return Ok(Self {
                transport,
                driver,
                state: AppState::default(),
                pending_mutation: None,
                command_queue: CommandQueue::new(),
            });
        };
        Self::new_for_entry(transport, driver, entry)
    }

    pub fn new_for_entry(
        transport: Box<dyn Transport>,
        driver: Box<dyn DeviceDriver>,
        entry: &RuntimeEntry,
    ) -> Result<Self> {
        if !driver.definition().supported {
            bail!("driver {} is unsupported", driver.definition().name);
        }
        if (driver.definition().vid, driver.definition().pid)
            != (entry.profile.identity.vid, entry.profile.identity.pid)
        {
            bail!(
                "driver {} identity does not match selected profile {}",
                driver.definition().name,
                entry.profile.identity.name
            );
        }
        Ok(Self {
            transport,
            driver,
            state: AppState::from_entry(entry),
            pending_mutation: None,
            command_queue: CommandQueue::new(),
        })
    }

    pub fn driver_definition(&self) -> &antelope_protocol::DriverDefinition {
        self.driver.definition()
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.refresh_queried_state()
    }

    pub fn transport_available(&self) -> Result<bool> {
        self.transport.is_available()
    }

    pub fn refresh_queried_state(&mut self) -> Result<()> {
        let queries = self.driver.startup_requests().to_vec();
        for query in queries {
            self.write_query(query)?;
        }
        Ok(())
    }

    fn write_query(&mut self, query: QueryRequest) -> Result<()> {
        let batch = self.driver.encode(Action::Query(query))?;
        for frame in batch.frames {
            self.state.observe_query_request(&frame);
            self.transport.write(&frame)?;
        }
        for refresh_query in batch.refresh_requests {
            self.write_query(refresh_query)?;
        }
        Ok(())
    }

    fn write_batch(&mut self, batch: CommandBatch) -> Result<()> {
        for frame in batch.frames {
            self.transport.write(&frame)?;
        }
        for query in batch.refresh_requests {
            self.write_query(query)?;
        }
        Ok(())
    }

    pub fn apply_profile(&mut self, profile: &DeviceProfile) -> Result<()> {
        profile.validate()?;
        let mut actions = Vec::new();

        let saved_outputs = [
            ("monitor", &profile.outputs.monitor),
            ("hp1", &profile.outputs.hp1),
            ("hp2", &profile.outputs.hp2),
        ];
        for (index, (name, saved)) in saved_outputs.into_iter().enumerate() {
            let address = self
                .state
                .outputs()
                .get(index)
                .map(|output| output.address)
                .ok_or_else(|| anyhow::anyhow!("saved control outputs.{name} is unavailable"))?;
            actions.push(Action::SetOutput {
                address,
                control: OutputControl::Level,
                value: ControlValue::Int(i32::from(saved.volume_step)),
            });
            actions.push(Action::SetOutput {
                address,
                control: OutputControl::Dim,
                value: ControlValue::Bool(false),
            });
            actions.push(Action::SetOutput {
                address,
                control: OutputControl::Mute,
                value: ControlValue::Bool(false),
            });
            match saved.mode.into_device() {
                OutputMode::Normal => {}
                OutputMode::Mute => actions.push(Action::SetOutput {
                    address,
                    control: OutputControl::Mute,
                    value: ControlValue::Bool(true),
                }),
                OutputMode::Dim => actions.push(Action::SetOutput {
                    address,
                    control: OutputControl::Dim,
                    value: ControlValue::Bool(true),
                }),
                OutputMode::Unknown(_) => unreachable!(),
            }
        }

        let input_space = self
            .state
            .input_spaces
            .first()
            .ok_or_else(|| anyhow::anyhow!("saved control preamps.input1 is unavailable"))?;
        for (index, (name, saved)) in [
            ("input1", &profile.preamps.input1),
            ("input2", &profile.preamps.input2),
        ]
        .into_iter()
        .enumerate()
        {
            let address = input_space
                .inputs
                .get(index)
                .map(|input| input.address)
                .ok_or_else(|| anyhow::anyhow!("saved control preamps.{name} is unavailable"))?;
            actions.extend([
                Action::SetInput {
                    address,
                    control: InputControl::Mode,
                    value: ControlValue::Enum(i32::from(saved.mode.into_device().code())),
                },
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(i32::from(saved.gain_raw)),
                },
                Action::SetInput {
                    address,
                    control: InputControl::Phantom,
                    value: ControlValue::Bool(saved.phantom_on),
                },
                Action::SetInput {
                    address,
                    control: InputControl::Phase,
                    value: ControlValue::Bool(saved.phase_inverted),
                },
            ]);
        }

        let assignments = profile.assignment_table()?;
        // Legacy Zen Go profiles address its fixed 16-channel assignment table
        // as destination 0.  Raw profile evidence retains only partial logical
        // routing groups, so destination 0 is intentionally absent from the
        // normalized catalog; use driver's validated legacy shape in that path.
        let routing_channel_count = self
            .routing_channel_count(0)
            .ok_or_else(|| anyhow::anyhow!("saved control assignments is unavailable"))?;
        if routing_channel_count != assignments.len() {
            bail!(
                "saved control assignments requires {} channels, profile exposes {}",
                assignments.len(),
                routing_channel_count
            );
        }
        let sources: Vec<_> = assignments
            .into_iter()
            .map(routing_source_from_assignment)
            .collect();
        for entry in &profile.assignments {
            let changed_channel = u16::from(
                entry
                    .channel
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("saved assignment channel is invalid"))?,
            );
            actions.push(Action::SetRoutingGroup {
                destination: 0,
                changed_channel: Some(changed_channel),
                sources: sources.clone(),
            });
        }

        for (surface_index, strips) in [&profile.mixers.mix1, &profile.mixers.mix2]
            .into_iter()
            .enumerate()
        {
            let surface = self.state.mixers().get(surface_index).ok_or_else(|| {
                anyhow::anyhow!(
                    "saved control mixers.mix{} is unavailable",
                    surface_index + 1
                )
            })?;
            for strip in strips.iter().step_by(2) {
                let strip_index = usize::from(
                    strip
                        .channel
                        .checked_sub(1)
                        .ok_or_else(|| anyhow::anyhow!("saved mixer channel is invalid"))?,
                );
                if surface.strips.get(strip_index + 1).is_none() {
                    bail!(
                        "saved control mixers.mix{}.link{} is unavailable",
                        surface_index + 1,
                        strip.channel
                    );
                }
                actions.push(Action::SetLink {
                    surface: surface.surface,
                    pair: u16::try_from(strip_index / 2)
                        .map_err(|_| anyhow::anyhow!("saved mixer link index overflow"))?,
                    enabled: strip.linked,
                });
            }
            for strip in strips {
                let strip_index = usize::from(
                    strip
                        .channel
                        .checked_sub(1)
                        .ok_or_else(|| anyhow::anyhow!("saved mixer channel is invalid"))?,
                );
                let current = surface.strips.get(strip_index).ok_or_else(|| {
                    anyhow::anyhow!(
                        "saved control mixers.mix{}.strip{} is unavailable",
                        surface_index + 1,
                        strip.channel
                    )
                })?;
                let send = if self.state.mixer_send_surfaces.contains(&surface.surface) {
                    Some(current.send.ok_or_else(|| {
                        anyhow::anyhow!(
                            "saved control mixers.mix{}.strip{}.send state is unavailable",
                            surface_index + 1,
                            strip.channel
                        )
                    })?)
                } else {
                    None
                };
                actions.push(Action::SetMixerStripState {
                    address: MixerAddress {
                        surface: surface.surface,
                        strip: current.strip,
                    },
                    fader: i32::from(strip.level_raw),
                    pan: i32::from(PanState::from_raw(strip.pan_raw).raw()),
                    muted: strip.muted,
                    soloed: strip.soloed,
                    send,
                });
            }
        }

        let mut batches = Vec::with_capacity(actions.len());
        for (index, action) in actions.iter().enumerate() {
            batches.push(self.driver.encode(action.clone()).map_err(|error| {
                anyhow::anyhow!("saved profile action {index} is unavailable: {error}")
            })?);
        }

        self.flush_commands()?;
        for batch in batches {
            self.write_batch(batch)?;
        }
        profile.apply_to_state(&mut self.state);
        self.pending_mutation = None;
        self.state.ui.last_message = "Applied profile".to_string();
        Ok(())
    }

    fn routing_channel_count(&self, destination: u16) -> Option<usize> {
        self.state
            .routing_capabilities
            .iter()
            .find(|group| group.destination == destination)
            .map(|group| usize::from(group.channel_count))
            .or_else(|| {
                (destination == 0 && self.driver.definition().id == "zen-go-synergy-core")
                    .then_some(16)
            })
    }

    fn shared_assignment_sources(&self, destination: u16) -> Result<Vec<RoutingSource>> {
        let channel_count = self
            .routing_channel_count(destination)
            .ok_or_else(|| anyhow::anyhow!("routing destination {destination} unavailable"))?;
        if let Some(group) = self.state.routing_group(destination) {
            if group.sources.len() == channel_count {
                return Ok(group.sources.clone());
            }
        }
        let mut sources = Vec::with_capacity(channel_count);
        for index in 0..channel_count {
            let assignment = self
                .state
                .mixer
                .channels
                .iter()
                .find_map(|surface| surface.get(index).and_then(|slot| slot.assignment))
                .ok_or_else(|| {
                    anyhow::anyhow!("assignment table is incomplete for CH {:02}", index + 1)
                })?;
            sources.push(routing_source_from_assignment(assignment));
        }
        Ok(sources)
    }

    pub fn send(&mut self, action: Action, pending: Option<PendingMutation>) -> Result<()> {
        let project_completed_mixer = matches!(action, Action::SetMixer { .. });
        let action = self.complete_dynamic_action(action)?;
        let batch = self.driver.encode(action.clone())?;
        let queueable = !matches!(action, Action::SetRoutingGroup { .. })
            && batch.frames.len() == 1
            && batch.refresh_requests.is_empty();
        if queueable {
            if !self.command_queue.enqueue(action.clone()) {
                bail!("command queue is full; action was not enqueued");
            }
        } else {
            self.flush_commands()?;
            self.write_batch(batch)?;
        }
        if project_completed_mixer || !matches!(action, Action::SetMixerStripState { .. }) {
            self.apply_command_state_update(&action);
        }
        self.pending_mutation = pending;
        self.state.ui.last_message = format!("Sent {:?}", action);
        Ok(())
    }

    fn complete_dynamic_action(&self, action: Action) -> Result<Action> {
        match action {
            Action::SetMixer {
                address,
                control,
                value,
            } => self
                .state
                .complete_mixer_action(address, |strip| match (control, value) {
                    (MixerControl::Fader, ControlValue::Int(value)) => strip.fader = Some(value),
                    (MixerControl::Pan, ControlValue::Int(value)) => strip.pan = Some(value),
                    (MixerControl::Send, ControlValue::Int(value)) => strip.send = Some(value),
                    (MixerControl::Mute, ControlValue::Bool(value)) => strip.muted = Some(value),
                    (MixerControl::Solo, ControlValue::Bool(value)) => strip.soloed = Some(value),
                    (MixerControl::Parameter(id), value) => {
                        if let Some(parameter) = strip
                            .parameters
                            .iter_mut()
                            .find(|parameter| parameter.0 == id)
                        {
                            parameter.1 = value;
                        }
                    }
                    _ => {}
                })
                .ok_or_else(|| anyhow::anyhow!("mixer address is unavailable or incomplete")),
            Action::SetRouting {
                destination,
                channel,
                source,
            } => {
                let group = self
                    .state
                    .routing
                    .iter()
                    .find(|group| group.destination == destination)
                    .ok_or_else(|| {
                        anyhow::anyhow!("routing destination {destination} unavailable")
                    })?;
                let index = usize::from(channel);
                let mut sources = group.sources.clone();
                let slot = sources
                    .get_mut(index)
                    .ok_or_else(|| anyhow::anyhow!("routing channel {channel} unavailable"))?;
                *slot = source;
                Ok(Action::SetRoutingGroup {
                    destination,
                    changed_channel: Some(channel),
                    sources,
                })
            }
            action => Ok(action),
        }
    }

    /// Applies immediate state updates for actions that affect visible state.
    fn apply_command_state_update(&mut self, action: &Action) {
        match action {
            Action::SetOutput {
                address,
                control,
                value,
            } => {
                let Some(mut output) = self
                    .state
                    .outputs()
                    .iter()
                    .find(|output| output.address == *address)
                    .cloned()
                else {
                    return;
                };
                match (control, value) {
                    (OutputControl::Level, ControlValue::Int(level)) => {
                        output.level = Some(*level);
                    }
                    (OutputControl::Mute, ControlValue::Bool(muted)) => {
                        output.muted = Some(*muted);
                        if *muted {
                            output.dimmed = Some(false);
                        }
                    }
                    (OutputControl::Dim, ControlValue::Bool(dimmed)) => {
                        output.dimmed = Some(*dimmed);
                        if *dimmed {
                            output.muted = Some(false);
                        }
                    }
                    (OutputControl::Parameter(parameter), value) => {
                        if let Some((_, current)) =
                            output.parameters.iter_mut().find(|(id, _)| id == parameter)
                        {
                            *current = *value;
                        } else {
                            output.parameters.push((*parameter, *value));
                        }
                    }
                    _ => return,
                }
                self.state.apply_output_patch(vec![output]);
            }
            Action::SetMixerStripState {
                address,
                fader,
                pan,
                muted,
                soloed,
                send,
            } => {
                let Some(surface) = self
                    .state
                    .mixers_mut()
                    .iter_mut()
                    .find(|surface| surface.surface == address.surface)
                else {
                    return;
                };
                let strip = if address.strip == 0 {
                    surface.master.as_mut()
                } else {
                    surface
                        .strips
                        .iter_mut()
                        .find(|strip| strip.strip == address.strip)
                };
                let Some(strip) = strip else {
                    return;
                };
                strip.fader = Some(*fader);
                strip.pan = Some(*pan);
                strip.muted = Some(*muted);
                strip.soloed = Some(*soloed);
                strip.send = *send;
                self.state.sync_compatibility_views();
            }
            Action::SetRoutingGroup {
                destination,
                sources,
                ..
            } => {
                let Some(group) = self
                    .state
                    .routing
                    .iter_mut()
                    .find(|group| group.destination == *destination)
                else {
                    if *destination != 0
                        || self.driver.definition().id != "zen-go-synergy-core"
                        || sources.len() != 16
                    {
                        return;
                    }
                    let Some(assignments) = sources
                        .iter()
                        .map(|source| {
                            let index = u8::try_from(source.index).ok()?;
                            MixerAssignment::from_ordinary_strip_bytes([source.bank, index])
                        })
                        .collect::<Option<Vec<_>>>()
                    else {
                        return;
                    };
                    if self
                        .state
                        .mixer
                        .channels
                        .iter()
                        .any(|channels| channels.len() < assignments.len())
                    {
                        return;
                    }
                    for channels in &mut self.state.mixer.channels {
                        for (channel, assignment) in channels.iter_mut().zip(&assignments) {
                            channel.assignment = Some(*assignment);
                        }
                    }
                    return;
                };
                if group.sources.len() == sources.len() {
                    group.sources.clone_from(sources);
                }
            }
            Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(value),
            } => {
                self.state.device.status.clock_source = Some(ClockSource::from_code(*value as u8));
            }
            Action::SetGlobal {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(value),
            } => {
                let rate = SampleRate::from_code(*value as u8);
                self.state.device.status.sample_rate = Some(rate);
                self.state.device.status.sample_rate_hz = rate.hz();
            }
            _ => {}
        }
    }

    /// Flushes all pending commands from the queue to the transport.
    pub fn flush_commands(&mut self) -> Result<()> {
        self.command_queue
            .flush(self.transport.as_ref(), self.driver.as_ref())?;
        Ok(())
    }

    fn mixer_address_from_ui(&self, mixer: MixerSurface, channel: u8) -> Result<MixerAddress> {
        let strip_index = usize::from(
            channel
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("mixer channel must be one-based"))?,
        );
        self.state
            .mixers()
            .get(mixer.index())
            .and_then(|surface| {
                surface.strips.get(strip_index).map(|strip| MixerAddress {
                    surface: surface.surface,
                    strip: strip.strip,
                })
            })
            .ok_or_else(|| anyhow::anyhow!("mixer channel {channel} is unavailable"))
    }

    fn send_complete_mixer_change<F>(&mut self, address: MixerAddress, mutate: F) -> Result<()>
    where
        F: Fn(&mut antelope_protocol::DynamicMixerStrip) + Copy,
    {
        let surface = self
            .state
            .mixers()
            .iter()
            .find(|surface| surface.surface == address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer address {address:?} unavailable"))?;
        let addresses = if address.strip == 0 {
            surface
                .master
                .as_ref()
                .filter(|master| master.strip == 0)
                .ok_or_else(|| anyhow::anyhow!("mixer address {address:?} unavailable"))?;
            vec![address]
        } else {
            let strip_index = surface
                .strips
                .iter()
                .position(|strip| strip.strip == address.strip)
                .ok_or_else(|| anyhow::anyhow!("mixer address {address:?} unavailable"))?;
            let indexes = if surface.strips[strip_index].linked == Some(true) {
                let left = strip_index - (strip_index % 2);
                vec![
                    left,
                    left.checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("linked mixer pair overflow"))?,
                ]
            } else {
                vec![strip_index]
            };
            indexes
                .into_iter()
                .map(|index| {
                    surface
                        .strips
                        .get(index)
                        .map(|strip| MixerAddress {
                            surface: address.surface,
                            strip: strip.strip,
                        })
                        .ok_or_else(|| anyhow::anyhow!("linked mixer strip unavailable"))
                })
                .collect::<Result<Vec<_>>>()?
        };

        let mut actions = Vec::with_capacity(addresses.len());
        for pair_address in addresses {
            let action = self
                .state
                .complete_mixer_action(pair_address, mutate)
                .ok_or_else(|| anyhow::anyhow!("mixer strip state is incomplete"))?;
            actions.push(action);
        }

        let mut batches = Vec::with_capacity(actions.len());
        for action in &actions {
            batches.push(self.driver.encode(action.clone())?);
        }
        self.flush_commands()?;
        for (action, batch) in actions.iter().zip(batches) {
            self.write_batch(batch)?;
            self.apply_command_state_update(action);
        }
        let pending = PendingMutation::Mixer(
            actions
                .iter()
                .filter_map(|action| {
                    let Action::SetMixerStripState { address, .. } = action else {
                        return None;
                    };
                    self.state
                        .mixers()
                        .iter()
                        .find(|surface| surface.surface == address.surface)
                        .and_then(|surface| {
                            if address.strip == 0 {
                                surface.master.as_ref()
                            } else {
                                surface
                                    .strips
                                    .iter()
                                    .find(|strip| strip.strip == address.strip)
                            }
                        })
                        .cloned()
                        .map(|strip| super::PendingMixerStrip {
                            address: *address,
                            strip,
                        })
                })
                .collect(),
        );
        self.pending_mutation = Some(pending);
        Ok(())
    }

    pub fn send_mixer_level_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        level: u8,
    ) -> Result<()> {
        let address = self.mixer_address_from_ui(mixer, channel)?;
        self.send_complete_mixer_change(address, |strip| strip.fader = Some(i32::from(level)))
    }

    pub fn send_mixer_mute_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    ) -> Result<()> {
        let address = self.mixer_address_from_ui(mixer, channel)?;
        self.send_complete_mixer_change(address, |strip| strip.muted = Some(muted))
    }

    pub fn send_mixer_solo_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    ) -> Result<()> {
        let address = self.mixer_address_from_ui(mixer, channel)?;
        self.send_complete_mixer_change(address, |strip| strip.soloed = Some(soloed))
    }

    pub fn send_mixer_link_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        enabled: bool,
    ) -> Result<()> {
        let address = self.mixer_address_from_ui(mixer, channel)?;
        self.send_mixer_link_address(address, enabled)
    }

    fn send_mixer_link_address(&mut self, address: MixerAddress, enabled: bool) -> Result<()> {
        let surface = self
            .state
            .mixers()
            .iter()
            .find(|surface| surface.surface == address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer surface unavailable"))?;
        let strip_index = surface
            .strips
            .iter()
            .position(|strip| strip.strip == address.strip)
            .ok_or_else(|| anyhow::anyhow!("mixer strip unavailable"))?;
        let left_index = strip_index - (strip_index % 2);
        let right_index = left_index
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("link pair overflow"))?;
        if surface.strips.get(right_index).is_none() {
            bail!("link pair is incomplete");
        }
        let pair =
            u16::try_from(left_index).map_err(|_| anyhow::anyhow!("link pair index overflow"))? / 2;
        let action = Action::SetLink {
            surface: address.surface,
            pair,
            enabled,
        };
        let batch = self.driver.encode(action)?;
        let mut pending = Vec::new();
        for index in [left_index, right_index] {
            let strip = self
                .state
                .mixers_mut()
                .iter_mut()
                .find(|surface| surface.surface == address.surface)
                .and_then(|surface| surface.strips.get_mut(index))
                .ok_or_else(|| anyhow::anyhow!("link strip unavailable"))?;
            strip.linked = Some(enabled);
            pending.push(super::PendingMixerStrip {
                address: MixerAddress {
                    surface: address.surface,
                    strip: strip.strip,
                },
                strip: strip.clone(),
            });
        }
        self.flush_commands()?;
        self.write_batch(batch)?;
        self.pending_mutation = Some(PendingMutation::Mixer(pending));
        self.state.sync_compatibility_views();
        Ok(())
    }

    pub fn apply_intent(&mut self, intent: Intent, area: Rect) -> Result<()> {
        let pending = intent.pending_mutation(&self.state);
        match intent {
            Intent::Quit => {
                self.state.ui.quit_requested = true;
            }
            Intent::ToggleRawView => self.state.toggle_raw_view(),
            Intent::ToggleHotkeysPopup => self.state.toggle_hotkeys_popup(),
            Intent::OpenProfilesPopup => self.handle_open_profiles_popup(),
            Intent::CloseProfilesPopup => self.handle_close_profiles_popup(),
            Intent::OpenRoutingPopup => self.handle_open_routing_popup(),
            Intent::CloseRoutingPopup => self.handle_close_routing_popup(),
            Intent::OpenOptionsPopup => self.handle_open_options_popup(),
            Intent::CloseOptionsPopup => self.handle_close_options_popup(),
            Intent::SetRefreshRate(rate) => self.handle_set_refresh_rate(rate),
            Intent::CyclePeakThreshold(increase) => self.handle_cycle_peak_threshold(increase),
            Intent::TogglePeakEnabled => self.handle_toggle_peak_enabled(),
            Intent::CyclePeakHoldDuration(duration) => {
                self.handle_cycle_peak_hold_duration(duration)
            }
            Intent::ToggleAutoSave => self.handle_toggle_auto_save(),
            Intent::SelectProfile(index) => self.handle_select_profile(index),
            Intent::LoadSelectedProfile => self.handle_load_selected_profile(),
            Intent::StartSaveProfile => self.handle_start_save_profile(),
            Intent::StartRenameProfile => self.handle_start_rename_profile(),
            Intent::DeleteSelectedProfile => self.handle_delete_selected_profile(),
            Intent::PageMixerStripsLeft => self.handle_page_mixer_strips(area, false),
            Intent::PageMixerStripsRight => self.handle_page_mixer_strips(area, true),
            Intent::OpenSampleRateSelector => self.handle_open_sample_rate_selector(),
            Intent::OpenClockSourceSelector => self.handle_open_clock_source_selector(),
            Intent::SelectRawPacketTab(tab) => self.handle_select_raw_packet_tab(tab),
            Intent::SelectRawMapScope(scope) => self.handle_select_raw_map_scope(scope),
            Intent::CycleRawMapScope { forward } => self.handle_cycle_raw_map_scope(forward),
            Intent::ScrollRawDump { increase, page } => self.handle_scroll_raw_dump(increase, page),
            Intent::SelectOutput(index) => self.handle_output_select(index),
            Intent::AdjustOutputLevel { index, increase } => {
                self.handle_output_adjust(index, increase, pending)?
            }
            Intent::SetOutputLevel { index, step } => {
                self.handle_output_set_level(index, step, pending)?
            }
            Intent::ToggleOutputDim(index) => self.handle_output_toggle_dim(index, pending)?,
            Intent::ToggleOutputMute(index) => self.handle_output_toggle_mute(index, pending)?,
            Intent::SelectQueryReplyEntry(index) => self.handle_select_query_reply_entry(index),
            Intent::ScrollQueryReplyList { increase } => {
                self.handle_scroll_query_reply_list(increase)
            }
            Intent::SelectMixerSurface { surface } => self.handle_select_mixer_surface(surface)?,
            Intent::SelectSurface(surface) => self.handle_select_surface(surface, pending)?,
            Intent::SelectMixerChannel(index) => self.handle_select_mixer_channel(index),
            Intent::AdjustMixerLevel { index, increase } => {
                self.handle_adjust_mixer_level(index, increase, pending)?
            }
            Intent::SetMixerLevel { index, level } => {
                self.handle_set_mixer_level(index, level, pending)?
            }
            Intent::AdjustMixerPan { index, right } => {
                self.handle_adjust_mixer_pan(index, right, pending)?
            }
            Intent::SetMixerPan { index, pan } => self.handle_set_mixer_pan(index, pan, pending)?,
            Intent::ToggleMixerMute(channel) => self.handle_toggle_mixer_mute(channel, pending)?,
            Intent::ToggleMixerSolo(channel) => self.handle_toggle_mixer_solo(channel, pending)?,
            Intent::ToggleMixerLink(channel) => self.handle_toggle_mixer_link(channel, pending)?,
            Intent::AdjustMixerLevelAt { address, increase } => {
                self.handle_adjust_mixer_level_at(address, increase)?
            }
            Intent::SetMixerLevelAt { address, level } => {
                self.handle_set_mixer_level_at(address, level)?
            }
            Intent::AdjustMixerPanAt { address, right } => {
                self.handle_adjust_mixer_pan_at(address, right)?
            }
            Intent::SetMixerPanAt { address, pan } => self.handle_set_mixer_pan_at(address, pan)?,
            Intent::SetMixerSendAt { address, send } => {
                self.handle_set_mixer_send_at(address, send)?
            }
            Intent::ToggleMixerMuteAt { address } => self.handle_toggle_mixer_mute_at(address)?,
            Intent::ToggleMixerSoloAt { address } => self.handle_toggle_mixer_solo_at(address)?,
            Intent::ToggleMixerLinkAt { address } => self.handle_toggle_mixer_link_at(address)?,
            Intent::OpenAssignmentPicker(strip) => self.handle_open_assignment_picker(strip)?,
            Intent::OpenAssignmentPickerAt { address } => {
                self.handle_open_assignment_picker_at(address, false)?
            }
            Intent::PickAssignment { strip, assignment } => {
                self.handle_pick_assignment(strip, assignment, pending)?
            }
            Intent::PickAssignmentAt {
                address,
                assignment,
            } => self.handle_pick_assignment_at(address, assignment, pending)?,
            Intent::CloseAssignmentPicker => self.handle_close_assignment_picker(),
            Intent::CloseSelectorPopup => self.handle_close_selector_popup(),
            Intent::SelectPreampInput(input) => self.handle_select_preamp_input(input),
            Intent::AdjustPreampGain { input, increase } => {
                self.handle_adjust_preamp_gain(input, increase, pending)?
            }
            Intent::SetPreampGain { input, raw } => {
                self.handle_set_preamp_gain(input, raw, pending)?
            }
            Intent::OpenPreampModeSelector(input) => self.handle_open_preamp_mode_selector(input),
            Intent::CyclePreampMode(input) => self.handle_cycle_preamp_mode(input, pending)?,
            Intent::PickSampleRate(rate) => self.handle_pick_sample_rate(rate, pending)?,
            Intent::PickClockSource(source) => self.handle_pick_clock_source(source, pending)?,
            Intent::PickPreampMode { input, mode } => {
                self.handle_pick_preamp_mode(input, mode, pending)?
            }
            Intent::TogglePreampPhase(input) => self.handle_toggle_preamp_phase(input, pending)?,
            Intent::TogglePreampPhantom(input) => {
                self.handle_toggle_preamp_phantom(input, pending)?
            }
            Intent::AdjustInputGainAt { address, increase } => {
                self.handle_adjust_input_gain_at(address, increase, pending)?
            }
            Intent::SetInputGainAt { address, raw } => {
                self.handle_set_input_gain_at(address, raw, pending)?
            }
            Intent::AdjustInputParameterAt {
                address,
                parameter_id,
                increase,
            } => self.handle_adjust_input_parameter_at(address, parameter_id, increase, pending)?,
            Intent::SetInputParameterAt {
                address,
                parameter_id,
                value,
            } => self.handle_set_input_parameter_at(address, parameter_id, value, pending)?,
            Intent::CycleInputModeAt { address } => {
                self.handle_cycle_input_mode_at(address, pending)?
            }
            Intent::SetInputModeAt { address, mode } => {
                self.handle_set_input_mode_at(address, mode, pending)?
            }
            Intent::ToggleInputPhaseAt { address } => {
                self.handle_toggle_input_phase_at(address, pending)?
            }
            Intent::ToggleInputPhantomAt { address } => {
                self.handle_toggle_input_phantom_at(address, pending)?
            }
            Intent::AdjustFocused(increase) => self.handle_adjust_focused(increase, pending)?,
            Intent::ToggleFocusedMute => self.handle_toggle_focused_mute(pending)?,
            Intent::ToggleFocusedDim => self.handle_toggle_focused_dim(pending)?,
            Intent::ToggleRoutingPopup => self.handle_toggle_routing_popup(),
            Intent::RefreshQueriedState => self.handle_refresh_queried_state()?,
            Intent::CycleFocus => self.handle_cycle_focus(),
            Intent::MovePopupSelection(down) => self.handle_move_popup_selection(down),
            Intent::ProfileEditorChar(ch) => self.handle_profile_editor_char(ch),
            Intent::ProfileEditorBackspace => self.handle_profile_editor_backspace(),
            Intent::ProfileEditorCommit => self.handle_profile_editor_commit(),
            Intent::ProfileEditorCancel => self.handle_profile_editor_cancel(),
            Intent::CaptureRawBaseline => self.handle_capture_raw_baseline(),
            Intent::ClearRawBaseline => self.handle_clear_raw_baseline(),
            Intent::ToggleOptionsPopup => self.handle_toggle_options_popup(),
        }
        Ok(())
    }

    pub fn poll_device(&mut self, timeout: Duration) -> Result<bool> {
        // Flush pending commands before reading so device sees latest state
        self.flush_commands()?;

        let mut next_timeout = timeout;
        let mut state_dirty = false;

        for _ in 0..MAX_FRAMES_PER_POLL {
            let Some(bytes) = self.transport.read(next_timeout)? else {
                break;
            };

            next_timeout = Duration::ZERO;

            if let Some(event) = self.driver.decode(&bytes)? {
                if matches!(event, DeviceEvent::Snapshot { .. }) {
                    state_dirty |= self.confirm_pending_write();
                }
                state_dirty |= self.state.observe_event(event);
            }
        }

        Ok(state_dirty)
    }

    pub fn confirm_pending_write(&mut self) -> bool {
        self.pending_mutation
            .take()
            .is_some_and(|pending| self.state.apply_pending_mutation(pending))
    }

    fn handle_output_select(&mut self, index: usize) {
        self.state.ui.focus = FocusArea::Outputs;
        if index < self.state.outputs().len() {
            self.state.output.selected = index;
        }
    }

    fn handle_output_adjust(
        &mut self,
        index: usize,
        increase: bool,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        let output = self
            .state
            .outputs()
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("output index {index} unavailable"))?;
        self.state.output.selected = index;
        let (min, max) = self
            .state
            .output_range(OutputControl::Level)
            .ok_or_else(|| anyhow::anyhow!("output level range unavailable"))?;
        let current = output.level.unwrap_or(min).clamp(min, max);
        let next = if increase {
            current.saturating_sub(1).max(min)
        } else {
            current.saturating_add(1).min(max)
        };
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Level,
                value: ControlValue::Int(next),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_set_level(
        &mut self,
        index: usize,
        step: u8,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        let address = self
            .state
            .outputs()
            .get(index)
            .map(|output| output.address)
            .ok_or_else(|| anyhow::anyhow!("output index {index} unavailable"))?;
        self.state.output.selected = index;
        let (min, max) = self
            .state
            .output_range(OutputControl::Level)
            .ok_or_else(|| anyhow::anyhow!("output level range unavailable"))?;
        self.send(
            Action::SetOutput {
                address,
                control: OutputControl::Level,
                value: ControlValue::Int(i32::from(step).clamp(min, max)),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_toggle_dim(
        &mut self,
        index: usize,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        let output = self
            .state
            .outputs()
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("output index {index} unavailable"))?;
        self.state.output.selected = index;
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Dim,
                value: ControlValue::Bool(!output.dimmed.unwrap_or(false)),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_toggle_mute(
        &mut self,
        index: usize,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        let output = self
            .state
            .outputs()
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("output index {index} unavailable"))?;
        self.state.output.selected = index;
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Mute,
                value: ControlValue::Bool(!output.muted.unwrap_or(false)),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_open_sample_rate_selector(&mut self) {
        if self.state.device.status.clock_source == Some(ClockSource::Internal) {
            self.state.popup.selected_index = self
                .state
                .device
                .status
                .sample_rate
                .and_then(|current| {
                    SampleRate::all_confirmed()
                        .iter()
                        .position(|rate| *rate == current)
                })
                .unwrap_or(0);
            self.state.popup.selector_popup = Some(SelectorPopupState {
                kind: SelectorPopupKind::SampleRate,
            });
        }
    }

    fn handle_open_clock_source_selector(&mut self) {
        self.state.popup.selected_index = self
            .state
            .device
            .status
            .clock_source
            .and_then(|current| {
                ClockSource::all_confirmed()
                    .iter()
                    .position(|source| *source == current)
            })
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::ClockSource,
        });
    }

    fn handle_pick_sample_rate(
        &mut self,
        rate: SampleRate,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.send(
            Action::SetGlobal {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(i32::from(rate.code())),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_pick_clock_source(
        &mut self,
        source: ClockSource,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.send(
            Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(i32::from(source.code())),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_select_raw_packet_tab(&mut self, tab: RawPacketTab) {
        self.state.raw_view.select_tab(tab);
    }

    fn handle_select_raw_map_scope(&mut self, scope: RawMapScope) {
        self.state.raw_view.select_scope(scope);
    }

    fn handle_cycle_raw_map_scope(&mut self, forward: bool) {
        self.state.raw_view.cycle_scope(forward);
    }

    fn handle_scroll_raw_dump(&mut self, increase: bool, page: bool) {
        self.state.raw_view.scroll_raw_view(increase, page);
    }

    fn handle_select_query_reply_entry(&mut self, index: usize) {
        self.state.raw_view.selected_query_reply_entry = Some(
            index.min(
                self.state
                    .raw_view
                    .recent_query_reply_entries
                    .len()
                    .saturating_sub(1),
            ),
        );
        self.state.raw_view.reset_raw_view_scroll();
    }

    fn handle_scroll_query_reply_list(&mut self, increase: bool) {
        self.state.cycle_query_reply_entry(increase);
    }

    fn handle_capture_raw_baseline(&mut self) {
        self.state.capture_raw_baseline();
        self.state.ui.last_message = "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
    }

    fn handle_clear_raw_baseline(&mut self) {
        self.state.clear_raw_baseline();
        self.state.ui.last_message = "Cleared raw baseline".to_string();
    }

    fn handle_open_routing_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.routing_open = true;
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = self.state.mixer.selected_channel.min(7);
        self.state.ui.last_message =
            "Routing popup mirrors mixer assignments for USB recording channels 1-8".to_string();
    }

    fn handle_close_routing_popup(&mut self) {
        self.state.popup.routing_open = false;
        self.state.ui.last_message = "Closed routing popup".to_string();
    }

    fn handle_toggle_routing_popup(&mut self) {
        self.state.popup.routing_open = !self.state.popup.routing_open;
        self.state.ui.last_message = if self.state.popup.routing_open {
            "Routing popup mirrors mixer assignments for USB recording channels 1-8".to_string()
        } else {
            "Closed routing popup".to_string()
        };
    }

    fn handle_open_options_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.routing_open = false;
        self.state.popup.options_open = true;
        self.state.ui.last_message = "Options popup opened".to_string();
    }

    fn handle_close_options_popup(&mut self) {
        self.state.popup.options_open = false;
        self.state.ui.last_message = "Closed options popup".to_string();
    }

    fn handle_toggle_options_popup(&mut self) {
        self.state.toggle_options_popup();
        self.state.ui.last_message = if self.state.popup.options_open {
            "Options popup opened".to_string()
        } else {
            "Closed options popup".to_string()
        };
    }

    fn handle_set_refresh_rate(&mut self, rate: RefreshRate) {
        self.state.ui.settings.refresh_rate = rate;
        self.state.ui.last_message = format!("Refresh rate set to {}", rate.label());
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_cycle_peak_threshold(&mut self, increase: bool) {
        const PEAK_THRESHOLD_CHOICES: [u8; 10] =
            [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x0a, 0x0f, 0x14];
        let current = self.state.ui.settings.peak_threshold_raw;
        let pos = PEAK_THRESHOLD_CHOICES
            .iter()
            .position(|&v| v == current)
            .unwrap_or(3);
        let next_pos = if increase {
            (pos + 1).min(PEAK_THRESHOLD_CHOICES.len() - 1)
        } else {
            pos.saturating_sub(1)
        };
        self.state.ui.settings.peak_threshold_raw = PEAK_THRESHOLD_CHOICES[next_pos];
        let db = self.state.ui.settings.peak_threshold_db();
        self.state.ui.last_message = format!("Peak threshold set to {} dB", db);
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_toggle_peak_enabled(&mut self) {
        self.state.ui.settings.peak_enabled = !self.state.ui.settings.peak_enabled;
        if self.state.ui.settings.peak_enabled {
            self.state.ui.last_message = "Peak detection enabled".to_string();
        } else {
            self.state.preamp.peaks.fill(None);
            for peaks in &mut self.state.mixer.peaks {
                peaks.fill(None);
            }
            self.state.ui.last_message = "Peak detection disabled".to_string();
        }
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_cycle_peak_hold_duration(&mut self, duration: PeakHoldDuration) {
        self.state.ui.settings.peak_hold_duration = duration;
        self.state.ui.last_message = format!("Peak hold duration set to {}", duration.label());
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_toggle_auto_save(&mut self) {
        self.state.ui.settings.auto_save = !self.state.ui.settings.auto_save;
        if self.state.ui.settings.auto_save {
            self.state.ui.last_message = "Auto-save enabled".to_string();
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        } else {
            self.state.ui.last_message = "Auto-save disabled".to_string();
        }
    }

    fn input_at_ui(&self, input: u8) -> Result<antelope_protocol::DynamicInputState> {
        self.state
            .input_spaces
            .first()
            .and_then(|space| space.inputs.get(usize::from(input)))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("input index {input} unavailable"))
    }

    fn input_at_address(
        &self,
        address: InputAddress,
    ) -> Result<antelope_protocol::DynamicInputState> {
        self.state
            .input_spaces
            .iter()
            .find(|space| space.space_id == address.space)
            .and_then(|space| space.inputs.iter().find(|input| input.address == address))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("input address {address:?} unavailable"))
    }

    fn ensure_input_control(&self, address: InputAddress, control: InputControl) -> Result<()> {
        if !self.state.ui_profile.supports_input(address, control) {
            bail!("input control {control:?} is unsupported for {address:?}");
        }
        Ok(())
    }

    fn handle_adjust_input_gain_at(
        &mut self,
        address: InputAddress,
        increase: bool,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Gain)?;
        let slot = self.input_at_address(address)?;
        let (min, max) = self
            .state
            .input_range(address, slot.mode)
            .ok_or_else(|| anyhow::anyhow!("input gain range unavailable"))?;
        let current = slot.gain.unwrap_or(min).clamp(min, max);
        let raw = if increase {
            current.saturating_add(1).min(max)
        } else {
            current.saturating_sub(1).max(min)
        };
        self.handle_set_input_gain_at(address, raw, pending)
    }

    fn handle_set_input_gain_at(
        &mut self,
        address: InputAddress,
        raw: i32,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Gain)?;
        self.input_at_address(address)?;
        self.send(
            Action::SetInput {
                address,
                control: InputControl::Gain,
                value: ControlValue::Int(raw),
            },
            pending,
        )
    }

    fn handle_adjust_input_parameter_at(
        &mut self,
        address: InputAddress,
        parameter_id: u16,
        increase: bool,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        let control = InputControl::Parameter(parameter_id);
        self.ensure_input_control(address, control)?;
        let current = self.input_at_address(address)?.gain.unwrap_or(0);
        let value = if increase {
            current.saturating_add(1)
        } else {
            current.saturating_sub(1)
        };
        self.handle_set_input_parameter_at(address, parameter_id, value, pending)
    }

    fn handle_set_input_parameter_at(
        &mut self,
        address: InputAddress,
        parameter_id: u16,
        value: i32,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        let control = InputControl::Parameter(parameter_id);
        self.ensure_input_control(address, control)?;
        self.input_at_address(address)?;
        self.send(
            Action::SetInput {
                address,
                control,
                value: ControlValue::Int(value),
            },
            pending,
        )
    }

    fn handle_cycle_input_mode_at(
        &mut self,
        address: InputAddress,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Mode)?;
        let slot = self.input_at_address(address)?;
        let current = PreampMode::from_raw(slot.mode.unwrap_or_default() as u8);
        let mode = match current {
            PreampMode::Mic => PreampMode::Line,
            PreampMode::Line => PreampMode::HiZ,
            PreampMode::HiZ | PreampMode::Unknown(_) => PreampMode::Mic,
        };
        self.handle_set_input_mode_at(address, mode, pending)
    }

    fn handle_set_input_mode_at(
        &mut self,
        address: InputAddress,
        mode: PreampMode,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Mode)?;
        self.input_at_address(address)?;
        self.send(
            Action::SetInput {
                address,
                control: InputControl::Mode,
                value: ControlValue::Enum(i32::from(mode.code())),
            },
            pending,
        )
    }

    fn handle_toggle_input_phase_at(
        &mut self,
        address: InputAddress,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Phase)?;
        let slot = self.input_at_address(address)?;
        self.send(
            Action::SetInput {
                address,
                control: InputControl::Phase,
                value: ControlValue::Bool(!slot.phase.unwrap_or(false)),
            },
            pending,
        )
    }

    fn handle_toggle_input_phantom_at(
        &mut self,
        address: InputAddress,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.ensure_input_control(address, InputControl::Phantom)?;
        let slot = self.input_at_address(address)?;
        self.send(
            Action::SetInput {
                address,
                control: InputControl::Phantom,
                value: ControlValue::Bool(!slot.phantom.unwrap_or(false)),
            },
            pending,
        )
    }

    fn handle_select_preamp_input(&mut self, input: usize) {
        self.state.ui.focus = FocusArea::Preamp;
        if self
            .state
            .input_spaces
            .first()
            .is_some_and(|space| input < space.inputs.len())
        {
            self.state.preamp.selected_input = input;
        }
    }

    fn handle_adjust_preamp_gain(
        &mut self,
        input: u8,
        increase: bool,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        let current = slot
            .gain
            .and_then(|gain| u8::try_from(gain).ok())
            .unwrap_or(0);
        let next = next_preamp_gain_raw(current, increase);
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Gain,
                value: ControlValue::Int(i32::from(next)),
            },
            pending,
        )
    }

    fn handle_set_preamp_gain(
        &mut self,
        input: u8,
        raw: u8,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Gain,
                value: ControlValue::Int(i32::from(raw)),
            },
            pending,
        )
    }

    fn handle_open_preamp_mode_selector(&mut self, input: u8) {
        self.state.ui.focus = FocusArea::Preamp;
        let Ok(slot) = self.input_at_ui(input) else {
            return;
        };
        self.state.preamp.selected_input = usize::from(input);
        let current = PreampMode::from_raw(slot.mode.unwrap_or_default() as u8);
        self.state.popup.selected_index = [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
            .iter()
            .position(|mode| *mode == current)
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::PreampMode { input },
        });
    }

    fn handle_cycle_preamp_mode(
        &mut self,
        input: u8,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        let current = PreampMode::from_raw(slot.mode.unwrap_or_default() as u8);
        let next = match current {
            PreampMode::Mic => PreampMode::Line,
            PreampMode::Line => PreampMode::HiZ,
            PreampMode::HiZ | PreampMode::Unknown(_) => PreampMode::Mic,
        };
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Mode,
                value: ControlValue::Enum(i32::from(next.code())),
            },
            pending,
        )
    }

    fn handle_pick_preamp_mode(
        &mut self,
        input: u8,
        mode: PreampMode,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Mode,
                value: ControlValue::Enum(i32::from(mode.code())),
            },
            pending,
        )
    }

    fn handle_toggle_preamp_phase(
        &mut self,
        input: u8,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Phase,
                value: ControlValue::Bool(!slot.phase.unwrap_or(false)),
            },
            pending,
        )
    }

    fn handle_toggle_preamp_phantom(
        &mut self,
        input: u8,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        let slot = self.input_at_ui(input)?;
        self.state.preamp.selected_input = usize::from(input);
        self.send(
            Action::SetInput {
                address: slot.address,
                control: InputControl::Phantom,
                value: ControlValue::Bool(!slot.phantom.unwrap_or(false)),
            },
            pending,
        )
    }

    fn handle_page_mixer_strips(&mut self, area: Rect, left: bool) {
        self.state.ui.focus = FocusArea::Mixer;
        let visible = crate::ui::mixer_strip_viewport_capacity(area, &self.state);
        self.state.page_mixer_strip_viewport(left, visible);
    }

    fn mixer_strip_at_ui(
        &self,
        index: usize,
    ) -> Result<(MixerAddress, antelope_protocol::DynamicMixerStrip)> {
        let surface_index = self
            .state
            .active_mixer_surface()
            .ok_or_else(|| anyhow::anyhow!("no active mixer surface"))?;
        let surface = self
            .state
            .mixers()
            .get(surface_index)
            .ok_or_else(|| anyhow::anyhow!("active mixer surface unavailable"))?;
        let strip = surface
            .strips
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mixer strip index {index} unavailable"))?;
        Ok((
            MixerAddress {
                surface: surface.surface,
                strip: strip.strip,
            },
            strip,
        ))
    }

    fn mixer_strip_at_address(
        &self,
        address: MixerAddress,
    ) -> Result<antelope_protocol::DynamicMixerStrip> {
        let surface = self
            .state
            .mixers()
            .iter()
            .find(|surface| surface.surface == address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer address {address:?} unavailable"))?;
        if address.strip == 0 {
            surface
                .master
                .as_ref()
                .filter(|master| master.strip == 0)
                .cloned()
        } else {
            surface
                .strips
                .iter()
                .find(|strip| strip.strip == address.strip)
                .cloned()
        }
        .ok_or_else(|| anyhow::anyhow!("mixer address {address:?} unavailable"))
    }

    fn ensure_mixer_control(&self, address: MixerAddress, control: MixerControl) -> Result<()> {
        if !self
            .state
            .ui_profile
            .supports_mixer(address.surface, control)
        {
            bail!("mixer control {control:?} is unsupported for {address:?}");
        }
        self.mixer_strip_at_address(address)?;
        Ok(())
    }

    fn handle_adjust_mixer_level_at(
        &mut self,
        address: MixerAddress,
        increase: bool,
    ) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Fader)?;
        let strip = self.mixer_strip_at_address(address)?;
        let semantics = self
            .state
            .mixer_fader(address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer fader semantics unavailable"))?;
        let current = strip
            .fader
            .ok_or_else(|| anyhow::anyhow!("mixer fader value unavailable"))?
            .clamp(semantics.min, semantics.max);
        let next = step_fader(current, increase, semantics);
        self.send_complete_mixer_change(address, |strip| strip.fader = Some(next))
    }

    fn handle_set_mixer_level_at(&mut self, address: MixerAddress, level: u8) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Fader)?;
        let semantics = self
            .state
            .mixer_fader(address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer fader semantics unavailable"))?;
        let level = i32::from(level).clamp(semantics.min, semantics.max);
        self.send_complete_mixer_change(address, |strip| strip.fader = Some(level))
    }

    fn handle_adjust_mixer_pan_at(&mut self, address: MixerAddress, right: bool) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Pan)?;
        let strip = self.mixer_strip_at_address(address)?;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Pan)
            .ok_or_else(|| anyhow::anyhow!("mixer pan range unavailable"))?;
        let current = strip.pan.unwrap_or((min + max) / 2).clamp(min, max);
        let next = if right {
            current.saturating_add(1).min(max)
        } else {
            current.saturating_sub(1).max(min)
        };
        self.send_complete_mixer_change(address, |strip| strip.pan = Some(next))
    }

    fn handle_set_mixer_pan_at(&mut self, address: MixerAddress, pan: PanState) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Pan)?;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Pan)
            .ok_or_else(|| anyhow::anyhow!("mixer pan range unavailable"))?;
        let value = i32::from(pan.raw()).clamp(min, max);
        self.send_complete_mixer_change(address, |strip| strip.pan = Some(value))
    }

    fn handle_set_mixer_send_at(&mut self, address: MixerAddress, send: i32) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Send)?;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Send)
            .ok_or_else(|| anyhow::anyhow!("mixer send range unavailable"))?;
        self.send_complete_mixer_change(address, |strip| strip.send = Some(send.clamp(min, max)))
    }

    fn handle_toggle_mixer_mute_at(&mut self, address: MixerAddress) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Mute)?;
        let strip = self.mixer_strip_at_address(address)?;
        self.send_complete_mixer_change(address, |slot| {
            slot.muted = Some(!strip.muted.unwrap_or(false))
        })
    }

    fn handle_toggle_mixer_solo_at(&mut self, address: MixerAddress) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Solo)?;
        let strip = self.mixer_strip_at_address(address)?;
        self.send_complete_mixer_change(address, |slot| {
            slot.soloed = Some(!strip.soloed.unwrap_or(false))
        })
    }

    fn handle_toggle_mixer_link_at(&mut self, address: MixerAddress) -> Result<()> {
        if address.strip == 0 || !self.state.ui_profile.supports_link(address.surface) {
            bail!("mixer link is unsupported for {address:?}");
        }
        let strip = self.mixer_strip_at_address(address)?;
        self.send_mixer_link_address(address, !strip.linked.unwrap_or(false))
    }

    fn handle_select_mixer_surface(&mut self, surface: u8) -> Result<()> {
        let index = self
            .state
            .mixers()
            .iter()
            .position(|candidate| candidate.surface == surface)
            .ok_or_else(|| anyhow::anyhow!("mixer surface {surface} unavailable"))?;
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.surface_index = index;
        self.state.mixer.selected_channel = 0;
        self.state.mixer.strip_scroll = 0;
        Ok(())
    }

    fn handle_select_mixer_channel(&mut self, index: usize) {
        self.state.ui.focus = FocusArea::Mixer;
        if self.mixer_strip_at_ui(index).is_ok() {
            self.state.mixer.selected_channel = index;
        }
    }

    fn handle_adjust_mixer_level(
        &mut self,
        index: usize,
        increase: bool,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let (address, strip) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        let semantics = self
            .state
            .mixer_fader(address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer fader semantics unavailable"))?;
        let current = strip
            .fader
            .ok_or_else(|| anyhow::anyhow!("mixer fader value unavailable"))?
            .clamp(semantics.min, semantics.max);
        let next = step_fader(current, increase, semantics);
        self.send_complete_mixer_change(address, |strip| strip.fader = Some(next))
    }

    fn handle_set_mixer_level(
        &mut self,
        index: usize,
        level: u8,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let (address, _) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        let semantics = self
            .state
            .mixer_fader(address.surface)
            .ok_or_else(|| anyhow::anyhow!("mixer fader semantics unavailable"))?;
        let level = i32::from(level).clamp(semantics.min, semantics.max);
        self.send_complete_mixer_change(address, |strip| strip.fader = Some(level))
    }

    fn handle_adjust_mixer_pan(
        &mut self,
        index: usize,
        right: bool,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let (address, strip) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Pan)
            .ok_or_else(|| anyhow::anyhow!("mixer pan range unavailable"))?;
        let current = strip.pan.unwrap_or((min + max) / 2).clamp(min, max);
        let next = if right {
            current.saturating_add(1).min(max)
        } else {
            current.saturating_sub(1).max(min)
        };
        self.send_complete_mixer_change(address, |strip| strip.pan = Some(next))
    }

    fn handle_set_mixer_pan(
        &mut self,
        index: usize,
        pan: PanState,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let (address, _) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Pan)
            .ok_or_else(|| anyhow::anyhow!("mixer pan range unavailable"))?;
        let value = i32::from(pan.raw()).clamp(min, max);
        self.send_complete_mixer_change(address, |strip| strip.pan = Some(value))
    }

    fn handle_toggle_mixer_mute(
        &mut self,
        channel: u8,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let index = usize::from(
            channel
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("mixer channel must be one-based"))?,
        );
        let (address, strip) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        self.send_complete_mixer_change(address, |slot| {
            slot.muted = Some(!strip.muted.unwrap_or(false))
        })
    }

    fn handle_toggle_mixer_solo(
        &mut self,
        channel: u8,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let index = usize::from(
            channel
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("mixer channel must be one-based"))?,
        );
        let (address, strip) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        self.send_complete_mixer_change(address, |slot| {
            slot.soloed = Some(!strip.soloed.unwrap_or(false))
        })
    }

    fn handle_toggle_mixer_link(
        &mut self,
        channel: u8,
        _pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        let index = usize::from(
            channel
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("mixer channel must be one-based"))?,
        );
        let (address, strip) = self.mixer_strip_at_ui(index)?;
        self.state.mixer.selected_channel = index;
        self.send_mixer_link_address(address, !strip.linked.unwrap_or(false))
    }

    fn handle_open_assignment_picker(&mut self, strip: u8) -> Result<()> {
        let strip_index = usize::from(
            strip
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("mixer strip must be one-based"))?,
        );
        let (address, _) = self.mixer_strip_at_ui(strip_index)?;
        if !self.state.legacy_routing_assignment_available() {
            bail!("routing assignment control is unsupported");
        }
        self.handle_open_assignment_picker_at(address, true)
    }

    fn handle_open_assignment_picker_at(
        &mut self,
        address: MixerAddress,
        legacy: bool,
    ) -> Result<()> {
        let strip = u8::try_from(address.strip)
            .map_err(|_| anyhow::anyhow!("mixer strip {} is out of range", address.strip))?;
        let (surface_index, strip_index) = self
            .state
            .mixers()
            .iter()
            .enumerate()
            .find(|(_, surface)| {
                surface.surface == address.surface
                    && surface
                        .strips
                        .iter()
                        .any(|strip| strip.strip == address.strip)
            })
            .map(|(index, surface)| {
                (
                    index,
                    surface
                        .strips
                        .iter()
                        .position(|strip| strip.strip == address.strip)
                        .expect("strip found above"),
                )
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "mixer address {}:{} is unavailable",
                    address.surface,
                    address.strip
                )
            })?;
        if if legacy {
            !self.state.legacy_routing_assignment_available()
        } else {
            !self
                .state
                .routing_assignment_available(address.surface, address.strip)
        } {
            bail!("routing assignment control is unsupported");
        }
        let current_assignment = self
            .state
            .mixer
            .channels
            .get(surface_index)
            .and_then(|surface| surface.get(strip_index))
            .and_then(|channel| channel.assignment);

        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = strip_index;
        if !antelope_protocol::MixerStrip::assignment_write_is_grounded(strip) {
            self.state.ui.last_message =
                "Assignment picking is not grounded for the selected strip.".to_string();
            return Ok(());
        }

        self.state.popup.selected_index = current_assignment
            .and_then(|current| {
                MixerAssignment::grounded_choices()
                    .iter()
                    .position(|assignment| *assignment == current)
            })
            .unwrap_or(0);
        self.state.popup.assignment_picker = Some(AssignmentPickerState { strip });
        self.state.popup.assignment_picker_address = Some(address);
        self.state.ui.last_message = format!("Pick source assignment for CH {strip:02}");
        Ok(())
    }

    fn handle_pick_assignment(
        &mut self,
        strip: u8,
        assignment: MixerAssignment,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        if !self.state.legacy_routing_assignment_available() {
            bail!("routing assignment control is unsupported");
        }
        let address = self
            .state
            .popup
            .assignment_picker_address
            .unwrap_or(MixerAddress {
                surface: self
                    .state
                    .active_mixer_surface()
                    .and_then(|index| self.state.mixers().get(index))
                    .map_or(0, |surface| surface.surface),
                strip: u16::from(strip),
            });
        self.handle_pick_assignment_for(address, assignment, pending, 0, true)
    }

    fn handle_pick_assignment_at(
        &mut self,
        address: MixerAddress,
        assignment: MixerAssignment,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.handle_pick_assignment_for(
            address,
            assignment,
            pending,
            u16::from(address.surface),
            false,
        )
    }

    fn handle_pick_assignment_for(
        &mut self,
        address: MixerAddress,
        assignment: MixerAssignment,
        pending: Option<PendingMutation>,
        destination: u16,
        legacy: bool,
    ) -> Result<()> {
        let strip = u8::try_from(address.strip)
            .map_err(|_| anyhow::anyhow!("routing strip {} is out of range", address.strip))?;
        if if legacy {
            !self.state.legacy_routing_assignment_available()
        } else {
            !self
                .state
                .routing_assignment_available(address.surface, address.strip)
        } {
            bail!("routing assignment control is unsupported");
        }
        self.state.popup.assignment_picker = None;
        self.state.popup.assignment_picker_address = None;
        self.state.popup.selected_index = 0;
        let changed_channel = u16::from(
            strip
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("routing strip must be one-based"))?,
        );
        let mut sources = self.shared_assignment_sources(destination)?;
        let slot = sources
            .get_mut(usize::from(changed_channel))
            .ok_or_else(|| anyhow::anyhow!("invalid routing strip {strip}"))?;
        *slot = routing_source_from_assignment(assignment);
        self.send(
            Action::SetRoutingGroup {
                destination,
                changed_channel: Some(changed_channel),
                sources,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_close_assignment_picker(&mut self) {
        self.state.popup.assignment_picker = None;
        self.state.popup.assignment_picker_address = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Closed assignment picker".to_string();
    }

    fn handle_open_profiles_popup(&mut self) {
        self.state.popup.assignment_picker = None;
        self.state.popup.selector_popup = None;
        self.state.popup.routing_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.profile_names = crate::profile::list_profile_names().unwrap_or_default();
        self.state.clamp_profile_selection();
        self.state.popup.profiles_open = true;
        self.state.ui.last_message = if self.state.popup.profile_names.is_empty() {
            "No saved profiles yet. Use SAVE to create one.".to_string()
        } else {
            "Select a profile to load, or use SAVE/RENAME/DELETE.".to_string()
        };
    }

    fn handle_close_profiles_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.ui.last_message = "Closed profiles popup".to_string();
    }

    fn handle_select_profile(&mut self, index: usize) {
        self.state.popup.selected_index =
            index.min(self.state.popup.profile_names.len().saturating_sub(1));
    }

    fn handle_load_selected_profile(&mut self) {
        if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
            let profile_result = crate::profile::DeviceProfile::read_named(&name);
            match profile_result {
                Ok(profile) => {
                    let apply_result = self.apply_profile(&profile);
                    if let Err(e) = apply_result {
                        self.state.ui.last_message = format!("Profile error: {e}");
                    } else {
                        self.state.popup.profiles_open = false;
                        self.state.popup.profile_editor = None;
                        self.state.ui.last_message = format!("Loaded profile {name}");
                    }
                }
                Err(e) => {
                    self.state.ui.last_message = format!("Profile error: {e}");
                }
            }
        } else {
            self.state.ui.last_message = "No profile selected to load.".to_string();
        }
    }

    fn handle_start_save_profile(&mut self) {
        if self.state.popup.profiles_open {
            let current_name = self.state.selected_profile_name().map(str::to_string);
            let value = current_name.clone().unwrap_or_default();
            self.state.popup.profile_editor = Some(ProfileEditorState {
                mode: ProfileEditorMode::Save,
                original_name: current_name,
                value,
            });
            self.state.ui.last_message =
                "Enter a profile name, then press Enter to save.".to_string();
        }
    }

    fn handle_start_rename_profile(&mut self) {
        if self.state.selected_profile_name().is_some() {
            let current_name = self.state.selected_profile_name().map(str::to_string);
            let value = current_name.clone().unwrap_or_default();
            self.state.popup.profile_editor = Some(ProfileEditorState {
                mode: ProfileEditorMode::Rename,
                original_name: current_name,
                value,
            });
            self.state.ui.last_message =
                "Edit the profile name, then press Enter to rename.".to_string();
        } else {
            self.state.ui.last_message = "No profile selected to rename.".to_string();
        }
    }

    fn handle_delete_selected_profile(&mut self) {
        if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
            match crate::profile::delete_profile(&name) {
                Ok(()) => {
                    self.state.popup.profile_names =
                        crate::profile::list_profile_names().unwrap_or_default();
                    self.state.clamp_profile_selection();
                    self.state.ui.last_message = format!("Deleted profile {name}");
                }
                Err(e) => {
                    self.state.ui.last_message = format!("Profile error: {e}");
                }
            }
        } else {
            self.state.ui.last_message = "No profile selected to delete.".to_string();
        }
    }

    fn handle_profile_editor_char(&mut self, ch: String) {
        if let Some(editor) = self.state.popup.profile_editor.as_mut() {
            editor.value.push_str(&ch);
        }
    }

    fn handle_profile_editor_backspace(&mut self) {
        if let Some(editor) = self.state.popup.profile_editor.as_mut() {
            editor.value.pop();
        }
    }

    fn handle_profile_editor_commit(&mut self) {
        if let Some(editor) = self.state.popup.profile_editor.take() {
            let name = editor.value.trim().to_string();
            if name.is_empty() {
                self.state.ui.last_message = "Profile name cannot be empty".to_string();
                self.state.popup.profile_editor = Some(editor);
            } else {
                match editor.mode {
                    ProfileEditorMode::Save => {
                        let profile = DeviceProfile::capture(&self.state);
                        match profile {
                            Ok(profile) => match profile.write_named(&name) {
                                Ok(path) => {
                                    self.state.popup.profiles_open = false;
                                    self.state.ui.last_message =
                                        format!("Saved profile to {}", path.display());
                                }
                                Err(e) => {
                                    self.state.ui.last_message = format!("Profile error: {e}");
                                }
                            },
                            Err(e) => {
                                self.state.ui.last_message = format!("Profile error: {e}");
                            }
                        }
                    }
                    ProfileEditorMode::Rename => {
                        if let Some(original) = &editor.original_name {
                            if original != &name {
                                match crate::profile::rename_profile(original, &name) {
                                    Ok(_path) => {
                                        self.state.popup.profile_names =
                                            crate::profile::list_profile_names()
                                                .unwrap_or_default();
                                        self.state.clamp_profile_selection();
                                        self.state.ui.last_message =
                                            format!("Renamed {original} to {name}");
                                    }
                                    Err(e) => {
                                        self.state.ui.last_message = format!("Profile error: {e}");
                                    }
                                }
                            } else {
                                self.state.ui.last_message = "Profile name unchanged".to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_profile_editor_cancel(&mut self) {
        self.state.popup.profile_editor = None;
        self.state.ui.last_message = "Cancelled profile edit".to_string();
    }

    fn handle_select_surface(
        &mut self,
        surface: Surface,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.send(
            Action::SetGlobal {
                control: GlobalControl::Surface,
                value: ControlValue::Enum(i32::from(surface.code())),
            },
            pending,
        )?;
        self.flush_commands()?;
        self.refresh_queried_state()?;
        Ok(())
    }

    fn handle_cycle_focus(&mut self) {
        self.state.cycle_focus();
    }

    fn handle_move_popup_selection(&mut self, down: bool) {
        let item_count = if self.state.popup.assignment_picker.is_some() {
            antelope_protocol::MixerAssignment::grounded_choices().len()
        } else if self.state.popup.profiles_open {
            self.state.popup.profile_names.len()
        } else if let Some(popup) = self.state.popup.selector_popup {
            match popup.kind {
                SelectorPopupKind::SampleRate => SampleRate::all_confirmed().len(),
                SelectorPopupKind::ClockSource => ClockSource::all_confirmed().len(),
                SelectorPopupKind::PreampMode { .. } => 3,
            }
        } else {
            0
        };
        if item_count == 0 {
            return;
        }
        self.state.popup.selected_index = if down {
            (self.state.popup.selected_index + 1) % item_count
        } else {
            self.state
                .popup
                .selected_index
                .checked_sub(1)
                .unwrap_or(item_count - 1)
        };
    }

    fn handle_adjust_focused(
        &mut self,
        increase: bool,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        match self.state.ui.focus {
            FocusArea::Outputs => {
                let index = self.state.output.selected;
                let output = self.state.output.states[index];
                let next = if increase {
                    output.volume.saturating_sub(1)
                } else {
                    output.volume.saturating_add(1).min(0x60)
                };
                self.send(
                    Action::SetOutput {
                        address: OutputAddress {
                            id: u16::from(output.target.index()),
                        },
                        control: OutputControl::Level,
                        value: ControlValue::Int(i32::from(next)),
                    },
                    pending,
                )?;
            }
            FocusArea::Mixer => {
                let (address, _) = self.mixer_strip_at_ui(self.state.mixer.selected_channel)?;
                self.handle_adjust_mixer_level_at(address, increase)?;
            }
            FocusArea::Preamp => {
                let input = self
                    .state
                    .input_spaces
                    .first()
                    .and_then(|space| space.inputs.get(self.state.preamp.selected_input))
                    .ok_or_else(|| anyhow::anyhow!("selected input unavailable"))?;
                self.handle_adjust_input_gain_at(input.address, increase, pending)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_toggle_focused_mute(&mut self, pending: Option<PendingMutation>) -> Result<()> {
        match self.state.ui.focus {
            FocusArea::Outputs => {
                let index = self.state.output.selected;
                let output = self.state.output.states[index];
                self.send(
                    Action::SetOutput {
                        address: OutputAddress {
                            id: u16::from(output.target.index()),
                        },
                        control: OutputControl::Mute,
                        value: ControlValue::Bool(output.mode != OutputMode::Mute),
                    },
                    pending,
                )?;
            }
            FocusArea::Mixer => {
                let (address, strip) = self.mixer_strip_at_ui(self.state.mixer.selected_channel)?;
                self.ensure_mixer_control(address, MixerControl::Mute)?;
                self.send_complete_mixer_change(address, |slot| {
                    slot.muted = Some(!strip.muted.unwrap_or(false))
                })?;
            }
            FocusArea::Preamp => {
                let input = self
                    .state
                    .input_spaces
                    .first()
                    .and_then(|space| space.inputs.get(self.state.preamp.selected_input))
                    .ok_or_else(|| anyhow::anyhow!("selected input unavailable"))?;
                self.handle_toggle_input_phantom_at(input.address, pending)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_toggle_focused_dim(&mut self, pending: Option<PendingMutation>) -> Result<()> {
        if self.state.ui.focus == FocusArea::Outputs {
            let index = self.state.output.selected;
            let output = self.state.output.states[index];
            self.send(
                Action::SetOutput {
                    address: OutputAddress {
                        id: u16::from(output.target.index()),
                    },
                    control: OutputControl::Dim,
                    value: ControlValue::Bool(output.mode != OutputMode::Dim),
                },
                pending,
            )?;
        }
        Ok(())
    }

    fn handle_close_selector_popup(&mut self) {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Closed selector".to_string();
    }

    fn handle_refresh_queried_state(&mut self) -> Result<()> {
        self.refresh_queried_state()?;
        self.state.ui.last_message = "Sent captured 0x74 startup/state refresh sweep".to_string();
        Ok(())
    }
}

/// Converts a saved-profile mixer assignment to a normalized routing source.
pub(crate) fn routing_source_from_assignment(assignment: MixerAssignment) -> RoutingSource {
    match assignment {
        MixerAssignment::Preamp(channel) => RoutingSource {
            bank: 0x00,
            index: u16::from(channel - 1),
        },
        MixerAssignment::ComputerPlay(channel) => RoutingSource {
            bank: 0x01,
            index: u16::from(channel - 1),
        },
        MixerAssignment::SpdifIn(channel) => RoutingSource {
            bank: 0x02,
            index: u16::from(channel - 1),
        },
        MixerAssignment::Mute => RoutingSource {
            bank: 0x08,
            index: 0,
        },
        MixerAssignment::Oscillator(channel) => RoutingSource {
            bank: 0x09,
            index: u16::from(channel - 1),
        },
        MixerAssignment::EmuMic(channel) => RoutingSource {
            bank: 0x0a,
            index: u16::from(channel - 1),
        },
    }
}

fn step_fader(current: i32, increase: bool, semantics: antelope_protocol::FaderSemantics) -> i32 {
    let delta = match (increase, semantics.direction) {
        (true, antelope_protocol::FaderDirection::Direct)
        | (false, antelope_protocol::FaderDirection::Attenuation) => 1,
        _ => -1,
    };
    current
        .saturating_add(delta)
        .clamp(semantics.min, semantics.max)
}

fn next_preamp_gain_raw(current: u8, up: bool) -> u8 {
    if up {
        current.saturating_add(1).min(0x41)
    } else {
        current.saturating_sub(1)
    }
}

#[cfg(test)]
mod correction_tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use antelope_protocol::{
        CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
        DynamicOutputState, InputAddress, MixerAddress, MixerControl, OutputAddress, OutputControl,
        QueryRequest, RuntimeDriverKind,
    };

    use super::*;
    use crate::transport::MockTransport;

    struct AcceptingDriver {
        definition: DriverDefinition,
    }

    impl AcceptingDriver {
        fn new() -> Self {
            Self {
                definition: crate::device::builtin_zen_go_driver()
                    .expect("Zen Go driver")
                    .definition()
                    .clone(),
            }
        }
    }

    impl DeviceDriver for AcceptingDriver {
        fn definition(&self) -> &DriverDefinition {
            &self.definition
        }

        fn startup_requests(&self) -> &[QueryRequest] {
            &[]
        }

        fn encode(&self, _action: Action) -> std::result::Result<CommandBatch, DriverError> {
            Ok(CommandBatch {
                frames: vec![vec![0; 64]],
                refresh_requests: Vec::new(),
            })
        }

        fn decode(&self, _bytes: &[u8]) -> std::result::Result<Option<DeviceEvent>, DriverError> {
            Ok(None)
        }
    }

    #[test]
    fn typed_dynamic_addresses_validate_capability_and_exact_topology() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");

        let unknown = MixerAddress {
            surface: 0,
            strip: u16::MAX,
        };
        assert!(controller
            .apply_intent(
                Intent::SetMixerLevelAt {
                    address: unknown,
                    level: 20,
                },
                Rect::default(),
            )
            .is_err());
        assert!(transport.take_writes().is_empty());

        let mut entry = crate::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen Go entry")
            .clone();
        entry
            .profile
            .params
            .retain(|param| param.name != "mix_solo");
        controller.state = AppState::from_entry(&entry);
        assert!(controller
            .apply_intent(
                Intent::ToggleMixerSoloAt {
                    address: MixerAddress {
                        surface: 0,
                        strip: 1
                    },
                },
                Rect::default(),
            )
            .is_err());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn typed_master_and_non_first_input_addresses_reach_controller() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        let mut entry = crate::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen Go entry")
            .clone();
        entry.driver_kind = RuntimeDriverKind::ZenGo;
        let mut space = entry.profile.address_spaces[0].clone();
        space.id = "second".into();
        space.space_id = 9;
        space.count = Some(1);
        entry.profile.address_spaces.push(space);
        let mut input = entry.profile.inputs[0].clone();
        input.id = "second_1".into();
        input.space = "second".into();
        input.space_id = 9;
        input.index = 0;
        entry.profile.inputs.push(input);
        controller.state = AppState::from_entry(&entry);
        let mut master = controller.state.mixers()[0].strips[0].clone();
        master.strip = 0;
        master.name = "Master".into();
        master.fader = Some(32);
        master.pan = Some(32);
        master.muted = Some(false);
        master.soloed = Some(false);
        controller.state.mixers_mut()[0].master = Some(master);

        controller
            .apply_intent(
                Intent::SetMixerLevelAt {
                    address: MixerAddress {
                        surface: 0,
                        strip: 0,
                    },
                    level: 18,
                },
                Rect::default(),
            )
            .expect("master control");
        assert_eq!(transport.take_writes().len(), 1);

        controller
            .apply_intent(
                Intent::SetInputGainAt {
                    address: InputAddress { space: 9, index: 0 },
                    raw: 12,
                },
                Rect::default(),
            )
            .expect("second input space");
        controller.flush_commands().expect("flush typed input");
        assert_eq!(transport.take_writes().len(), 1);
    }

    #[test]
    fn profile_mixer_surface_selection_is_navigation_only() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        let target = controller.state.mixers()[1].surface;

        controller
            .apply_intent(
                Intent::SelectMixerSurface { surface: target },
                Rect::default(),
            )
            .expect("select declared mixer surface");

        assert_eq!(controller.state.mixer.surface_index, 1);
        assert!(transport.take_writes().is_empty());
        let previous = controller.state.mixer.surface_index;
        assert!(controller
            .apply_intent(
                Intent::SelectMixerSurface { surface: u8::MAX },
                Rect::default(),
            )
            .is_err());
        assert_eq!(controller.state.mixer.surface_index, previous);
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn assignment_picker_with_empty_mixer_geometry_is_safe_and_does_not_write() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        controller.state.mixer.surfaces.clear();
        controller.state.mixer.channels.clear();

        let result = catch_unwind(AssertUnwindSafe(|| {
            controller.apply_intent(Intent::OpenAssignmentPicker(1), Rect::default())
        }));
        assert!(result.is_ok(), "empty mixer geometry must not panic");
        let error = result
            .expect("no panic")
            .expect_err("missing mixer strip must return an error");
        assert!(error.to_string().contains("mixer surface"));
        assert!(controller.state.popup.assignment_picker.is_none());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn saturated_queue_rejects_partial_mixer_without_state_mutation() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        for strip in 1..=64 {
            assert!(controller
                .command_queue
                .enqueue(Action::SetMixerStripState {
                    address: MixerAddress { surface: 1, strip },
                    fader: 32,
                    pan: 32,
                    muted: false,
                    soloed: false,
                    send: None,
                }));
        }

        let address = MixerAddress {
            surface: 0,
            strip: 1,
        };
        let strip = &mut controller.state.mixers_mut()[0].strips[0];
        strip.fader = Some(20);
        strip.pan = Some(31);
        strip.muted = Some(false);
        strip.soloed = Some(true);
        controller.state.sync_compatibility_views();

        let dynamic_before = controller.state.mixers()[0].strips[0].clone();
        let compatibility_before = controller.state.mixer.channels[0][0].clone();
        let queue_len_before = controller.command_queue.len();
        let mut original_pending: DynamicOutputState = controller.state.outputs()[2].clone();
        original_pending.level = Some(55);
        controller.pending_mutation = Some(PendingMutation::Output(original_pending.clone()));
        controller.state.ui.last_message = "unchanged".into();
        let writes_before = transport.take_writes();

        let error = controller
            .send(
                Action::SetMixer {
                    address,
                    control: MixerControl::Fader,
                    value: ControlValue::Int(21),
                },
                Some(PendingMutation::Mixer(Vec::new())),
            )
            .expect_err("partial mixer action must be rejected when queue is full");

        assert!(error.to_string().contains("queue"));
        assert_eq!(transport.take_writes(), writes_before);
        assert_eq!(controller.command_queue.len(), queue_len_before);
        assert_eq!(controller.state.mixers()[0].strips[0], dynamic_before);
        assert_eq!(controller.state.mixer.channels[0][0], compatibility_before);
        match controller.pending_mutation.as_ref() {
            Some(PendingMutation::Output(output)) => assert_eq!(output, &original_pending),
            other => panic!("pending mutation changed: {other:?}"),
        }
        assert_eq!(controller.state.ui.last_message, "unchanged");
    }

    #[test]
    fn full_command_queue_rejects_output_before_projection() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        for strip in 1..=64 {
            assert!(controller
                .command_queue
                .enqueue(Action::SetMixerStripState {
                    address: MixerAddress { surface: 0, strip },
                    fader: 32,
                    pan: 32,
                    muted: false,
                    soloed: false,
                    send: None,
                }));
        }
        assert_eq!(controller.command_queue.len(), 64);

        let dynamic_before = controller.state.outputs().to_vec();
        let compatibility_before = controller.state.output.states.clone();
        let mut original_pending: DynamicOutputState = controller.state.outputs()[2].clone();
        original_pending.level = Some(55);
        controller.pending_mutation = Some(PendingMutation::Output(original_pending.clone()));
        controller.state.ui.last_message = "unchanged".into();

        let mut attempted_pending = controller.state.outputs()[0].clone();
        attempted_pending.level = Some(17);
        let error = controller
            .send(
                Action::SetOutput {
                    address: OutputAddress { id: 0 },
                    control: OutputControl::Level,
                    value: ControlValue::Int(17),
                },
                Some(PendingMutation::Output(attempted_pending)),
            )
            .expect_err("new output key must be rejected when queue is full");

        assert!(error.to_string().contains("queue"));
        assert_eq!(controller.state.outputs(), dynamic_before);
        assert_eq!(controller.state.output.states, compatibility_before);
        match controller.pending_mutation.as_ref() {
            Some(PendingMutation::Output(output)) => assert_eq!(output, &original_pending),
            other => panic!("pending mutation changed: {other:?}"),
        }
        assert_eq!(controller.command_queue.len(), 64);
        assert_eq!(controller.state.ui.last_message, "unchanged");
        assert!(transport.take_writes().is_empty());
    }
}
