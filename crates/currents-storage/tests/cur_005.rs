//! CUR-005 qualification exercises the pinned native suite, not a mock kernel.
use std::{path::PathBuf, process::Command};

fn native_test(name: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = root.join("vendor/doubleentry/Cargo.toml");
    assert!(
        manifest.exists(),
        "CUR-005 pinned private native source is absent"
    );
    let output = Command::new("cargo")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_TEST_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0")
        .args(["test", "--locked", "--manifest-path"])
        .arg(manifest)
        .args([
            "--features",
            "sqlite,testing",
            "--lib",
            name,
            "--",
            "--exact",
        ])
        .output()
        .expect("run pinned native boundary assertion");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed; 0 failed; 0 ignored"),
        "missing named assertion: {stdout}"
    );
}

#[test]
fn cur_005_1() {
    native_test("store::sql::write::currents::tests::atomic_rollback");
}
#[test]
fn cur_005_2() {
    native_test("store::sql::write::currents::tests::commit_uncertainty");
}
#[test]
fn cur_005_3() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let verified = Command::new("python3")
        .arg(root.join("patches/doubleentry/verify.py"))
        .output()
        .expect("verify native provenance");
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let output = Command::new("cargo")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_TEST_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0")
        .args(["test", "--locked", "--manifest-path"])
        .arg(root.join("vendor/doubleentry/Cargo.toml"))
        .args(["--features", "sqlite,testing"])
        .output()
        .expect("run complete pinned upstream suite");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("sqlite_conforms ... ok"),
        "native conformance not executed: {stdout}"
    );
    assert!(
        stdout.contains("test result: ok."),
        "zero executed upstream tests"
    );
}
#[test]
fn cur_005_4() {
    native_test("store::sql::write::currents::tests::schema_guard");
}
