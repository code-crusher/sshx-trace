//! Build script for the `sshx` client.
//!
//! When targeting Windows, this embeds a version resource, an application icon,
//! and a manifest into the executable. Without them, the binary has no
//! identifying metadata at all, which makes antivirus engines (including
//! Microsoft Defender) far more likely to flag it as malware. See issue #144.

fn main() {
    println!("cargo:rerun-if-changed=windows/sshx.ico");
    println!("cargo:rerun-if-changed=windows/sshx.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("windows/sshx.ico")
        .set_manifest_file("windows/sshx.manifest")
        .set("CompanyName", "Eric Zhang")
        .set(
            "FileDescription",
            "A secure web-based, collaborative terminal.",
        )
        .set("InternalName", "sshx")
        .set("LegalCopyright", "Copyright (c) 2023 Eric Zhang")
        .set("OriginalFilename", "sshx.exe")
        .set("ProductName", "sshx");
    res.compile().expect("failed to compile Windows resources");
}
