//! CUR-006 real target, feature and linked-symbol qualification.
use crate::checks::{cargo, check_native_environment, checked, native_source, root};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};
const ASSERTIONS: [&str; 4] = ["cur_006_1", "cur_006_2", "cur_006_3", "cur_006_4"];
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn symbols(path: &Path) -> Result<Value, String> {
    let name = path.to_str().ok_or("invalid artifact path")?;
    // -gU is Mach-O's defined externals. GNU nm spells it --defined-only.
    let args = if cfg!(target_os = "macos") {
        vec!["-gU", name]
    } else {
        vec!["-g", "--defined-only", name]
    };
    let nm = env::var_os("CURRENTS_NM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("nm"));
    checked(&nm, &["--version"], &root(), None)?;
    let text = checked(&nm, &args, &root(), None)?;
    let mut counts = serde_json::Map::new();
    for required in ["sqlite3_open_v2", "sqlite3_key", "sqlcipher_version"] {
        let count = text
            .lines()
            .filter(|line| {
                line.split_whitespace()
                    .last()
                    .is_some_and(|s| s.trim_start_matches('_') == required)
            })
            .count();
        if count != 1 {
            return Err(format!(
                "expected exactly one definition of {required} in {name}, found {count}"
            ));
        }
        counts.insert(required.into(), json!(count));
    }
    let links = if path.extension().is_some_and(|e| e == "a") {
        String::from("static archive; provider linkage checked in executable")
    } else if cfg!(target_os = "macos") {
        checked(Path::new("otool"), &["-L", name], &root(), None)?
    } else {
        checked(Path::new("ldd"), &[name], &root(), None)?
    };
    let lower = links.to_ascii_lowercase();
    if lower.contains("libsqlite3")
        || lower.contains("libsqlcipher")
        || lower.contains("libcrypto.")
        || lower.contains("libssl.")
    {
        return Err(format!("unqualified dynamic SQLite/crypto link in {name}"));
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(
        json!({"path":path,"sha256":sha(&bytes),"definitions":counts,"symbol_output_sha256":sha(text.as_bytes()),"dynamic_links":links}),
    )
}

/// The standalone driver suite has a separate exact lock and the root sys patch.
/// Running it from the workspace root does not inherit provider feature choices.
pub(crate) fn driver(program: &Path, rustc: Option<&Path>) -> Result<Value, String> {
    check_native_environment()?;
    checked(
        Path::new("python3"),
        &["patches/sqlx-sqlite/verify.py"],
        &root(),
        None,
    )?;
    let manifest = "vendor/sqlx-sqlite/Cargo.toml";
    let features = "bundled,unlock-notify,time";
    let metadata = checked(
        program,
        &[
            "metadata",
            "--locked",
            "--manifest-path",
            manifest,
            "--features",
            features,
            "--format-version",
            "1",
        ],
        &root(),
        rustc,
    )?;
    let metadata: Value = serde_json::from_str(&metadata).map_err(|e| e.to_string())?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("missing driver packages")?;
    for (name, version, local) in [
        ("sqlx-sqlite", "0.9.0", true),
        ("libsqlite3-sys", "0.37.0", true),
        ("sqlx-core", "0.9.0", false),
        (
            "sqlcipher-src",
            "420.0.0+sqlcipher-4.20.0-sqlite-3.54.1-libtomcrypt-1.18.2",
            false,
        ),
        ("tokio", "1.54.1", false),
        ("time", "0.3.55", false),
    ] {
        let selected: Vec<_> = packages.iter().filter(|p| p["name"] == name).collect();
        if selected.len() != 1 || selected[0]["version"] != version {
            return Err(format!(
                "driver qualification has wrong {name} source/version"
            ));
        }
        if local
            && (selected[0]["source"] != Value::Null
                || selected[0]["manifest_path"].as_str()
                    != root().join("vendor").join(name).join("Cargo.toml").to_str())
        {
            return Err(format!(
                "driver qualification must use the reviewed local {name}"
            ));
        }
    }
    let tree = checked(
        program,
        &[
            "tree",
            "--locked",
            "--manifest-path",
            manifest,
            "--features",
            features,
            "-e",
            "features",
            "-i",
            "libsqlite3-sys",
        ],
        &root(),
        rustc,
    )?;
    let expected = if cfg!(target_os = "macos") {
        "bundled-sqlcipher"
    } else {
        "bundled-sqlcipher-vendored-openssl"
    };
    if !tree.contains(expected)
        || (cfg!(target_os = "macos") && tree.contains("bundled-sqlcipher-vendored-openssl"))
    {
        return Err("driver qualification has wrong native provider".into());
    }
    // An excluded standalone package otherwise writes vendor/sqlx-sqlite/target.
    // Keep all build output in the caller-owned workspace target directory.
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"));
    let target = target.to_str().ok_or("invalid driver target directory")?;
    let run = checked(
        program,
        &[
            "test",
            "--locked",
            "--manifest-path",
            manifest,
            "--target-dir",
            target,
            "--lib",
            "--features",
            features,
            "--",
            "--test-threads=1",
        ],
        &root(),
        rustc,
    )?;
    if !run.contains("14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out") {
        return Err(
            "driver qualification did not execute all 7 upstream and 7 lifecycle assertions".into(),
        );
    }
    Ok(
        json!({"manifest":manifest,"lock":"vendor/sqlx-sqlite/Cargo.lock","upstream_tests":7,
        "lifecycle_tests":7,"feature_graph_sha256":sha(tree.as_bytes()),"output_sha256":sha(run.as_bytes())}),
    )
}

pub fn qualify() -> Result<Value, String> {
    if !matches!(
        (env::consts::OS, env::consts::ARCH),
        ("macos", "aarch64") | ("linux", "x86_64")
    ) {
        return Err(
            "BLOCKED: CUR-006 requires independent Mac arm64 or Linux x86_64 execution".into(),
        );
    }
    check_native_environment()?;
    let source = native_source()?;
    let driver = driver(Path::new("cargo"), None)?;
    let boundary = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-cipher",
        "--lib",
        "--",
        "--test-threads=1",
    ])?;
    if !boundary.contains("1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out") {
        return Err("private cipher failed-initializer boundary was not qualified".into());
    }
    let provenance = root().join("patches/doubleentry/verify.py");
    checked(
        Path::new("python3"),
        &[provenance.to_str().ok_or("invalid provenance path")?],
        &root(),
        None,
    )?;
    let tree = cargo(&["tree", "--locked", "-e", "features", "-i", "libsqlite3-sys"])?;
    let expected = if cfg!(target_os = "macos") {
        "bundled-sqlcipher"
    } else {
        "bundled-sqlcipher-vendored-openssl"
    };
    if !tree.contains(expected) || !tree.starts_with("libsqlite3-sys v0.37.0") {
        return Err("wrong SQLCipher target feature graph".into());
    }
    if cfg!(target_os = "macos") && tree.contains("bundled-sqlcipher-vendored-openssl") {
        return Err("Mac must use Apple crypto".into());
    }
    let list = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-storage",
        "--test",
        "cur_006",
        "--",
        "--list",
    ])?;
    let tests: BTreeSet<_> = list
        .lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .collect();
    if tests != ASSERTIONS.into_iter().collect() {
        return Err(format!("CUR-006 missing assertions: {list}"));
    }
    let run = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-storage",
        "--test",
        "cur_006",
        "--",
        "--test-threads=1",
    ])?;
    if !run.contains("4 passed; 0 failed; 0 ignored") {
        return Err("CUR-006 incomplete execution".into());
    }
    let artifacts = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-storage",
        "--test",
        "cur_006",
        "--no-run",
        "--message-format=json",
    ])?;
    let executable = artifacts
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            (v["reason"] == "compiler-artifact" && v["target"]["name"] == "cur_006")
                .then(|| v["executable"].as_str().map(PathBuf::from))
                .flatten()
        })
        .ok_or("missing actual compiled cipher host executable")?;
    cargo(&[
        "build",
        "--locked",
        "-p",
        "currents-transfer",
        "--message-format=json",
    ])?;
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"));
    let library = target.join("debug/libcurrents_transfer.a");
    Ok(
        json!({"assertions":ASSERTIONS,"driver":driver,"private_boundary_output_sha256":sha(boundary.as_bytes()),"host":{"os":env::consts::OS,"arch":env::consts::ARCH},"source":source,"feature_graph_sha256":sha(tree.as_bytes()),"host_artifact":symbols(&executable)?,"shared_library":symbols(&library)?,"production_enabled":false,"limits":["Synthetic host libraries only; not iOS/device or signed production qualification.","Process-exit artifacts are tested; privileged memory dumps and physical power loss are not certified."]}),
    )
}
