//! Explicit verification registry. An unregistered check can never become green.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
const MEMBERS: &[&str] = &[
    "types",
    "protocol",
    "cipher",
    "transfer",
    "domain",
    "accounting",
    "storage",
    "service",
    "planning",
    "acquisition",
    "ffi",
    "cli",
    "mcp",
    "verify",
];
const ASSERTIONS: &[&str] = &["cur_001_1", "cur_001_2", "cur_001_3", "cur_001_4"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}
fn output(program: &Path, args: &[&str], cwd: &Path, rustc: Option<&Path>) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if let Some(rustc) = rustc {
        command.env("RUSTC", rustc);
    }
    command
        .output()
        .map_err(|e| format!("BLOCKED: {}: {e}", program.display()))
}
fn checked(program: &Path, args: &[&str], cwd: &Path, rustc: Option<&Path>) -> Result<String> {
    let out = output(program, args, cwd, rustc)?;
    crate::report::record(program, args, cwd, rustc, &out);
    let evidence_dir = root().join("target/qualification");
    fs::create_dir_all(&evidence_dir).map_err(|e| e.to_string())?;
    let record = json!({"program":program,"args":args,"cwd":cwd,"rustc":rustc,"exit":out.status.code(),"stdout":String::from_utf8_lossy(&out.stdout),"stderr":String::from_utf8_lossy(&out.stderr)});
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(evidence_dir.join("commands.jsonl"))
        .map_err(|e| e.to_string())?;
    writeln!(log, "{record}").map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "{} {} failed ({}):\n{}\n{}",
            program.display(),
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
fn cargo(args: &[&str]) -> Result<String> {
    checked(Path::new("cargo"), args, &root(), None)
}
fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn metadata(manifest: &Path) -> Result<Value> {
    let text = cargo(&[
        "metadata",
        "--offline",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        manifest.to_str().ok_or("invalid path")?,
    ])?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
fn packages(meta: &Value) -> Result<&Vec<Value>> {
    meta["packages"]
        .as_array()
        .ok_or_else(|| "missing Cargo packages".into())
}
fn workspace() -> Result<Value> {
    let meta = metadata(&root().join("Cargo.toml"))?;
    let found: BTreeSet<_> = packages(&meta)?
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    let mut expected: BTreeSet<String> = MEMBERS.iter().map(|n| format!("currents-{n}")).collect();
    expected.insert("xtask".into());
    if found != expected.iter().map(String::as_str).collect() {
        return Err("workspace membership differs from C01".into());
    }
    for p in packages(&meta)? {
        if p["publish"] != json!([]) {
            return Err(format!("{} must set publish=false", p["name"]));
        }
        if p["edition"] != "2024" {
            return Err(format!("{} must use edition 2024", p["name"]));
        }
    }
    Ok(json!({"members":found,"publish":false,"edition":"2024"}))
}
fn allowed_internal(name: &str) -> &'static [&'static str] {
    match name {
        "currents-types" => &[],
        "currents-protocol" => &["currents-types"],
        "currents-domain"
        | "currents-planning"
        | "currents-accounting"
        | "currents-acquisition" => &["currents-types"],
        "currents-cipher" => &[],
        "currents-transfer" => &["currents-types", "currents-protocol", "currents-cipher"],
        "currents-storage" => &["currents-types", "currents-accounting", "currents-cipher"],
        "currents-service" => &[
            "currents-types",
            "currents-domain",
            "currents-planning",
            "currents-storage",
        ],
        "currents-ffi" | "currents-cli" | "currents-mcp" => {
            &["currents-service", "currents-protocol"]
        }
        "currents-verify" => &[
            "currents-service",
            "currents-storage",
            "currents-protocol",
            "currents-types",
        ],
        _ => &[],
    }
}
fn direction(manifest: &Path) -> Result<Value> {
    let meta = metadata(manifest)?;
    for p in packages(&meta)? {
        let name = p["name"].as_str().ok_or("missing package name")?;
        if !name.starts_with("currents-") {
            continue;
        }
        for dep in p["dependencies"].as_array().ok_or("missing dependencies")? {
            // Dev-only dependencies do not ship; build dependencies still matter.
            if dep["kind"] == "dev" {
                continue;
            }
            let d = dep["name"].as_str().ok_or("missing dependency name")?;
            let pure = matches!(
                name,
                "currents-types" | "currents-domain" | "currents-planning"
            );
            let safe = matches!(
                d,
                "serde"
                    | "serde_json"
                    | "time"
                    | "thiserror"
                    | "uuid"
                    | "num-bigint"
                    | "num-integer"
                    | "num-traits"
                    | "sha2"
                    | "jiff"
                    | "jiff-tzdb"
            );
            let forbidden = (d.starts_with("currents-") && !allowed_internal(name).contains(&d))
                || (pure && !d.starts_with("currents-") && !safe)
                || (d == "doubleentry" && name != "currents-accounting")
                || (d.starts_with("sqlx") || d == "libsqlite3-sys")
                    && !matches!(name, "currents-cipher")
                || (name == "currents-cipher"
                    && !matches!(
                        d,
                        "sqlx"
                            | "sqlx-core"
                            | "sqlx-sqlite"
                            | "libsqlite3-sys"
                            | "zeroize"
                            | "secrecy"
                    ))
                || (name == "currents-verify"
                    && (d == "reqwest"
                        || d.starts_with("objc")
                        || d.starts_with("core-foundation")));
            if forbidden {
                return Err(format!("forbidden dependency: {name} -> {d}"));
            }
        }
    }
    Ok(json!({"manifest":manifest,"dependency_direction":"accepted"}))
}
fn inventory(path: &Path, artifact_root: Option<&Path>) -> Result<Value> {
    let data = read_json(path)?;
    if data["schema_version"] != 1 {
        return Err("unsupported pin inventory schema".into());
    }
    let entries = data["artifacts"].as_array().ok_or("missing artifacts")?;
    if entries.is_empty() {
        return Err("BLOCKED: zero pinned local artifacts; supply the qualified tool/source cache inventory".into());
    }
    let mut names = BTreeSet::new();
    let mut evidence = vec![];
    for entry in entries {
        let field = |key| {
            entry[key]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("missing artifact {key}"))
        };
        let name = field("name")?;
        if !names.insert(name) {
            return Err(format!("duplicate artifact: {name}"));
        }
        let version = field("version")?;
        if version == "latest" || version.contains(['*', '^', '~']) {
            return Err(format!("unqualified version: {name}"));
        }
        let source = field("source")?;
        if !source.starts_with("https://") {
            return Err(format!("source must be HTTPS: {name}"));
        }
        let digest = field("sha256")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(format!("invalid checksum: {name}"));
        }
        let artifact = artifact_root
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")))
            .join(field("path")?);
        let bytes = fs::read(&artifact).map_err(|e| format!("missing artifact {name}: {e}"))?;
        if sha(&bytes) != digest {
            return Err(format!("checksum mismatch: {name}"));
        }
        evidence.push(json!({"name":name,"version":version,"source":source,"sha256":digest}));
    }
    Ok(json!({"artifacts":evidence}))
}
fn msrv_tools() -> Result<(PathBuf, PathBuf, String)> {
    let bin = env::var_os("CURRENTS_MSRV_BIN")
        .ok_or("BLOCKED: set CURRENTS_MSRV_BIN to the verified Rust 1.94.0 bin directory")?;
    let rustc = PathBuf::from(&bin).join("rustc");
    let cargo = PathBuf::from(bin).join("cargo");
    let version = checked(&rustc, &["--version"], &root(), None)?;
    if !version.starts_with("rustc 1.94.0 ") {
        return Err(format!("wrong MSRV compiler: {version}"));
    }
    checked(&cargo, &["--version"], &root(), None)?;
    Ok((cargo, rustc, version))
}
fn check_native_environment() -> Result<()> {
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy();
        if key.ends_with("OPENSSL_DIR")
            || key.ends_with("OPENSSL_NO_VENDOR")
            || key.ends_with("OPENSSL_LIB_DIR")
            || key.ends_with("OPENSSL_INCLUDE_DIR")
            || key == "LIBSQLITE3_SYS_USE_PKG_CONFIG"
            || key == "SQLITE3_LIB_DIR"
            || key == "SQLITE3_INCLUDE_DIR"
        {
            return Err(format!(
                "BLOCKED: clear native provider override {key} before qualification"
            ));
        }
    }
    Ok(())
}
fn core(msrv: bool, manifest: Option<&Path>) -> Result<Value> {
    check_native_environment()?;
    let (program, rustc, version) = if msrv {
        let (c, r, v) = msrv_tools()?;
        (c, Some(r), v)
    } else {
        (
            PathBuf::from("cargo"),
            None,
            checked(Path::new("rustc"), &["--version"], &root(), None)?,
        )
    };
    if !msrv && !version.starts_with("rustc 1.99.0 ") {
        return Err(format!("wrong release compiler: {version}"));
    }
    let manifest = manifest
        .map(|path| path.to_str().ok_or("invalid manifest path"))
        .transpose()?;
    let mut build = vec!["build", "--locked", "--workspace", "--all-targets"];
    let mut test = vec!["test", "--locked", "--workspace"];
    for args in [&mut build, &mut test] {
        if let Some(path) = manifest {
            args.extend(["--manifest-path", path]);
        }
    }
    test.extend(["--", "--test-threads=1"]);
    let mut evidence = vec![];
    for args in [build, test] {
        let text = checked(&program, &args, &root(), rustc.as_deref())?;
        evidence.push(json!({"command":args,"stdout_sha256":sha(text.as_bytes())}));
    }
    Ok(
        json!({"compiler":version.trim(),"commands":evidence,"host":env::consts::OS,"arch":env::consts::ARCH}),
    )
}
fn ticket(name: &str) -> Result<Value> {
    if name == "CUR-002" {
        let assertions = ["cur_002_1", "cur_002_2", "cur_002_3", "cur_002_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-types",
            "--test",
            "cur_002",
            "--",
            "--list",
        ])?;
        let tests: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if tests != assertions.iter().copied().collect() {
            return Err(format!("test registry mismatch or zero tests: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-types",
            "--test",
            "cur_002",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err(format!("incomplete test execution: {text}"));
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes())}),
        );
    }
    if name != "CUR-001" {
        return Err(format!("BLOCKED: no registered test suite for {name}"));
    }
    let args = [
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_001",
        "--",
        "--list",
    ];
    let list = cargo(&args)?;
    let tests: BTreeSet<_> = list
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    if tests != ASSERTIONS.iter().copied().collect() {
        return Err(format!("test registry mismatch or zero tests: {list}"));
    }
    let text = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_001",
        "--",
        "--test-threads=1",
    ])?;
    if !text.contains("4 passed; 0 failed; 0 ignored") {
        return Err(format!("incomplete test execution: {text}"));
    }
    Ok(json!({"ticket":name,"assertions":ASSERTIONS,"output_sha256":sha(text.as_bytes())}))
}
fn native_source() -> Result<Value> {
    let text = cargo(&["metadata", "--locked", "--format-version", "1"])?;
    let meta: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let packages = packages(&meta)?;
    let sys: Vec<_> = packages
        .iter()
        .filter(|p| p["name"] == "libsqlite3-sys")
        .collect();
    if sys.len() != 1 || sys[0]["version"] != "0.37.0" || !sys[0]["source"].is_null() {
        return Err("native source must use exactly the private sys0.37 adaptation".into());
    }
    let expected = root().join("vendor/libsqlite3-sys/Cargo.toml");
    if sys[0]["manifest_path"].as_str() != expected.to_str() {
        return Err("unexpected sys manifest path".into());
    }
    let inventory = read_json(&root().join("tools/versions.lock.json"))?;
    let native = &inventory["native_source"];
    let source = packages
        .iter()
        .find(|p| p["name"] == "sqlcipher-src")
        .ok_or("missing fixed native source package")?;
    if source["version"] != native["version"] {
        return Err("native source version mismatch".into());
    }
    let manifest = PathBuf::from(
        source["manifest_path"]
            .as_str()
            .ok_or("missing source manifest")?,
    );
    for (name, digest) in native["files"]
        .as_object()
        .ok_or("missing source file inventory")?
    {
        let bytes = fs::read(manifest.parent().unwrap().join(name)).map_err(|e| e.to_string())?;
        if digest.as_str() != Some(sha(&bytes).as_str()) {
            return Err(format!("native source checksum mismatch: {name}"));
        }
    }
    Ok(
        json!({"sys_manifest":expected,"source":native,"scope":"fixed upstream source bytes and one native stack; runtime behavior separately tested"}),
    )
}
fn quality() -> Result<Value> {
    native_source()?;
    workspace()?;
    direction(&root().join("Cargo.toml"))?;
    cargo(&["fmt", "--all", "--", "--check"])?;
    cargo(&[
        "clippy",
        "--locked",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["doc", "--locked", "--workspace", "--no-deps"])?;
    let tests = ticket("CUR-001")?;
    Ok(json!({"tests":tests,"core":core(false, None)?}))
}
fn option(args: &[String], name: &str) -> Result<Option<PathBuf>> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    args.get(i + 1)
        .map(PathBuf::from)
        .map(Some)
        .ok_or_else(|| format!("missing {name} value"))
}
pub fn run(args: Vec<String>) -> Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let gate = args.get(1).map(String::as_str).unwrap_or("");
    for (program, version_args) in [
        ("rustc", &["-vV"][..]),
        ("cargo", &["--version"][..]),
        ("cc", &["--version"][..]),
    ] {
        let _ = checked(Path::new(program), version_args, &root(), None);
    }
    let result = (|| {
        match (command,gate) {
        ("ticket",name)=>ticket(name),
        ("check","quality")=>quality(), ("check","core")=>core(false, None), ("check","msrv")=>core(true, None),
        ("qualify","workspace")=>workspace(),
        ("qualify","native-source")=>native_source(),
        ("qualify","dependency-direction")=>direction(&option(&args,"--manifest-path")?.unwrap_or(root().join("Cargo.toml"))),
        ("qualify","pin-inventory")=>inventory(&option(&args,"--inventory")?.unwrap_or(root().join("tools/versions.lock.json")), option(&args,"--artifact-root")?.as_deref()),
        ("qualify","msrv") if args.iter().any(|s|s=="--probe")=>msrv_tools().map(|(_,_,v)|json!({"compiler":v.trim(),"scope":"compiler identity only; not a production feature qualification"})),
        ("qualify","msrv")=>core(true, option(&args,"--manifest-path")?.as_deref()),
        ("check","native"|"security"|"deep"|"release"|"all") | ("gates",_)=>Err(format!("BLOCKED: {command} {gate} requires suites registered by later owning tickets")),
        _=>Err(format!("BLOCKED: unregistered command: {}",args.join(" "))),
    }
    })();
    let head = checked(Path::new("git"), &["rev-parse", "HEAD"], &root(), None)
        .unwrap_or_else(|_| "unknown".into());
    let dirty = checked(Path::new("git"), &["status", "--porcelain"], &root(), None)
        .map(|s| !s.is_empty())
        .unwrap_or(true);
    let report = crate::report::build(&root(), &args, &head, dirty, &result);
    let report_dir = root().join("target/qualification");
    fs::create_dir_all(&report_dir).map_err(|e| e.to_string())?;
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    let filename = format!(
        "report-{}-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    fs::write(report_dir.join(filename), report_bytes).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    result.map(|_| ())
}
