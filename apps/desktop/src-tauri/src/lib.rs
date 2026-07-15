use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{Manager, WebviewWindow};

pub mod application;
pub mod commands;
pub mod state;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
                let _ = window.show();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = state::AppState::initialize(None)
                .map_err(|error| format!("MPrism 初始化失败: {error}"))?;
            app.manage(state);
            if let Some(window) = app.get_webview_window("main") {
                apply_window_material(&window);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::set_theme,
            commands::upsert_provider,
            commands::delete_provider,
            commands::set_defaults,
            commands::discover_models,
            commands::create_session,
            commands::list_sessions,
            commands::load_session,
            commands::update_session,
            commands::delete_session,
            commands::start_chat,
            commands::cancel_chat,
        ])
        .build(tauri::generate_context!())
        .expect("error while building MPrism");

    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            if let Some(state) = handle.try_state::<state::AppState>() {
                tauri::async_runtime::block_on(state.shutdown());
            }
        }
    });
}

/// Apply Mica on supported Windows versions; ignore failures for solid fallback.
fn apply_window_material(window: &WebviewWindow) {
    let effects = EffectsBuilder::new()
        .effect(Effect::Mica)
        .state(EffectState::FollowsWindowActiveState)
        .build();

    if let Err(err) = window.set_effects(effects) {
        eprintln!("window effects unavailable, using solid background: {err}");
    }
}
