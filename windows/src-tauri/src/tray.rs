// Notification-area icon: Open, Weekly recap, Wardrobe, Settings, Pause, Quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
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
}

/// Relabels the menu in the current language.
pub fn retitle(app: &AppHandle) {
    let Some(items) = app.try_state::<Items>() else { return };
    for (label, item) in &items.0 {
        let _ = item.set_text(t(label));
    }
}
