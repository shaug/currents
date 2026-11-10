//! Native prerequisites are independent of a working CLT/Rust installation.
//! This partial gate intentionally cannot certify an untested Xcode/OS matrix.
use crate::checks::checked;
use serde_json::{Value, json};
use std::collections::BTreeSet;
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
        &["--sdk", "macosx", "--show-sdk-version"],
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
    if full_selected && arch.trim() == "arm64" && !xcode.is_empty() {
        let observation = current_host(root)?;
        return Err(format!(
            "BLOCKED: current-host native assertions passed; evidence: {}. cur_004_1 still requires actual macOS 15.0 arm64 plus current-OS evidence at the same candidate; cur_004_2 requires a same-candidate Linux core build with native unavailable; cur_004_3 is the separate CLT-only public counterexample; cur_004_4 still requires the qualified iOS 18.0 destination. Observed OS: {os}; Swift: {swift}; SDK: {sdk}; Xcode: {xcode}. No production or minimum-OS qualification is implied.",
            observation.display()
        ));
    }
    Err(format!(
        "BLOCKED: native-platform requires full Xcode with Swift 6, actual macOS 15.0 arm64 and current-OS build/launch evidence, and a qualified iOS 18.0 destination. Selected developer directory: {selected:?}; discovered full Xcode: {installations:?}; discovery scope: /Applications and ~/Applications (3 levels), selected developer directory, Spotlight. OS: {os}; architecture: {arch}; Swift: {swift}; selected SDK: {sdk}; selected full Xcode: {xcode}. A usable selected full Xcode, minimum-OS runner and qualified iOS destination are required. CLT Swift/C synthetic probes do not qualify a native app. No signing, entitlement acquisition or developer-directory setting was changed."
    ))
}

fn validate_native_tests(report: &Value) -> Result<(), String> {
    fn visit(nodes: &[Value], passed: &mut BTreeSet<String>) -> Result<(), String> {
        for node in nodes {
            if node["nodeType"] == "Test Case" {
                if node["result"] != "Passed" {
                    return Err(format!("native assertion did not pass: {node}"));
                }
                let name = node["name"]
                    .as_str()
                    .ok_or("native assertion has no name")?;
                if !passed.insert(name.to_owned()) {
                    return Err(format!("duplicate native assertion: {name}"));
                }
            }
            if let Some(children) = node["children"].as_array() {
                visit(children, passed)?;
            }
        }
        Ok(())
    }
    let nodes = report["testNodes"]
        .as_array()
        .ok_or("missing native testNodes")?;
    let mut passed = BTreeSet::new();
    visit(nodes, &mut passed)?;
    for required in ["cur_004_1()", "cur_004_4()"] {
        if !passed.contains(required) {
            return Err(format!("missing native assertion: {required}"));
        }
    }
    Ok(())
}

fn current_host(root: &Path) -> Result<PathBuf, String> {
    // Fresh result bundles prevent a previous invocation from supplying evidence.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = root.join(format!("target/qualification/native-{nonce}"));
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for configuration in ["Debug", "Release"] {
        let bundle = directory.join(format!("{configuration}.xcresult"));
        let bundle_arg = bundle.to_str().ok_or("non-UTF8 native evidence path")?;
        checked(
            Path::new("/usr/bin/xcodebuild"),
            &[
                "test",
                "-project",
                "native/Currents.xcodeproj",
                "-scheme",
                "CurrentsPlatformTests",
                "-testPlan",
                "CurrentsSynthetic",
                "-destination",
                "platform=macOS,arch=arm64",
                "-configuration",
                configuration,
                "-derivedDataPath",
                "target/cur-004-xcode/DerivedData",
                "-resultBundlePath",
                bundle_arg,
                "CODE_SIGNING_ALLOWED=NO",
            ],
            root,
            None,
        )?;
        let text = checked(
            Path::new("/usr/bin/xcrun"),
            &[
                "xcresulttool",
                "get",
                "test-results",
                "tests",
                "--path",
                bundle_arg,
                "--compact",
            ],
            root,
            None,
        )?;
        let report: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        validate_native_tests(&report)?;
        results.push(json!({"configuration":configuration,"result_bundle":bundle,"tests":report}));
    }
    let path = directory.join("current-host.json");
    let head = checked(Path::new("git"), &["rev-parse", "HEAD"], root, None)?;
    let dirty = checked(Path::new("git"), &["status", "--porcelain"], root, None)?;
    let artifact = json!({"candidate_sha":head.trim(),"dirty":!dirty.is_empty(),
        "assertion_registry":["cur_004_1","cur_004_2","cur_004_3","cur_004_4"],
        "scope":"current-host synthetic bundle launch only; full native qualification remains blocked",
        "observations":results});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&artifact).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_result_registry_requires_observed_passes() {
        let report = json!({"testNodes":[{"nodeType":"Test Plan", "children":[
            {"nodeType":"Test Case", "name":"cur_004_1()", "result":"Passed"},
            {"nodeType":"Test Case", "name":"cur_004_4()", "result":"Passed"}
        ]}]});
        assert!(validate_native_tests(&report).is_ok());
        assert!(validate_native_tests(&json!({"testNodes":[]})).is_err());
        for outcome in ["Skipped", "Failed", "Expected Failure", "Unknown"] {
            let mut invalid = report.clone();
            invalid["testNodes"][0]["children"][1]["result"] = json!(outcome);
            assert!(validate_native_tests(&invalid).is_err());
        }
        let mut missing = report.clone();
        missing["testNodes"][0]["children"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(validate_native_tests(&missing).is_err());
    }
}
