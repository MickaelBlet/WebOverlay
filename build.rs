fn main() {
    // tauri-build only watches tauri.conf.json: without this, a changed icon keeps the stale
    // Windows resource (exe/window icon) and the stale `generate_context!` icon in cached builds.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
