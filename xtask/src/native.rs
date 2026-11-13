//! Native prerequisites are independent of a working CLT/Rust installation.
//! This partial gate intentionally cannot certify an untested Xcode/OS matrix.
mod evidence;
use crate::checks::checked;
#[cfg(test)]
use evidence::evaluate_matrix;
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

fn selected_developer(path: &Path) -> Option<PathBuf> {
    // Apple accepts either the application bundle or Contents/Developer.
    let nested = path.join("Contents/Developer");
    [path, nested.as_path()]
        .into_iter()
        .find(|candidate| candidate.join("usr/bin/xcodebuild").is_file())
        .map(Path::to_path_buf)
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
    if let Some(path) = &selected
        && let Some(developer) = selected_developer(Path::new(path.trim()))
    {
        installations.push(developer);
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
        .is_some_and(|p| selected_developer(Path::new(p.trim())).is_some());
    let xcode = if full_selected {
        checked(Path::new("/usr/bin/xcodebuild"), &["-version"], root, None).unwrap_or_default()
    } else {
        String::new()
    };
    if full_selected && arch.trim() == "arm64" && !xcode.is_empty() {
        let observation = current_host(root)?;
        return Err(format!(
            "BLOCKED: current-host native assertions passed; evidence: {}. cur_004_1 still requires actual macOS 15.0 arm64 plus current-OS evidence at the same candidate; cur_004_2 requires a same-candidate Linux core build with native unavailable; cur_004_3 is the separate CLT-only public counterexample; cur_004_4 still requires the qualified iOS 18.x destination. Observed OS: {os}; Swift: {swift}; SDK: {sdk}; Xcode: {xcode}. No production or minimum-OS qualification is implied.",
            observation.display()
        ));
    }
    Err(format!(
        "BLOCKED: native-platform requires full Xcode with Swift 6, actual macOS 15.0 arm64 and current-OS build/launch evidence, and a qualified iOS 18.x destination. Selected developer directory: {selected:?}; discovered full Xcode: {installations:?}; discovery scope: /Applications and ~/Applications (3 levels), selected developer directory, Spotlight. OS: {os}; architecture: {arch}; Swift: {swift}; selected SDK: {sdk}; selected full Xcode: {xcode}. A usable selected full Xcode, minimum-OS runner and qualified iOS destination are required. CLT Swift/C synthetic probes do not qualify a native app. No signing, entitlement acquisition or developer-directory setting was changed."
    ))
}

fn current_host(root: &Path) -> Result<PathBuf, String> {
    evidence::emit(root, "macos-current")
}

pub fn command(root: &Path, args: &[String]) -> Result<Value, String> {
    if args.is_empty() {
        qualify(root)
    } else {
        evidence::command(root, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_both_supported_developer_directory_forms() {
        let root =
            std::env::temp_dir().join(format!("currents-xcode-selection-{}", std::process::id()));
        let app = root.join("Xcode.app");
        let developer = app.join("Contents/Developer");
        fs::create_dir_all(developer.join("usr/bin")).unwrap();
        fs::write(developer.join("usr/bin/xcodebuild"), b"synthetic marker").unwrap();
        let app_result = selected_developer(&app);
        let developer_result = selected_developer(&developer);
        let missing_result = selected_developer(&root.join("CLT"));
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(app_result, Some(developer.clone()));
        assert_eq!(developer_result, Some(developer));
        assert_eq!(missing_result, None);
    }

    #[test]
    fn native_result_registry_requires_observed_passes() {
        let report = json!({"testNodes":[{"nodeType":"Test Plan", "children":[
            {"nodeType":"Test Case", "name":"cur_004_1()", "result":"Passed"},
            {"nodeType":"Test Case", "name":"cur_004_4()", "result":"Passed"},
            {"nodeType":"Test Case", "name":"cur_004_actor_c_roundtrip()", "result":"Passed"}
        ]}]});
        assert!(evidence::native_names(&report, false).is_ok());
        assert!(evidence::native_names(&json!({"testNodes":[]}), false).is_err());
        for outcome in ["Skipped", "Failed", "Expected Failure", "Unknown"] {
            let mut invalid = report.clone();
            invalid["testNodes"][0]["children"][1]["result"] = json!(outcome);
            assert!(evidence::native_names(&invalid, false).is_err());
        }
        let mut missing = report.clone();
        missing["testNodes"][0]["children"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(evidence::native_names(&missing, false).is_err());
    }
    fn receipt(program: &str, args: &[&str], stdout: &str) -> Value {
        json!({"program":program,"args":args,"exit":0,"stdout":stdout,"stderr":"",
            "stdout_sha256":crate::checks::sha(stdout.as_bytes()),"stderr_sha256":crate::checks::sha(b"")})
    }

    fn synthetic_matrix() -> (String, String, Value, Vec<Value>) {
        let candidate = "a".repeat(40);
        let tree = "b".repeat(40);
        let destination = "platform=iOS Simulator,id=UNIT-TEST-ONLY";
        let lock = json!({"native_platform":{"observed":{"runner_os":"macOS 27.0.1"},"qualified_ios_destination":destination}});
        let snapshot = json!({
            "head":receipt("git", &["rev-parse","HEAD"], &candidate),
            "tree":receipt("git", &["rev-parse","HEAD^{tree}"], &tree),
            "status":receipt("git", &["status","--porcelain","--untracked-files=all"], ""),
            "native_manifest":receipt("git", &["ls-tree","-r","--name-only","HEAD","--","native"], "native/fixture.swift\n"),
            "native_files":[{"path":"native/fixture.swift", "expected":receipt("git", &["show","HEAD:native/fixture.swift"], "fixture"), "actual_sha256":crate::checks::sha(b"fixture")} ]});
        let mut evidence = Vec::new();
        for role in ["macos-minimum", "macos-current", "ios", "linux", "clt"] {
            let mut e = json!({"schema_version":1,"role":role,"directory":"target/qualification/unit-test-only",
                "before":snapshot,"after":snapshot,"commands":{},"runs":[]});
            let c = &mut e["commands"];
            c["kernel"] = receipt(
                "uname",
                &["-s"],
                if role == "linux" {
                    "Linux\n"
                } else {
                    "Darwin\n"
                },
            );
            if role == "linux" {
                for phase in ["before", "after"] {
                    e[phase]["status"] = receipt(
                        "git",
                        &["status", "--porcelain", "--untracked-files=all"],
                        " D native/fixture.swift\n",
                    );
                    e[phase]["native_absent"] = receipt("/bin/test", &["!", "-e", "native"], "");
                }
                e["commands"]["arch"] = receipt("uname", &["-m"], "x86_64\n");
                e["commands"]["rustc"] = receipt(
                    "rustc",
                    &["-vV"],
                    "rustc 1.99.0\nhost: x86_64-unknown-linux-gnu\nrelease: 1.99.0\n",
                );
                e["commands"]["build"] = receipt(
                    "cargo",
                    &[
                        "build",
                        "--locked",
                        "--workspace",
                        "--all-targets",
                        "--target",
                        "x86_64-unknown-linux-gnu",
                    ],
                    "",
                );
            } else if role == "clt" {
                c["clt"] = receipt(
                    "/bin/test",
                    &["-d", "/Library/Developer/CommandLineTools"],
                    "",
                );
                c["test"] = receipt(
                    "cargo",
                    &[
                        "test",
                        "--locked",
                        "-p",
                        "currents-ffi",
                        "--test",
                        "cur_004",
                        "cur_004_3",
                        "--",
                        "--exact",
                        "--nocapture",
                    ],
                    "test cur_004_3 ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out\n",
                );
            } else {
                let os = if role == "macos-minimum" {
                    "15.0"
                } else {
                    "27.0.1"
                };
                c["os"] = receipt("/usr/bin/sw_vers", &["-productVersion"], os);
                c["os_build"] = receipt("/usr/bin/sw_vers", &["-buildVersion"], "UNIT-TEST-BUILD");
                c["arch"] = receipt("uname", &["-m"], "arm64\n");
                c["xcode"] = receipt(
                    "/usr/bin/xcodebuild",
                    &["-version"],
                    "Xcode 27.2\nBuild version 27B52\n",
                );
                c["swift"] = receipt(
                    "/usr/bin/xcrun",
                    &["swift", "--version"],
                    "Apple Swift version 6.3\n",
                );
                let sdk = if role == "ios" {
                    "iphonesimulator"
                } else {
                    "macosx"
                };
                c["sdk"] = receipt(
                    "/usr/bin/xcrun",
                    &["--sdk", sdk, "--show-sdk-version"],
                    "27.2\n",
                );
                for configuration in ["Debug", "Release"] {
                    let bundle =
                        format!("target/qualification/unit-test-only/{configuration}.xcresult");
                    let dest = if role == "ios" {
                        destination
                    } else {
                        "platform=macOS,arch=arm64"
                    };
                    let (scheme, plan) = if role == "ios" {
                        ("CurrentsCapture", "CurrentsCaptureSynthetic")
                    } else {
                        ("CurrentsPlatformTests", "CurrentsSynthetic")
                    };
                    let names = if role == "ios" {
                        vec!["cur_004_4()", "cur_004_actor_c_roundtrip()"]
                    } else {
                        vec!["cur_004_1()", "cur_004_4()", "cur_004_actor_c_roundtrip()"]
                    };
                    let count = names.len();
                    let tests = json!({"testNodes":names.iter().map(|name| json!({"nodeType":"Test Case","name":name,"result":"Passed"})).collect::<Vec<_>>()});
                    let summary = json!({"result":"Passed","totalTestCount":count,"passedTests":count,"failedTests":0,"skippedTests":0,"expectedFailures":0,"testFailures":[],"devicesAndConfigurations":[{"passedTests":count,"failedTests":0,"skippedTests":0,"expectedFailures":0,"device":{"architecture":"arm64","deviceId":if role == "ios" {"UNIT-TEST-ONLY"} else {"MAC-UNIT-TEST"},"osVersion":if role == "ios" {"18.0"} else {os},"osBuildNumber":"UNIT-TEST-BUILD","platform":if role == "ios" {"iOS Simulator"} else {"macOS"}}}]});
                    e["runs"].as_array_mut().unwrap().push(json!({"configuration":configuration,
                        "build":receipt("/usr/bin/xcodebuild", &["test","-project","native/Currents.xcodeproj","-scheme",scheme,"-testPlan",plan,"-destination",dest,"-configuration",configuration,"-derivedDataPath","target/qualification/unit-test-only/DerivedData","-resultBundlePath",&bundle,"CODE_SIGNING_ALLOWED=NO"], "** TEST SUCCEEDED **\n"),
                        "tests":receipt("/usr/bin/xcrun", &["xcresulttool","get","test-results","tests","--path",&bundle,"--compact"], &tests.to_string()),
                        "summary":receipt("/usr/bin/xcrun", &["xcresulttool","get","test-results","summary","--path",&bundle,"--compact"], &summary.to_string())}));
                }
            }
            evidence.push(e);
        }
        (candidate, tree, lock, evidence)
    }

    // Synthetic evaluator fixtures only. These do not run or qualify Apple targets.
    #[test]
    fn matrix_accepts_complete_candidate_bound_command_evidence() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        let result = evaluate_matrix(&candidate, &tree, &lock, &evidence).unwrap();
        assert_eq!(result["production_status"], "Blocked");
        assert_eq!(result["observed_tests"].as_array().unwrap().len(), 18);
        // Minimum-host Xcode need not match the newer current-host toolchain.
        let mut different_xcode = evidence.clone();
        different_xcode[0]["commands"]["xcode"]["stdout"] =
            json!("Xcode 16.2\nBuild version 16C5032a\n");
        different_xcode[0]["commands"]["sdk"]["stdout"] = json!("15.2\n");
        different_xcode.iter_mut().for_each(rehash_receipts);
        assert!(evaluate_matrix(&candidate, &tree, &lock, &different_xcode).is_ok());
    }

    #[test]
    fn matrix_rejects_stale_dirty_missing_and_wrong_environment() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        for (pointer, value) in [
            ("/schema_version", json!(999)),
            ("/before/head/stdout", json!("old")),
            ("/after/head/stdout", json!("changed")),
            ("/before/status/stdout", json!(" M xtask/src/native.rs\n")),
            ("/after/tree/stdout", json!("wrong tree")),
            ("/commands/arch/stdout", json!("x86_64\n")),
            ("/commands/os/stdout", json!("15.8\n")),
            ("/commands/xcode/exit", json!(1)),
        ] {
            let mut bad = evidence.clone();
            *bad[0].pointer_mut(pointer).unwrap() = value;
            assert!(
                evaluate_matrix(&candidate, &tree, &lock, &bad).is_err(),
                "{pointer}"
            );
        }
        assert!(evaluate_matrix(&candidate, &tree, &lock, &evidence[..4]).is_err());
        let mut duplicate = evidence.clone();
        duplicate[4] = evidence[0].clone();
        assert!(evaluate_matrix(&candidate, &tree, &lock, &duplicate).is_err());
        assert!(
            evaluate_matrix(
                &candidate,
                &tree,
                &json!({"native_platform":{"qualified_ios_destination":null}}),
                &evidence
            )
            .is_err()
        );
    }

    fn rehash_receipts(value: &mut Value) {
        if value.get("stdout_sha256").is_some() {
            value["stdout_sha256"] = json!(crate::checks::sha(
                value["stdout"].as_str().unwrap().as_bytes()
            ));
            value["stderr_sha256"] = json!(crate::checks::sha(
                value["stderr"].as_str().unwrap().as_bytes()
            ));
        }
        match value {
            Value::Object(map) => map.values_mut().for_each(rehash_receipts),
            Value::Array(items) => items.iter_mut().for_each(rehash_receipts),
            _ => {}
        }
    }

    #[test]
    fn matrix_rejects_wrong_receipts_even_with_valid_hashes() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        for (index, pointer, value) in [
            (0, "/before/head/stdout", json!("stale")),
            (0, "/after/head/stdout", json!("changed")),
            (0, "/after/tree/stdout", json!("c".repeat(40))),
            (0, "/before/native_files/0/actual_sha256", json!("wrong")),
            (0, "/after/status/stdout", json!(" M Cargo.toml\n")),
            (0, "/commands/os/stdout", json!("15.8")),
            (0, "/commands/arch/stdout", json!("x86_64")),
            (
                0,
                "/commands/swift/stdout",
                json!("Apple Swift version 5.9"),
            ),
            (1, "/commands/os/stdout", json!("15.0")),
            (3, "/before/status/stdout", json!(" D Cargo.toml\n")),
            (3, "/commands/kernel/stdout", json!("Darwin")),
            (3, "/commands/build/args/0", json!("check")),
            (3, "/commands/build/exit", json!(101)),
            (3, "/commands/build/args/5", json!("aarch64-apple-darwin")),
            (
                3,
                "/commands/rustc/stdout",
                json!("rustc 1.99.0\nhost: x86_64-apple-darwin\nrelease: 1.99.0\n"),
            ),
            (3, "/commands/arch/stdout", json!("aarch64")),
            (
                4,
                "/commands/test/stdout",
                json!("test result: ok. 0 passed; 0 failed; 0 ignored"),
            ),
        ] {
            let mut bad = evidence.clone();
            *bad[index].pointer_mut(pointer).unwrap() = value;
            bad.iter_mut().for_each(rehash_receipts);
            assert!(
                evaluate_matrix(&candidate, &tree, &lock, &bad).is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn matrix_rejects_zero_skipped_duplicate_and_wrong_runtime() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        for (index, section, pointer, value) in [
            (0, "summary", "", json!({})),
            (0, "summary", "/testFailures", Value::Null),
            (0, "summary", "/totalTestCount", json!(0)),
            (0, "summary", "/skippedTests", json!(1)),
            (0, "summary", "/failedTests", json!(1)),
            (
                0,
                "summary",
                "/devicesAndConfigurations/0/device/osVersion",
                json!("15.1"),
            ),
            (
                0,
                "summary",
                "/devicesAndConfigurations/0/device/platform",
                json!("iOS Simulator"),
            ),
            (
                2,
                "summary",
                "/devicesAndConfigurations/0/device/osVersion",
                json!("17.6"),
            ),
            (
                2,
                "summary",
                "/devicesAndConfigurations/0/device/deviceId",
                json!("OTHER"),
            ),
            (
                2,
                "summary",
                "/devicesAndConfigurations/0/device/architecture",
                json!("x86_64"),
            ),
            (0, "tests", "/testNodes/0/result", json!("Skipped")),
            (0, "tests", "/testNodes/1/name", json!("cur_004_1()")),
            (0, "tests", "/testNodes", json!([])),
        ] {
            let mut bad = evidence.clone();
            let mut report: Value =
                serde_json::from_str(bad[index]["runs"][0][section]["stdout"].as_str().unwrap())
                    .unwrap();
            *report.pointer_mut(pointer).unwrap() = value;
            bad[index]["runs"][0][section]["stdout"] = json!(report.to_string());
            bad.iter_mut().for_each(rehash_receipts);
            assert!(
                evaluate_matrix(&candidate, &tree, &lock, &bad).is_err(),
                "{index}/{section}{pointer}"
            );
        }
        let mut malformed = evidence.clone();
        malformed[0]["runs"][0]["summary"]["stdout"] = json!("not JSON");
        malformed.iter_mut().for_each(rehash_receipts);
        assert!(evaluate_matrix(&candidate, &tree, &lock, &malformed).is_err());
        let mut bad = evidence.clone();
        bad[0]["runs"][1] = bad[0]["runs"][0].clone();
        assert!(evaluate_matrix(&candidate, &tree, &lock, &bad).is_err());
    }

    fn with_ios_runtime(evidence: &[Value], version: &str) -> Vec<Value> {
        let mut evidence = evidence.to_vec();
        for run in evidence[2]["runs"].as_array_mut().unwrap() {
            let mut summary: Value =
                serde_json::from_str(run["summary"]["stdout"].as_str().unwrap()).unwrap();
            summary["devicesAndConfigurations"][0]["device"]["osVersion"] = json!(version);
            run["summary"]["stdout"] = json!(summary.to_string());
        }
        evidence.iter_mut().for_each(rehash_receipts);
        evidence
    }

    #[test]
    fn matrix_accepts_ios_18_minor_and_patch_runtimes_and_records_actual_version() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        for version in ["18.0", "18.0.1", "18.5", "18.5.1", "18.6", "18.6.2"] {
            let result = evaluate_matrix(
                &candidate,
                &tree,
                &lock,
                &with_ios_runtime(&evidence, version),
            )
            .unwrap_or_else(|error| panic!("{version}: {error}"));
            let observations: Vec<_> = result["observed_tests"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|test| test["role"] == "ios")
                .collect();
            assert_eq!(observations.len(), 4);
            assert!(
                observations
                    .iter()
                    .all(|test| test["runtime"]["osVersion"] == version)
            );
            assert_eq!(result["production_status"], "Blocked");
        }
    }

    #[test]
    fn matrix_rejects_ios_below_floor_or_malformed_runtime() {
        let (candidate, tree, lock, evidence) = synthetic_matrix();
        for version in [
            "17.0",
            "17.6.1",
            "19.0",
            "",
            "18",
            "18.",
            "18..5",
            "18.5.",
            "18.5.1.2",
            "18.5beta",
            "18.-1",
            "18.+5",
            "18.05",
            "018.5",
            " 18.5",
            "18.5 ",
            "18.4294967296",
        ] {
            assert!(
                evaluate_matrix(
                    &candidate,
                    &tree,
                    &lock,
                    &with_ios_runtime(&evidence, version)
                )
                .is_err(),
                "{version}"
            );
        }
    }

    #[test]
    fn aggregate_report_keeps_observed_assertions_distinct_from_executed_commands() {
        let result = Ok(
            json!({"observed_tests":[{"source":"native-evidence-receipt","role":"macos-minimum","test":{"name":"cur_004_1()","outcome":"Passed"}}]}),
        );
        let report = crate::report::build(
            Path::new("/nonexistent"),
            &[
                "qualify".into(),
                "native-platform".into(),
                "--aggregate".into(),
            ],
            "sha",
            false,
            &result,
        );
        assert_eq!(report["tests"][0]["source"], "native-evidence-receipt");
        assert_eq!(report["tests"][0]["test"]["name"], "cur_004_1()");
        assert!(
            report["evidence"]["commands"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn native_evidence_cli_rejects_incomplete_or_production_modes() {
        for args in [
            vec!["--aggregate"],
            vec!["--aggregate", "one.json"],
            vec!["--emit", "production"],
            vec!["--emit", "ios", "--allow-missing"],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert!(evidence::command(Path::new("/nonexistent"), &args).is_err());
        }
    }

    #[test]
    fn linux_build_explicitly_targets_measured_host() {
        // Explicit Cargo --target takes precedence over CARGO_BUILD_TARGET and config.
        assert_eq!(
            evidence::linux_build_args("x86_64-unknown-linux-gnu"),
            [
                "build",
                "--locked",
                "--workspace",
                "--all-targets",
                "--target",
                "x86_64-unknown-linux-gnu"
            ]
        );
    }
}
