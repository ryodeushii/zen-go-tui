use std::{
    collections::HashMap,
    error::Error as StdError,
    fmt,
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use ratatui::layout::Rect;

use crate::command_queue::{CommandQueue, QueueEntryId, QueueEntryOutcome};
use crate::profile::DeviceProfile;
use crate::transport::Transport;
use antelope_protocol::{
    Action, AuraVerbParameter, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverError,
    DynamicStatePatch, GlobalControl, InputAddress, InputControl, MixerAddress, MixerAssignment,
    MixerControl, MixerSurface, OutputAddress, OutputControl, OutputMode, OutputTrimAddress,
    PanState, PreampMode, QueryRequest, RoutingSource, RuntimeEntry, SampleRate, Surface,
    SurroundGlobalControl,
};

use super::picker::{
    AssignmentPickerState, RoutingEditorState, RoutingSourcePickerState, SelectorPopupKind,
    SelectorPopupState,
};
use super::profile_editor::{ProfileEditorMode, ProfileEditorState};
use super::types::{
    AuraVerbControlFocus, FocusArea, Intent, PeakHoldDuration, PendingMutation, RawMapScope,
    RawPacketTab, RefreshRate, SurroundControlFocus, UiPage,
};
use super::AppState;

pub(crate) const MAX_FRAMES_PER_POLL: usize = 32;
const AURAVERB_READBACK_TIMEOUT: Duration = Duration::from_secs(2);
const SURROUND_READBACK_TIMEOUT: Duration = Duration::from_secs(2);

/// An AuraVerb write was rejected because this session lacks fresh complete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuraVerbWriteUnavailable;

impl fmt::Display for AuraVerbWriteUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AuraVerb write requires authoritative or pending Mix-1 readback state"
        )
    }
}

impl StdError for AuraVerbWriteUnavailable {}

pub fn is_auraverb_write_unavailable(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<AuraVerbWriteUnavailable>().is_some())
}

/// A Surround write was rejected because this session no longer has writable readback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurroundWriteUnavailable;

impl fmt::Display for SurroundWriteUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Surround global write requires authoritative or pending captured 2.0 state"
        )
    }
}

impl StdError for SurroundWriteUnavailable {}

pub fn is_surround_write_unavailable(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<SurroundWriteUnavailable>().is_some())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollWritePolicy {
    FlushPending,
    ReceiveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkedMixerBehavior {
    MirrorPair,
    TargetOnly,
}

struct QueuedMutation {
    id: QueueEntryId,
    pending: Option<PendingMutation>,
    clock: Option<QueuedClockMutation>,
}

#[derive(Debug, Clone, Copy)]
struct QueuedClockMutation {
    authoritative: Option<i32>,
    requested: i32,
    readback_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOutputModeStatus {
    Queued(QueueEntryId),
    AwaitingReadback,
    /// Delivery was uncertain; no protocol freshness barrier can safely release this guard.
    Recovery,
}

#[derive(Debug, Clone, Copy)]
struct PendingOutputMode {
    expected: OutputMode,
    status: PendingOutputModeStatus,
}

pub struct Controller {
    transport: Box<dyn Transport>,
    driver: Box<dyn DeviceDriver>,
    pub state: AppState,
    pub(crate) pending_mutation: Option<PendingMutation>,
    command_queue: CommandQueue,
    queued_mutations: Vec<QueuedMutation>,
    /// Zen Go output mode writes remain guarded until expected readback; uncertain delivery
    /// stays locked for the session because snapshots have no freshness correlation.
    pending_output_modes: HashMap<OutputAddress, PendingOutputMode>,
    clock_readback_revision: u64,
    auraverb_readback_deadline: Option<Instant>,
    surround_readback_deadline: Option<Instant>,
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
                queued_mutations: Vec::new(),
                pending_output_modes: HashMap::new(),
                clock_readback_revision: 0,
                auraverb_readback_deadline: None,
                surround_readback_deadline: None,
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
            queued_mutations: Vec::new(),
            pending_output_modes: HashMap::new(),
            clock_readback_revision: 0,
            auraverb_readback_deadline: None,
            surround_readback_deadline: None,
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
                    pan: i32::from(PanState::from_raw(strip.pan_raw).display_percent()),
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
        let clock_transition = match &action {
            Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(requested),
            } => Some(QueuedClockMutation {
                authoritative: self.state.device.status.clock_source,
                requested: *requested,
                readback_revision: self.clock_readback_revision,
            }),
            _ => None,
        };
        let output_mode_transition = if self.state.uses_zen_go_output_safety() {
            match &action {
                Action::SetOutput {
                    address,
                    control,
                    value: ControlValue::Bool(enabled),
                } => match control {
                    OutputControl::Mute => Some((
                        *address,
                        if *enabled {
                            OutputMode::Mute
                        } else {
                            OutputMode::Normal
                        },
                    )),
                    OutputControl::Dim => Some((
                        *address,
                        if *enabled {
                            OutputMode::Dim
                        } else {
                            OutputMode::Normal
                        },
                    )),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        };
        if let Action::SetOutput {
            address,
            control,
            value: ControlValue::Bool(enabled),
        } = &action
        {
            if matches!(control, OutputControl::Mute | OutputControl::Dim) {
                self.ensure_output_mode_action_allowed(*address, *control, *enabled)?;
            }
        }
        let batch = self.driver.encode(action.clone())?;
        let queueable = !matches!(
            action,
            Action::SetRoutingGroup { .. }
                | Action::SetGlobal {
                    control: GlobalControl::TalkbackButton,
                    ..
                }
        ) && batch.frames.len() == 1
            && batch.refresh_requests.is_empty();
        if queueable {
            let id = self
                .command_queue
                .enqueue_with_id(action.clone())
                .ok_or_else(|| anyhow::anyhow!("command queue is full; action was not enqueued"))?;
            self.remember_queued_mutation(id, pending, clock_transition);
            if let Some((address, expected)) = output_mode_transition {
                self.pending_output_modes.insert(
                    address,
                    PendingOutputMode {
                        expected,
                        status: PendingOutputModeStatus::Queued(id),
                    },
                );
            }
        } else {
            self.flush_commands()?;
            if let Some((address, expected)) = output_mode_transition {
                self.pending_output_modes.insert(
                    address,
                    PendingOutputMode {
                        expected,
                        status: PendingOutputModeStatus::Recovery,
                    },
                );
            }
            if let Err(error) = self.write_batch(batch) {
                if let Some((address, expected)) = output_mode_transition {
                    self.mark_uncertain_output_mode(address, expected);
                }
                return Err(error);
            }
            if let Some((address, pending_mode)) = output_mode_transition {
                if let Some(guard) = self.pending_output_modes.get_mut(&address) {
                    guard.status = PendingOutputModeStatus::AwaitingReadback;
                    guard.expected = pending_mode;
                }
            }
            self.pending_mutation = pending;
        }
        if project_completed_mixer || !matches!(action, Action::SetMixerStripState { .. }) {
            self.apply_command_state_update(&action);
        }
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
                        if *muted && !self.state.uses_zen_go_output_safety() {
                            output.dimmed = Some(false);
                        }
                    }
                    (OutputControl::Dim, ControlValue::Bool(dimmed)) => {
                        output.dimmed = Some(*dimmed);
                        if *dimmed && !self.state.uses_zen_go_output_safety() {
                            output.muted = Some(false);
                        }
                    }
                    // Mono remains device-authoritative. The queued mutation is
                    // promoted only after a successful transport write and a
                    // subsequent device event; failed delivery never lights it.
                    (OutputControl::Mono, ControlValue::Bool(_)) => return,
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
                self.state.device.status.clock_source = Some(*value);
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

    fn remember_queued_mutation(
        &mut self,
        id: QueueEntryId,
        pending: Option<PendingMutation>,
        clock: Option<QueuedClockMutation>,
    ) {
        if let Some(queued) = self
            .queued_mutations
            .iter_mut()
            .find(|queued| queued.id == id)
        {
            queued.pending = pending;
            if let Some(clock) = clock {
                if let Some(existing) = queued
                    .clock
                    .as_mut()
                    .filter(|existing| existing.readback_revision == clock.readback_revision)
                {
                    existing.requested = clock.requested;
                } else {
                    queued.clock = Some(clock);
                }
            }
        } else {
            self.queued_mutations
                .push(QueuedMutation { id, pending, clock });
        }
    }

    fn input_gain_control(state: &AppState, address: InputAddress) -> Option<InputControl> {
        state
            .ui_profile
            .input_capabilities(address)
            .iter()
            .find(|capability| capability.kind == antelope_protocol::RuntimeInputControlKind::Gain)
            .and_then(|capability| capability.control)
    }

    fn input_mode_is_declared(state: &AppState, address: InputAddress, mode: i32) -> Option<bool> {
        let profile = state.runtime_profile.as_ref()?;
        let input = profile
            .inputs
            .iter()
            .find(|input| input.space_id == address.space && input.index == address.index)?;
        let space = profile
            .address_spaces
            .iter()
            .find(|space| space.space_id == input.space_id)?;
        let capability = space.input_capabilities.iter().find(|capability| {
            capability.kind == antelope_protocol::RuntimeInputControlKind::Mode
        })?;
        let parameter = profile
            .params
            .iter()
            .find(|param| param.name == capability.parameter)?;
        Some(parameter.values.iter().any(|(value, _)| *value == mode))
    }

    fn validate_queued_input_gain(
        state: &AppState,
        effective_modes: &mut HashMap<InputAddress, Option<i32>>,
        action: &Action,
    ) -> Result<()> {
        let Action::SetInput {
            address,
            control,
            value,
        } = action
        else {
            return Ok(());
        };
        if *control == InputControl::Mode {
            if let ControlValue::Enum(mode) = value {
                if Self::input_mode_is_declared(state, *address, *mode) == Some(false) {
                    bail!("queued input mode {mode} is not declared for {address:?}");
                }
                effective_modes.insert(*address, Some(*mode));
            }
            return Ok(());
        }
        if Self::input_gain_control(state, *address) != Some(*control) {
            return Ok(());
        }
        let ControlValue::Int(raw) = value else {
            return Ok(());
        };
        let mode = *effective_modes.entry(*address).or_insert_with(|| {
            state
                .input_spaces
                .iter()
                .find(|space| space.space_id == address.space)
                .and_then(|space| space.inputs.iter().find(|input| input.address == *address))
                .and_then(|input| input.mode)
        });
        let (minimum, maximum) = state
            .input_range(*address, mode)
            .ok_or_else(|| anyhow::anyhow!("input gain range unavailable before queue flush"))?;
        if !(minimum..=maximum).contains(raw) {
            bail!(
                "queued input gain {raw} outside effective mode range {minimum}..={maximum} before queue flush"
            );
        }
        Ok(())
    }

    /// Flushes all pending commands from the queue to the transport.
    pub fn flush_commands(&mut self) -> Result<()> {
        let mut queued_mutations = std::mem::take(&mut self.queued_mutations);
        let mut pending_mutation = self.pending_mutation.take();
        let mut outcomes = Vec::new();
        let mut effective_input_modes = HashMap::new();
        let result = self.command_queue.flush_with_validation(
            self.transport.as_ref(),
            self.driver.as_ref(),
            |action| {
                Self::validate_queued_input_gain(&self.state, &mut effective_input_modes, action)
            },
            |id, outcome| outcomes.push((id, outcome)),
        );
        for (id, outcome) in outcomes {
            let mut queued = queued_mutations
                .iter()
                .position(|queued| queued.id == id)
                .map(|index| queued_mutations.swap_remove(index));
            if outcome == QueueEntryOutcome::Sent {
                if let Some(queued) = queued.as_mut() {
                    pending_mutation = queued.pending.take();
                }
            }
            self.handle_queue_outcome(id, outcome, queued.and_then(|queued| queued.clock));
        }
        self.pending_mutation = pending_mutation;
        self.queued_mutations.clear();
        result.map(|_| ())
    }

    fn handle_queue_outcome(
        &mut self,
        id: QueueEntryId,
        outcome: QueueEntryOutcome,
        clock: Option<QueuedClockMutation>,
    ) {
        if let Some(clock) = clock {
            self.reconcile_clock_queue_outcome(clock, outcome);
        }

        let Some(address) = self
            .pending_output_modes
            .iter()
            .find(|(_, guard)| guard.status == PendingOutputModeStatus::Queued(id))
            .map(|(address, _)| *address)
        else {
            return;
        };
        match outcome {
            QueueEntryOutcome::Sent => {
                if let Some(guard) = self.pending_output_modes.get_mut(&address) {
                    guard.status = PendingOutputModeStatus::AwaitingReadback;
                }
            }
            QueueEntryOutcome::Failed => {
                if let Some(guard) = self.pending_output_modes.get_mut(&address) {
                    guard.status = PendingOutputModeStatus::Recovery;
                }
                self.invalidate_output_mode(address);
            }
            QueueEntryOutcome::Unsent => {
                self.pending_output_modes.remove(&address);
                self.invalidate_output_mode(address);
            }
        }
    }

    fn reconcile_clock_queue_outcome(
        &mut self,
        clock: QueuedClockMutation,
        outcome: QueueEntryOutcome,
    ) {
        if outcome == QueueEntryOutcome::Sent
            || clock.readback_revision != self.clock_readback_revision
        {
            return;
        }
        self.state.device.status.clock_source = match outcome {
            QueueEntryOutcome::Failed => None,
            QueueEntryOutcome::Unsent => clock.authoritative,
            QueueEntryOutcome::Sent => unreachable!(),
        };
    }

    fn mark_uncertain_output_mode(&mut self, address: OutputAddress, expected: OutputMode) {
        self.pending_output_modes.insert(
            address,
            PendingOutputMode {
                expected,
                status: PendingOutputModeStatus::Recovery,
            },
        );
        self.invalidate_output_mode(address);
    }

    fn invalidate_output_mode(&mut self, address: OutputAddress) {
        if let Some(output) = self
            .state
            .output
            .dynamic
            .iter_mut()
            .find(|output| output.address == address)
        {
            output.muted = None;
            output.dimmed = None;
        }
        if let Some(state) = self.state.output.states.get_mut(usize::from(address.id)) {
            state.mode = OutputMode::Unknown(u8::MAX);
        }
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
        self.send_complete_mixer_change_with_link_behavior(
            address,
            LinkedMixerBehavior::MirrorPair,
            mutate,
        )
    }

    fn send_target_mixer_change<F>(&mut self, address: MixerAddress, mutate: F) -> Result<()>
    where
        F: Fn(&mut antelope_protocol::DynamicMixerStrip) + Copy,
    {
        self.send_complete_mixer_change_with_link_behavior(
            address,
            LinkedMixerBehavior::TargetOnly,
            mutate,
        )
    }

    fn send_complete_mixer_change_with_link_behavior<F>(
        &mut self,
        address: MixerAddress,
        linked_behavior: LinkedMixerBehavior,
        mutate: F,
    ) -> Result<()>
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
            let indexes = match linked_behavior {
                LinkedMixerBehavior::TargetOnly => vec![strip_index],
                LinkedMixerBehavior::MirrorPair => match surface.strips[strip_index].linked {
                    Some(true) => {
                        let left = strip_index - (strip_index % 2);
                        vec![
                            left,
                            left.checked_add(1)
                                .ok_or_else(|| anyhow::anyhow!("linked mixer pair overflow"))?,
                        ]
                    }
                    Some(false) | None => vec![strip_index],
                },
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
        self.state.normalize_ui_page();
        let pending = intent.pending_mutation(&self.state);
        if !matches!(
            intent,
            Intent::SetAuraVerbEnabled(_) | Intent::SetAuraVerbParameter { .. }
        ) {
            self.state.ui.auraverb_drag = None;
        }
        if !matches!(
            intent,
            Intent::SetSurroundGlobalLevel(_) | Intent::SetSurroundGlobalDelay(_)
        ) {
            self.state.ui.surround_drag = None;
        }
        match intent {
            Intent::Quit => {
                self.state.ui.quit_requested = true;
            }
            Intent::SelectUiPage(page) => self.handle_select_ui_page(page),
            Intent::ToggleRawView => self.state.toggle_raw_view(),
            Intent::ToggleHotkeysPopup => self.state.toggle_hotkeys_popup(),
            Intent::OpenProfilesPopup => self.handle_open_profiles_popup(),
            Intent::CloseProfilesPopup => self.handle_close_profiles_popup(),
            Intent::OpenRoutingPopup => self.handle_open_routing_popup(),
            Intent::CloseRoutingPopup => self.handle_close_routing_popup(),
            Intent::SelectRoutingDestination { destination } => {
                self.handle_select_routing_destination(destination)?
            }
            Intent::SelectRoutingChannel {
                destination,
                channel,
            } => self.handle_select_routing_channel(destination, channel)?,
            Intent::OpenRoutingSourcePicker {
                destination,
                channel,
            } => self.handle_open_routing_source_picker(destination, channel)?,
            Intent::CloseRoutingSourcePicker => self.handle_close_routing_source_picker(),
            Intent::PickRoutingSource {
                destination,
                channel,
                source,
            } => self.handle_pick_routing_source(destination, channel, source, pending)?,
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
            Intent::OpenSettingsSelector => self.handle_open_settings_selector(),
            Intent::OpenBrightnessSelector => self.handle_open_brightness_selector(),
            Intent::OpenOutputTrimSelector(address) => {
                self.handle_open_output_trim_selector(address)
            }
            Intent::OpenTalkbackButton => self.handle_open_talkback_button(),
            Intent::OpenTalkbackSourceSelector => self.handle_open_talkback_source_selector(),
            Intent::OpenTalkbackGainSelector => self.handle_open_talkback_gain_selector(),
            Intent::SelectRawPacketTab(tab) => self.handle_select_raw_packet_tab(tab),
            Intent::SelectRawMapScope(scope) => self.handle_select_raw_map_scope(scope),
            Intent::CycleRawMapScope { forward } => self.handle_cycle_raw_map_scope(forward),
            Intent::ScrollRawDump { increase, page } => self.handle_scroll_raw_dump(increase, page),
            Intent::SelectOutput(index) => self.handle_output_select(index),
            Intent::SetAuraVerbEnabled(enabled) => self.handle_set_auraverb(None, Some(enabled))?,
            Intent::SetAuraVerbParameter { parameter, value } => {
                self.handle_set_auraverb(Some((parameter, value)), None)?
            }
            Intent::SetSurroundGlobalLevel(value) => {
                self.handle_set_surround_global(SurroundGlobalControl::Level, value)?
            }
            Intent::SetSurroundGlobalDelay(value) => {
                self.handle_set_surround_global(SurroundGlobalControl::Delay, u16::from(value))?
            }
            Intent::AdjustOutputLevel { index, increase } => {
                self.handle_output_adjust(index, increase, pending)?
            }
            Intent::SetOutputLevel { index, step } => {
                self.handle_output_set_level(index, step, pending)?
            }
            Intent::ToggleOutputDim(index) => self.handle_output_toggle_dim(index, pending)?,
            Intent::ToggleOutputMute(index) => self.handle_output_toggle_mute(index, pending)?,
            Intent::ToggleOutputMono(index) => self.handle_output_toggle_mono(index, pending)?,
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
            Intent::PickRoutingSourceAt { address, source } => {
                self.handle_pick_routing_source_at(address, source, pending)?
            }
            Intent::CloseAssignmentPicker => self.handle_close_assignment_picker(),
            Intent::CloseSelectorPopup => self.handle_close_selector_popup()?,
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
            Intent::PickBrightness(value) => self.handle_pick_brightness(value)?,
            Intent::PickOutputTrim { address, value } => {
                self.handle_pick_output_trim(address, value)?
            }
            Intent::SetTalkbackButton(pressed) => self.handle_set_talkback_button(pressed)?,
            Intent::PickTalkbackSource(source) => self.handle_pick_talkback_source(source)?,
            Intent::PickTalkbackGain(gain) => self.handle_pick_talkback_gain(gain)?,
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
            Intent::SetInputPairLink { address, enabled } => {
                self.handle_set_input_pair_link(address, enabled)?
            }
            Intent::AdjustFocused(increase) => self.handle_adjust_focused(increase, pending)?,
            Intent::ToggleFocusedMute => self.handle_toggle_focused_mute(pending)?,
            Intent::ToggleFocusedDim => self.handle_toggle_focused_dim(pending)?,
            Intent::ToggleRoutingPopup => self.handle_toggle_routing_popup(),
            Intent::RefreshQueriedState => self.handle_refresh_queried_state()?,
            Intent::CycleFocus => self.handle_cycle_focus(),
            Intent::CycleSurroundFocus { forward } => self.handle_cycle_surround_focus(forward),
            Intent::SelectSurroundControl(focus) => {
                self.state.ui.surround_focus = focus;
                if matches!(
                    focus,
                    SurroundControlFocus::Speaker | SurroundControlFocus::EqBank
                ) {
                    self.state.ui.surround_drag = None;
                }
            }
            Intent::NavigateSurroundEq { focus, forward } => {
                self.handle_navigate_surround_eq(focus, forward)
            }
            Intent::SelectAuraVerbControl(focus) => self.state.ui.auraverb_focus = focus,
            Intent::CycleAuraVerbFocus { forward } => {
                self.handle_cycle_auraverb_focus(forward, area)
            }
            Intent::ScrollAuraVerbPage { down } => self.handle_scroll_auraverb_page(down, area),
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

    fn handle_set_auraverb(
        &mut self,
        parameter_change: Option<(AuraVerbParameter, u8)>,
        enabled_change: Option<bool>,
    ) -> Result<()> {
        self.expire_auraverb_readback();
        if !self.state.auraverb_controls_enabled() {
            self.state.ui.auraverb_drag = None;
            return Err(AuraVerbWriteUnavailable.into());
        }
        let mut expected =
            self.state
                .auraverb
                .as_ref()
                .and_then(|cache| match cache.freshness {
                    super::AuraVerbFreshness::Authoritative => cache.state.clone(),
                    super::AuraVerbFreshness::PendingReadback => cache.pending_expected.clone(),
                    super::AuraVerbFreshness::AwaitingReadback
                    | super::AuraVerbFreshness::Stale => None,
                })
                .ok_or(AuraVerbWriteUnavailable)?;
        if let Some((parameter, value)) = parameter_change {
            expected.set_value(parameter, value);
        }
        if let Some(enabled) = enabled_change {
            expected.enabled = enabled;
        }
        let (operation, target) = self
            .state
            .auraverb_contract()
            .map(|contract| (contract.operation, contract.target))
            .ok_or(AuraVerbWriteUnavailable)?;
        let batch = self.driver.encode(Action::SetWholeState {
            operation,
            target,
            enabled: expected.enabled,
            fields: expected.whole_state_fields(),
        })?;
        self.flush_commands()?;
        if let Err(error) = self.write_batch(batch) {
            if let Some(cache) = self.state.auraverb.as_mut() {
                cache.freshness = super::AuraVerbFreshness::Stale;
            }
            self.state.ui.auraverb_drag = None;
            self.auraverb_readback_deadline = None;
            return Err(error);
        }
        if let Some(cache) = self.state.auraverb.as_mut() {
            cache.pending_expected = Some(expected);
            cache.freshness = super::AuraVerbFreshness::PendingReadback;
            self.auraverb_readback_deadline = Some(Instant::now() + AURAVERB_READBACK_TIMEOUT);
        }
        self.state.ui.last_message = "Sent complete AuraVerb Mix-1 state".into();
        Ok(())
    }

    fn handle_set_surround_global(
        &mut self,
        control: SurroundGlobalControl,
        value: u16,
    ) -> Result<()> {
        self.expire_surround_readback();
        let template =
            self.state
                .surround_global
                .as_ref()
                .and_then(|cache| match cache.freshness {
                    super::SurroundFreshness::Authoritative => cache.state.as_ref(),
                    super::SurroundFreshness::PendingReadback => cache.pending_expected.as_ref(),
                    super::SurroundFreshness::AwaitingReadback
                    | super::SurroundFreshness::Stale => None,
                })
                .filter(|state| state.writable)
                .map(|state| state.template.clone())
                .ok_or(SurroundWriteUnavailable)?;
        let expected = self
            .driver
            .expected_surround_global_state(&template, control, value)?;
        let batch = self.driver.encode(Action::SetSurroundGlobal {
            template,
            control,
            value,
        })?;
        self.flush_commands()?;
        if let Err(error) = self.write_batch(batch) {
            if let Some(cache) = self.state.surround_global.as_mut() {
                cache.freshness = super::SurroundFreshness::Stale;
            }
            self.state.ui.surround_drag = None;
            self.surround_readback_deadline = None;
            return Err(error);
        }
        if let Some(cache) = self.state.surround_global.as_mut() {
            cache.pending_expected = Some(expected);
            cache.freshness = super::SurroundFreshness::PendingReadback;
            self.surround_readback_deadline = Some(Instant::now() + SURROUND_READBACK_TIMEOUT);
        }
        self.state.ui.last_message = format!("Sent Surround {control:?} {value}");
        Ok(())
    }

    fn clock_source_readback(event: &DeviceEvent) -> Option<i32> {
        let globals = match event {
            DeviceEvent::Snapshot { state, .. } => &state.globals,
            DeviceEvent::QueryReply {
                patch: Some(DynamicStatePatch::Globals(globals)),
                ..
            } => globals,
            _ => return None,
        };
        globals.iter().find_map(|global| match global {
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(value),
            } => Some(*value),
            _ => None,
        })
    }

    fn snapshot_output_mode(event: &DeviceEvent, address: OutputAddress) -> Option<OutputMode> {
        let DeviceEvent::Snapshot { state, .. } = event else {
            return None;
        };
        let output = state
            .outputs
            .iter()
            .find(|output| output.address == address)?;
        match (output.muted, output.dimmed) {
            (Some(false), Some(false)) => Some(OutputMode::Normal),
            (Some(true), Some(false)) => Some(OutputMode::Mute),
            (Some(false), Some(true)) => Some(OutputMode::Dim),
            _ => None,
        }
    }

    fn resolve_output_mode_snapshots(
        &mut self,
        snapshot_modes: &[(OutputAddress, Option<OutputMode>)],
    ) {
        for (address, mode) in snapshot_modes {
            let Some(mode) = mode else {
                continue;
            };
            let remove =
                self.pending_output_modes
                    .get(address)
                    .is_some_and(|guard| match guard.status {
                        PendingOutputModeStatus::Queued(_) => false,
                        // Successful queued writes use best-effort expected-mode
                        // confirmation; a queued entry itself remains protected.
                        PendingOutputModeStatus::AwaitingReadback => guard.expected == *mode,
                        // Failed delivery has no freshness barrier, so a snapshot
                        // cannot prove that the device applied or rejected the write.
                        PendingOutputModeStatus::Recovery => false,
                    });
            if remove {
                self.pending_output_modes.remove(address);
            }
        }
    }

    // An expired AuraVerb transaction locks writes until a fresh Controller/device session.
    // Late untagged replies cannot distinguish old state from a new authorization boundary.
    fn expire_auraverb_readback(&mut self) -> bool {
        let expired = self
            .auraverb_readback_deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if !expired {
            return false;
        }
        self.auraverb_readback_deadline = None;
        self.state.ui.auraverb_drag = None;
        let Some(cache) = self.state.auraverb.as_mut() else {
            return false;
        };
        if cache.freshness != super::AuraVerbFreshness::PendingReadback {
            return false;
        }
        cache.freshness = super::AuraVerbFreshness::Stale;
        true
    }

    // An expired Surround transaction locks writes until a fresh Controller/device session.
    // Late replies cannot safely restore authority because the protocol has no transaction ID.
    fn expire_surround_readback(&mut self) -> bool {
        let expired = self
            .surround_readback_deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if !expired {
            return false;
        }
        self.surround_readback_deadline = None;
        let Some(cache) = self.state.surround_global.as_mut() else {
            return false;
        };
        if cache.freshness != super::SurroundFreshness::PendingReadback {
            return false;
        }
        cache.freshness = super::SurroundFreshness::Stale;
        self.state.ui.surround_drag = None;
        true
    }

    pub fn poll_device(&mut self, timeout: Duration) -> Result<bool> {
        self.poll_device_with_policy(timeout, PollWritePolicy::FlushPending)
    }

    /// Drain received device events without sending queued commands.
    ///
    /// The runtime uses this while the device selector is open so the current
    /// session does not accumulate frames, while an explicit switch can still
    /// discard the old controller and its queued commands before opening the
    /// replacement session.
    pub fn poll_device_without_writes(&mut self, timeout: Duration) -> Result<bool> {
        self.poll_device_with_policy(timeout, PollWritePolicy::ReceiveOnly)
    }

    fn poll_device_with_policy(
        &mut self,
        timeout: Duration,
        write_policy: PollWritePolicy,
    ) -> Result<bool> {
        if write_policy == PollWritePolicy::FlushPending {
            // Flush pending commands before reading so device sees latest state.
            self.flush_commands()?;
        }

        let mut next_timeout = timeout;
        let mut state_dirty = self.expire_auraverb_readback();
        state_dirty |= self.expire_surround_readback();

        for _ in 0..MAX_FRAMES_PER_POLL {
            let Some(bytes) = self.transport.read(next_timeout)? else {
                break;
            };

            next_timeout = Duration::ZERO;

            let decoded = match self.driver.decode(&bytes) {
                Ok(decoded) => decoded,
                Err(DriverError::InvalidActionWithMeterInvalidation { detail, targets }) => {
                    self.state.invalidate_meters(&targets);
                    return Err(DriverError::InvalidActionWithMeterInvalidation {
                        detail,
                        targets,
                    }
                    .into());
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(event) = decoded {
                let clock_source_readback = Self::clock_source_readback(&event);
                let snapshot_modes = if matches!(event, DeviceEvent::Snapshot { .. }) {
                    self.pending_output_modes
                        .keys()
                        .map(|address| (*address, Self::snapshot_output_mode(&event, *address)))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                if matches!(event, DeviceEvent::Snapshot { .. }) {
                    // Confirmation only updates the current session's state; it
                    // does not send a command and is safe in receive-only mode.
                    state_dirty |= self.confirm_pending_write();
                }
                state_dirty |= self.state.observe_event(event);
                if self.state.auraverb.as_ref().is_some_and(|cache| {
                    cache.freshness != super::AuraVerbFreshness::PendingReadback
                }) {
                    self.auraverb_readback_deadline = None;
                }
                if self.state.surround_global.as_ref().is_some_and(|cache| {
                    cache.freshness != super::SurroundFreshness::PendingReadback
                }) {
                    self.surround_readback_deadline = None;
                }
                if self.state.device.status.clock_source == clock_source_readback
                    && clock_source_readback.is_some()
                {
                    self.clock_readback_revision = self.clock_readback_revision.wrapping_add(1);
                }
                if !snapshot_modes.is_empty() {
                    self.resolve_output_mode_snapshots(&snapshot_modes);
                }
            }
        }

        state_dirty |= self.expire_auraverb_readback();
        state_dirty |= self.expire_surround_readback();
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
        let Some(output) = self.state.outputs().get(index).cloned() else {
            return Ok(());
        };
        if !self
            .state
            .ui_profile
            .supports_output(output.address, OutputControl::Level)
        {
            return Ok(());
        }
        let Some((min, max)) = self.state.output_range(OutputControl::Level) else {
            return Ok(());
        };
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index;
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
        let Some(output) = self.state.outputs().get(index).cloned() else {
            return Ok(());
        };
        if !self
            .state
            .ui_profile
            .supports_output(output.address, OutputControl::Level)
        {
            return Ok(());
        }
        let Some((min, max)) = self.state.output_range(OutputControl::Level) else {
            return Ok(());
        };
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index;
        let address = output.address;
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

    fn ensure_output_mode_action_allowed(
        &self,
        address: OutputAddress,
        control: OutputControl,
        enabled: bool,
    ) -> Result<()> {
        if !self.state.uses_zen_go_output_safety() {
            return Ok(());
        }
        if let Some(guard) = self.pending_output_modes.get(&address) {
            match guard.status {
                PendingOutputModeStatus::Recovery => bail!(
                    "Zen Go output mode delivery is uncertain; mode controls are disabled for this session; verify actual device mode before starting a new session"
                ),
                PendingOutputModeStatus::Queued(_) | PendingOutputModeStatus::AwaitingReadback => {
                    bail!(
                        "Zen Go output mode transition is awaiting state readback; wait before changing mode"
                    )
                }
            }
        }
        let mode = self.state.observed_output_mode(address).ok_or_else(|| {
            anyhow::anyhow!("Zen Go output mode is unknown; wait for state readback")
        })?;
        if enabled
            && matches!(
                (mode, control),
                (OutputMode::Mute, OutputControl::Dim) | (OutputMode::Dim, OutputControl::Mute)
            )
        {
            bail!(
                "Zen Go direct {} to {} transition is unverified; return to observed Normal first",
                mode.label(),
                match control {
                    OutputControl::Mute => "Mute",
                    OutputControl::Dim => "Dim",
                    _ => unreachable!(),
                }
            );
        }
        Ok(())
    }

    fn handle_output_toggle_dim(
        &mut self,
        index: usize,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        let Some(output) = self.state.outputs().get(index).cloned() else {
            return Ok(());
        };
        if !self
            .state
            .ui_profile
            .supports_output(output.address, OutputControl::Dim)
        {
            return Ok(());
        }
        let enabled = !output.dimmed.unwrap_or(false);
        self.ensure_output_mode_action_allowed(output.address, OutputControl::Dim, enabled)?;
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index;
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Dim,
                value: ControlValue::Bool(enabled),
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
        let Some(output) = self.state.outputs().get(index).cloned() else {
            return Ok(());
        };
        if !self
            .state
            .ui_profile
            .supports_output(output.address, OutputControl::Mute)
        {
            return Ok(());
        }
        let enabled = !output.muted.unwrap_or(false);
        self.ensure_output_mode_action_allowed(output.address, OutputControl::Mute, enabled)?;
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index;
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Mute,
                value: ControlValue::Bool(enabled),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_toggle_mono(
        &mut self,
        index: usize,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        let Some(output) = self.state.outputs().get(index).cloned() else {
            return Ok(());
        };
        if !self
            .state
            .ui_profile
            .supports_output(output.address, OutputControl::Mono)
        {
            return Ok(());
        }
        let Some(current) = output.mono else {
            return Ok(());
        };
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index;
        self.send(
            Action::SetOutput {
                address: output.address,
                control: OutputControl::Mono,
                value: ControlValue::Bool(!current),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_open_sample_rate_selector(&mut self) {
        if self
            .state
            .ui_profile
            .clock_source_is_internal(self.state.device.status.clock_source)
        {
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
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::ClockSource)
            || self.state.ui_profile.clock_source_choices().is_empty()
        {
            return;
        }
        self.state.popup.selected_index = self
            .state
            .device
            .status
            .clock_source
            .and_then(|current| {
                self.state
                    .ui_profile
                    .clock_source_choices()
                    .iter()
                    .position(|choice| choice.value == current)
            })
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::ClockSource,
        });
    }

    fn handle_open_settings_selector(&mut self) {
        if !self.state.ui_profile.supports_settings() {
            return;
        }
        self.state.popup.selected_index = 0;
        self.state.popup.selector_parent_index = None;
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::Settings,
        });
    }

    fn handle_open_brightness_selector(&mut self) {
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::Brightness)
        {
            return;
        }
        self.state.popup.selector_parent_index = Some(self.state.popup.selected_index);
        self.state.popup.selected_index = self
            .state
            .global_value(GlobalControl::Brightness)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= 100)
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::Brightness,
        });
    }

    fn handle_open_output_trim_selector(&mut self, address: OutputTrimAddress) {
        let control = GlobalControl::OutputTrim(address);
        if !self.state.ui_profile.supports_global(control) {
            return;
        }
        self.state.popup.selector_parent_index = Some(self.state.popup.selected_index);
        self.state.popup.selected_index = self
            .state
            .global_value(control)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= 6)
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::OutputTrim {
                target: address.target,
            },
        });
    }

    fn handle_open_talkback_button(&mut self) {
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::TalkbackButton)
        {
            return;
        }
        self.state.popup.selector_parent_index = Some(self.state.popup.selected_index);
        self.state.popup.selected_index = 0;
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::TalkbackButton,
        });
    }

    fn handle_open_talkback_source_selector(&mut self) {
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::TalkbackSource)
        {
            return;
        }
        self.state.popup.selector_parent_index = Some(self.state.popup.selected_index);
        // Passive state contains only source modulo 4, never a safe full selection.
        self.state.popup.selected_index = 0;
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::TalkbackSource,
        });
    }

    fn handle_open_talkback_gain_selector(&mut self) {
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::TalkbackGain)
        {
            return;
        }
        self.state.popup.selector_parent_index = Some(self.state.popup.selected_index);
        self.state.popup.selected_index = self
            .state
            .global_value(GlobalControl::TalkbackGain)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= 96)
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::TalkbackGain,
        });
    }

    fn return_to_settings_selector(&mut self) {
        self.state.popup.selected_index =
            self.state.popup.selector_parent_index.take().unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::Settings,
        });
    }

    fn handle_pick_brightness(&mut self, value: i32) -> Result<()> {
        if !(0..=100).contains(&value)
            || !self
                .state
                .ui_profile
                .supports_global(GlobalControl::Brightness)
        {
            return Ok(());
        }
        self.send(
            Action::SetGlobal {
                control: GlobalControl::Brightness,
                value: ControlValue::Int(value),
            },
            None,
        )?;
        self.return_to_settings_selector();
        Ok(())
    }

    fn handle_pick_output_trim(&mut self, address: OutputTrimAddress, value: i32) -> Result<()> {
        if !(0..=6).contains(&value)
            || !self
                .state
                .ui_profile
                .supports_global(GlobalControl::OutputTrim(address))
        {
            return Ok(());
        }
        self.send(Action::SetOutputTrim { address, value }, None)?;
        self.return_to_settings_selector();
        Ok(())
    }

    fn handle_set_talkback_button(&mut self, pressed: bool) -> Result<()> {
        if !self
            .state
            .ui_profile
            .supports_global(GlobalControl::TalkbackButton)
            || (pressed && self.state.popup.talkback_button_held)
        {
            return Ok(());
        }
        let action = Action::SetGlobal {
            control: GlobalControl::TalkbackButton,
            value: ControlValue::Bool(pressed),
        };
        if pressed {
            self.send(action, None)?;
        } else {
            // Release must not wait behind or coalesce with a bounded settings queue.
            // Press is always written immediately, so no queued press can be reordered here.
            let batch = self.driver.encode(action)?;
            self.write_batch(batch)?;
        }
        self.state.popup.talkback_button_held = pressed;
        Ok(())
    }

    /// Best-effort release for modal exit/focus loss/quit. A physical disconnect can still
    /// prevent delivery, so callers must not present this as a guaranteed hardware release.
    pub fn release_talkback_if_held(&mut self) -> Result<()> {
        if self.state.popup.talkback_button_held {
            self.handle_set_talkback_button(false)?;
        }
        Ok(())
    }

    fn handle_pick_talkback_source(&mut self, source: i32) -> Result<()> {
        if !(0..=12).contains(&source)
            || !self
                .state
                .ui_profile
                .supports_global(GlobalControl::TalkbackSource)
        {
            return Ok(());
        }
        self.send(
            Action::SetGlobal {
                control: GlobalControl::TalkbackSource,
                value: ControlValue::Enum(source),
            },
            None,
        )?;
        self.return_to_settings_selector();
        Ok(())
    }

    fn handle_pick_talkback_gain(&mut self, gain: i32) -> Result<()> {
        if !(0..=96).contains(&gain)
            || !self
                .state
                .ui_profile
                .supports_global(GlobalControl::TalkbackGain)
        {
            return Ok(());
        }
        self.send(
            Action::SetGlobal {
                control: GlobalControl::TalkbackGain,
                value: ControlValue::Int(gain),
            },
            None,
        )?;
        self.return_to_settings_selector();
        Ok(())
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
        value: i32,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        if !self
            .state
            .ui_profile
            .clock_source_choices()
            .iter()
            .any(|choice| choice.value == value)
        {
            return Ok(());
        }
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.send(
            Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(value),
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
        self.state.popup.routing_source_picker = None;
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = self.state.mixer.selected_channel.min(7);
        if self.state.ui_profile.driver_kind == antelope_protocol::RuntimeDriverKind::ZenGo {
            self.state.popup.routing_editor = None;
            self.state.ui.last_message =
                "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                    .to_string();
        } else {
            self.state.popup.routing_editor =
                self.state
                    .routing_capabilities
                    .first()
                    .map(|group| RoutingEditorState {
                        destination: group.destination,
                        channel: 0,
                    });
            self.state.ui.last_message = "Routing editor opened".to_string();
        }
    }

    fn handle_close_routing_popup(&mut self) {
        self.state.popup.routing_open = false;
        if self.state.ui_profile.driver_kind != antelope_protocol::RuntimeDriverKind::ZenGo {
            self.state.popup.routing_editor = None;
            self.state.popup.routing_source_picker = None;
            self.state.popup.selected_index = 0;
        }
        self.state.ui.last_message = "Closed routing popup".to_string();
    }

    fn handle_toggle_routing_popup(&mut self) {
        if self.state.ui_profile.driver_kind == antelope_protocol::RuntimeDriverKind::ZenGo {
            self.state.popup.routing_open = !self.state.popup.routing_open;
            self.state.ui.last_message = if self.state.popup.routing_open {
                "Routing popup mirrors mixer assignments for USB recording channels 1-8".to_string()
            } else {
                "Closed routing popup".to_string()
            };
        } else if self.state.popup.routing_open {
            self.handle_close_routing_popup();
        } else {
            self.handle_open_routing_popup();
        }
    }

    fn handle_select_routing_destination(&mut self, destination: u16) -> Result<()> {
        if self.state.ui_profile.driver_kind == antelope_protocol::RuntimeDriverKind::ZenGo {
            bail!("general routing editor is unsupported");
        }
        let capability = self
            .state
            .routing_capabilities
            .iter()
            .find(|group| group.destination == destination)
            .ok_or_else(|| anyhow::anyhow!("routing destination {destination} unavailable"))?;
        let channel = self
            .state
            .popup
            .routing_editor
            .map_or(0, |editor| editor.channel)
            .min(capability.channel_count.saturating_sub(1));
        self.state.popup.routing_editor = Some(RoutingEditorState {
            destination,
            channel,
        });
        Ok(())
    }

    fn handle_select_routing_channel(&mut self, destination: u16, channel: u16) -> Result<()> {
        let capability = self
            .state
            .routing_capabilities
            .iter()
            .find(|group| group.destination == destination)
            .ok_or_else(|| anyhow::anyhow!("routing destination {destination} unavailable"))?;
        if self.state.ui_profile.driver_kind == antelope_protocol::RuntimeDriverKind::ZenGo
            || channel >= capability.channel_count
        {
            bail!("routing channel {channel} unavailable for destination {destination}");
        }
        self.state.popup.routing_editor = Some(RoutingEditorState {
            destination,
            channel,
        });
        Ok(())
    }

    fn handle_open_routing_source_picker(&mut self, destination: u16, channel: u16) -> Result<()> {
        if !self
            .state
            .general_routing_channel_available(destination, channel)
        {
            bail!("routing destination {destination} channel {channel} is unavailable");
        }
        let current = self
            .state
            .routing_group(destination)
            .and_then(|group| group.sources.get(usize::from(channel)))
            .copied();
        let choices = self
            .state
            .routing_source_choices_for_destination(destination);
        self.state.popup.selected_index = current
            .and_then(|current| choices.iter().position(|choice| choice.source == current))
            .unwrap_or(0);
        self.state.popup.routing_editor = Some(RoutingEditorState {
            destination,
            channel,
        });
        self.state.popup.routing_source_picker = Some(RoutingSourcePickerState {
            destination,
            channel,
        });
        self.state.ui.last_message = format!(
            "Pick source for destination {destination} channel {}",
            channel + 1
        );
        Ok(())
    }

    fn handle_close_routing_source_picker(&mut self) {
        self.state.popup.routing_source_picker = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Returned to routing editor".to_string();
    }

    fn handle_pick_routing_source(
        &mut self,
        destination: u16,
        channel: u16,
        source: RoutingSource,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        if !self
            .state
            .general_routing_channel_available(destination, channel)
        {
            bail!("routing destination {destination} channel {channel} is unavailable");
        }
        // Profile routing encodes a complete ordered group and rejects the legacy
        // changed-channel hint. The typed UI intent still identifies the exact slot.
        self.commit_routing_source(destination, channel, source, pending, true, false)?;
        self.handle_close_routing_source_picker();
        Ok(())
    }

    fn handle_open_options_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.routing_open = false;
        self.state.popup.routing_editor = None;
        self.state.popup.routing_source_picker = None;
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

    fn input_gain_range_for_control(
        &self,
        address: InputAddress,
        control: InputControl,
        mode: Option<i32>,
    ) -> Result<Option<(i32, i32)>> {
        if Self::input_gain_control(&self.state, address) != Some(control) {
            return Ok(None);
        }
        self.state
            .input_range(address, mode)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("input gain range unavailable"))
    }

    fn validate_input_gain_value(
        &self,
        address: InputAddress,
        control: InputControl,
        mode: Option<i32>,
        raw: i32,
    ) -> Result<()> {
        let Some((minimum, maximum)) = self.input_gain_range_for_control(address, control, mode)?
        else {
            return Ok(());
        };
        if !(minimum..=maximum).contains(&raw) {
            bail!("input gain {raw} outside current mode range {minimum}..={maximum}");
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
        let slot = self.input_at_address(address)?;
        self.validate_input_gain_value(address, InputControl::Gain, slot.mode, raw)?;
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
        let slot = self.input_at_address(address)?;
        let current = slot.gain.unwrap_or(0);
        let value = if let Some((minimum, maximum)) =
            self.input_gain_range_for_control(address, control, slot.mode)?
        {
            if increase {
                current
                    .clamp(minimum, maximum)
                    .saturating_add(1)
                    .min(maximum)
            } else {
                current
                    .clamp(minimum, maximum)
                    .saturating_sub(1)
                    .max(minimum)
            }
        } else if increase {
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
        let slot = self.input_at_address(address)?;
        self.validate_input_gain_value(address, control, slot.mode, value)?;
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
        if Self::input_mode_is_declared(&self.state, address, i32::from(mode.code())) == Some(false)
        {
            bail!("input mode {mode:?} is not declared for {address:?}");
        }
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

    fn handle_set_input_pair_link(&mut self, address: InputAddress, enabled: bool) -> Result<()> {
        self.input_at_address(address)?;
        if !self.state.ui_profile.supports_input_link(address) {
            bail!("input pair link is unsupported for {address:?}");
        }
        let target = self
            .state
            .ui_profile
            .input_link_target(address)
            .ok_or_else(|| anyhow::anyhow!("input pair link target is unavailable"))?;
        self.send(
            Action::SetLink {
                surface: target.protocol_space,
                pair: target.pair,
                enabled,
            },
            None,
        )?;
        self.state.ui.last_message = format!(
            "Requested S/PDIF link {}; device readback unavailable",
            if enabled { "on" } else { "off" }
        );
        Ok(())
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
        self.handle_adjust_input_gain_at(slot.address, increase, pending)
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
        self.handle_set_input_gain_at(slot.address, i32::from(raw), pending)
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
        let current = strip
            .pan
            .ok_or_else(|| anyhow::anyhow!("mixer pan value unavailable"))?
            .clamp(min, max);
        let next = if right {
            current.saturating_add(1).min(max)
        } else {
            current.saturating_sub(1).max(min)
        };
        self.send_target_mixer_change(address, |strip| {
            strip.pan = Some(next);
        })
    }

    fn handle_set_mixer_pan_at(&mut self, address: MixerAddress, pan: i32) -> Result<()> {
        self.ensure_mixer_control(address, MixerControl::Pan)?;
        let (min, max) = self
            .state
            .mixer_range(address.surface, MixerControl::Pan)
            .ok_or_else(|| anyhow::anyhow!("mixer pan range unavailable"))?;
        self.send_target_mixer_change(address, |strip| {
            strip.pan = Some(pan.clamp(min, max));
        })
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
        let current = strip
            .pan
            .ok_or_else(|| anyhow::anyhow!("mixer pan value unavailable"))?
            .clamp(min, max);
        let next = if right {
            current.saturating_add(1).min(max)
        } else {
            current.saturating_sub(1).max(min)
        };
        self.send_target_mixer_change(address, |strip| {
            strip.pan = Some(next);
        })
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
        let value = i32::from(pan.display_percent()).clamp(min, max);
        self.send_target_mixer_change(address, |strip| {
            strip.pan = Some(value);
        })
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
        let current_routing_source = self
            .state
            .routing_assignment_destination(address.surface)
            .and_then(|destination| self.state.routing_group(destination))
            .and_then(|group| group.sources.get(strip_index))
            .copied();

        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = strip_index;
        if !antelope_protocol::MixerStrip::assignment_write_is_grounded(strip) {
            self.state.ui.last_message =
                "Assignment picking is not grounded for the selected strip.".to_string();
            return Ok(());
        }

        let routing_choices = self.state.routing_source_choices(address.surface);
        self.state.popup.selected_index = if routing_choices.is_empty() {
            current_assignment.and_then(|current| {
                MixerAssignment::grounded_choices()
                    .iter()
                    .position(|assignment| *assignment == current)
            })
        } else {
            current_routing_source.and_then(|current| {
                routing_choices
                    .iter()
                    .position(|choice| choice.source == current)
            })
        }
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

    fn handle_pick_routing_source_at(
        &mut self,
        address: MixerAddress,
        source: RoutingSource,
        pending: Option<PendingMutation>,
    ) -> Result<()> {
        let destination = self
            .state
            .routing_assignment_destination(address.surface)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "mixer surface {} has no routing destination",
                    address.surface
                )
            })?;
        self.handle_pick_routing_source_for(address, source, pending, destination, false, false)
    }

    fn handle_pick_assignment_for(
        &mut self,
        address: MixerAddress,
        assignment: MixerAssignment,
        pending: Option<PendingMutation>,
        destination: u16,
        legacy: bool,
    ) -> Result<()> {
        self.handle_pick_routing_source_for(
            address,
            routing_source_from_assignment(assignment),
            pending,
            destination,
            legacy,
            true,
        )
    }

    fn handle_pick_routing_source_for(
        &mut self,
        address: MixerAddress,
        source: RoutingSource,
        pending: Option<PendingMutation>,
        destination: u16,
        legacy: bool,
        include_changed_channel: bool,
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
        let changed_channel = u16::from(
            strip
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("routing strip must be one-based"))?,
        );
        self.commit_routing_source(
            destination,
            changed_channel,
            source,
            pending,
            !legacy,
            include_changed_channel,
        )?;
        self.state.popup.assignment_picker = None;
        self.state.popup.assignment_picker_address = None;
        self.state.popup.selected_index = 0;
        Ok(())
    }

    fn commit_routing_source(
        &mut self,
        destination: u16,
        channel: u16,
        source: RoutingSource,
        pending: Option<PendingMutation>,
        validate_profile_source: bool,
        include_changed_channel: bool,
    ) -> Result<()> {
        if validate_profile_source
            && !self
                .state
                .routing_source_choices_for_destination(destination)
                .iter()
                .any(|choice| choice.source == source)
        {
            bail!(
                "routing source {}:{} is unavailable for destination {destination}",
                source.bank,
                source.index
            );
        }
        let mut sources = self.shared_assignment_sources(destination)?;
        let slot = sources
            .get_mut(usize::from(channel))
            .ok_or_else(|| anyhow::anyhow!("routing channel {channel} unavailable"))?;
        *slot = source;
        self.send(
            Action::SetRoutingGroup {
                destination,
                changed_channel: include_changed_channel.then_some(channel),
                sources,
            },
            pending,
        )
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
        self.state.popup.routing_editor = None;
        self.state.popup.routing_source_picker = None;
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

    fn handle_select_ui_page(&mut self, page: UiPage) {
        let unavailable = match page {
            UiPage::AuraVerb => !self.state.auraverb_page_available(),
            UiPage::Surround => !self.state.surround_page_available(),
            UiPage::Mixer => false,
        };
        self.state.ui.page = if unavailable {
            self.state.ui.last_message = format!(
                "{} is unavailable for the active device profile",
                match page {
                    UiPage::AuraVerb => "AuraVerb",
                    UiPage::Surround => "Surround",
                    UiPage::Mixer => "Mixer",
                }
            );
            UiPage::Mixer
        } else {
            self.state.ui.last_message = match page {
                UiPage::Mixer => "Mixer page selected".to_string(),
                UiPage::AuraVerb => "AuraVerb Mix-1 SEND FX page selected".to_string(),
                UiPage::Surround => "Surround page selected".to_string(),
            };
            page
        };
    }

    fn handle_cycle_focus(&mut self) {
        self.state.cycle_focus();
    }

    fn handle_cycle_surround_focus(&mut self, forward: bool) {
        let current = self.state.ui.surround_focus.index();
        let count = SurroundControlFocus::ALL.len();
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.state.ui.surround_focus = SurroundControlFocus::ALL[next];
        if matches!(
            self.state.ui.surround_focus,
            SurroundControlFocus::Speaker | SurroundControlFocus::EqBank
        ) {
            self.state.ui.surround_drag = None;
        }
    }

    fn handle_navigate_surround_eq(&mut self, focus: SurroundControlFocus, forward: bool) {
        self.state.ui.surround_focus = focus;
        self.state.ui.surround_drag = None;
        match focus {
            SurroundControlFocus::Speaker => {
                let active = self.state.active_surround_speaker_indices();
                if active.is_empty() {
                    return;
                }
                let current = active
                    .iter()
                    .position(|index| *index == self.state.ui.surround_speaker_index)
                    .unwrap_or(0);
                let next = if forward {
                    (current + 1) % active.len()
                } else {
                    (current + active.len() - 1) % active.len()
                };
                self.state.ui.surround_speaker_index = active[next];
            }
            SurroundControlFocus::EqBank => {
                self.state.ui.surround_eq_bank = if forward {
                    (self.state.ui.surround_eq_bank + 1) % 2
                } else {
                    (self.state.ui.surround_eq_bank + 1) % 2
                };
            }
            SurroundControlFocus::Level | SurroundControlFocus::Delay => {}
        }
    }

    fn handle_cycle_auraverb_focus(&mut self, forward: bool, area: Rect) {
        let current = self.state.ui.auraverb_focus.index();
        let last = AuraVerbControlFocus::ALL.len() - 1;
        let next = if forward {
            (current + 1) % AuraVerbControlFocus::ALL.len()
        } else {
            current.checked_sub(1).unwrap_or(last)
        };
        self.state.ui.auraverb_focus = AuraVerbControlFocus::ALL[next];
        crate::ui::ensure_auraverb_focus_visible(area, &mut self.state);
    }

    fn handle_scroll_auraverb_page(&mut self, down: bool, area: Rect) {
        crate::ui::scroll_auraverb_page(area, &mut self.state, down);
    }

    fn handle_move_popup_selection(&mut self, down: bool) {
        let item_count = if let Some(picker) = self.state.popup.routing_source_picker {
            self.state
                .routing_source_choices_for_destination(picker.destination)
                .len()
        } else if self.state.popup.assignment_picker.is_some() {
            self.state
                .popup
                .assignment_picker_address
                .map(|address| self.state.routing_source_choices(address.surface).len())
                .filter(|count| *count > 0)
                .unwrap_or_else(|| antelope_protocol::MixerAssignment::grounded_choices().len())
        } else if self.state.popup.profiles_open {
            self.state.popup.profile_names.len()
        } else if let Some(popup) = self.state.popup.selector_popup {
            match popup.kind {
                SelectorPopupKind::SampleRate => SampleRate::all_confirmed().len(),
                SelectorPopupKind::ClockSource => {
                    self.state.ui_profile.clock_source_choices().len()
                }
                SelectorPopupKind::PreampMode { .. } => 3,
                SelectorPopupKind::Settings => self.state.ui_profile.setting_rows().len(),
                SelectorPopupKind::Brightness => 101,
                SelectorPopupKind::OutputTrim { .. } => {
                    self.state.ui_profile.output_trim_value_labels().len()
                }
                SelectorPopupKind::TalkbackButton => 2,
                SelectorPopupKind::TalkbackSource => {
                    self.state.ui_profile.talkback_source_choices().len()
                }
                SelectorPopupKind::TalkbackGain => 97,
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
                self.handle_output_adjust(index, increase, pending)?;
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
                self.handle_output_toggle_mute(index, pending)?;
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
            self.handle_output_toggle_dim(index, pending)?;
        }
        Ok(())
    }

    fn handle_close_selector_popup(&mut self) -> Result<()> {
        self.release_talkback_if_held()?;
        if self.state.popup.selector_popup.is_some_and(|popup| {
            matches!(
                popup.kind,
                SelectorPopupKind::Brightness
                    | SelectorPopupKind::OutputTrim { .. }
                    | SelectorPopupKind::TalkbackButton
                    | SelectorPopupKind::TalkbackSource
                    | SelectorPopupKind::TalkbackGain
            )
        }) {
            self.return_to_settings_selector();
            return Ok(());
        }
        self.state.popup.selector_popup = None;
        self.state.popup.selector_parent_index = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Closed selector".to_string();
        Ok(())
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

#[cfg(test)]
mod correction_tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use antelope_protocol::{
        CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
        DynamicDeviceState, DynamicOutputState, InputAddress, MixerAddress, MixerControl,
        OutputAddress, OutputControl, ProfileDriver, QueryRequest, RuntimeDriverKind,
    };

    use super::*;
    use crate::app::{AuraVerbFreshness, SurroundFreshness};
    use crate::transport::{MockTransport, Transport};

    struct AcceptingDriver {
        definition: DriverDefinition,
        decoded_event: Option<DeviceEvent>,
    }

    impl AcceptingDriver {
        fn new() -> Self {
            Self {
                definition: crate::device::builtin_zen_go_driver()
                    .expect("Zen Go driver")
                    .definition()
                    .clone(),
                decoded_event: None,
            }
        }

        fn with_event(event: DeviceEvent) -> Self {
            Self {
                decoded_event: Some(event),
                ..Self::new()
            }
        }
    }

    fn empty_snapshot_event() -> DeviceEvent {
        DeviceEvent::Snapshot {
            state: DynamicDeviceState {
                globals: Vec::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                mixers: Vec::new(),
                meters: Vec::new(),
                routing: Vec::new(),
                zen_go_compatibility: None,
            },
            raw: Vec::new(),
        }
    }

    fn clock_readback_event(value: i32) -> DeviceEvent {
        DeviceEvent::QueryReply {
            query_id: 1,
            sub_id: 0,
            body: Vec::new(),
            patch: Some(DynamicStatePatch::Globals(vec![
                antelope_protocol::DynamicGlobalState {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(value),
                },
            ])),
            raw: Vec::new(),
        }
    }

    fn first_input_pending(
        controller: &mut Controller,
        gain: i32,
    ) -> (InputAddress, PendingMutation) {
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        let mut input = controller.state.input_spaces[0].inputs[0].clone();
        let address = input.address;
        input.gain = Some(gain);
        (address, PendingMutation::Input(input))
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
            Ok(self.decoded_event.clone())
        }
    }

    #[test]
    fn receive_only_poll_drains_frames_without_flushing_queued_commands() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(0),
                },
                None,
            )
            .expect("queue command");
        transport.push_read(vec![0x75]);

        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("receive-only poll");

        assert!(transport.take_writes().is_empty());
        assert!(transport
            .read(Duration::ZERO)
            .expect("remaining transport read")
            .is_none());
    }

    #[test]
    fn receive_only_poll_does_not_confirm_queued_unsent_mutation() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(empty_snapshot_event())),
        )
        .expect("controller");
        let (address, pending) = first_input_pending(&mut controller, 17);
        let before = controller.state.input_spaces[0].inputs[0].gain;
        controller
            .send(
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(17),
                },
                Some(pending),
            )
            .expect("queue command");
        assert!(controller.pending_mutation.is_none());
        transport.push_read(vec![0x73]);

        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("receive-only poll");

        assert!(controller.pending_mutation.is_none());
        assert_eq!(controller.state.input_spaces[0].inputs[0].gain, before);
        assert!(transport.take_writes().is_empty());

        controller.flush_commands().expect("flush queued command");
        assert!(controller.pending_mutation.is_some());
        transport.push_read(vec![0x73]);
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("confirm flushed command");
        assert_eq!(controller.state.input_spaces[0].inputs[0].gain, Some(17));
    }

    #[test]
    fn receive_only_poll_confirms_mutation_after_queue_flush() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(empty_snapshot_event())),
        )
        .expect("controller");
        let (address, pending) = first_input_pending(&mut controller, 17);
        controller
            .send(
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(17),
                },
                Some(pending),
            )
            .expect("queue command");
        controller.flush_commands().expect("flush command");
        assert_eq!(transport.take_writes().len(), 1);
        assert!(controller.pending_mutation.is_some());
        assert_ne!(controller.state.input_spaces[0].inputs[0].gain, Some(17));
        transport.push_read(vec![0x73]);

        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("receive-only poll");

        assert_eq!(controller.state.input_spaces[0].inputs[0].gain, Some(17));
        assert!(controller.pending_mutation.is_none());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn normal_poll_flushes_queued_mutation_before_confirmation() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(empty_snapshot_event())),
        )
        .expect("controller");
        let (address, pending) = first_input_pending(&mut controller, 17);
        controller
            .send(
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(17),
                },
                Some(pending),
            )
            .expect("queue command");
        transport.push_read(vec![0x73]);

        controller.poll_device(Duration::ZERO).expect("normal poll");

        assert_eq!(transport.take_writes().len(), 1);
        assert_eq!(controller.state.input_spaces[0].inputs[0].gain, Some(17));
        assert!(controller.pending_mutation.is_none());
    }

    #[test]
    fn coalesced_queue_replacement_promotes_only_latest_mutation() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        let address = controller.state.outputs()[0].address;
        for level in [17, 23] {
            let mut pending_output = controller.state.outputs()[0].clone();
            pending_output.level = Some(level);
            controller
                .send(
                    Action::SetOutput {
                        address,
                        control: OutputControl::Level,
                        value: ControlValue::Int(level),
                    },
                    Some(PendingMutation::Output(pending_output)),
                )
                .expect("queue command");
        }
        assert!(controller.pending_mutation.is_none());

        controller
            .flush_commands()
            .expect("flush coalesced command");

        assert_eq!(transport.take_writes().len(), 1);
        match controller.pending_mutation.as_ref() {
            Some(PendingMutation::Output(output)) => assert_eq!(output.level, Some(23)),
            other => panic!("latest mutation was not promoted: {other:?}"),
        }
    }

    #[test]
    fn sent_mutation_remains_confirmable_while_newer_coalesced_mutation_is_queued() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(empty_snapshot_event())),
        )
        .expect("controller");
        let (address, pending) = first_input_pending(&mut controller, 17);
        controller
            .send(
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(17),
                },
                Some(pending),
            )
            .expect("queue sent mutation");
        controller.flush_commands().expect("flush sent mutation");
        transport.take_writes();
        assert!(matches!(
            controller.pending_mutation.as_ref(),
            Some(PendingMutation::Input(input)) if input.gain == Some(17)
        ));

        let (address, pending) = first_input_pending(&mut controller, 23);
        controller
            .send(
                Action::SetInput {
                    address,
                    control: InputControl::Gain,
                    value: ControlValue::Int(23),
                },
                Some(pending),
            )
            .expect("queue newer mutation");
        match controller.pending_mutation.as_ref() {
            Some(PendingMutation::Input(input)) => assert_eq!(input.gain, Some(17)),
            other => panic!("sent mutation was replaced early: {other:?}"),
        }
        transport.push_read(vec![0x73]);

        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("receive-only poll");

        assert_eq!(controller.state.input_spaces[0].inputs[0].gain, Some(17));
        assert!(controller.pending_mutation.is_none());
        assert!(transport.take_writes().is_empty());
    }

    #[derive(Clone, Default)]
    struct FailingTransport {
        reads: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>>,
        writes: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    }

    impl FailingTransport {
        fn push_read(&self, bytes: Vec<u8>) {
            self.reads
                .lock()
                .expect("failing transport reads")
                .push_back(bytes);
        }

        fn take_writes(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.writes.lock().expect("failing transport writes"))
        }
    }

    impl Transport for FailingTransport {
        fn write(&self, data: &[u8]) -> anyhow::Result<()> {
            self.writes
                .lock()
                .expect("failing transport writes")
                .push(data.to_vec());
            Err(anyhow::anyhow!("write failed"))
        }

        fn read(&self, _timeout: Duration) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self
                .reads
                .lock()
                .expect("failing transport reads")
                .pop_front())
        }
    }

    fn orion_entry() -> RuntimeEntry {
        crate::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "orion_studio_3")
            .expect("Orion profile")
            .clone()
    }

    fn orion_controller(transport: MockTransport) -> Controller {
        let entry = orion_entry();
        let driver = ProfileDriver::new(entry.clone()).expect("Orion profile driver");
        Controller::new_for_entry(Box::new(transport), Box::new(driver), &entry)
            .expect("Orion controller")
    }

    fn hex_bytes(text: &str) -> Vec<u8> {
        let compact = text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        compact
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                    .expect("Surround fixture hex")
            })
            .collect()
    }

    fn auraverb_readback_fixture() -> Vec<u8> {
        hex_bytes(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/auraverb/readback_mix1_poweroff_on2.hex"
        ))
    }

    fn auraverb_readback(state: &antelope_protocol::AuraVerbState) -> Vec<u8> {
        let mut report = auraverb_readback_fixture();
        report[17..28].copy_from_slice(&[
            state.room_size,
            state.color,
            state.pre_delay,
            100,
            state.early_reflection_gain,
            state.late_reflection_delay,
            state.richness,
            state.reverb_time,
            state.reverb_level,
            u8::from(state.enabled),
            0xff,
        ]);
        report
    }

    #[test]
    fn auraverb_rapid_writes_preserve_complete_pending_state_and_ignore_delayed_readbacks() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let error = controller
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .expect_err("no write before authoritative readback");
        assert!(is_auraverb_write_unavailable(&error));
        assert!(!crate::transport::is_device_error(&error));

        let initial_readback = auraverb_readback_fixture();
        transport.push_read(initial_readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("authoritative AuraVerb readback");
        let range_error = controller
            .apply_intent(
                Intent::SetAuraVerbParameter {
                    parameter: AuraVerbParameter::Color,
                    value: 101,
                },
                Rect::default(),
            )
            .expect_err("out-of-range parameter is rejected by the profile driver");
        assert!(!is_auraverb_write_unavailable(&range_error));
        assert!(!crate::transport::is_device_error(&range_error));
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Authoritative
        );
        controller
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .expect("complete enable write");
        let enable_writes = transport.take_writes();
        assert_eq!(enable_writes.len(), 2);
        assert_eq!(
            enable_writes[0],
            hex_bytes(include_str!(
                "../../antelope-protocol/tests/fixtures/orion/auraverb/enabled_on.hex"
            ))
        );

        controller
            .apply_intent(
                Intent::SetAuraVerbParameter {
                    parameter: AuraVerbParameter::Color,
                    value: 0,
                },
                Rect::default(),
            )
            .expect("rapid parameter write");
        let color_writes = transport.take_writes();
        assert_eq!(color_writes.len(), 2);
        assert_eq!(
            color_writes[0],
            hex_bytes(include_str!(
                "../../antelope-protocol/tests/fixtures/orion/auraverb/color_0.hex"
            ))
        );

        controller
            .apply_intent(
                Intent::SetAuraVerbParameter {
                    parameter: AuraVerbParameter::PreDelay,
                    value: 100,
                },
                Rect::default(),
            )
            .expect("second rapid parameter write");
        let latest_writes = transport.take_writes();
        assert_eq!(latest_writes.len(), 2);
        assert_eq!((latest_writes[0][20], latest_writes[0][21]), (0, 100));
        assert_eq!(
            &latest_writes[0][19..29],
            &[81, 0, 100, 100, 11, 13, 24, 66, 50, 1]
        );

        let expected = controller
            .state
            .auraverb
            .as_ref()
            .unwrap()
            .pending_expected
            .clone()
            .unwrap();
        for delayed in [
            initial_readback,
            auraverb_readback(&antelope_protocol::AuraVerbState {
                pre_delay: 0,
                ..expected.clone()
            }),
        ] {
            transport.push_read(delayed);
            controller
                .poll_device_without_writes(Duration::ZERO)
                .expect("ignore delayed AuraVerb state");
            let cache = controller.state.auraverb.as_ref().unwrap();
            assert_eq!(cache.freshness, AuraVerbFreshness::PendingReadback);
            assert_eq!(cache.pending_expected.as_ref(), Some(&expected));
        }

        transport.push_read(auraverb_readback(&expected));
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("confirm latest complete state");
        let cache = controller.state.auraverb.as_ref().unwrap();
        assert_eq!(cache.freshness, AuraVerbFreshness::Authoritative);
        assert_eq!(cache.state.as_ref(), Some(&expected));
        assert!(cache.pending_expected.is_none());
    }

    #[test]
    fn auraverb_timeout_disconnect_and_failed_io_fail_closed_until_new_session() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let readback = auraverb_readback_fixture();
        transport.push_read(readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        controller
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .unwrap();
        transport.take_writes();
        controller.auraverb_readback_deadline = Some(Instant::now() - Duration::from_millis(1));
        let error = controller
            .apply_intent(
                Intent::SetAuraVerbParameter {
                    parameter: AuraVerbParameter::Color,
                    value: 50,
                },
                Rect::default(),
            )
            .expect_err("deadline is checked before authorization");
        assert!(is_auraverb_write_unavailable(&error));
        assert_eq!(
            controller.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Stale
        );
        transport.push_read(readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            controller.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Stale
        );

        let mut recovered = orion_controller(transport.clone());
        transport.push_read(readback.clone());
        recovered
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            recovered.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Authoritative
        );
        recovered.state.mark_disconnected();
        transport.push_read(readback);
        recovered
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            recovered.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Stale
        );
        let disconnected_error = recovered
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .expect_err("disconnected session remains fail-closed");
        assert!(is_auraverb_write_unavailable(&disconnected_error));

        let failing = FailingTransport {
            reads: std::sync::Arc::new(std::sync::Mutex::new(
                [auraverb_readback_fixture()].into_iter().collect(),
            )),
            writes: Default::default(),
        };
        let entry = orion_entry();
        let driver = ProfileDriver::new(entry.clone()).unwrap();
        let mut failed =
            Controller::new_for_entry(Box::new(failing), Box::new(driver), &entry).unwrap();
        failed
            .poll_device_without_writes(Duration::ZERO)
            .expect("actual AuraVerb fixture grants initial authority");
        let transport_error = failed
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .expect_err("transport failure propagates");
        assert!(!is_auraverb_write_unavailable(&transport_error));
        assert_eq!(
            failed.state.auraverb.as_ref().unwrap().freshness,
            AuraVerbFreshness::Stale
        );
        let stale_error = failed
            .apply_intent(Intent::SetAuraVerbEnabled(true), Rect::default())
            .expect_err("failed-I/O session stays stale");
        assert!(is_auraverb_write_unavailable(&stale_error));
    }

    fn surround_fixture() -> (Vec<u8>, Vec<u8>) {
        (
            hex_bytes(include_str!(
                "../../antelope-protocol/tests/fixtures/orion/surround_global_20_eq_post.hex"
            )),
            hex_bytes(include_str!(
                "../../antelope-protocol/tests/fixtures/orion/surround_global_20_readback.hex"
            )),
        )
    }

    fn surround_readback(command: &[u8]) -> Vec<u8> {
        let mut readback = vec![0_u8; 320];
        readback[..16]
            .copy_from_slice(&[0x75, 0, 0, 0, 0x40, 0x01, 0, 0, 0x1b, 0, 0, 0, 0, 0, 0, 0]);
        readback[16..318].copy_from_slice(&command[18..]);
        readback
    }

    #[test]
    fn surround_rapid_writes_ignore_old_and_out_of_order_readbacks_until_complete_match() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        assert!(controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .is_err());

        let (captured, old_readback) = surround_fixture();
        transport.push_read(old_readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("read authoritative Surround state");

        controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .expect("first complete-state write");
        let first_writes = transport.take_writes();
        assert_eq!(first_writes.len(), 2);
        let first_command = first_writes[0].clone();
        assert_eq!(&first_command[22..24], &0_u16.to_le_bytes());

        controller
            .apply_intent(Intent::SetSurroundGlobalDelay(45), Rect::default())
            .expect("rapid write uses latest complete expected state");
        let second_writes = transport.take_writes();
        assert_eq!(second_writes.len(), 2);
        let latest_command = second_writes[0].clone();
        assert_eq!(latest_command[20], 45);
        assert_eq!(&latest_command[22..24], &0_u16.to_le_bytes());

        let cache = controller.state.surround_global.as_ref().unwrap();
        assert_eq!(cache.freshness, SurroundFreshness::PendingReadback);
        assert_eq!(cache.state.as_ref().unwrap().level_raw, 600);
        let optimistic = cache.pending_expected.as_ref().unwrap();
        assert_eq!((optimistic.level_raw, optimistic.delay_tenths_ms), (0, 45));

        for delayed in [old_readback, surround_readback(&first_command)] {
            transport.push_read(delayed);
            controller
                .poll_device_without_writes(Duration::ZERO)
                .expect("ignore delayed readback");
            let cache = controller.state.surround_global.as_ref().unwrap();
            assert_eq!(cache.freshness, SurroundFreshness::PendingReadback);
            assert_eq!(cache.state.as_ref().unwrap().level_raw, 600);
            assert_eq!(cache.pending_expected.as_ref().unwrap().delay_tenths_ms, 45);
        }

        transport.push_read(surround_readback(&latest_command));
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("latest complete confirmation");
        let cache = controller.state.surround_global.as_ref().unwrap();
        assert_eq!(cache.freshness, SurroundFreshness::Authoritative);
        assert_eq!(
            (
                cache.state.as_ref().unwrap().level_raw,
                cache.state.as_ref().unwrap().delay_tenths_ms
            ),
            (0, 45)
        );
        assert!(cache.pending_expected.is_none());
        assert_ne!(latest_command, captured);
    }

    #[test]
    fn surround_write_rejects_expired_pending_state_before_polling() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let (_, readback) = surround_fixture();
        transport.push_read(readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .unwrap();
        transport.take_writes();

        controller.surround_readback_deadline = Some(Instant::now() - Duration::from_millis(1));
        let error = controller
            .apply_intent(Intent::SetSurroundGlobalDelay(45), Rect::default())
            .expect_err("expired pending state must reject the write");
        assert!(is_surround_write_unavailable(&error));
        assert!(!crate::transport::is_device_error(&error));
        assert!(transport.take_writes().is_empty());
        assert!(controller.surround_readback_deadline.is_none());
        let cache = controller.state.surround_global.as_ref().unwrap();
        assert_eq!(cache.freshness, SurroundFreshness::Stale);
        assert_eq!(cache.pending_expected.as_ref().unwrap().level_raw, 0);

        transport.push_read(readback);
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            controller.state.surround_global.as_ref().unwrap().freshness,
            SurroundFreshness::Stale
        );
        assert!(controller
            .apply_intent(Intent::SetSurroundGlobalDelay(45), Rect::default())
            .is_err());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn surround_pending_timeout_and_disconnect_fail_closed_without_losing_states() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let (_, readback) = surround_fixture();
        transport.push_read(readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .unwrap();
        transport.take_writes();

        controller.surround_readback_deadline = Some(Instant::now() - Duration::from_millis(1));
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        let cache = controller.state.surround_global.as_ref().unwrap();
        assert_eq!(cache.freshness, SurroundFreshness::Stale);
        assert_eq!(cache.state.as_ref().unwrap().level_raw, 600);
        assert_eq!(cache.pending_expected.as_ref().unwrap().level_raw, 0);
        transport.push_read(readback.clone());
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            controller.state.surround_global.as_ref().unwrap().freshness,
            SurroundFreshness::Stale
        );
        assert!(controller
            .apply_intent(Intent::SetSurroundGlobalDelay(45), Rect::default())
            .is_err());

        let mut disconnected = orion_controller(transport.clone());
        transport.push_read(readback);
        disconnected
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        disconnected
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .unwrap();
        let writes = transport.take_writes();
        disconnected.state.mark_disconnected();
        transport.push_read(surround_readback(&writes[0]));
        disconnected
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        assert_eq!(
            disconnected
                .state
                .surround_global
                .as_ref()
                .unwrap()
                .freshness,
            SurroundFreshness::Stale
        );
        assert_eq!(
            disconnected
                .state
                .surround_global
                .as_ref()
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .level_raw,
            600
        );
    }

    #[test]
    fn unknown_surround_readback_never_grants_write_authority() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let (mut command, _) = surround_fixture();
        command[18] = 0x04;
        transport.push_read(surround_readback(&command));
        controller
            .poll_device_without_writes(Duration::ZERO)
            .unwrap();
        let cache = controller.state.surround_global.as_ref().unwrap();
        assert_eq!(cache.freshness, SurroundFreshness::Stale);
        assert_eq!(cache.state.as_ref().unwrap().format_name, None);
        assert!(controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .is_err());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn failed_surround_write_marks_cache_stale_and_never_confirms_requested_value() {
        let entry = orion_entry();
        let driver = ProfileDriver::new(entry.clone()).expect("Orion profile driver");
        let transport = FailingTransport::default();
        let mut controller =
            Controller::new_for_entry(Box::new(transport.clone()), Box::new(driver), &entry)
                .expect("Orion controller");
        let (_, readback) = surround_fixture();
        let event = controller
            .driver
            .decode(&readback)
            .expect("decode")
            .expect("event");
        assert!(controller.state.observe_event(event));

        assert!(controller
            .apply_intent(Intent::SetSurroundGlobalLevel(0), Rect::default())
            .is_err());
        let cache = controller
            .state
            .surround_global
            .as_ref()
            .expect("capability");
        assert_eq!(cache.freshness, SurroundFreshness::Stale);
        assert_eq!(
            cache.state.as_ref().expect("old state retained").level_raw,
            600
        );
        assert_eq!(transport.take_writes().len(), 1);
    }

    #[test]
    fn zen_profile_has_no_surround_global_capability() {
        let catalog = crate::device::ProfileCatalog::builtin();
        let zen = catalog
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen profile");
        assert!(AppState::from_entry(zen).surround_global.is_none());
    }

    #[test]
    fn mode_bearing_input_gain_requires_a_known_mode_and_its_range() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let address = controller.state.input_spaces[0].inputs[0].address;

        for mode in [None, Some(99)] {
            controller.state.input_spaces[0].inputs[0].mode = mode;
            let error = controller
                .apply_intent(Intent::SetInputGainAt { address, raw: 0 }, Rect::default())
                .expect_err("unknown mode must reject gain");
            assert!(error.to_string().contains("range unavailable"));
            assert_eq!(controller.command_queue.len(), 0);
            assert!(transport.take_writes().is_empty());
        }

        controller.state.input_spaces[0].inputs[0].mode = Some(3);
        for raw in [0, 20] {
            controller
                .apply_intent(Intent::SetInputGainAt { address, raw }, Rect::default())
                .expect("Direct gain endpoint");
            controller.flush_commands().expect("flush Direct gain");
            let writes = transport.take_writes();
            assert_eq!(writes.len(), 1);
            assert_eq!(writes[0][16..19], [0x50, 0, raw as u8]);
        }
        for raw in [-6, 21] {
            let error = controller
                .apply_intent(Intent::SetInputGainAt { address, raw }, Rect::default())
                .expect_err("gain outside Direct range must fail");
            assert!(error
                .to_string()
                .contains("outside current mode range 0..=20"));
            assert_eq!(controller.command_queue.len(), 0);
            assert!(transport.take_writes().is_empty());
        }

        controller.state.input_spaces[0].inputs[0].mode = Some(1);
        controller
            .apply_intent(Intent::SetInputGainAt { address, raw: -6 }, Rect::default())
            .expect("Line minimum");
        controller.flush_commands().expect("flush Line gain");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x50, 0, 0xfa]);
    }

    #[test]
    fn unknown_zen_physical_mode_rejects_gain_before_queueing() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        let address = controller.state.input_spaces[0].inputs[0].address;

        for mode in [None, Some(99)] {
            controller.state.input_spaces[0].inputs[0].mode = mode;
            let error = controller
                .apply_intent(Intent::SetInputGainAt { address, raw: 0 }, Rect::default())
                .expect_err("unknown Zen mode must reject gain");
            assert!(error.to_string().contains("range unavailable"));
            assert_eq!(controller.command_queue.len(), 0);
            assert!(transport.take_writes().is_empty());
        }
    }

    #[test]
    fn queued_mode_changes_define_the_following_gain_range() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let address = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        controller
            .apply_intent(
                Intent::SetInputModeAt {
                    address,
                    mode: PreampMode::Line,
                },
                Rect::default(),
            )
            .expect("queue Line mode");
        controller
            .apply_intent(Intent::SetInputGainAt { address, raw: 70 }, Rect::default())
            .expect("queue gain valid in the current Mic mode");

        let error = controller
            .flush_commands()
            .expect_err("gain must be checked against preceding queued Line mode");
        assert!(error.to_string().contains("effective mode range -6..=20"));
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x4f, 0, 1]);
    }

    #[test]
    fn mode_gain_mode_sequence_preserves_wire_order() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let address = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        for intent in [
            Intent::SetInputModeAt {
                address,
                mode: PreampMode::Line,
            },
            Intent::SetInputGainAt { address, raw: 10 },
            Intent::SetInputModeAt {
                address,
                mode: PreampMode::Mic,
            },
        ] {
            controller
                .apply_intent(intent, Rect::default())
                .expect("queue ordered input action");
        }

        controller
            .flush_commands()
            .expect("flush ordered input actions");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0][16..19], [0x4f, 0, 1]);
        assert_eq!(writes[1][16..19], [0x50, 0, 10]);
        assert_eq!(writes[2][16..19], [0x4f, 0, 0]);
    }

    #[test]
    fn rejected_queued_mode_does_not_advance_following_gain_validation() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let address = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        controller
            .command_queue
            .enqueue_with_id(Action::SetInput {
                address,
                control: InputControl::Mode,
                value: ControlValue::Enum(99),
            })
            .expect("queue raw undeclared mode");
        controller
            .command_queue
            .enqueue_with_id(Action::SetInput {
                address,
                control: InputControl::Gain,
                value: ControlValue::Int(70),
            })
            .expect("queue gain valid in authoritative Mic mode");

        let error = controller
            .flush_commands()
            .expect_err("undeclared mode remains a flush error");
        assert!(error.to_string().contains("mode 99 is not declared"));
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x50, 0, 70]);
    }

    #[test]
    fn queued_input_gain_is_revalidated_if_mode_changes_before_flush() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let address = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        controller
            .apply_intent(Intent::SetInputGainAt { address, raw: 70 }, Rect::default())
            .expect("queue Mic gain");
        assert_eq!(controller.command_queue.len(), 1);

        controller.state.input_spaces[0].inputs[0].mode = Some(1);
        let error = controller
            .flush_commands()
            .expect_err("Line mode must invalidate queued Mic gain");
        assert!(error.to_string().contains("before queue flush"));
        assert_eq!(controller.command_queue.len(), 0);
        assert!(controller.pending_mutation.is_none());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn legacy_gain_intents_reject_unknown_mode_without_queueing() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        controller.state.input_spaces[0].inputs[0].mode = None;

        for intent in [
            Intent::SetPreampGain { input: 0, raw: 10 },
            Intent::AdjustPreampGain {
                input: 0,
                increase: true,
            },
        ] {
            let error = controller
                .apply_intent(intent, Rect::default())
                .expect_err("legacy gain must use address-aware mode guard");
            assert!(error.to_string().contains("range unavailable"));
            assert_eq!(controller.command_queue.len(), 0);
        }
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn modeless_digital_input_gain_uses_its_scalar_range() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let adat = controller
            .state
            .input_spaces
            .iter()
            .find(|space| space.kind == "adat_inputs")
            .expect("ADAT input bank");
        let address = adat.inputs[0].address;
        assert_eq!(adat.inputs[0].mode, None);
        assert_eq!(controller.state.input_range(address, None), Some((-6, 12)));
        let spdif = controller
            .state
            .input_spaces
            .iter()
            .find(|space| space.kind == "spdif_inputs")
            .expect("S/PDIF input bank");
        let spdif_address = spdif.inputs[0].address;
        assert_eq!(spdif.inputs[0].mode, None);
        assert_eq!(
            controller.state.input_range(spdif_address, None),
            Some((-6, 12))
        );

        controller
            .apply_intent(
                Intent::SetInputParameterAt {
                    address,
                    parameter_id: 0x5b,
                    value: -6,
                },
                Rect::default(),
            )
            .expect("modeless ADAT gain");
        controller.flush_commands().expect("flush ADAT gain");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x5b, 0, 0xfa]);

        controller
            .apply_intent(
                Intent::SetInputParameterAt {
                    address: spdif_address,
                    parameter_id: 0x5c,
                    value: 12,
                },
                Rect::default(),
            )
            .expect("modeless S/PDIF gain");
        controller.flush_commands().expect("flush S/PDIF gain");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x5c, 0, 12]);

        for (address, parameter_id, value) in [(address, 0x5b, 13), (spdif_address, 0x5c, -7)] {
            let error = controller
                .apply_intent(
                    Intent::SetInputParameterAt {
                        address,
                        parameter_id,
                        value,
                    },
                    Rect::default(),
                )
                .expect_err("digital gain outside declared range must reject");
            assert!(error
                .to_string()
                .contains("outside current mode range -6..=12"));
            assert_eq!(controller.command_queue.len(), 0);
            assert!(transport.take_writes().is_empty());
        }

        for (address, parameter_id, value) in [(address, 0x5b, 13), (spdif_address, 0x5c, -7)] {
            controller
                .command_queue
                .enqueue_with_id(Action::SetInput {
                    address,
                    control: InputControl::Parameter(parameter_id),
                    value: ControlValue::Int(value),
                })
                .expect("queue raw digital gain");
            let error = controller
                .flush_commands()
                .expect_err("flush must revalidate declared parameter gain");
            assert!(error
                .to_string()
                .contains("outside effective mode range -6..=12"));
            assert!(transport.take_writes().is_empty());
        }
    }

    #[test]
    fn invalid_gain_does_not_discard_valid_unrelated_output() {
        let transport = MockTransport::default();
        let mut controller = orion_controller(transport.clone());
        let input = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        controller
            .apply_intent(
                Intent::SetInputGainAt {
                    address: input,
                    raw: 70,
                },
                Rect::default(),
            )
            .expect("queue Mic gain");
        controller.state.input_spaces[0].inputs[0].mode = Some(1);
        let mut pending_output = controller.state.outputs()[0].clone();
        pending_output.level = Some(20);
        controller
            .send(
                Action::SetOutput {
                    address: pending_output.address,
                    control: OutputControl::Level,
                    value: ControlValue::Int(20),
                },
                Some(PendingMutation::Output(pending_output)),
            )
            .expect("queue unrelated output");

        let error = controller
            .flush_commands()
            .expect_err("invalid gain remains an honest flush error");
        assert!(error.to_string().contains("before queue flush"));
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_ne!(writes[0][16], 0x50);
        assert!(matches!(
            controller.pending_mutation.as_ref(),
            Some(PendingMutation::Output(output)) if output.level == Some(20)
        ));
        assert_eq!(controller.command_queue.len(), 0);
    }

    #[test]
    fn failed_mode_write_prevents_following_gain_write() {
        let transport = FailingTransport::default();
        let entry = crate::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "orion_studio_3")
            .expect("Orion profile")
            .clone();
        let driver = ProfileDriver::new(entry.clone()).expect("Orion profile driver");
        let mut controller =
            Controller::new_for_entry(Box::new(transport.clone()), Box::new(driver), &entry)
                .expect("Orion controller");
        let address = controller.state.input_spaces[0].inputs[0].address;
        controller.state.input_spaces[0].inputs[0].mode = Some(0);
        controller
            .apply_intent(
                Intent::SetInputModeAt {
                    address,
                    mode: PreampMode::Line,
                },
                Rect::default(),
            )
            .expect("queue mode");
        controller
            .apply_intent(Intent::SetInputGainAt { address, raw: 10 }, Rect::default())
            .expect("queue following gain");

        assert!(controller.flush_commands().is_err());
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][16..19], [0x4f, 0, 1]);
        assert!(controller.pending_mutation.is_none());
    }

    #[test]
    fn failed_spdif_link_request_never_creates_confirmed_state() {
        let entry = crate::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "orion_studio_3")
            .expect("Orion profile")
            .clone();
        let driver = ProfileDriver::new(entry.clone()).expect("Orion profile driver");
        let mut controller = Controller::new_for_entry(
            Box::new(FailingTransport::default()),
            Box::new(driver),
            &entry,
        )
        .expect("Orion controller");
        let spdif = controller
            .state
            .input_spaces
            .iter()
            .find(|space| space.kind == "spdif_inputs")
            .expect("S/PDIF bank");
        let address = spdif.inputs[0].address;
        assert!(spdif.inputs.iter().all(|input| input.gain.is_none()));

        controller
            .apply_intent(
                Intent::SetInputPairLink {
                    address,
                    enabled: true,
                },
                Rect::default(),
            )
            .expect("queue explicit link-on request");
        assert!(controller.flush_commands().is_err());

        let spdif = controller
            .state
            .input_spaces
            .iter()
            .find(|space| space.kind == "spdif_inputs")
            .expect("S/PDIF bank");
        assert!(spdif.inputs.iter().all(|input| input.gain.is_none()));
        assert!(controller.pending_mutation.is_none());
        assert!(controller
            .state
            .ui
            .last_message
            .contains("readback unavailable"));
    }

    #[test]
    fn failed_queue_write_does_not_promote_pending_mutation() {
        let mut controller = Controller::new(
            Box::new(FailingTransport::default()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        let mut pending_output = controller.state.outputs()[0].clone();
        pending_output.level = Some(17);
        controller
            .send(
                Action::SetOutput {
                    address: pending_output.address,
                    control: OutputControl::Level,
                    value: ControlValue::Int(17),
                },
                Some(PendingMutation::Output(pending_output)),
            )
            .expect("queue command");

        assert!(controller.flush_commands().is_err());
        assert!(controller.pending_mutation.is_none());
        assert_eq!(controller.command_queue.len(), 0);
    }

    #[test]
    fn failed_clock_write_invalidates_optimistic_state_and_gates_sample_rate() {
        let mut controller = Controller::new(
            Box::new(FailingTransport::default()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        controller.state.device.status.clock_source = Some(2);
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(0),
                },
                None,
            )
            .expect("queue clock command");
        assert_eq!(controller.state.device.status.clock_source, Some(0));

        assert!(controller.flush_commands().is_err());

        assert_eq!(controller.state.device.status.clock_source, None);
        controller
            .apply_intent(Intent::OpenSampleRateSelector, Rect::default())
            .expect("sample-rate intent");
        assert!(controller.state.popup.selector_popup.is_none());
    }

    #[test]
    fn provably_unsent_coalesced_clock_changes_restore_pre_queue_value() {
        let mut controller = Controller::new(
            Box::new(FailingTransport::default()),
            Box::new(AcceptingDriver::new()),
        )
        .expect("controller");
        controller.state.device.status.clock_source = Some(2);
        let output = controller.state.outputs()[0].address;
        controller
            .send(
                Action::SetOutput {
                    address: output,
                    control: OutputControl::Level,
                    value: ControlValue::Int(17),
                },
                None,
            )
            .expect("queue preceding command");
        for value in [0, 1] {
            controller
                .send(
                    Action::SetGlobal {
                        control: GlobalControl::ClockSource,
                        value: ControlValue::Enum(value),
                    },
                    None,
                )
                .expect("coalesce clock command");
        }
        assert_eq!(controller.state.device.status.clock_source, Some(1));

        assert!(controller.flush_commands().is_err());

        assert_eq!(controller.state.device.status.clock_source, Some(2));
    }

    #[test]
    fn clock_readback_repairs_uncertain_state() {
        let transport = FailingTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(clock_readback_event(2))),
        )
        .expect("controller");
        controller.state.device.status.clock_source = Some(1);
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(0),
                },
                None,
            )
            .expect("queue clock command");
        assert!(controller.flush_commands().is_err());
        assert_eq!(controller.state.device.status.clock_source, None);

        transport.push_read(vec![0x73]);
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("clock readback");

        assert_eq!(controller.state.device.status.clock_source, Some(2));
    }

    #[test]
    fn fresher_clock_readback_is_not_replaced_by_older_failed_queue_entry() {
        let transport = FailingTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(AcceptingDriver::with_event(clock_readback_event(1))),
        )
        .expect("controller");
        controller.state.device.status.clock_source = Some(2);
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(0),
                },
                None,
            )
            .expect("queue clock command");

        transport.push_read(vec![0x73]);
        controller
            .poll_device_without_writes(Duration::ZERO)
            .expect("newer clock readback");
        assert_eq!(controller.state.device.status.clock_source, Some(1));

        assert!(controller.flush_commands().is_err());

        assert_eq!(controller.state.device.status.clock_source, Some(1));
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
    fn dynamic_fourth_output_honors_profile_range_and_unsupported_controls_noop() {
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
        let mut output = entry.profile.outputs[2].clone();
        output.id = 3;
        output.name = "Output 4".into();
        entry.profile.outputs.push(output);
        entry
            .profile
            .params
            .iter_mut()
            .find(|param| param.name == "bus_level")
            .expect("output level parameter")
            .range = Some((10, 20));
        entry
            .profile
            .params
            .retain(|param| param.name != "bus_mute" && param.name != "bus_dim");
        controller.state = AppState::from_entry(&entry);
        controller.state.output.selected = 3;
        controller.state.ui.focus = FocusArea::Outputs;
        controller.state.output.dynamic[3].level = Some(20);
        controller.state.output.dynamic[3].muted = Some(false);
        controller.state.output.dynamic[3].dimmed = Some(false);

        controller
            .apply_intent(Intent::AdjustFocused(false), Rect::default())
            .expect("adjust fourth output");
        assert_eq!(controller.state.outputs()[3].level, Some(20));

        controller
            .apply_intent(
                Intent::SetOutputLevel { index: 3, step: 0 },
                Rect::default(),
            )
            .expect("clamp fourth output level to profile minimum");
        assert_eq!(controller.state.outputs()[3].level, Some(10));
        controller
            .apply_intent(
                Intent::SetOutputLevel {
                    index: 3,
                    step: u8::MAX,
                },
                Rect::default(),
            )
            .expect("clamp fourth output level to profile maximum");
        assert_eq!(controller.state.outputs()[3].level, Some(20));
        let queued = controller.command_queue.len();

        controller
            .apply_intent(Intent::ToggleFocusedMute, Rect::default())
            .expect("unsupported mute is a no-op");
        controller
            .apply_intent(Intent::ToggleFocusedDim, Rect::default())
            .expect("unsupported dim is a no-op");
        assert_eq!(controller.state.outputs()[3].muted, Some(false));
        assert_eq!(controller.state.outputs()[3].dimmed, Some(false));
        assert_eq!(controller.command_queue.len(), queued);

        controller
            .apply_intent(
                Intent::SetOutputLevel {
                    index: usize::MAX,
                    step: 15,
                },
                Rect::default(),
            )
            .expect("missing output set is a no-op");
        controller
            .apply_intent(
                Intent::AdjustOutputLevel {
                    index: usize::MAX,
                    increase: true,
                },
                Rect::default(),
            )
            .expect("missing output adjust is a no-op");
        assert_eq!(controller.state.output.selected, 3);
        assert_eq!(controller.state.outputs()[3].level, Some(20));
        assert_eq!(controller.command_queue.len(), queued);
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
        controller.state.input_spaces[1].inputs[0].mode = Some(0);
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
                    pan: 0,
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
        strip.pan = Some(30);
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
                    pan: 0,
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
