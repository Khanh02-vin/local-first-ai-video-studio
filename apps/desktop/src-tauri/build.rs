fn main() {
    // bundle.resources must exist when tauri_build validates the config, but two
    // entries are gitignored build artifacts: ../../web/build-node (npm run
    // build:node) and resources/node (fetch-bundled-assets.sh). cargo check/test
    // on a fresh clone runs before either generator, so create empty placeholders —
    // real content is produced by beforeBuildCommand / the fetch step before
    // anything is actually bundled.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for dir in [manifest.join("../../web/build-node"), manifest.join("resources/node")] {
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    tauri_build::build()
}
