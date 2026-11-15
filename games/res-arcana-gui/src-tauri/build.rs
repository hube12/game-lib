//! Copies the shared web components next to the page before Tauri embeds the
//! frontend, so `web/` is complete without a bundler.

fn main() {
    gamelib_gui::web::write_to(std::path::Path::new("../web/gamelib-gui")).expect("copy the shared web components");
    println!("cargo:rerun-if-changed=../../../gui/web");
    tauri_build::build();
}
