fn main() {
    // Embed icon and metadata into Windows executables
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("src/assets/icon.ico");
        res.set("ProductName", "OpenCrabs");
        res.set("FileDescription", "OpenCrabs — AI Agent");
        // The GUI binary statically imports TaskDialogIndirect, and only
        // comctl32 v6 exports it. v6 activates only when the exe declares the
        // Common-Controls dependency in a manifest. gpui ships that manifest
        // behind its `windows-manifest` feature, which `default-features =
        // false` drops (Cargo.toml:318), and its build script could not link
        // the resource into our binaries anyway: build-script link args apply
        // only to the package that owns them. So it is embedded here (#2965).
        // The DPI block mirrors gpui's manifest, since without a manifest the
        // exe declares no DPI awareness at all.
        res.set_manifest(concat!(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" "#,
            r#"xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">"#,
            r#"<asmv3:application><asmv3:windowsSettings>"#,
            r#"<dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">"#,
            r#"true</dpiAware>"#,
            r#"<dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">"#,
            r#"PerMonitorV2</dpiAwareness>"#,
            r#"</asmv3:windowsSettings></asmv3:application>"#,
            r#"<dependency><dependentAssembly>"#,
            r#"<assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" "#,
            r#"version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df"/>"#,
            r#"</dependentAssembly></dependency></assembly>"#,
        ));
        res.compile().expect("Failed to compile Windows resources");
    }
}
