//! Native prerequisites are independent of a working CLT/Rust installation.
//! This partial gate intentionally cannot certify an untested Xcode/OS matrix.
use crate::checks::checked;
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn discover(directory: &Path, depth: u8, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("Contents/Developer/usr/bin/xcodebuild").is_file() {
            found.push(path);
        } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            discover(&path, depth - 1, found);
        }
    }
}

pub fn qualify(root: &Path) -> Result<Value, String> {
    let mut installations = Vec::new();
    discover(Path::new("/Applications"), 3, &mut installations);
    if let Some(home) = env::var_os("HOME") {
        discover(
            &PathBuf::from(home).join("Applications"),
            3,
            &mut installations,
        );
    }
    let selected = env::var("DEVELOPER_DIR")
        .ok()
        .or_else(|| checked(Path::new("/usr/bin/xcode-select"), &["-p"], root, None).ok());
    if let Some(path) = &selected {
        let developer = Path::new(path.trim());
        if developer.join("usr/bin/xcodebuild").is_file() {
            installations.push(developer.to_path_buf());
        }
    }
    // Spotlight may find full Xcode outside the conventional application folders.
    let spotlight = checked(
        Path::new("/usr/bin/mdfind"),
        &["kMDItemCFBundleIdentifier == 'com.apple.dt.Xcode'"],
        root,
        None,
    )
    .ok();
    for path in spotlight.as_deref().unwrap_or("").lines() {
        let path = PathBuf::from(path);
        if path.join("Contents/Developer/usr/bin/xcodebuild").is_file() {
            installations.push(path);
        }
    }
    installations.sort();
    installations.dedup();
    let os = checked(Path::new("/usr/bin/sw_vers"), &[], root, None).unwrap_or_default();
    let arch = checked(Path::new("/usr/bin/uname"), &["-m"], root, None).unwrap_or_default();
    let swift =
        checked(Path::new("/usr/bin/swift"), &["--version"], root, None).unwrap_or_default();
    let sdk = checked(
        Path::new("/usr/bin/xcrun"),
        &["--show-sdk-version"],
        root,
        None,
    )
    .unwrap_or_default();
    let full_selected = selected
        .as_ref()
        .is_some_and(|p| Path::new(p.trim()).join("usr/bin/xcodebuild").is_file());
    let xcode = if full_selected {
        checked(Path::new("/usr/bin/xcodebuild"), &["-version"], root, None).unwrap_or_default()
    } else {
        String::new()
    };
    Err(format!(
        "BLOCKED: native-platform requires full Xcode with Swift 6, actual macOS 15.0 arm64 and current-OS build/launch evidence, and a qualified iOS 18.0 destination. Selected developer directory: {selected:?}; discovered full Xcode: {installations:?}; discovery scope: /Applications and ~/Applications (3 levels), selected developer directory, Spotlight. OS: {os}; architecture: {arch}; Swift: {swift}; selected SDK: {sdk}; selected full Xcode: {xcode}. The Xcode project/scheme matrix, minimum-OS runner and qualified iOS destination are not yet established. CLT Swift/C synthetic probes do not qualify a native app. No signing, entitlement acquisition or developer-directory setting was changed."
    ))
}
