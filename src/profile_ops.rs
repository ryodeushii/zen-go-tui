//! Profile management operations (popup, editor, save/load/delete).

use anyhow::Result;

use zen_go_tui::app::Controller;
use zen_go_tui::device::DeviceSession;
use zen_go_tui::profile::{profile_path, DeviceProfile};

/// Execute a CLI profile command (save/load).
pub(crate) fn run_profile_command(
    mut session: DeviceSession,
    command: crate::cli::ProfileCommand,
) -> Result<()> {
    match command {
        crate::cli::ProfileCommand::Save { name } => {
            let controller = session.controller_mut();
            collect_profile_state(controller)?;
            let profile = DeviceProfile::capture(&controller.state)?;
            let path = profile.write_named(&name)?;
            println!("Saved profile to {}", path.display());
            Ok(())
        }
        crate::cli::ProfileCommand::Load { name } => {
            let profile = DeviceProfile::read_named(&name)?;
            let path = profile_path(&name)?;
            session.controller_mut().apply_profile(&profile)?;
            println!("Loaded profile from {}", path.display());
            Ok(())
        }
    }
}

/// Poll the device until state stabilizes, for profile capture.
fn collect_profile_state(controller: &mut Controller) -> Result<()> {
    const PROFILE_CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
    const PROFILE_CAPTURE_POLL: std::time::Duration = std::time::Duration::from_millis(50);
    const PROFILE_CAPTURE_IDLE_POLLS: usize = 2;

    controller.bootstrap()?;

    let deadline = std::time::Instant::now() + PROFILE_CAPTURE_TIMEOUT;
    let mut idle_polls = 0;
    while std::time::Instant::now() < deadline && idle_polls < PROFILE_CAPTURE_IDLE_POLLS {
        if controller.poll_device(PROFILE_CAPTURE_POLL)? {
            idle_polls = 0;
        } else {
            idle_polls += 1;
        }
    }

    Ok(())
}

/// Close the profiles popup and set a status message.
pub(crate) fn close_profiles_popup(controller: &mut Controller, message: &str) {
    controller.state.popup.profiles_open = false;
    controller.state.popup.profile_editor = None;
    controller.state.ui.last_message = message.to_string();
}

/// Append valid characters to the profile editor text.
pub(crate) fn append_profile_editor_text(controller: &mut Controller, text: &str) {
    let Some(editor) = controller.state.popup.profile_editor.as_mut() else {
        return;
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            editor.value.push(ch);
        }
    }
}

/// Load the currently selected profile onto the device.
pub(crate) fn load_selected_profile(controller: &mut Controller) -> Result<()> {
    let Some(name) = controller.state.selected_profile_name().map(str::to_string) else {
        controller.state.ui.last_message = "No profile selected to load.".to_string();
        return Ok(());
    };
    let profile = DeviceProfile::read_named(&name)?;
    controller.apply_profile(&profile)?;
    close_profiles_popup(controller, &format!("Loaded profile {name}"));
    Ok(())
}
