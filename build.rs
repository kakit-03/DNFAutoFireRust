use image::codecs::ico::IcoEncoder;
use image::{ExtendedColorType, ImageEncoder, ImageReader, imageops::FilterType};
use std::env;
use std::fs::File;
use std::path::PathBuf;

const APP_ICON_SIZE: u32 = 256;
const APP_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity
    version="1.0.0.0"
    processorArchitecture="*"
    name="dnf.autofire"
    type="win32"
  />
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
    </windowsSettings>
  </application>
</assembly>
"#;

fn main() {
    if let Err(error) = build_windows_resources() {
        panic!("failed to build Windows resources: {error}");
    }
}

fn build_windows_resources() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let source_icon_path = manifest_dir.join("tp.png");
    let output_icon_path = out_dir.join("tp.ico");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", source_icon_path.display());

    let rgba = ImageReader::open(&source_icon_path)?
        .decode()?
        .resize_exact(APP_ICON_SIZE, APP_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = rgba.dimensions();

    let output_icon_file = File::create(&output_icon_path)?;
    IcoEncoder::new(output_icon_file).write_image(
        rgba.as_raw(),
        width,
        height,
        ExtendedColorType::Rgba8,
    )?;

    let mut resources = winresource::WindowsResource::new();
    resources.set_manifest(APP_MANIFEST);
    resources.set_icon(output_icon_path.to_string_lossy().as_ref());
    resources.compile()?;
    Ok(())
}
