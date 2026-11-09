//! C16 qualification artifacts. Evidence belongs to one invocation, not the shared log.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, env, fs, path::Path, process::Output};

thread_local! {
    static COMMANDS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn record(program: &Path, args: &[&str], cwd: &Path, rustc: Option<&Path>, out: &Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let tests: Vec<_> = stdout
        .lines()
        .filter_map(|line| {
            let (name, outcome) = line.strip_prefix("test ")?.rsplit_once(" ... ")?;
            matches!(outcome, "ok" | "FAILED" | "ignored")
                .then(|| json!({"name":name,"outcome":outcome}))
        })
        .collect();
    COMMANDS.with(|commands| commands.borrow_mut().push(json!({
        "program":program,"args":args,"cwd":cwd,"rustc":rustc,"exit":out.status.code(),
        "stdout_sha256":digest(&out.stdout),"stderr_sha256":digest(&out.stderr),
        "tests":tests,
        "version_output":if args.contains(&"--version") || args.contains(&"-vV") {Some(stdout.to_string())} else {None}
    })));
}
pub fn build(
    root: &Path,
    args: &[String],
    head: &str,
    dirty: bool,
    result: &Result<Value, String>,
) -> Value {
    let commands = COMMANDS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let tests: Vec<_> =
        commands
            .iter()
            .flat_map(|c| {
                c["tests"].as_array().into_iter().flatten().map(
                    move |test| json!({"command":c["args"],"program":c["program"],"test":test}),
                )
            })
            .collect();
    let versions: Vec<_> = commands.iter().filter(|c| !c["version_output"].is_null()).map(|c| json!({"program":c["program"],"args":c["args"],"output":c["version_output"],"exit":c["exit"]})).collect();
    let mut paths = vec![
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
        root.join("rust-toolchain.toml"),
        root.join("tools/versions.lock.json"),
        root.join("vendor/libsqlite3-sys/CURRENTS-PROVENANCE.json"),
        root.join("vendor/libsqlite3-sys/build.rs"),
    ];
    for flag in ["--manifest-path", "--inventory"] {
        if let Some(i) = args.iter().position(|a| a == flag)
            && let Some(path) = args.get(i + 1)
        {
            paths.push(path.into());
            if flag == "--manifest-path" {
                paths.push(Path::new(path).with_file_name("Cargo.lock"));
            }
        }
    }
    let inputs: Vec<_> = paths
        .into_iter()
        .map(|path| match fs::read(&path) {
            Ok(bytes) => json!({"path":path,"sha256":digest(&bytes)}),
            Err(error) => json!({"path":path,"unavailable":error.to_string()}),
        })
        .collect();
    json!({
        "schema":"QualificationReportV1","schema_version":1,
        "gate":args.iter().take(2).cloned().collect::<Vec<_>>().join(" "),
        "inputs":{"command":args,"candidate_sha":head.trim(),"dirty":dirty,"files":inputs,"unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()},
        "platform":{"os":env::consts::OS,"arch":env::consts::ARCH,"target_override":env::var("CARGO_BUILD_TARGET").ok()},
        "versions":versions,"tests":tests,
        "evidence":{"result":result.as_ref().ok(),"error":result.as_ref().err(),"commands":commands},
        "status":if result.is_ok() {"Passed"} else {"Blocked"},
        "limitations":["Synthetic workspace only; no financial, signed-native or production qualification.","A pass applies only to the named gate and recorded host.","Empty tests means this command executed no test functions. Failed and unavailable prerequisites are Blocked under C16; evidence.error distinguishes them."]
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c16_report_fields_and_statuses() {
        let args = vec!["qualify".into(), "workspace".into()];
        for result in [
            Ok(json!({})),
            Err("failed assertion".into()),
            Err("BLOCKED: missing prerequisite".into()),
        ] {
            let report = build(
                Path::new("/nonexistent-currents-fixture"),
                &args,
                "synthetic-sha",
                false,
                &result,
            );
            let fields: std::collections::BTreeSet<_> = report
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                fields,
                [
                    "schema",
                    "schema_version",
                    "gate",
                    "inputs",
                    "platform",
                    "versions",
                    "tests",
                    "evidence",
                    "status",
                    "limitations"
                ]
                .into_iter()
                .collect()
            );
            assert_eq!(
                report["status"],
                if result.is_ok() { "Passed" } else { "Blocked" }
            );
            let schema: Value =
                serde_json::from_str(include_str!("../../docs/qualification-report.schema.json"))
                    .unwrap();
            let required: std::collections::BTreeSet<_> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(fields, required);
            assert_eq!(report["gate"], "qualify workspace");
            assert!(report["tests"].as_array().unwrap().is_empty());
            assert!(report["inputs"]["files"][0]["unavailable"].is_string());
        }
    }
    #[test]
    fn command_evidence_retains_test_names_and_output_digests() {
        use std::os::unix::process::ExitStatusExt;
        let out = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"test cur_001_1 ... ok\ntest native_version ... FAILED\n".to_vec(),
            stderr: b"diagnostic".to_vec(),
        };
        record(Path::new("cargo"), &["test"], Path::new("."), None, &out);
        let report = build(
            Path::new("."),
            &["check".into(), "core".into()],
            "sha",
            false,
            &Ok(json!({})),
        );
        assert_eq!(report["tests"][0]["test"]["name"], "cur_001_1");
        assert_eq!(report["tests"][1]["test"]["outcome"], "FAILED");
        assert_eq!(
            report["evidence"]["commands"][0]["stderr_sha256"],
            digest(b"diagnostic")
        );
        assert!(
            build(Path::new("."), &[], "sha", false, &Ok(json!({})))["tests"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
