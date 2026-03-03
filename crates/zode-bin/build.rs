use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Windows' default main-thread stack is 1 MB which is too small for the
    // deep egui render pipeline combined with post-quantum crypto types
    // (ML-DSA-65 / ML-KEM-768 keys are multi-KB each).  Request 8 MB.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }

    emit_build_version();
}

/// Build a version string of the form `0.2.0-abcdef1` (or `0.2.0-abcdef1-dirty`)
/// by combining the Cargo package version with the current git short hash.
/// Falls back to the plain package version when git is unavailable.
fn emit_build_version() {
    let pkg_version = env!("CARGO_PKG_VERSION");

    // Tell Cargo to re-run this script when git HEAD changes (commit, checkout, rebase).
    let git_dir = find_git_dir();
    if let Some(ref gd) = git_dir {
        let head = gd.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(content) = std::fs::read_to_string(&head) {
            if let Some(ref_path) = content.strip_prefix("ref: ") {
                println!(
                    "cargo:rerun-if-changed={}",
                    gd.join(ref_path.trim()).display()
                );
            }
        }
    }

    let short_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    let dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    let version = match short_hash {
        Some(hash) if dirty => format!("{pkg_version}+{hash}.dirty"),
        Some(hash) => format!("{pkg_version}+{hash}"),
        None => pkg_version.to_string(),
    };

    println!("cargo:rustc-env=BUILD_VERSION={version}");
}

fn find_git_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dir = manifest_dir.as_path();
    loop {
        let candidate = dir.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}
