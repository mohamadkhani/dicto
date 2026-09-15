fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let rc_path = manifest_dir.join("icon.rc");
        let icon_path = manifest_dir.join("icon.ico");
        println!("cargo:rerun-if-changed={}", rc_path.display());
        println!("cargo:rerun-if-changed={}", icon_path.display());
        let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
    }

    // Expose the latest git tag as GIT_TAG_VERSION so the About panel
    // shows the real release version instead of the (stale) Cargo.toml one.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/tags");
    let version = std::process::Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty());
    let version_str = version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=APP_VERSION={}", version_str);
}
