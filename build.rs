fn main() {
    println!("cargo:rerun-if-changed=assets/icons");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=PAPO_GIPHY_API_KEY");

    let patch = "patches/winit+0.30.13.patch";
    let marker = "target/patch/winit-0.30.13/.papo-applied.patch";
    println!("cargo:rerun-if-changed={patch}");
    println!("cargo:rerun-if-changed={marker}");
    let expected = std::fs::read(patch).expect("read the versioned winit patch");
    assert!(
        std::fs::read(marker).is_ok_and(|applied| applied == expected),
        "winit patch is missing or outdated: run python3 scripts/prepare-winit.py before Cargo",
    );

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=native/macos/webembed.m");
        println!("cargo:rerun-if-changed=native/macos/notify.m");
        let output = std::process::Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output().expect("macOS SDK path");
        assert!(output.status.success(), "xcrun failed");
        let sdk = String::from_utf8(output.stdout).expect("SDK path is UTF-8");
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let object = out.join("papo-webembed.o");
        let status = std::process::Command::new("xcrun")
            .args(["clang", "-fobjc-arc", "-fmodules", "-mmacosx-version-min=12.0",
                "-isysroot", sdk.trim(), "-c", "native/macos/webembed.m", "-o"])
            .arg(&object).status().expect("compile WKWebView bridge");
        assert!(status.success(), "WKWebView bridge compilation failed");
        let notify_object = out.join("papo-notify.o");
        let status = std::process::Command::new("xcrun")
            .args(["clang", "-fobjc-arc", "-fmodules", "-mmacosx-version-min=12.0",
                "-isysroot", sdk.trim(), "-c", "native/macos/notify.m", "-o"])
            .arg(&notify_object).status().expect("compile macOS notifications bridge");
        assert!(status.success(), "macOS notifications compilation failed");
        let archive = out.join("libpapo_webembed.a");
        let status = std::process::Command::new("xcrun")
            .args(["ar", "rcs"]).arg(&archive).arg(&object).arg(&notify_object)
            .status().expect("archive WKWebView and notifications bridge");
        assert!(status.success(), "archiving WKWebView bridge failed");
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=papo_webembed");
        for framework in ["AppKit", "WebKit", "QuartzCore", "CoreFoundation", "UserNotifications"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        return;
    }

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

    let rc = out.join("papo.rc");
    let escaped_icon = ico.display().to_string().replace('\\', "\\\\");
    std::fs::write(&rc, format!("1 ICON \"{escaped_icon}\"\n"))
        .expect("write Windows resource script");

    // The package also builds a cdylib for Android. Linking the resource
    // globally makes it collide with Turso's VERSION resource in papo.dll.
    // Keep the application icon strictly on the Windows executable.
    if let Err(error) =
        embed_resource::compile_for(&rc, ["papo"], embed_resource::NONE).manifest_required()
    {
        panic!("compile Windows application icon: {error}");
    }
}
