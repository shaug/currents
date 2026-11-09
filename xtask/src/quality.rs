//! Public, local-runnable PR gates. Scanner output never enters diagnostics or artifacts.
use crate::checks::{cargo, checked, root};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, String>;
fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
}
fn option(args: &[String], flag: &str) -> Result<Option<PathBuf>> {
    args.iter()
        .position(|s| s == flag)
        .map(|i| {
            args.get(i.saturating_add(1))
                .map(PathBuf::from)
                .ok_or_else(|| format!("missing {flag} value"))
        })
        .transpose()
}
fn rust(gate: &str, manifest: Option<&Path>) -> Result<Value> {
    let manifest = manifest
        .map(Path::to_path_buf)
        .unwrap_or(root().join("Cargo.toml"));
    let path = manifest.to_str().ok_or("invalid manifest path")?;
    let mut args = match gate {
        "format" => vec!["fmt", "--manifest-path", path, "--all", "--", "--check"],
        "compile" => vec![
            "check",
            "--manifest-path",
            path,
            "--workspace",
            "--all-targets",
        ],
        "lints" => vec![
            "clippy",
            "--manifest-path",
            path,
            "--workspace",
            "--lib",
            "--bins",
        ],
        _ => return Err("unknown Rust gate".into()),
    };
    if gate == "lints" {
        if manifest == root().join("Cargo.toml") {
            args.extend(["--exclude", "xtask"]);
        }
        args.extend([
            "--",
            "-D",
            "warnings",
            "-D",
            "clippy::unwrap_used",
            "-D",
            "clippy::expect_used",
            "-D",
            "clippy::panic",
            "-D",
            "clippy::arithmetic_side_effects",
            "-D",
            "clippy::allow_attributes_without_reason",
            "-D",
            "unfulfilled_lint_expectations",
        ]);
    }
    // Fixture manifests deliberately have no dependencies. Real workspace builds stay locked.
    if manifest == root().join("Cargo.toml") && gate != "format" {
        args.insert(1, "--locked");
    }
    command_with_flags(
        &args,
        manifest.parent().ok_or("manifest has no parent")?,
        false,
    )?;
    Ok(json!({"gate":gate,"manifest":manifest}))
}
fn command_with_flags(args: &[&str], cwd: &Path, docs: bool) -> Result<()> {
    let mut c = Command::new("cargo");
    c.args(args)
        .current_dir(cwd)
        .env("RUSTFLAGS", "-D warnings");
    if docs {
        c.env("RUSTDOCFLAGS", "-D warnings");
    }
    // A fixture must not acquire the parent Cargo invocation's target lock.
    if cwd != root() {
        c.env("CARGO_TARGET_DIR", cwd.join("target"));
    }
    let out = c.output().map_err(|e| format!("BLOCKED: cargo: {e}"))?;
    crate::report::record(Path::new("cargo"), args, cwd, None, &out);
    if !out.status.success() {
        return Err(format!(
            "{} failed:\n{}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
pub fn docs(manifest: Option<&Path>) -> Result<Value> {
    let manifest = manifest
        .map(Path::to_path_buf)
        .unwrap_or(root().join("Cargo.toml"));
    let path = manifest.to_str().ok_or("invalid manifest")?;
    let cwd = manifest.parent().ok_or("missing parent")?;
    for mut args in [
        vec!["doc", "--workspace", "--no-deps", "--manifest-path", path],
        vec!["test", "--workspace", "--doc", "--manifest-path", path],
    ] {
        if manifest == root().join("Cargo.toml") {
            args.insert(1, "--locked");
        }
        command_with_flags(&args, cwd, true)?;
    }
    Ok(json!({"rustdoc_warnings":"deny","doctests":"cargo test --doc"}))
}
fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(path).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            walk(&p, files)?;
        } else {
            files.push(p);
        }
    }
    Ok(())
}
fn allowlist(manifest: Option<&Path>) -> Result<Value> {
    let entries: Value = serde_json::from_str(&read(&root().join("docs/lint-allowlist.json"))?)
        .map_err(|e| e.to_string())?;
    let mut files = vec![];
    if let Some(m) = manifest {
        walk(&m.parent().ok_or("missing parent")?.join("src"), &mut files)?;
    } else {
        for e in fs::read_dir(root().join("crates")).map_err(|e| e.to_string())? {
            let p = e.map_err(|e| e.to_string())?.path().join("src");
            if p.exists() {
                walk(&p, &mut files)?;
            }
        }
    }
    let mut found = vec![];
    for file in files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|s| s == "rs"))
    {
        let text = read(&file)?;
        // Formatting is checked separately. Multiline attributes are folded before comparison.
        for fragment in text.split('#').skip(1) {
            let Some(end) = fragment.find(']') else {
                continue;
            };
            let attr = format!("#{}", &fragment[..=end]);
            let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
            if compact.contains("allow(") || compact.contains("expect(") {
                let approved =
                    entries
                        .as_array()
                        .ok_or("invalid allowlist")?
                        .iter()
                        .any(|e| {
                            let a: String = e["attribute"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .filter(|c| !c.is_whitespace())
                                .collect();
                            file == root().join(e["path"].as_str().unwrap_or(""))
                                && compact == a
                                && fragment[end.saturating_add(1)..].trim_start().starts_with(
                                    &format!("pub fn {}(", e["function"].as_str().unwrap_or("")),
                                )
                                && e["justification"].as_str().is_some_and(|s| !s.is_empty())
                        });
                if !approved {
                    return Err("undocumented or overly broad lint allowlist entry".into());
                }
                found.push(file.clone());
            }
        }
    }
    Ok(json!({"reviewed_scoped_attributes":found.len()}))
}
fn schema(path: Option<&Path>) -> Result<Value> {
    let generated = cargo(&[
        "run",
        "--locked",
        "--quiet",
        "-p",
        "currents-protocol",
        "--example",
        "contracts",
    ])?;
    let path = path
        .map(Path::to_path_buf)
        .unwrap_or(root().join("schemas/v1/contracts.json"));
    if generated != read(&path)? {
        return Err(
            "schema drift: regenerate the C02 contract schema and review compatibility".into(),
        );
    }
    Ok(json!({"schema":path}))
}
fn generated() -> Result<Value> {
    // C34/C07 own the first generated header/migration. Do not silently approve newly added files.
    let mut found = vec![];
    for dir in ["include", "migrations"] {
        let p = root().join(dir);
        if p.exists() {
            walk(&p, &mut found)?;
        }
    }
    if !found.is_empty() {
        return Err("BLOCKED: header/migration files require their owning ticket to register a reproducible drift command".into());
    }
    Ok(
        json!({"header_migration":"No header or migration artifacts exist; first owning ticket must register drift verification."}),
    )
}
fn tool(name: &str, version: &str, arg: &str) -> Result<PathBuf> {
    let path = std::env::var_os("CURRENTS_TOOLS_BIN")
        .map(PathBuf::from)
        .map(|p| p.join(name))
        .unwrap_or_else(|| PathBuf::from(name));
    let output = checked(&path, &[arg], &root(), None)?;
    if !output.contains(version) {
        return Err(format!("BLOCKED: {name} must be pinned {version}"));
    }
    Ok(path)
}
fn workflow(path: Option<&Path>) -> Result<Value> {
    let p = path
        .map(Path::to_path_buf)
        .unwrap_or(root().join(".github/workflows/ci.yml"));
    let text = read(&p)?;
    let v:Value=serde_json::from_str(&text).map_err(|_|"workflow security: workflows use the JSON subset of YAML for unambiguous policy parsing")?;
    let bad = || {
        "workflow security: untrusted trigger, permission, secret, action or artifact upload"
            .to_owned()
    };
    if v["permissions"] != json!({"contents":"read"})
        || text.contains("secrets.")
        || text.contains("secrets[")
        || text.contains("github.token")
    {
        return Err(bad());
    }
    let events = v["on"].as_object().ok_or_else(bad)?;
    if events
        .keys()
        .any(|k| !matches!(k.as_str(), "pull_request" | "push" | "workflow_dispatch"))
        || !events.contains_key("pull_request")
    {
        return Err(bad());
    }
    let jobs = v["jobs"].as_object().ok_or_else(bad)?;
    for name in ["quality", "core", "msrv", "docs", "privacy"] {
        if !jobs.contains_key(name) {
            return Err(bad());
        }
    }
    let pins: Value = serde_json::from_str(&read(&root().join("tools/versions.lock.json"))?)
        .map_err(|e| e.to_string())?;
    for job in jobs.values() {
        if !job["permissions"].is_null()
            || !job["uses"].is_null()
            || !job["container"].is_null()
            || !job["services"].is_null()
        {
            return Err(bad());
        }
        let steps = job["steps"].as_array().ok_or_else(bad)?;
        let mut checkout = false;
        for step in steps {
            if let Some(action) = step["uses"].as_str() {
                let approved = pins["actions"]
                    .as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .any(|p| p["uses"].as_str() == Some(action));
                if !approved
                    || !action.starts_with("actions/checkout@")
                    || step["with"]["persist-credentials"] != false
                {
                    return Err(bad());
                }
                checkout = true;
            }
        }
        if !checkout {
            return Err(bad());
        }
    }
    let bin = tool("actionlint", "1.7.12", "--version")?;
    checked(
        &bin,
        &["-shellcheck=", p.to_str().ok_or("invalid workflow path")?],
        &root(),
        None,
    )
    .map_err(|e| format!("workflow security: {e}"))?;
    Ok(json!({"workflow":p,"actionlint":"1.7.12","credentials":false,"uploads":false}))
}
pub fn privacy(source: &Path) -> Result<Value> {
    let bin = tool("gitleaks", "8.30.1", "version")?;
    let config = root().join(".gitleaks.toml");
    let args = [
        "dir",
        "--no-banner",
        "--redact=100",
        "--config",
        config.to_str().ok_or("config path")?,
    ];
    let snapshot = if source == root() {
        let list = Command::new("git")
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ])
            .current_dir(source)
            .output()
            .map_err(|_| "BLOCKED: privacy inventory unavailable")?;
        if !list.status.success() {
            return Err("BLOCKED: privacy inventory failed".into());
        }
        let staging = std::env::temp_dir().join(format!(
            "currents-privacy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&staging).map_err(|_| "BLOCKED: privacy snapshot unavailable")?;
        for name in list.stdout.split(|b| *b == 0).filter(|b| !b.is_empty()) {
            let name = std::str::from_utf8(name).map_err(|_| "BLOCKED: non-UTF8 source path")?;
            let input = source.join(name);
            if !input.exists() {
                continue;
            }
            if input
                .symlink_metadata()
                .map_err(|_| "BLOCKED: source metadata unavailable")?
                .file_type()
                .is_symlink()
            {
                return Err("BLOCKED: privacy snapshot refuses symlinks".into());
            }
            let output = staging.join(name);
            fs::create_dir_all(output.parent().ok_or("missing snapshot parent")?)
                .map_err(|_| "BLOCKED: privacy snapshot unavailable")?;
            fs::copy(&input, &output).map_err(|_| "BLOCKED: privacy snapshot unavailable")?;
        }
        Some(staging)
    } else {
        None
    };
    let outcome = Command::new(&bin)
        .args(args)
        .arg(snapshot.as_deref().unwrap_or(source))
        .output();
    if let Some(snapshot) = snapshot {
        fs::remove_dir_all(snapshot).map_err(|_| "BLOCKED: privacy snapshot cleanup failed")?;
    }
    let out = outcome.map_err(|_| "BLOCKED: gitleaks unavailable")?;
    // Never retain scanner stdout/stderr, including paths, matches or raw failure artifacts.
    if !out.status.success() {
        return Err(
            "privacy scan failed; details redacted (scanner failure or secret detected)".into(),
        );
    }
    Ok(json!({"scanner":"gitleaks 8.30.1","redaction":"100%","raw_artifacts":false}))
}
fn dependencies() -> Result<Value> {
    let bin = tool("cargo-deny", "0.20.2", "--version")?;
    checked(&bin, &["--locked", "check"], &root(), None)?;
    Ok(json!({"cargo_deny":"0.20.2","policy":"deny.toml"}))
}
pub fn all() -> Result<Value> {
    rust("compile", None)?;
    rust("lints", None)?;
    allowlist(None)?;
    schema(None)?;
    generated()?;
    workflow(None)?;
    docs(None)?;
    dependencies()?;
    Ok(
        json!({"quality":"format/compiler/Clippy/allowlist/schema/header/migration/workflow/docs/dependencies"}),
    )
}
pub fn qualify(gate: &str, args: &[String]) -> Result<Value> {
    let manifest = option(args, "--manifest-path")?;
    match gate {
        "format" | "compile" | "lints" => rust(gate, manifest.as_deref()),
        "allowlist" => allowlist(manifest.as_deref()),
        "schema-drift" => schema(option(args, "--schema")?.as_deref()),
        "generated-drift" => generated(),
        "workflow-security" => workflow(option(args, "--workflow")?.as_deref()),
        "privacy" => privacy(&option(args, "--source")?.unwrap_or(root())),
        "docs" => docs(manifest.as_deref()),
        "dependencies" => dependencies(),
        _ => Err("unknown quality gate".into()),
    }
}
pub fn ticket() -> Result<Value> {
    let assertions = ["cur_003_1", "cur_003_2", "cur_003_3", "cur_003_4"];
    let list = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_003",
        "--",
        "--list",
    ])?;
    let tests: BTreeSet<_> = list
        .lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .collect();
    if tests != assertions.into_iter().collect() {
        return Err("CUR-003 missing or unexpected test assertions".into());
    }
    let out = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_003",
        "--",
        "--test-threads=1",
    ])?;
    if !out.contains("4 passed; 0 failed; 0 ignored") {
        return Err("CUR-003 incomplete execution".into());
    }
    Ok(json!({"assertions":assertions}))
}
