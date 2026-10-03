use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let (major, minor, patch) = version_numbers(&version);
    let icon = std::env::current_dir()
        .expect("current dir")
        .join("assets")
        .join("icon.ico");
    let icon = icon.to_string_lossy().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "IkTerminal"
            VALUE "FileDescription", "IkTerminal"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "ikterminal"
            VALUE "OriginalFilename", "ikterminal.exe"
            VALUE "ProductName", "IkTerminal"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("ikterminal.rc");
    std::fs::write(&out, rc).expect("write resource script");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed Windows resources");
}

fn version_numbers(version: &str) -> (u16, u16, u16) {
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty());
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}
