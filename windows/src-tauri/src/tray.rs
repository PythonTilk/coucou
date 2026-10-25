// Notification-area icon: Open, Weekly recap, Wardrobe, Settings, Pause, Quit.
// "Check for updates…" / "Install update X…" comes from this build's own
// updater (updates.rs); upstream's update.rs is not built here.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::image::Image;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::i18n::{n_, t};
use crate::island::WINDOW_LABEL;

/// The menu's items, id and English label, in order. The labels are shown in
/// the interface language (i18n.rs) and follow it when it changes.
const ITEMS: [(&str, &str); 6] = [
    ("open", n_("Open Coucou")),
    ("recap", n_("Weekly recap")),
    ("wardrobe", n_("Wardrobe…")),
    ("settings", n_("Settings…")),
    ("pause", n_("Pause")),
    ("quit", n_("Quit")),
];

/// Kept so a language change relabels the menu without rebuilding the tray.
struct Items(Vec<(&'static str, MenuItem<Wry>)>);

/// The update entry (local to this build, English only): it names the build
/// that is waiting, and opens Settings, where it is installed.
struct UpdateItem(MenuItem<Wry>);

const UPDATE_IDLE: &str = "Check for updates…";

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut items = Vec::new();
    for (id, label) in ITEMS {
        items.push((label, MenuItem::with_id(app, id, t(label), true, None::<&str>)?));
    }
    let [open, recap, wardrobe, settings, pause, quit] = [0, 1, 2, 3, 4, 5].map(|i| &items[i].1);
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let update = MenuItem::with_id(app, "update", UPDATE_IDLE, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[open, recap, &sep1, wardrobe, settings, &update, pause, &sep2, quit])?;
    app.manage(Items(items));
    app.manage(UpdateItem(update));

    let mut builder = TrayIconBuilder::with_id("coucou")
        .tooltip("Coucou")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            // The Coucou window: dictation, notes and every setting.
            "open" => crate::show_hub(app, None),
            "update" => {
                crate::show_settings_window(app);
                let _ = app.emit_to("settings", "show-updates", ());
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::updates::check(&handle).await;
                });
            }
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// Names the build that can be installed in the menu and the tooltip, or puts
/// both back when there is none.
pub fn show_update(app: &AppHandle, version: Option<&str>) {
    if let Some(item) = app.try_state::<UpdateItem>() {
        let text = match version {
            Some(v) => format!("Install update {v}…"),
            None => UPDATE_IDLE.to_string(),
        };
        let _ = item.0.set_text(text);
    }
    if let Some(tray) = app.tray_by_id("coucou") {
        let tip = match version {
            Some(v) => format!("Coucou — update {v} is ready"),
            None => "Coucou".to_string(),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    tray_update_hint(app, version.is_some());
}

/// Relabels the menu in the current language.
pub fn retitle(app: &AppHandle) {
    let Some(items) = app.try_state::<Items>() else { return };
    for (label, item) in &items.0 {
        let _ = item.set_text(t(label));
    }
}

/// Coucou's update blue, #3B82F6, the colour of Mochi while a new version waits.
const UPDATE_BLUE: [f32; 3] = [59.0, 130.0, 246.0];

/// The tray Mochi in the update blue: the light body takes the blue and keeps its
/// shading, the dark outline and eyes stay as they are, transparency untouched.
pub fn tint_blue(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for p in rgba.chunks_exact(4) {
        let light = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
        // 0 for the outline and eyes, 1 for the white body.
        let body = ((light - 0.35) / 0.65).clamp(0.0, 1.0);
        for (i, blue) in UPDATE_BLUE.iter().enumerate() {
            let shaded = blue * (0.8 + 0.2 * light);
            out.push((p[i] as f32 * (1.0 - body) + shaded * body).round().clamp(0.0, 255.0) as u8);
        }
        out.push(p[3]);
    }
    out
}

/// Blue Mochi in the tray while this build's updater has a newer build waiting
/// (show_update), or the usual one.
fn tray_update_hint(app: &AppHandle, on: bool) {
    let Some(tray) = app.tray_by_id("coucou") else { return };
    let Some(base) = app.default_window_icon() else { return };
    let icon = if on {
        Image::new_owned(tint_blue(base.rgba()), base.width(), base.height())
    } else {
        base.clone().to_owned()
    };
    let _ = tray.set_icon(Some(icon));
}

#[cfg(test)]
mod tests {
    use super::tint_blue;

    #[test]
    fn the_body_turns_blue_the_outline_stays() {
        let px = [
            245, 240, 235, 255, // the off-white body
            10, 10, 10, 255, // the outline
            250, 250, 250, 0, // transparent around Mochi
        ];
        let out = tint_blue(&px);
        let body = &out[0..4];
        assert!(body[2] > 220 && body[0] < 80 && body[1] > 100 && body[1] < 150, "body {body:?}");
        assert_eq!(&out[4..8], &[10, 10, 10, 255], "the outline stays dark");
        assert_eq!(out[11], 0, "transparency is kept");
        assert_eq!(out.len(), px.len());
    }
}
