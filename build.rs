fn main() {
    // Embed icon and metadata into Windows executables
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "OpenCrabs");
        res.set("FileDescription", "OpenCrabs — AI Agent");
        res.compile().expect("Failed to compile Windows resources");
    }
}
