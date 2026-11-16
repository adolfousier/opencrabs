//! The Windows manifest stays in `build.rs`, where #2965 put it.
//!
//! The CI leg launches the GUI binary on a Windows runner, which is the real
//! check that the loader resolves `TaskDialogIndirect`. That leg only runs on
//! Windows, so a build script edit that drops the manifest leaves every test on
//! this machine green until a user reports the app will not start.
//!
//! This test pins the three things that make the manifest work, on any host.

const BUILD_RS: &str = include_str!("../../build.rs");

/// The resource object carries the manifest, and it is compiled afterwards.
#[test]
fn the_build_script_embeds_a_manifest() {
    let set = BUILD_RS
        .find("set_manifest(")
        .expect("build.rs must embed a manifest or the GUI cannot load (#2965)");
    let compile = BUILD_RS
        .find("res.compile()")
        .expect("the resource object must be compiled for the manifest to reach the exe");
    assert!(
        set < compile,
        "the manifest has to be set before the resource is compiled"
    );
}

/// Only comctl32 v6 exports the symbol gpui imports, and v6 activates only
/// when the exe declares the Common-Controls dependency.
#[test]
fn the_manifest_declares_common_controls_v6() {
    assert!(
        BUILD_RS.contains("Microsoft.Windows.Common-Controls"),
        "the manifest must declare the Common-Controls dependency (#2965)"
    );
    assert!(
        BUILD_RS.contains("version=\"6.0.0.0\""),
        "the declared dependency has to be v6, which is the only one that exports the symbol"
    );
    assert!(
        BUILD_RS.contains("PerMonitorV2"),
        "the manifest also carries the DPI block, since the exe had none before #2965"
    );
}

/// The embed is Windows only, so the other hosts keep building without a
/// resource compiler.
#[test]
fn the_manifest_is_gated_to_windows() {
    let cfg = BUILD_RS
        .find("#[cfg(target_os = \"windows\")]")
        .expect("the resource block is Windows only");
    let set = BUILD_RS
        .find("set_manifest(")
        .expect("build.rs must embed a manifest (#2965)");
    assert!(
        cfg < set,
        "the manifest has to sit inside the Windows block, not run on every host"
    );
}
