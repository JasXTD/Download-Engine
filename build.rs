use std::{env, fs, path::PathBuf, process::Command};
#[path = "scripts/localization-build.rs"]
mod localization;

fn main() {
    localization::generate();
    println!("cargo:rerun-if-changed=assets/download-engine.ico");
    println!("cargo:rerun-if-changed=assets/download-engine.rc");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    // Use the host SDK compiler, including when producing an ARM64 build.
    let host = env::var("HOST").expect("Cargo must set HOST");
    let architecture = host.split('-').next().expect("Cargo host architecture");
    let sdk = find_msvc_tools::find_windows_sdk(architecture)
        .expect("The Windows SDK is required to embed the Download Engine application icon");
    let compiler = sdk
        .path()
        .map(|directory| directory.join("rc.exe"))
        .find(|path| path.is_file())
        .expect("Cannot find rc.exe in the Windows SDK");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let parts: Vec<String> = ["MAJOR", "MINOR", "PATCH"]
        .iter()
        .map(|part| {
            env::var(format!("CARGO_PKG_VERSION_{part}"))
                .expect("Cargo version component")
                .parse::<u16>()
                .expect("Windows version components must fit in 16 bits")
                .to_string()
        })
        .collect();
    let template =
        fs::read_to_string("assets/download-engine.rc").expect("Windows resource template");
    let source = output.join("download-engine.rc");
    fs::write(
        &source,
        template
            .replace("@VERSION@", &version)
            .replace("@VERSION_NUM@", &format!("{},0", parts.join(","))),
    )
    .expect("Write versioned Windows resource");
    let resource = output.join("download-engine.res");
    let status = Command::new(compiler)
        .current_dir(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(source)
        .status()
        .expect("Cannot run the Windows resource compiler");
    assert!(
        status.success(),
        "Compiling the Download Engine icon and version resource failed"
    );
    for binary in ["de", "de-cli", "de-host"] {
        println!("cargo:rustc-link-arg-bin={binary}={}", resource.display());
    }
}
