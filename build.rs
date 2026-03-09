fn main() {
    // On Linux/Windows, whisper-rs-sys links before llama-cpp-sys-2. Both bundle
    // ggml statically. With --allow-multiple-definition, whisper's older ggml wins,
    // causing crashes when llama calls incompatible functions (issue #38).
    //
    // Fix: find llama's ggml libs and emit link-search/link-lib FIRST from our
    // build.rs. Our crate's link directives appear before dependency rlibs in the
    // final linker command, so llama's newer ggml wins the symbol resolution.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        return; // macOS two-level namespaces handle duplicates fine
    }

    // Find llama-cpp-sys-2's output directory by scanning the build dir.
    // The path is: target/<profile>/build/llama-cpp-sys-2-<hash>/out/lib/
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    // OUT_DIR = target/<profile>/build/opencrabs-<hash>/out
    // We need:  target/<profile>/build/llama-cpp-sys-2-*/out/lib/
    if let Some(build_dir) = std::path::Path::new(&out_dir)
        .parent()
        .and_then(|p| p.parent())
        && let Ok(entries) = std::fs::read_dir(build_dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("llama-cpp-sys-2-") {
                let lib_dir = entry.path().join("out").join("lib");
                if lib_dir.exists() {
                    // Emit link search path for llama's ggml — this appears
                    // before whisper's in the final link command, so llama's
                    // newer ggml symbols win with --allow-multiple-definition.
                    println!("cargo:rustc-link-search=native={}", lib_dir.display());
                    println!("cargo:rustc-link-lib=static=ggml");
                    println!("cargo:rustc-link-lib=static=ggml-base");
                    println!("cargo:rustc-link-lib=static=ggml-cpu");
                    break;
                }
            }
        }
    }
}
