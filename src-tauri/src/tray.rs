// System tray. Mirrors createTray in main.js.
//
// Menu: show/hide all pets, install/remove global integration, quit.
// Left-click shows all pet windows (no settings center — pet switching is done
// by right-clicking the pet itself).
//
// Install/uninstall run inline (not spawned) so we can show a system
// notification with the result — no silent fire-and-forget.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
use tauri_plugin_notification::NotificationExt;

use crate::bridge::server::ensure_pet_window;
use crate::cli::install;
use crate::state::runtime_state::{list_sessions, DEFAULT_SESSION_ID};

const TRAY_ICON: &[u8] = include_bytes!("../icons/icon.png");

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_all = MenuItem::with_id(app, "tray_show_all", "显示所有桌宠", true, None::<&str>)?;
    let hide_all = MenuItem::with_id(app, "tray_hide_all", "隐藏所有桌宠", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let install_user = MenuItem::with_id(app, "tray_install_user", "集成到 Claude Code(全局)", true, None::<&str>)?;
    let uninstall_user = MenuItem::with_id(app, "tray_uninstall_user", "移除全局集成", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&show_all, &hide_all, &sep1, &install_user, &uninstall_user, &sep2, &quit],
    )?;

    let tray_img = tauri::image::Image::from_bytes(TRAY_ICON).expect("tray icon");
    let tray = TrayIconBuilder::with_id("main")
        .icon(tray_img)
        .tooltip("ClaudePet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_all_pets(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show_all" => show_all_pets(app),
            "tray_hide_all" => hide_all_pets(app),
            "tray_install_user" => run_install(app),
            "tray_uninstall_user" => run_uninstall(app),
            "tray_quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    let _ = tray.set_show_menu_on_left_click(false);
    Ok(())
}

fn show_all_pets(app: &AppHandle) {
    for (id, _) in list_sessions() {
        if id == DEFAULT_SESSION_ID {
            continue;
        }
        ensure_pet_window(app, &id);
    }
    for win in app.webview_windows().values() {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn hide_all_pets(app: &AppHandle) {
    for (_label, win) in app.webview_windows() {
        let _ = win.hide();
    }
}

fn run_install(app: &AppHandle) {
    match install::install("user", true) {
        Ok(()) => {
            let _ = app.notification()
                .builder()
                .title("ClaudePet 集成成功")
                .body("已写入 ~/.claude/settings.json。新开 Claude Code 会话即可看到桌宠。")
                .show();
        }
        Err(e) => {
            let msg = format!("集成失败：{e:#}");
            let _ = app.notification()
                .builder()
                .title("ClaudePet 集成失败")
                .body(&msg)
                .show();
        }
    }
}

fn run_uninstall(app: &AppHandle) {
    match install::uninstall("user") {
        Ok(()) => {
            let _ = app.notification()
                .builder()
                .title("ClaudePet 已卸载")
                .body("已从 ~/.claude/settings.json 中移除 ClaudePet 集成。")
                .show();
        }
        Err(e) => {
            let msg = format!("卸载失败：{e:#}");
            let _ = app.notification()
                .builder()
                .title("ClaudePet 卸载失败")
                .body(&msg)
                .show();
        }
    }
}
