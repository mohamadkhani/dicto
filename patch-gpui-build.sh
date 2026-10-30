#!/bin/bash
# Patches gpui and gpui_windows in the Cargo git cache
# to fix cross-compilation from Linux to Windows.
#
# Fixes:
# 1. gpui: llvm-rc can't resolve relative paths from OUT_DIR
# 2. gpui_windows: shader compilation uses D3DCompileFromFile with a
#    hardcoded build-machine path. Replaced with D3DCompile from memory
#    using include_str! to embed HLSL source at compile time.
#
# Run after `cargo clean` or when git dependencies update.

set -e

# Resolve the checkout dir for the pinned rev (see root Cargo.toml [patch]):
# checkouts live under <hash>/<short-rev>, so match on the rev prefix instead
# of a hardcoded directory name that breaks on every rev bump.
PINNED_REV=c612da6503499b7242ce60e7aa893184d63043ec
CACHE_BASE=$(find ~/.cargo/git/checkouts/zed-* -maxdepth 1 -type d -name "${PINNED_REV:0:7}*" 2>/dev/null | head -1)

if [ -z "$CACHE_BASE" ]; then
    echo "Error: Could not find zed git cache."
    echo "Run 'cargo check' first to download dependencies, then re-run this script."
    exit 1
fi

# --- Patch 1: gpui build.rs (llvm-rc path fix) ---
GPUI_BUILD="$CACHE_BASE/crates/gpui/build.rs"

cat > "$GPUI_BUILD" << 'EOF'
#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    println!("cargo::rustc-check-cfg=cfg(gles)");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "windows" {
        #[cfg(feature = "windows-manifest")]
        embed_resource();
    }
}

#[cfg(feature = "windows-manifest")]
fn embed_resource() {
    let crate_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = crate_dir.join("resources/windows/gpui.manifest.xml");
    let rc_file = crate_dir.join("resources/windows/gpui.rc");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", rc_file.display());

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let fixed_rc = out_dir.join("gpui-fixed.rc");
    let rc_content = format!(
        "#define RT_MANIFEST 24\n1 RT_MANIFEST \"{}\"\n",
        manifest.display()
    );
    std::fs::write(&fixed_rc, rc_content).unwrap();

    embed_resource::compile(&fixed_rc, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
EOF
echo "Patched: $GPUI_BUILD"

# --- Patch 2: gpui_windows build.rs (stub shader bytes for cross-compilation) ---
GPUIWIN_BUILD="$CACHE_BASE/crates/gpui_windows/build.rs"

cat > "$GPUIWIN_BUILD" << 'BUILDEOF'
#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    #[cfg(not(debug_assertions))]
    {
        // When cross-compiling from Linux, fxc.exe is unavailable.
        // Generate stub shader bytes so the release build compiles.
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let path = std::path::Path::new(&out_dir).join("shaders_bytes.rs");

        if !path.exists() {
            let modules = [
                "quad", "shadow", "path_rasterization", "path_sprite",
                "underline", "monochrome_sprite", "subpixel_sprite",
                "polychrome_sprite", "emoji_rasterization",
            ];
            let mut content = String::new();
            for module in &modules {
                for suffix in &["VERTEX_BYTES", "FRAGMENT_BYTES"] {
                    content.push_str(&format!(
                        "const {}_{}: &[u8] = &[];\n",
                        module.to_uppercase(), suffix
                    ));
                }
            }
            std::fs::write(&path, content).unwrap();
        }
    }

    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    compile_shaders();
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
mod shader_compilation {
    use std::{
        fs, io::Write,
        path::{Path, PathBuf},
        process::{self, Command},
    };

    pub fn compile_shaders() {
        let shader_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/shaders.hlsl");
        let out_dir = std::env::var("OUT_DIR").unwrap();
        println!("cargo:rerun-if-changed={}", shader_path.display());
        let fxc_path = find_fxc_compiler();
        let modules = ["quad","shadow","path_rasterization","path_sprite","underline","monochrome_sprite","subpixel_sprite","polychrome_sprite"];
        let rust_binding_path = format!("{}/shaders_bytes.rs", out_dir);
        if Path::new(&rust_binding_path).exists() { fs::remove_file(&rust_binding_path).expect("Failed to remove"); }
        for module in &modules {
            compile_shader_for_module(module, &out_dir, &fxc_path, shader_path.to_str().unwrap(), &rust_binding_path);
        }
        let shader_path2 = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/color_text_raster.hlsl");
        compile_shader_for_module("emoji_rasterization", &out_dir, &fxc_path, shader_path2.to_str().unwrap(), &rust_binding_path);
    }

    pub fn find_latest_windows_sdk_binary(binary: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
        let key = windows_registry::LOCAL_MACHINE.open("SOFTWARE\\WOW6432Node\\Microsoft\\Microsoft SDKs\\Windows\\v10.0")?;
        let install_folder: String = key.get_string("InstallationFolder")?;
        let install_folder_bin = Path::new(&install_folder).join("bin");
        let mut versions: Vec<_> = std::fs::read_dir(&install_folder_bin)?.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect();
        versions.sort_by_key(|s| s.split('.').filter_map(|p| p.parse().ok()).collect::<Vec<u32>>());
        let arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", _ => Err(format!("Unsupported: {}", std::env::consts::ARCH))? };
        if let Some(highest) = versions.last() { return Ok(Some(install_folder_bin.join(highest).join(arch).join(binary))); }
        Ok(None)
    }

    fn find_fxc_compiler() -> String {
        if let Ok(path) = std::env::var("GPUI_FXC_PATH") && Path::new(&path).exists() { return path; }
        if let Ok(output) = std::process::Command::new("where.exe").arg("fxc.exe").output() && output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
        if let Ok(Some(path)) = find_latest_windows_sdk_binary("fxc.exe") { return path.to_string_lossy().into_owned(); }
        panic!("Failed to find fxc.exe");
    }

    fn compile_shader_for_module(module: &str, out_dir: &str, fxc_path: &str, shader_path: &str, rust_binding_path: &str) {
        for (suffix, target, stage) in [("vs", "vs_4_1", "vertex"), ("ps", "ps_4_1", "fragment")] {
            let output_file = format!("{}/{}_{}.h", out_dir, module, suffix);
            let const_name = format!("{}_{}_BYTES", module.to_uppercase(), stage.to_uppercase());
            let entry_name = format!("{}_{}", module, stage);
            let output = Command::new(fxc_path).args(["/T", target, "/E", &entry_name, "/Fh", &output_file, "/Vn", &const_name, "/O3", shader_path]).output();
            match output {
                Ok(result) if result.status.success() => {},
                Ok(result) => { println!("cargo::error=Shader compilation failed for {}:\n{}", entry_name, String::from_utf8_lossy(&result.stderr)); process::exit(1); }
                Err(e) => { println!("cargo::error=Failed to run fxc: {}", e); process::exit(1); }
            }
            let header = fs::read_to_string(&output_file).expect("Failed to read header");
            let def = { let s = header.find("const BYTE").unwrap(); let eq = header[s..].find('=').unwrap(); header[s+eq+1..].trim() };
            let binding = format!("const {}: &[u8] = &{}\n", const_name, def.replace('{', "[").replace('}', "]"));
            fs::OpenOptions::new().create(true).append(true).open(rust_binding_path).unwrap().write_all(binding.as_bytes()).unwrap();
        }
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
use shader_compilation::compile_shaders;
BUILDEOF
echo "Patched: $GPUIWIN_BUILD"

# --- Patch 3: gpui_windows directx_renderer.rs ---
# Replace D3DCompileFromFile (needs file path) with D3DCompile (from memory).
# This fixes the "path not found" error on Windows when the .exe was
# cross-compiled and env!("CARGO_MANIFEST_DIR") points to a Linux path.

RENDERER="$CACHE_BASE/crates/gpui_windows/src/directx_renderer.rs"

# Use sed to replace the build_shader_blob function
# First, let's check the file exists
if [ ! -f "$RENDERER" ]; then
    echo "Error: $RENDERER not found"
    exit 1
fi

# Create a Python script for the replacement (more reliable than sed for multiline)
python3 - "$RENDERER" << 'PYEOF'
import sys
renderer = sys.argv[1]
with open(renderer, 'r') as f:
    content = f.read()

# Add D3DCompile to the Fxc imports
old_import = 'Fxc::{D3DCOMPILE_DEBUG, D3DCOMPILE_SKIP_OPTIMIZATION, D3DCompileFromFile}'
new_import = 'Fxc::{D3DCOMPILE_DEBUG, D3DCOMPILE_SKIP_OPTIMIZATION, D3DCompile, D3DCompileFromFile}'
content = content.replace(old_import, new_import)

old_fn = '''    pub(super) fn build_shader_blob(entry: ShaderModule, target: ShaderTarget) -> Result<ID3DBlob> {
        unsafe {
            use windows::Win32::Graphics::{
                Direct3D::ID3DInclude, Hlsl::D3D_COMPILE_STANDARD_FILE_INCLUDE,
            };

            let shader_source = if matches!(entry, ShaderModule::EmojiRasterization) {
                include_str!("color_text_raster.hlsl")
            } else {
                include_str!("shaders.hlsl")
            };

            let entry_name = format!(
                "{}_{}\\0",
                entry.as_str(),
                match target {
                    ShaderTarget::Vertex => "vertex",
                    ShaderTarget::Fragment => "fragment",
                }
            );
            let target_profile = match target {
                ShaderTarget::Vertex => "vs_4_1\\0",
                ShaderTarget::Fragment => "ps_4_1\\0",
            };

            let mut compile_blob = None;
            let mut error_blob = None;

            let entry_point = PCSTR::from_raw(entry_name.as_ptr());
            let target_cstr = PCSTR::from_raw(target_profile.as_ptr());

            let include_handler = &std::mem::transmute::<usize, ID3DInclude>(
                D3D_COMPILE_STANDARD_FILE_INCLUDE as usize,
            );

            let ret = D3DCompile(
                shader_source.as_ptr() as *const _,
                shader_source.len(),
                windows_core::PCSTR::null(),
                None,
                include_handler,
                entry_point,
                target_cstr,
                D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION,
                0,
                &mut compile_blob,
                Some(&mut error_blob),
            );

            if ret.is_err() {
                let Some(error_blob) = error_blob else {
                    return Err(anyhow::anyhow!("{ret:?}"));
                };

                let error_string =
                    std::ffi::CStr::from_ptr(error_blob.GetBufferPointer() as *const i8)
                        .to_string_lossy();
                log::error!("Shader compile error: {}", error_string);
                return Err(anyhow::anyhow!("Compile error: {}", error_string));
            }
            Ok(compile_blob.unwrap())
        }
    }'''

new_fn = '''    pub(super) fn build_shader_blob(entry: ShaderModule, target: ShaderTarget) -> Result<ID3DBlob> {
        unsafe {
            use windows::Win32::Graphics::{
                Direct3D::ID3DInclude, Hlsl::D3D_COMPILE_STANDARD_FILE_INCLUDE,
            };

            // Resolve #include "alpha_correction.hlsl" at compile time
            // since we're compiling from memory and the include handler
            // can't resolve local file paths.
            const ALPHA_CORRECTION_HLSL: &str = include_str!("alpha_correction.hlsl");
            const SHADERS_HLSL: &str = include_str!("shaders.hlsl");
            const COLOR_TEXT_HLSL: &str = include_str!("color_text_raster.hlsl");

            let shader_source = if matches!(entry, ShaderModule::EmojiRasterization) {
                COLOR_TEXT_HLSL.replace("#include \\"alpha_correction.hlsl\\"", ALPHA_CORRECTION_HLSL)
            } else {
                SHADERS_HLSL.replace("#include \\"alpha_correction.hlsl\\"", ALPHA_CORRECTION_HLSL)
            };

            let entry_name = format!(
                "{}_{}\\0",
                entry.as_str(),
                match target {
                    ShaderTarget::Vertex => "vertex",
                    ShaderTarget::Fragment => "fragment",
                }
            );
            let target_profile = match target {
                ShaderTarget::Vertex => "vs_4_1\\0",
                ShaderTarget::Fragment => "ps_4_1\\0",
            };

            let mut compile_blob = None;
            let mut error_blob = None;

            let entry_point = PCSTR::from_raw(entry_name.as_ptr());
            let target_cstr = PCSTR::from_raw(target_profile.as_ptr());

            let include_handler = &std::mem::transmute::<usize, ID3DInclude>(
                D3D_COMPILE_STANDARD_FILE_INCLUDE as usize,
            );

            let ret = D3DCompile(
                shader_source.as_ptr() as *const _,
                shader_source.len(),
                windows_core::PCSTR::null(),
                None,
                include_handler,
                entry_point,
                target_cstr,
                D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION,
                0,
                &mut compile_blob,
                Some(&mut error_blob),
            );

            if ret.is_err() {
                let Some(error_blob) = error_blob else {
                    return Err(anyhow::anyhow!("{ret:?}"));
                };

                let error_string =
                    std::ffi::CStr::from_ptr(error_blob.GetBufferPointer() as *const i8)
                        .to_string_lossy();
                log::error!("Shader compile error: {}", error_string);
                return Err(anyhow::anyhow!("Compile error: {}", error_string));
            }
            Ok(compile_blob.unwrap())
        }
    }'''

if old_fn in content:
    content = content.replace(old_fn, new_fn)
    with open(renderer, 'w') as f:
        f.write(content)
    print("Patched directx_renderer.rs: replaced D3DCompileFromFile with D3DCompile")
else:
    print("WARNING: Could not find the target function in directx_renderer.rs")
    print("The file may have already been patched or the function has changed.")
    sys.exit(1)
PYEOF

echo ""
echo "All patches applied successfully."
echo "You can now build with: cargo xwin build --target x86_64-pc-windows-msvc -p dicto"