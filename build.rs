fn main() {
    println!("cargo:rerun-if-changed=assets/icons");
    println!("cargo:rerun-if-changed=Cargo.toml");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("papo.ico");

    let sizes = [16u16, 22, 24, 32, 48, 64, 128, 256];
    let mut images = Vec::new();
    for size in sizes {
        let path = manifest.join("assets").join("icons").join(format!("{size}.png"));
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        images.push((size, bytes));
    }

    // ICO supports PNG payloads directly. Build a multi-size container from
    // the canonical PNG assets so Windows Explorer/taskbar and Setup all use
    // the same artwork without another hand-maintained source file.
    let header_len = 6 + images.len() * 16;
    let total_len = header_len + images.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
    let mut data = Vec::with_capacity(total_len);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = header_len as u32;
    for (size, bytes) in &images {
        data.push(if *size == 256 { 0 } else { *size as u8 });
        data.push(if *size == 256 { 0 } else { *size as u8 });
        data.push(0);
        data.push(0);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(&offset.to_le_bytes());
        offset += bytes.len() as u32;
    }
    for (_, bytes) in images {
        data.extend_from_slice(&bytes);
    }
    std::fs::write(&ico, data).expect("write Windows icon");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ico.to_str().expect("UTF-8 icon path"));
    resource
        .compile()
        .expect("compile Windows application resources");
}
