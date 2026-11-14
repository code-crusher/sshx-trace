//! Checks for the Windows resources embedded into the `sshx` executable.
//!
//! Antivirus engines are much more likely to flag an executable that carries no
//! icon, version information, or application manifest, so the build script
//! embeds all three on Windows targets (see issue #144). These tests validate
//! the resource files themselves, as well as the resources that end up in the
//! compiled binary.

const ICON: &[u8] = include_bytes!("../windows/sshx.ico");
const MANIFEST: &str = include_str!("../windows/sshx.manifest");

/// Read a little-endian `u16` at an offset, panicking if it is out of bounds.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

/// Read a little-endian `u32` at an offset, panicking if it is out of bounds.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn icon_is_well_formed() {
    assert_eq!(u16_at(ICON, 0), 0, "ICONDIR reserved field must be zero");
    assert_eq!(u16_at(ICON, 2), 1, "ICONDIR type must be an icon");

    let count = u16_at(ICON, 4) as usize;
    assert!(
        count >= 4,
        "icon should contain multiple sizes, got {count}"
    );

    let mut sizes = Vec::new();
    for i in 0..count {
        let entry = 6 + 16 * i;

        // A zero dimension is how the ICO format encodes 256 pixels.
        let width = match ICON[entry] {
            0 => 256,
            width => width as usize,
        };
        let height = match ICON[entry + 1] {
            0 => 256,
            height => height as usize,
        };
        let bits_per_pixel = u16_at(ICON, entry + 6);
        let len = u32_at(ICON, entry + 8) as usize;
        let offset = u32_at(ICON, entry + 12) as usize;

        assert_eq!(width, height, "icon image {i} is not square");
        assert_eq!(bits_per_pixel, 32, "icon images should be 32-bit RGBA");
        assert!(len > 0, "icon image {i} is empty");
        assert!(
            offset + len <= ICON.len(),
            "icon image {i} extends past the end of the file"
        );
        sizes.push(width);
    }

    for size in [16, 32, 48, 256] {
        assert!(
            sizes.contains(&size),
            "icon is missing the {size}x{size} size"
        );
    }
}

#[test]
fn manifest_does_not_request_elevation() {
    assert!(
        MANIFEST.starts_with("<?xml"),
        "manifest should start with an XML declaration"
    );
    assert!(
        MANIFEST.contains(r#"<requestedExecutionLevel level="asInvoker" uiAccess="false"/>"#),
        "manifest must run as the invoking user, not as an administrator"
    );
    assert!(
        !MANIFEST.contains("requireAdministrator") && !MANIFEST.contains("highestAvailable"),
        "sshx is a user-level application and must not request elevation"
    );
}

#[test]
fn manifest_declares_supported_windows_versions() {
    // Without a compatibility section, Windows applies compatibility shims as if
    // the executable targeted an ancient version of Windows.
    for (name, guid) in [
        ("Windows 7", "35138b9a-5d96-4fbd-8e2d-a2440225f93a"),
        ("Windows 8", "4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38"),
        ("Windows 8.1", "1f676c76-80e1-4239-95bb-83d0f6d0da78"),
        ("Windows 10/11", "8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a"),
    ] {
        assert!(
            MANIFEST.contains(&format!("{{{guid}}}")),
            "manifest is missing the supportedOS entry for {name}"
        );
    }
}

/// Returns the bytes of the resource directory of a PE image, if it has one.
#[cfg(windows)]
fn resource_directory(image: &[u8]) -> Option<&[u8]> {
    let pe_offset = u32_at(image, 0x3c) as usize;
    if &image[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return None;
    }

    let coff = pe_offset + 4;
    let num_sections = u16_at(image, coff + 2) as usize;
    let optional_size = u16_at(image, coff + 16) as usize;
    let optional = coff + 20;

    // The data directories start at a different offset for PE32 and PE32+.
    let data_dirs = match u16_at(image, optional) {
        0x20b => optional + 112,
        0x10b => optional + 96,
        _ => return None,
    };
    let rsrc_rva = u32_at(image, data_dirs + 2 * 8) as usize;
    let rsrc_size = u32_at(image, data_dirs + 2 * 8 + 4) as usize;
    if rsrc_rva == 0 || rsrc_size == 0 {
        return None;
    }

    for i in 0..num_sections {
        let section = optional + optional_size + 40 * i;
        let virtual_size = u32_at(image, section + 8) as usize;
        let virtual_address = u32_at(image, section + 12) as usize;
        let raw_size = u32_at(image, section + 16) as usize;
        let raw_offset = u32_at(image, section + 20) as usize;

        let span = virtual_size.max(raw_size);
        if (virtual_address..virtual_address + span).contains(&rsrc_rva) {
            let start = raw_offset + (rsrc_rva - virtual_address);
            let end = (start + rsrc_size).min(raw_offset + raw_size);
            return image.get(start..end);
        }
    }
    None
}

/// Search for a byte pattern, used to look for resource strings.
#[cfg(windows)]
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Search for a string, in either UTF-8 or UTF-16LE encoding.
///
/// Resource compilers are not consistent about the encoding they use for
/// manifest and string table entries, so accept both.
#[cfg(windows)]
fn contains_str(haystack: &[u8], needle: &str) -> bool {
    let utf16: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    contains(haystack, needle.as_bytes()) || contains(haystack, &utf16)
}

#[cfg(windows)]
#[test]
fn executable_has_embedded_resources() {
    // The build script links the resources into every target of this package,
    // so the test executable is representative of `sshx.exe` itself.
    let path = std::env::current_exe().expect("failed to locate test executable");
    let image = std::fs::read(&path).expect("failed to read test executable");

    let resources = resource_directory(&image)
        .unwrap_or_else(|| panic!("{} has no resource directory", path.display()));
    assert!(!resources.is_empty(), "resource directory is empty");

    assert!(
        contains_str(resources, "ProductName") && contains_str(resources, "FileDescription"),
        "executable is missing its version information"
    );
    assert!(
        contains_str(resources, "requestedExecutionLevel"),
        "executable is missing its application manifest"
    );
    assert!(
        contains(resources, b"\x89PNG"),
        "executable is missing its icon"
    );
}
