//! Partial CUR-004 public counterexample. Native launch qualification is separate.
use std::{path::Path, process::Command};

#[test]
fn cur_004_3() {
    // A CLT-only selection must not become a native app PASS, even when a full
    // Xcode is installed elsewhere. This fixture changes only the child process.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let out = Command::new("cargo")
        .current_dir(root)
        .env("DEVELOPER_DIR", "/Library/Developer/CommandLineTools")
        .args(["xtask", "qualify", "native-platform"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let report = String::from_utf8_lossy(&out.stdout);
    assert!(report.contains("\"status\": \"Blocked\""));
    assert!(report.contains("full Xcode"), "{report}");
    assert!(report.contains("macOS 15.0 arm64"), "{report}");
}
