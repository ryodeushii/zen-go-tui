//! Generated Antelope hardware definitions.
//!
//! Profile JSON is consumed at generation time only.  Runtime code uses the
//! checked-in catalog, so building this crate does not require Antelope-Ctl.

pub mod definition;
pub mod discovery;
#[rustfmt::skip]
pub mod generated;
pub mod profile;
pub mod session;

pub use definition::{
    AddressSpaceDefinition, AddressSpaceKind, AddressingMode, CandidatePreampMeterDefinition,
    ConstraintDefinition, DecoderDefinition, DefinitionStatus, DeviceDefinition, DeviceEntry,
    DeviceIdentity, FaderDirectionDefinition, FaderSemanticsDefinition, FrameDefinition,
    FrameEndianDefinition, FrameFieldDefinition, FrameKind, FrameOperationDefinition,
    HazardDefinition, InputCapabilityDefinition, InputControlKind, InputDefinition,
    LinkDomainDefinition, LinkDomainKind, MeterMappingDefinition, MeterTargetDefinition,
    MixerDefinition, MixerReadbackLayoutDefinition, OutputDefinition, ParamDefinition,
    ParamOffsetDefinition, ParamRangeDefinition, ParamReference, ParamValueDefinition,
    ParamValueType, Provenance, ReadbackCategoryDefinition, ReadbackDefinition, Readiness,
    RoutingGroupDefinition, RoutingSourceDomainDefinition, SafeQueryDefinition,
    StartupQueryDefinition, StateReportDefinition, Status, SupportLevel, TransportDefinition,
    TransportKind,
};
pub use discovery::{
    classify_candidate, classify_candidate_details, classify_candidates,
    enumerate_antelope_devices, sort_candidates, CandidateClassification, CandidateStatus,
    DeviceCandidate, ANTELOPE_VID,
};
pub use generated::DEVICE_CATALOG;
pub use profile::{catalog_readiness, ProfileCatalog};
pub use session::{
    builtin_zen_go_driver, classify_runtime_candidates, replace_session, select_candidate,
    select_reconnect_candidate, DevicePickerState, DeviceSelection, DeviceSession, PickerEntry,
    RuntimeDeviceState, SelectionMatch,
};

#[cfg(test)]
mod tests {
    use super::{DeviceEntry, Readiness, DEVICE_CATALOG};

    fn entry(name: &str) -> &'static DeviceEntry {
        DEVICE_CATALOG
            .iter()
            .find(|entry| entry.definition.identity.name == name)
            .expect("catalog entry")
    }

    #[test]
    fn catalog_contains_only_five_hardware_profiles() {
        assert_eq!(DEVICE_CATALOG.len(), 5);
        assert!(DEVICE_CATALOG
            .iter()
            .all(|entry| { !entry.definition.provenance.source_path.contains("mic") }));
    }

    #[test]
    fn readiness_is_separate_from_profile_data() {
        assert_eq!(
            entry("Antelope Zen Go Synergy Core").readiness,
            Readiness::Supported
        );
        assert_eq!(
            entry("Antelope Orion Studio Synergy Core").readiness,
            Readiness::Supported
        );
        assert_eq!(
            entry("Antelope Discrete 8 Pro Synergy Core").readiness,
            Readiness::Partial
        );
        assert_eq!(
            entry("Antelope Discrete 4 Synergy Core").readiness,
            Readiness::Unverified
        );
        assert_eq!(
            entry("Antelope Discrete 4 Pro Synergy Core").readiness,
            Readiness::Unverified
        );
    }

    #[test]
    fn canonical_catalog_records_profile_driven_control_interfaces() {
        for name in [
            "Antelope Zen Go Synergy Core",
            "Antelope Orion Studio Synergy Core",
            "Antelope Discrete 8 Pro Synergy Core",
        ] {
            assert_eq!(
                entry(name).definition.transport.expected_interface_number,
                Some(3),
                "{name} control interface"
            );
        }
        for name in [
            "Antelope Discrete 4 Synergy Core",
            "Antelope Discrete 4 Pro Synergy Core",
        ] {
            assert_eq!(
                entry(name).definition.transport.expected_interface_number,
                Some(3),
                "{name} control interface from current canonical profile"
            );
        }
    }

    #[test]
    fn orion_catalog_preserves_full_profile_geometry() {
        let orion = entry("Antelope Orion Studio Synergy Core");
        let physical_inputs = orion
            .definition
            .inputs
            .iter()
            .filter(|input| input.space == "physical_inputs")
            .count();
        let adat_inputs = orion
            .definition
            .inputs
            .iter()
            .filter(|input| input.space == "adat_inputs")
            .count();
        let spdif_inputs = orion
            .definition
            .inputs
            .iter()
            .filter(|input| input.space == "spdif_inputs")
            .count();
        assert_eq!((physical_inputs, adat_inputs, spdif_inputs), (12, 16, 2));
        assert_eq!(orion.definition.outputs.len(), 6);
        assert_eq!(orion.definition.mixers.len(), 4);
        assert!(orion
            .definition
            .mixers
            .iter()
            .all(|mixer| mixer.strip_count == 32));
    }
}
