//! Profile management operations (popup, editor, save/load/delete).

use anyhow::Result;

use zen_go_tui::app::{Controller, ProfileEditorMode, ProfileEditorState};
use zen_go_tui::profile::{
    delete_profile, list_profile_names, profile_path, rename_profile, DeviceProfile,
};

/// Execute a CLI profile command (save/load).
pub(crate) fn run_profile_command(
    transport: Box<dyn zen_go_tui::transport::Transport>,
    command: crate::cli::ProfileCommand,
) -> Result<()> {
    match command {
        crate::cli::ProfileCommand::Save { name } => {
            let mut controller = Controller::new(transport);
            collect_profile_state(&mut controller)?;
            let profile = DeviceProfile::capture(&controller.state)?;
            let path = profile.write_named(&name)?;
            println!("Saved profile to {}", path.display());
            Ok(())
        }
        crate::cli::ProfileCommand::Load { name } => {
            let profile = DeviceProfile::read_named(&name)?;
            let path = profile_path(&name)?;
            let mut controller = Controller::new(transport);
            controller.apply_profile(&profile)?;
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

/// Refresh the list of saved profile names in controller state.
pub(crate) fn refresh_profile_names(controller: &mut Controller) -> Result<()> {
    controller.state.profile_names = list_profile_names()?;
    controller.state.clamp_profile_selection();
    Ok(())
}

/// Open the profiles popup, clearing other popups.
pub(crate) fn open_profiles_popup(controller: &mut Controller) -> Result<()> {
    refresh_profile_names(controller)?;
    controller.state.assignment_picker = None;
    controller.state.selector_popup = None;
    controller.state.routing_popup_open = false;
    controller.state.profile_editor = None;
    controller.state.profiles_popup_open = true;
    controller.state.last_message = if controller.state.profile_names.is_empty() {
        "No saved profiles yet. Use SAVE to Create one.".to_string()
    } else {
        "Select a profile to load, or use SAVE/RENAME/DELETE.".to_string()
    };
    Ok(())
}

/// Close the profiles popup and set a status message.
pub(crate) fn close_profiles_popup(controller: &mut Controller, message: &str) {
    controller.state.profiles_popup_open = false;
    controller.state.profile_editor = None;
    controller.state.last_message = message.to_string();
}

/// Start the profile name editor for save or rename.
pub(crate) fn start_profile_editor(controller: &mut Controller, mode: ProfileEditorMode) {
    let current_name = controller.state.selected_profile_name().map(str::to_string);
    let value = match mode {
        ProfileEditorMode::Save => current_name.clone().unwrap_or_default(),
        ProfileEditorMode::Rename => current_name.clone().unwrap_or_default(),
    };
    controller.state.profile_editor = Some(ProfileEditorState {
        mode,
        original_name: current_name,
        value,
    });
    controller.state.last_message = match mode {
        ProfileEditorMode::Save => "Enter a profile name, then press Enter to save.".to_string(),
        ProfileEditorMode::Rename => {
            "Edit the profile name, then press Enter to rename.".to_string()
        }
    };
}

/// Append valid characters to the profile editor text.
pub(crate) fn append_profile_editor_text(controller: &mut Controller, text: &str) {
    let Some(editor) = controller.state.profile_editor.as_mut() else {
        return;
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            editor.value.push(ch);
        }
    }
}

/// Commit the profile editor action (save or rename).
pub(crate) fn commit_profile_editor(controller: &mut Controller) -> Result<()> {
    let Some(editor) = controller.state.profile_editor.clone() else {
        return Ok(());
    };

    match editor.mode {
        ProfileEditorMode::Save => {
            let profile = DeviceProfile::capture(&controller.state)?;
            let path = profile.write_named(&editor.value)?;
            refresh_profile_names(controller)?;
            controller.state.popup_selected_index = controller
                .state
                .profile_names
                .iter()
                .position(|name| *name == editor.value)
                .unwrap_or(0);
            controller.state.profile_editor = None;
            controller.state.last_message = format!("Saved profile to {}", path.display());
        }
        ProfileEditorMode::Rename => {
            let Some(original_name) = editor.original_name.as_deref() else {
                anyhow::bail!("no profile selected to rename")
            };
            let path = rename_profile(original_name, &editor.value)?;
            refresh_profile_names(controller)?;
            controller.state.popup_selected_index = controller
                .state
                .profile_names
                .iter()
                .position(|name| *name == editor.value)
                .unwrap_or(0);
            controller.state.profile_editor = None;
            controller.state.last_message = format!("Renamed profile to {}", path.display());
        }
    }

    Ok(())
}

/// Load the currently selected profile onto the device.
pub(crate) fn load_selected_profile(controller: &mut Controller) -> Result<()> {
    let Some(name) = controller.state.selected_profile_name().map(str::to_string) else {
        controller.state.last_message = "No profile selected to load.".to_string();
        return Ok(());
    };
    let profile = DeviceProfile::read_named(&name)?;
    controller.apply_profile(&profile)?;
    close_profiles_popup(controller, &format!("Loaded profile {name}"));
    Ok(())
}

/// Delete the currently selected profile.
pub(crate) fn delete_selected_profile(controller: &mut Controller) -> Result<()> {
    let Some(name) = controller.state.selected_profile_name().map(str::to_string) else {
        controller.state.last_message = "No profile selected to delete.".to_string();
        return Ok(());
    };
    delete_profile(&name)?;
    refresh_profile_names(controller)?;
    controller.state.last_message = format!("Deleted profile {name}");
    Ok(())
}

/// Wrap a profile operation result, swallowing errors into a status message.
pub(crate) fn handle_profile_result(controller: &mut Controller, result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            controller.state.last_message = format!("Profile error: {error}");
            Ok(())
        }
    }
}
