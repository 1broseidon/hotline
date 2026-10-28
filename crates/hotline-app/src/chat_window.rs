//! One teammate's chat in a window of its own, opened from the rail's menu,
//! so two conversations can sit side by side. It is the desk's own page,
//! told which teammate and desk it is for (`window.__hotlineChat`), and it
//! leaves the room-wide work to the main window: the rail, toasts, the dock
//! badge, global hotkeys and the menu bar.

use tauri::Manager;

/// The window's label from what it shows: one window per teammate per desk,
/// in the characters a label may hold.
fn label(desk_id: &str, persona_id: &str) -> String {
    let clean = |part: &str| -> String {
        part.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("chat-{}-{}", clean(desk_id), clean(persona_id))
}

/// Opens the teammate's chat in its own window, or brings the one already
/// open to the front.
#[tauri::command]
pub fn chat_window(
    app: tauri::AppHandle,
    desk_id: String,
    persona_id: String,
    name: String,
) -> Result<(), String> {
    let label = label(&desk_id, &persona_id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let open = serde_json::json!({ "deskId": desk_id, "personaId": persona_id });
    crate::desk_window(&app, &label, Some((&name, &open)))
        .map(|_| ())
        .map_err(|error| format!("The window could not open: {error}"))
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn a_window_is_named_by_its_desk_and_teammate_in_label_characters() {
        assert_eq!(
            label("local", "0c1d2e3f-0000-4000-8000-00000000a11e"),
            "chat-local-0c1d2e3f-0000-4000-8000-00000000a11e"
        );
        assert_eq!(label("my desk", "a/b:c"), "chat-my_desk-a_b_c");
    }
}
