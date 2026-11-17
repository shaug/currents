//! Route qualification is separate from capture or financial-account access.
use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
use currents_types::*;
use serde_json::Value;
fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn context(rows: usize, generation: u128) -> ImportContextV1 {
    ImportContextV1 {
        receipt_id: id(generation),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(1),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-11T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-12T13:00:00Z\"").unwrap(),
        coverage: vec![CoverageV1 {
            scope: DateScope {
                start: None,
                end: None,
                basis: DateBasis::Effective,
            },
            status: CoverageStatus::Unknown,
        }],
        row_ids: (0..rows)
            .map(|n| ImportRowIdsV1 {
                observation: id(10000 + n as u128),
                product: id(generation + 20000 + n as u128),
                facet: id(40000 + n as u128),
            })
            .collect(),
    }
}
fn profile(format: ImportFormatV1) -> ParserProfileV1 {
    ParserProfileV1 {
        schema_version: 1,
        format,
        parser_version: "1".into(),
        unit: UnitDefinition {
            id: UnitId(id(2).0),
            exponent: 2,
            native_code: "USD".into(),
            kind: UnitKind::Currency,
        },
    }
}
fn parse(bytes: Vec<u8>, rows: usize) -> Result<CaptureBatchV1, EngineError> {
    FileParser::parse(
        BoundedEvidence::new("synthetic.csv", bytes, context(rows, 100))?,
        profile(ImportFormatV1::SyntheticCsv),
    )
}
const CSV: &str = include_str!("../../../fixtures/imports/synthetic-v1.csv");

use std::{path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .into()
}
fn gate() -> Value {
    let output = Command::new("cargo")
        .current_dir(root())
        .args(["xtask", "qualify", "financekit"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn cur_044_1() {
    let report = gate();
    assert_eq!(report["status"], "Blocked");
    let error = report["evidence"]["error"].as_str().unwrap();
    assert!(error.contains("financekit_route_unverified"));
    for reason in [
        "organization_eligibility_unverified",
        "managed_entitlement_unverified",
        "bundle_binary_binding_unverified",
        "usage_description_unverified",
    ] {
        assert!(error.contains(reason), "{error}");
    }
}
#[test]
fn cur_044_3() {
    let batch = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    assert!(batch.receipt.complete);
    assert_eq!(batch.receipt.source.source, "synthetic");
    let report = gate();
    let error = report["evidence"]["error"].as_str().unwrap();
    assert!(error.contains("statement_import_not_automation"));
    assert!(error.contains("owner_limitation_unassessed"));
    assert!(error.contains("actual_coverage_cur057_unverified"));
}
#[test]
fn cur_044_4() {
    let report = gate();
    let error = report["evidence"]["error"].as_str().unwrap();
    assert!(error.contains("app_store_distribution_unverified"));
    assert!(error.contains("personal_account_not_eligible_evidence"));
    assert!(error.contains("automatic_capture_disabled"));
}
#[test]
fn cur_044_no_account_probe() {
    let report = gate(); // Ensure the actual xtask binary is built before testing its boundary.
    let rejected = Command::new("cargo")
        .current_dir(root())
        .args([
            "--quiet",
            "xtask",
            "qualify",
            "financekit",
            "--account=private-sentinel",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains("private-sentinel"));
    let rejected_report: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected_report["status"], "Blocked");
    assert!(
        rejected_report["evidence"]["error"]
            .as_str()
            .unwrap()
            .contains("unsupported_arguments")
    );
    let error = report["evidence"]["error"].as_str().unwrap();
    assert!(error.contains("route_only_no_account_probe"));
    assert!(report["tests"].as_array().unwrap().is_empty());
    for command in report["evidence"]["commands"].as_array().unwrap() {
        let program = command["program"].as_str().unwrap();
        assert!(
            matches!(program, "git" | "rustc" | "cargo" | "cc"),
            "unexpected command {program}"
        );
    }
}
