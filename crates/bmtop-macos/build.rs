fn main() {
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("macos")) {
        cc::Build::new()
            .file("src/bmtop_sys.c")
            .file("src/bmtop_soc.c")
            .flag("-fblocks")
            .include("src")
            .compile("bmtop_sys");
        // 链路/雷雳/FPS：ObjC 编译单元（CoreWLAN 是 ObjC API）。
        cc::Build::new()
            .file("src/bmtop_link.m")
            .flag("-fobjc-arc")
            .include("src")
            .compile("bmtop_link");
        println!("cargo:rustc-link-lib=framework=CoreWLAN");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=IOKit");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        // libIOReport 实体在 dyld shared cache 里，链接靠 SDK 的 .tbd 桩。
        if let Some(sdk) = sdk_path() {
            println!("cargo:rustc-link-search=native={sdk}/usr/lib");
        }
        println!("cargo:rustc-link-lib=dylib=IOReport");
        // bmtop_soc.c 用 objc_autoreleasePoolPush/Pop 包住 IOReport 采样。
        println!("cargo:rustc-link-lib=dylib=objc");
        println!("cargo:rerun-if-changed=src/bmtop_sys.c");
        println!("cargo:rerun-if-changed=src/bmtop_sys.h");
        println!("cargo:rerun-if-changed=src/bmtop_soc.c");
        println!("cargo:rerun-if-changed=src/bmtop_soc.h");
        println!("cargo:rerun-if-changed=src/bmtop_link.m");
        println!("cargo:rerun-if-changed=src/bmtop_link.h");
    }
}

fn sdk_path() -> Option<String> {
    let output = std::process::Command::new("xcrun")
        .args(["--show-sdk-path"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|path| !path.is_empty())
}
