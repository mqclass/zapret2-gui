use std::path::Path;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_default();

    if target_os == "windows" && profile == "release" {
        // Ensure app.rc resource definition points to app.manifest (ID 1, RT_MANIFEST = 24)
        if !Path::new("app.rc").exists() {
            std::fs::write("app.rc", "1 24 \"app.manifest\"\n").unwrap();
        }

        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=app.rc");

        // Compile and embed the UAC manifest requiring Administrator privileges directly into the PE executable
        embed_resource::compile("app.rc", embed_resource::NONE);
    }

    // Configure Tauri build without the default manifest so embed-resource controls the PE manifest
    let windows_attrs = tauri_build::WindowsAttributes::new_without_app_manifest();
    let attrs = tauri_build::Attributes::new().windows_attributes(windows_attrs);

    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}

