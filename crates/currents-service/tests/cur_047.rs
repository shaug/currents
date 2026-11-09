//! Public stdin contract tests. All input values are synthetic.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture() -> Value {
    serde_json::from_slice(
        &std::fs::read(root().join("fixtures/setup/synthetic-household/inventory.json")).unwrap(),
    )
    .unwrap()
}
fn inspect(input: &Value) -> (bool, Value, String) {
    let mut child = Command::new("cargo")
        .args(["xtask", "household-readiness"])
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    let report = serde_json::from_str(&text).unwrap_or_else(|_| {
        panic!(
            "missing readiness report: {text}; {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), report, text)
}
#[test]
fn cur_047_1() {
    // Dropping unknown/unconnected components, or manufacturing zero, must fail.
    let (ok, r, _) = inspect(&fixture());
    assert!(ok);
    assert_eq!(r["inventory_status"], "Passed");
    let rows = r["components"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    for (i, kind) in [
        "cash",
        "card",
        "debt",
        "investment",
        "manual_asset",
        "manual_liability",
        "apple_card",
        "unknown",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(rows[i]["kind"], *kind);
        assert_eq!(rows[i]["position"], "unknown");
    }
    assert_eq!(r["actual_household"]["status"], "Blocked");
    let mut private = fixture();
    private["profile"] = json!("actual_household");
    private["components"][0]["institution"] =
        json!({"status":"confirmed","value":"SYNTHETIC-PRIVATE-CANARY"});
    let (ok, r, text) = inspect(&private);
    assert!(!ok);
    assert_eq!(r["actual_household"]["status"], "Blocked");
    assert!(!text.contains("SYNTHETIC-PRIVATE-CANARY"));
    assert!(!text.contains("component-0"));
}
#[test]
fn cur_047_2() {
    // Missing material meanings must remain distinguishable, never silently adopted.
    let (_, r, _) = inspect(&fixture());
    for (field, status) in [
        ("source_precedence", "conflict"),
        ("freshness_threshold", "not_assessed"),
        ("opening", "unknown"),
        ("withholding", "unresolved"),
        ("ownership", "unknown"),
    ] {
        assert!(
            r["unresolved"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["field"].as_str().unwrap().ends_with(field) && v["state"] == status),
            "{field}: {r}"
        );
    }
    for v in r["unresolved"].as_array().unwrap() {
        assert!(!v["next_action"].as_str().unwrap().is_empty());
    }
    let mut input = fixture();
    input["meaning"]["withholding"] =
        json!({"status":"confirmed","value":"Owner interpretation, synthetic"});
    let (_, r, _) = inspect(&input);
    assert!(
        !r["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["field"] == "meaning.withholding")
    );
    input["meaning"]["withholding"] = json!({"status":"confirmed"});
    assert!(!inspect(&input).0);
}
#[test]
fn cur_047_3() {
    // Manifest and coverage must be complete/bijective; a fallback cannot waive automation.
    let input = fixture();
    let (_, r, _) = inspect(&input);
    for (i, route) in [
        "automatic",
        "file",
        "manual",
        "unknown",
        "automatic",
        "file",
        "manual",
        "unknown",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(r["components"][i]["route"], *route);
        assert_eq!(r["components"][i]["limitation"], "unassessed");
    }
    for field in ["components", "coverage"] {
        let mut bad = input.clone();
        bad[field].as_array_mut().unwrap().pop();
        assert!(!inspect(&bad).0, "silently dropped {field}");
    }
    let mut bad = input.clone();
    bad["coverage"][1] = bad["coverage"][0].clone();
    assert!(!inspect(&bad).0);
    bad = input.clone();
    bad["components"][0]["unexpected"] = json!("SYNTHETIC-PRIVATE-CANARY");
    let (ok, _, text) = inspect(&bad);
    assert!(!ok);
    assert!(!text.contains("SYNTHETIC-PRIVATE-CANARY"));
    bad = input.clone();
    bad["components"][6]["automatic_required"] = json!({"status":"confirmed","value":"yes"});
    let (_, r, _) = inspect(&bad);
    assert!(
        r["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["field"] == "coverage[6].limitation")
    );
    bad["coverage"][6]["limitation"] = json!({"status":"owner_accepted","detail":"Synthetic owner accepts statement-only Apple Card scope; automation remains blocked"});
    let (_, r, _) = inspect(&bad);
    assert_eq!(r["components"][6]["limitation"], "owner_accepted");
    assert_eq!(r["actual_household"]["status"], "Blocked");
}
