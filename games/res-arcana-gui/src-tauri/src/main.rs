//! The Res Arcana desktop app: a window around the page in `../web`, with one
//! command (`api`) that carries the JSON protocol of `gamelib-gui`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use gamelib_gui::Backend;
use gamelib_res_arcana_gui::{ResArcanaUi, backend_with_logs};

/// The running game, shared by every call.
struct Game(Mutex<Backend<ResArcanaUi>>);

/// Answers one JSON request of the page (`transport.js` calls this).
#[tauri::command]
fn api(game: tauri::State<'_, Game>, request: String) -> String {
    game.0.lock().map_or_else(
        |_| r#"{"type":"error","message":"the game state is unavailable"}"#.to_owned(),
        |mut backend| backend.handle_json(&request),
    )
}

fn main() {
    tauri::Builder::default()
        .manage(Game(Mutex::new(backend_with_logs())))
        .invoke_handler(tauri::generate_handler![api])
        .run(tauri::generate_context!())
        .expect("the Res Arcana window failed to start");
}
