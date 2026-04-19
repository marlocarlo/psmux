use super::*;

pub fn handle_key(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match app.mode {
        Mode::Passthrough => key_overlays::handle_key_passthrough(app, key),
        Mode::Prefix { armed_at } => key_prefix::handle_key_prefix(app, key, armed_at),
        Mode::CommandPrompt { .. } => key_command_prompt::handle_key_command_prompt(app, key),
        Mode::WindowChooser { selected, .. } => key_overlays::handle_key_window_chooser(app, key, selected),
        Mode::WindowIndexPrompt { .. } => key_overlays::handle_key_window_index_prompt(app, key),
        Mode::RenamePrompt { .. } => key_overlays::handle_key_rename_prompt(app, key),
        Mode::RenameSessionPrompt { .. } => key_overlays::handle_key_rename_session_prompt(app, key),
        Mode::CopyMode => key_copy_mode::handle_key_copy_mode(app, key),
        Mode::CopySearch { .. } => key_copy_mode::handle_key_copy_search(app, key),
        Mode::PaneChooser { .. } => key_overlays::handle_key_pane_chooser(app, key),
        Mode::MenuMode { .. } => key_overlays::handle_key_menu_mode(app, key),
        Mode::PopupMode { .. } => key_popup_customize::handle_key_popup_mode(app, key),
        Mode::ConfirmMode { .. } => key_overlays::handle_key_confirm_mode(app, key),
        Mode::ClockMode => key_overlays::handle_key_clock_mode(app),
        Mode::CustomizeMode { .. } => key_popup_customize::handle_key_customize_mode(app, key),
        Mode::BufferChooser { selected } => key_overlays::handle_key_buffer_chooser(app, key, selected),
    }
}
