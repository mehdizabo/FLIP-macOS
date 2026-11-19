fn main() {
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let mut p = std::path::PathBuf::from(out_dir);
        // target/<profile>/build/<crate-hash>/out -> target/<profile>
        for _ in 0..3 {
            if !p.pop() {
                break;
            }
        }
        let stale_bridge = p.join("flip-macos-bridge");
        if stale_bridge.exists() {
            let _ = std::fs::remove_file(&stale_bridge);
        }
    }
    tauri_build::build()
}
