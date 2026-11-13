//! Local command receipts, not signed attestations. Never execute imported receipts.
use crate::checks::sha;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, String>;
const ROLES: [&str; 5] = ["macos-minimum", "macos-current", "ios", "linux", "clt"];
const STATUS_ARGS: &[&str] = &["status", "--porcelain", "--untracked-files=all"];
const BUILD_ARGS: &[&str] = &["build", "--locked", "--workspace", "--all-targets"];
const CLT_ARGS: &[&str] = &[
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
];

pub(super) fn linux_build_args(target: &str) -> Vec<&str> {
    let mut args = BUILD_ARGS.to_vec();
    args.extend(["--target", target]);
    args
}

fn linux_target(commands: &Value) -> Result<&str> {
    let arch = receipt(&commands["arch"], "uname", &["-m"])?.trim();
    let compiler = receipt(&commands["rustc"], "rustc", &["-vV"])?;
    require(
        compiler.starts_with("rustc ")
            && compiler.lines().any(|line| {
                line.strip_prefix("release: ")
                    .is_some_and(|v| !v.is_empty())
            }),
        "missing Rust toolchain identity",
    )?;
    let hosts: Vec<_> = compiler
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    require(hosts.len() == 1, "missing or ambiguous Rust host triple")?;
    let host = hosts[0];
    let parts: Vec<_> = host.split('-').collect();
    require(
        parts.len() >= 3
            && parts[2] == "linux"
            && parts[0] == if arch == "arm64" { "aarch64" } else { arch },
        "Rust target is not the native Linux host",
    )?;
    Ok(host)
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(format!("BLOCKED: native evidence: {message}"))
    }
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("BLOCKED: missing native evidence {key}"))
}
fn receipt<'a>(value: &'a Value, program: &str, args: &[&str]) -> Result<&'a str> {
    require(
        value["program"] == program && value["args"] == json!(args),
        "unexpected command or arguments",
    )?;
    require(
        value["exit"].as_i64() == Some(0),
        "command failed or exit status missing",
    )?;
    let stdout = string(value, "stdout")?;
    let stderr = string(value, "stderr")?;
    require(
        value["stdout_sha256"] == sha(stdout.as_bytes())
            && value["stderr_sha256"] == sha(stderr.as_bytes()),
        "raw output hash mismatch",
    )?;
    Ok(stdout)
}
fn keys(value: &Value, expected: &[&str]) -> Result<()> {
    let actual: BTreeSet<_> = value
        .as_object()
        .ok_or("BLOCKED: expected evidence object")?
        .keys()
        .map(String::as_str)
        .collect();
    require(
        actual == expected.iter().copied().collect(),
        "missing or unexpected evidence fields",
    )
}
fn destination(lock: &Value) -> Result<(&str, &str, &str)> {
    let text = lock["native_platform"]["qualified_ios_destination"]
        .as_str()
        .ok_or("BLOCKED: tools/versions.lock.json has no qualified iOS 18.x destination")?;
    for platform in ["iOS Simulator", "iOS"] {
        if let Some(id) = text.strip_prefix(&format!("platform={platform},id=")) {
            require(
                !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                "invalid locked iOS identifier",
            )?;
            return Ok((text, platform, id));
        }
    }
    Err("BLOCKED: qualified_ios_destination must be exactly platform=iOS Simulator,id=<qualified ID> or platform=iOS,id=<qualified ID>".into())
}
fn current_os(lock: &Value) -> Result<&str> {
    lock["native_platform"]["observed"]["runner_os"]
        .as_str()
        .and_then(|s| s.strip_prefix("macOS "))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "BLOCKED: missing current macOS inventory version".into())
}
fn snapshot_valid(snapshot: &Value, candidate: &str, tree: &str, linux: bool) -> Result<()> {
    require(
        receipt(&snapshot["head"], "git", &["rev-parse", "HEAD"])?.trim() == candidate,
        "stale or changed candidate",
    )?;
    require(
        receipt(&snapshot["tree"], "git", &["rev-parse", "HEAD^{tree}"])?.trim() == tree,
        "candidate tree mismatch",
    )?;
    let status = receipt(&snapshot["status"], "git", STATUS_ARGS)?;
    if linux {
        receipt(
            &snapshot["native_absent"],
            "/bin/test",
            &["!", "-e", "native"],
        )?;
        require(
            !status.is_empty()
                && status.lines().all(|line| {
                    line.strip_prefix(" D native/")
                        .is_some_and(|p| !p.is_empty() && !p.contains(".."))
                }),
            "Linux permits only unstaged native removals",
        )?;
    } else {
        require(status.is_empty(), "dirty candidate")?;
        let manifest = receipt(
            &snapshot["native_manifest"],
            "git",
            &["ls-tree", "-r", "--name-only", "HEAD", "--", "native"],
        )?;
        let names: Vec<_> = manifest.lines().collect();
        let files = snapshot["native_files"]
            .as_array()
            .ok_or("BLOCKED: missing native input hashes")?;
        require(
            !names.is_empty() && names.len() == files.len(),
            "incomplete native input manifest",
        )?;
        let mut seen = BTreeSet::new();
        for (name, file) in names.iter().zip(files) {
            require(
                name.starts_with("native/")
                    && !name.contains("..")
                    && seen.insert(*name)
                    && file["path"] == *name,
                "invalid or duplicated native input",
            )?;
            let source = receipt(&file["expected"], "git", &["show", &format!("HEAD:{name}")])?;
            require(
                file["actual_sha256"] == sha(source.as_bytes()),
                "modified native fixture bytes",
            )?;
        }
    }
    Ok(())
}
pub(super) fn native_names(report: &Value, ios: bool) -> Result<BTreeSet<String>> {
    fn visit(nodes: &[Value], names: &mut BTreeSet<String>) -> Result<()> {
        for node in nodes {
            if node["nodeType"] == "Test Case" {
                require(
                    node["result"] == "Passed",
                    "native assertion failed, skipped or ignored",
                )?;
                require(
                    names.insert(string(node, "name")?.to_owned()),
                    "duplicate native assertion",
                )?;
            }
            if let Some(children) = node.get("children") {
                visit(
                    children
                        .as_array()
                        .ok_or("BLOCKED: malformed native test children")?,
                    names,
                )?;
            }
        }
        Ok(())
    }
    let mut names = BTreeSet::new();
    visit(
        report["testNodes"]
            .as_array()
            .ok_or("BLOCKED: missing native testNodes")?,
        &mut names,
    )?;
    let mut expected: BTreeSet<String> = ["cur_004_4()", "cur_004_actor_c_roundtrip()"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if !ios {
        expected.insert("cur_004_1()".into());
    }
    require(
        names == expected,
        "zero, missing or unexpected native assertions",
    )?;
    Ok(names)
}
fn ios_18_runtime(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts[0] == "18"
        && parts.iter().all(|part| {
            !part.is_empty()
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

fn summary_valid(
    summary: &Value,
    count: usize,
    platform: &str,
    exact_macos_version: Option<&str>,
    id: Option<&str>,
    build: Option<&str>,
) -> Result<()> {
    require(
        summary["result"] == "Passed"
            && summary["totalTestCount"] == count
            && summary["passedTests"] == count,
        "native summary has zero or inconsistent test counts",
    )?;
    for key in ["failedTests", "skippedTests", "expectedFailures"] {
        require(
            summary[key].as_u64() == Some(0),
            "native summary contains failed/skipped/expected failures",
        )?;
    }
    require(
        summary["testFailures"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "native failure details missing or nonempty",
    )?;
    let devices = summary["devicesAndConfigurations"]
        .as_array()
        .ok_or("BLOCKED: missing native destination observation")?;
    require(devices.len() == 1, "ambiguous native destination")?;
    let run = &devices[0];
    require(
        run["passedTests"] == count,
        "destination test count differs",
    )?;
    for key in ["failedTests", "skippedTests", "expectedFailures"] {
        require(
            run[key].as_u64() == Some(0),
            "destination contains unsuccessful tests",
        )?;
    }
    let device = &run["device"];
    let actual_version = string(device, "osVersion")?;
    let version_matches = match exact_macos_version {
        Some(expected) => actual_version == expected,
        None => ios_18_runtime(actual_version),
    };
    require(
        device["platform"] == platform && device["architecture"] == "arm64" && version_matches,
        "wrong actual OS, version or architecture",
    )?;
    require(
        !string(device, "deviceId")?.is_empty() && !string(device, "osBuildNumber")?.is_empty(),
        "missing actual runtime identifier/build",
    )?;
    if let Some(id) = id {
        require(device["deviceId"] == id, "wrong qualified iOS destination")?;
    }
    if let Some(build) = build {
        require(
            device["osBuildNumber"] == build,
            "host/runtime build mismatch",
        )?;
    }
    Ok(())
}
fn apple_args<'a>(
    scheme: &'a str,
    plan: &'a str,
    dest: &'a str,
    configuration: &'a str,
    derived: &'a str,
    bundle: &'a str,
) -> Vec<&'a str> {
    vec![
        "test",
        "-project",
        "native/Currents.xcodeproj",
        "-scheme",
        scheme,
        "-testPlan",
        plan,
        "-destination",
        dest,
        "-configuration",
        configuration,
        "-derivedDataPath",
        derived,
        "-resultBundlePath",
        bundle,
        "CODE_SIGNING_ALLOWED=NO",
    ]
}
fn result_args<'a>(section: &'a str, bundle: &'a str) -> Vec<&'a str> {
    vec![
        "xcresulttool",
        "get",
        "test-results",
        section,
        "--path",
        bundle,
        "--compact",
    ]
}
fn apple_environment(commands: &Value, role: &str, lock: &Value) -> Result<()> {
    require(
        receipt(&commands["arch"], "uname", &["-m"])?.trim() == "arm64",
        "unsupported native host architecture",
    )?;
    let os = receipt(&commands["os"], "/usr/bin/sw_vers", &["-productVersion"])?.trim();
    if role != "ios" {
        require(
            os == if role == "macos-minimum" {
                "15.0"
            } else {
                current_os(lock)?
            },
            "wrong macOS host version",
        )?;
    }
    require(
        !receipt(
            &commands["os_build"],
            "/usr/bin/sw_vers",
            &["-buildVersion"],
        )?
        .trim()
        .is_empty(),
        "missing host build",
    )?;
    let xcode = receipt(&commands["xcode"], "/usr/bin/xcodebuild", &["-version"])?;
    let mut lines = xcode.lines();
    require(
        lines.next().is_some_and(|s| {
            s.strip_prefix("Xcode ").is_some_and(|v| {
                v.split('.')
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some_and(|v| v >= 16)
            })
        }) && lines.next().is_some_and(|s| {
            s.strip_prefix("Build version ")
                .is_some_and(|v| !v.is_empty())
        }),
        "missing full Xcode version/build",
    )?;
    let swift = receipt(
        &commands["swift"],
        "/usr/bin/xcrun",
        &["swift", "--version"],
    )?;
    let major = swift
        .split("Swift version ")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse::<u32>().ok());
    require(major.is_some_and(|n| n >= 6), "Swift 6 or newer required")?;
    let sdk = if role == "ios" && destination(lock)?.1 == "iOS Simulator" {
        "iphonesimulator"
    } else if role == "ios" {
        "iphoneos"
    } else {
        "macosx"
    };
    let sdk_version = receipt(
        &commands["sdk"],
        "/usr/bin/xcrun",
        &["--sdk", sdk, "--show-sdk-version"],
    )?
    .trim();
    require(
        !sdk_version.is_empty()
            && sdk_version
                .split('.')
                .all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())),
        "missing SDK version",
    )
}
fn evaluate_entry(entry: &Value, candidate: &str, tree: &str, lock: &Value) -> Result<Vec<Value>> {
    require(entry["schema_version"] == 1, "unsupported evidence version")?;
    let role = string(entry, "role")?;
    require(ROLES.contains(&role), "unknown matrix role")?;
    for phase in ["before", "after"] {
        snapshot_valid(&entry[phase], candidate, tree, role == "linux")?;
    }
    let c = &entry["commands"];
    let kernel = receipt(&c["kernel"], "uname", &["-s"])?.trim();
    require(
        kernel == if role == "linux" { "Linux" } else { "Darwin" },
        "wrong host OS",
    )?;
    if role == "linux" {
        keys(c, &["kernel", "arch", "rustc", "build"])?;
        let target = linux_target(c)?;
        receipt(&c["build"], "cargo", &linux_build_args(target))?;
        return Ok(vec![
            json!({"source":"native-evidence-receipt","role":role,"criterion":"cur_004_2","observation":"Linux locked workspace all-targets build with native unavailable","outcome":"Passed"}),
        ]);
    }
    if role == "clt" {
        keys(c, &["kernel", "clt", "test"])?;
        receipt(
            &c["clt"],
            "/bin/test",
            &["-d", "/Library/Developer/CommandLineTools"],
        )?;
        let out = receipt(&c["test"], "cargo", CLT_ARGS)?;
        require(
            out.lines()
                .filter(|line| *line == "test cur_004_3 ... ok")
                .count()
                == 1
                && out
                    .lines()
                    .filter(|line| {
                        line.starts_with(
                            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;",
                        )
                    })
                    .count()
                    == 1,
            "CLT counterexample missing, zero or unsuccessful",
        )?;
        return Ok(vec![
            json!({"source":"native-evidence-receipt","role":role,"test":{"name":"cur_004_3","outcome":"Passed"}}),
        ]);
    }
    keys(
        c,
        &["kernel", "os", "os_build", "arch", "xcode", "swift", "sdk"],
    )?;
    apple_environment(c, role, lock)?;
    let ios = role == "ios";
    let (dest, platform, id) = if ios {
        destination(lock)?
    } else {
        ("platform=macOS,arch=arm64", "macOS", "")
    };
    let (scheme, plan) = if ios {
        ("CurrentsCapture", "CurrentsCaptureSynthetic")
    } else {
        ("CurrentsPlatformTests", "CurrentsSynthetic")
    };
    let directory = string(entry, "directory")?;
    require(!directory.is_empty(), "missing native evidence directory")?;
    let runs = entry["runs"]
        .as_array()
        .ok_or("BLOCKED: missing configuration runs")?;
    require(runs.len() == 2, "Debug and Release observations required")?;
    let mut observations = Vec::new();
    for (configuration, run) in ["Debug", "Release"].into_iter().zip(runs) {
        require(
            run["configuration"] == configuration,
            "missing or duplicate native configuration",
        )?;
        let bundle = format!("{directory}/{configuration}.xcresult");
        let derived = format!("{directory}/DerivedData");
        receipt(
            &run["build"],
            "/usr/bin/xcodebuild",
            &apple_args(scheme, plan, dest, configuration, &derived, &bundle),
        )?;
        let tests: Value = serde_json::from_str(receipt(
            &run["tests"],
            "/usr/bin/xcrun",
            &result_args("tests", &bundle),
        )?)
        .map_err(|e| e.to_string())?;
        let summary: Value = serde_json::from_str(receipt(
            &run["summary"],
            "/usr/bin/xcrun",
            &result_args("summary", &bundle),
        )?)
        .map_err(|e| e.to_string())?;
        let names = native_names(&tests, ios)?;
        let exact_macos_version = (!ios).then_some(c["os"]["stdout"].as_str().unwrap_or("").trim());
        summary_valid(
            &summary,
            names.len(),
            platform,
            exact_macos_version,
            ios.then_some(id),
            (!ios).then_some(c["os_build"]["stdout"].as_str().unwrap_or("").trim()),
        )?;
        observations.extend(names.into_iter().map(|name| json!({"source":"native-evidence-receipt","role":role,"configuration":configuration,"test":{"name":name,"outcome":"Passed"},"runtime":summary["devicesAndConfigurations"][0]["device"],"tests_stdout_sha256":run["tests"]["stdout_sha256"]})));
    }
    Ok(observations)
}
pub(super) fn evaluate_matrix(
    candidate: &str,
    tree: &str,
    lock: &Value,
    evidence: &[Value],
) -> Result<Value> {
    require(
        candidate.len() == 40
            && candidate.bytes().all(|c| c.is_ascii_hexdigit())
            && tree.len() == 40
            && tree.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid candidate/tree identity",
    )?;
    destination(lock)?;
    require(
        evidence.len() == ROLES.len(),
        "exactly five matrix roles required",
    )?;
    let mut seen = BTreeSet::new();
    let mut observations = Vec::new();
    for entry in evidence {
        require(seen.insert(string(entry, "role")?), "duplicate matrix role")?;
        observations.extend(evaluate_entry(entry, candidate, tree, lock)?);
    }
    require(seen == ROLES.into_iter().collect(), "incomplete matrix")?;
    Ok(
        json!({"candidate_sha":candidate,"tree":tree,"scope":"CUR-004 synthetic native platform matrix only","observed_tests":observations,"production_status":"Blocked","evidence":evidence}),
    )
}

fn capture(root: &Path, program: &str, args: &[&str]) -> Result<Value> {
    // All callers supply checked-in command templates, never artifact contents.
    let out = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("BLOCKED: {program}: {e}"))?;
    crate::report::record(Path::new(program), args, root, None, &out);
    let stdout = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
    let stderr = String::from_utf8(out.stderr).map_err(|e| e.to_string())?;
    Ok(
        json!({"program":program,"args":args,"exit":out.status.code(),"stdout_sha256":sha(stdout.as_bytes()),"stderr_sha256":sha(stderr.as_bytes()),"stdout":stdout,"stderr":stderr}),
    )
}
fn snapshot(root: &Path, linux: bool) -> Result<Value> {
    let mut value = json!({"head":capture(root,"git", &["rev-parse","HEAD"] )?,"tree":capture(root,"git", &["rev-parse","HEAD^{tree}"] )?,"status":capture(root,"git", STATUS_ARGS)?});
    if linux {
        value["native_absent"] = capture(root, "/bin/test", &["!", "-e", "native"])?;
    } else {
        let manifest = capture(
            root,
            "git",
            &["ls-tree", "-r", "--name-only", "HEAD", "--", "native"],
        )?;
        let mut files = Vec::new();
        for name in receipt(
            &manifest,
            "git",
            &["ls-tree", "-r", "--name-only", "HEAD", "--", "native"],
        )?
        .lines()
        {
            require(
                name.starts_with("native/") && !name.contains(".."),
                "invalid native input path",
            )?;
            files.push(json!({"path":name,"expected":capture(root,"git", &["show", &format!("HEAD:{name}")])?,"actual_sha256":sha(&fs::read(root.join(name)).map_err(|e| e.to_string())?)}));
        }
        value["native_manifest"] = manifest;
        value["native_files"] = json!(files);
    }
    Ok(value)
}
fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("BLOCKED: {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("BLOCKED: {}: {e}", path.display()))
}
fn write_artifact(path: &Path, entry: &Value) -> Result<()> {
    fs::write(
        path,
        serde_json::to_vec_pretty(entry).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
pub(super) fn emit(root: &Path, role: &str) -> Result<PathBuf> {
    require(ROLES.contains(&role), "unknown emit role")?;
    let lock = read_json(&root.join("tools/versions.lock.json"))?;
    if role == "ios" {
        destination(&lock)?;
    }
    let before = snapshot(root, role == "linux")?;
    let candidate = receipt(&before["head"], "git", &["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let tree = receipt(&before["tree"], "git", &["rev-parse", "HEAD^{tree}"])?
        .trim()
        .to_owned();
    snapshot_valid(&before, &candidate, &tree, role == "linux")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = format!("target/qualification/native-{nonce}-{}", std::process::id());
    fs::create_dir_all(root.join("target/qualification")).map_err(|e| e.to_string())?;
    fs::create_dir(root.join(&directory)).map_err(|e| e.to_string())?;
    let path = root.join(&directory).join("evidence.json");
    let mut entry = json!({"schema_version":1,"role":role,"directory":directory,"before":before,"commands":{},"runs":[]});
    let outcome: Result<()> = (|| {
        entry["commands"]["kernel"] = capture(root, "uname", &["-s"])?;
        let kernel = receipt(&entry["commands"]["kernel"], "uname", &["-s"])?.trim();
        require(
            kernel == if role == "linux" { "Linux" } else { "Darwin" },
            "wrong collector host OS",
        )?;
        if role == "linux" {
            entry["commands"]["arch"] = capture(root, "uname", &["-m"])?;
            entry["commands"]["rustc"] = capture(root, "rustc", &["-vV"])?;
            let target = linux_target(&entry["commands"])?.to_owned();
            entry["commands"]["build"] = capture(root, "cargo", &linux_build_args(&target))?;
        } else if role == "clt" {
            entry["commands"]["clt"] = capture(
                root,
                "/bin/test",
                &["-d", "/Library/Developer/CommandLineTools"],
            )?;
            receipt(
                &entry["commands"]["clt"],
                "/bin/test",
                &["-d", "/Library/Developer/CommandLineTools"],
            )?;
            entry["commands"]["test"] = capture(root, "cargo", CLT_ARGS)?;
        } else {
            let c = &mut entry["commands"];
            for (key, program, args) in [
                ("os", "/usr/bin/sw_vers", vec!["-productVersion"]),
                ("os_build", "/usr/bin/sw_vers", vec!["-buildVersion"]),
                ("arch", "uname", vec!["-m"]),
                ("xcode", "/usr/bin/xcodebuild", vec!["-version"]),
                ("swift", "/usr/bin/xcrun", vec!["swift", "--version"]),
            ] {
                c[key] = capture(root, program, &args)?;
            }
            let ios = role == "ios";
            let (dest, platform, _) = if ios {
                destination(&lock)?
            } else {
                ("platform=macOS,arch=arm64", "macOS", "")
            };
            let sdk = if platform == "iOS Simulator" {
                "iphonesimulator"
            } else if ios {
                "iphoneos"
            } else {
                "macosx"
            };
            c["sdk"] = capture(
                root,
                "/usr/bin/xcrun",
                &["--sdk", sdk, "--show-sdk-version"],
            )?;
            apple_environment(c, role, &lock)?;
            let (scheme, plan) = if ios {
                ("CurrentsCapture", "CurrentsCaptureSynthetic")
            } else {
                ("CurrentsPlatformTests", "CurrentsSynthetic")
            };
            for configuration in ["Debug", "Release"] {
                let bundle = format!("{directory}/{configuration}.xcresult");
                let derived = format!("{directory}/DerivedData");
                let build = capture(
                    root,
                    "/usr/bin/xcodebuild",
                    &apple_args(scheme, plan, dest, configuration, &derived, &bundle),
                )?;
                let mut run = json!({"configuration":configuration,"build":build});
                let succeeded = run["build"]["exit"] == 0;
                if succeeded {
                    for section in ["tests", "summary"] {
                        run[section] =
                            capture(root, "/usr/bin/xcrun", &result_args(section, &bundle))?;
                    }
                }
                entry["runs"].as_array_mut().unwrap().push(run);
                require(succeeded, "native test command failed")?;
            }
        }
        entry["after"] = snapshot(root, role == "linux")?;
        evaluate_entry(&entry, &candidate, &tree, &lock)?;
        Ok(())
    })();
    // Retain failures as failures. Incomplete artifacts cannot pass aggregation.
    write_artifact(&path, &entry)?;
    outcome.map_err(|e| format!("{e}; raw evidence: {}", path.display()))?;
    Ok(path)
}
pub(super) fn command(root: &Path, args: &[String]) -> Result<Value> {
    match args {
        [flag, role] if flag == "--emit" => {
            let path = emit(root, role)?;
            Err(format!("BLOCKED: collected {role} evidence at {}; complete same-candidate matrix aggregation still required. Production qualification remains Blocked.", path.display()))
        }
        [flag, paths @ ..] if flag == "--aggregate" => {
            require(paths.len() == 5, "--aggregate requires exactly five evidence.json paths")?;
            let before = snapshot(root, false)?;
            let candidate = receipt(&before["head"], "git", &["rev-parse", "HEAD"])?.trim();
            let tree = receipt(&before["tree"], "git", &["rev-parse", "HEAD^{tree}"])?.trim();
            snapshot_valid(&before, candidate, tree, false)?;
            let lock = read_json(&root.join("tools/versions.lock.json"))?;
            let mut evidence = Vec::new();
            let mut inputs = Vec::new();
            for path in paths {
                let bytes = fs::read(path).map_err(|e| format!("BLOCKED: {path}: {e}"))?;
                inputs.push(json!({"path":path,"sha256":sha(&bytes)}));
                evidence.push(serde_json::from_slice(&bytes).map_err(|e| format!("BLOCKED: {path}: {e}"))?);
            }
            let mut result = evaluate_matrix(candidate, tree, &lock, &evidence)?;
            snapshot_valid(&snapshot(root, false)?, candidate, tree, false)?;
            result["input_artifacts"] = json!(inputs);
            Ok(result)
        }
        _ => Err("BLOCKED: use --emit ROLE or --aggregate <five evidence.json paths>; no production mode".into()),
    }
}
