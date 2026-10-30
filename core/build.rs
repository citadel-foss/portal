//! With `embedded-tor`, compiles the Tor expert bundle for the build target into the binary.
//!
//! The bundle lives in `tor-bundle/<target>/` and is fetched by `scripts/fetch-tor.sh` the
//! first time a target is built. `PORTAL_TOR_BUNDLE_DIR` points at one prepared elsewhere,
//! for a build without network access.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PORTAL_TOR_BUNDLE_DIR");
    println!("cargo:rerun-if-changed=scripts/fetch-tor.sh");
    if env::var_os("CARGO_FEATURE_EMBEDDED_TOR").is_none() {
        return;
    }

    let target = env::var("TARGET").expect("cargo sets TARGET");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let bundle = match env::var_os("PORTAL_TOR_BUNDLE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = manifest_dir.join("tor-bundle").join(&target);
            if !dir.join("tor").is_file() {
                fetch(&manifest_dir, &target, &dir);
            }
            dir
        }
    };
    println!("cargo:rerun-if-changed={}", bundle.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&bundle)
        .unwrap_or_else(|e| panic!("cannot read Tor bundle {}: {e}", bundle.display()))
        .map(|entry| entry.expect("readable bundle entry").path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    assert!(
        files
            .iter()
            .any(|path| path.file_name() == Some("tor".as_ref())),
        "Tor bundle {} has no `tor` executable",
        bundle.display()
    );

    let mut source = String::from("pub(crate) const FILES: &[(&str, &[u8])] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_name().unwrap().to_str().expect("UTF-8 file name");
        writeln!(
            source,
            "    ({name:?}, include_bytes!({:?})),",
            path.display()
        )
        .unwrap();
    }
    source.push_str("];\n");
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out.join("tor_bundle.rs"), source).expect("write tor_bundle.rs");
}

fn fetch(manifest_dir: &Path, target: &str, dir: &Path) {
    let status = Command::new("sh")
        .arg(manifest_dir.join("scripts/fetch-tor.sh"))
        .arg(target)
        .arg(dir)
        .status()
        .unwrap_or_else(|e| panic!("could not run scripts/fetch-tor.sh: {e}"));
    assert!(
        status.success(),
        "scripts/fetch-tor.sh failed for {target}; set PORTAL_TOR_BUNDLE_DIR to a bundle \
         prepared with it to build without network access"
    );
}
