//! Embed the built frontend so one binary serves the web interface without a frontend directory.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Directory holding the built frontend, relative to this crate.
const FRONTEND_DIRECTORY: &str = "../../apps/frontend/dist";
/// Generated asset table included by `src/embedded.rs`.
const ASSET_TABLE: &str = "frontend_assets.rs";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_directory = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("cargo provides the manifest directory"),
    );
    let frontend_directory = manifest_directory.join(FRONTEND_DIRECTORY);
    let mut assets = Vec::new();
    collect(&frontend_directory, &frontend_directory, &mut assets);
    assets.sort();
    let destination =
        PathBuf::from(env::var("OUT_DIR").expect("cargo provides the output directory"))
            .join(ASSET_TABLE);
    fs::write(&destination, render(&frontend_directory, &assets))
        .expect("the generated asset table is writable");
}

/// Collect frontend files relative to the embedded root, rebuilding when any of them changes.
///
/// A missing directory is not an error: a build without a frontend embeds nothing and serves the
/// API alone. Emitting the absent path keeps this script rerunning until the frontend is built.
fn collect(root: &Path, directory: &Path, assets: &mut Vec<String>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, assets);
        } else if let Ok(relative) = path.strip_prefix(root) {
            assets.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Render the table mapping each embedded path to its file at an absolute location.
fn render(root: &Path, assets: &[String]) -> String {
    let mut table = String::from(
        "/// Assets of the frontend embedded when this binary was compiled, sorted by path.\n\
         pub(crate) static FRONTEND_ASSETS: &[(&str, &[u8])] = &[\n",
    );
    for asset in assets {
        let path = asset
            .split('/')
            .fold(root.to_path_buf(), |path, component| path.join(component));
        table.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            asset,
            path.to_string_lossy()
        ));
    }
    table.push_str("];\n");
    table
}
