//! Route-only assessment: no account, entitlement or authorization operations.
use serde_json::{Value, json};
use std::collections::BTreeSet;

const PREREQUISITES: [&str; 5] = [
    "organization_eligibility_unverified",
    "managed_entitlement_unverified",
    "bundle_binary_binding_unverified",
    "app_store_distribution_unverified",
    "usage_description_unverified",
];
#[derive(Clone, Copy, Default)]
struct Prerequisites {
    verified: [bool; 5],
    owner_limitation_accepted: bool,
}
impl Prerequisites {
    fn missing(&self) -> Vec<&'static str> {
        PREREQUISITES
            .into_iter()
            .zip(self.verified)
            .filter_map(|(reason, verified)| (!verified).then_some(reason))
            .collect()
    }
    fn limitation(&self) -> &'static str {
        if self.owner_limitation_accepted {
            "owner_limitation_accepted"
        } else {
            "owner_limitation_unassessed"
        }
    }
    fn automatic_capture_enabled(&self) -> bool {
        // Prerequisites alone cannot qualify account coverage (CUR-057), consent,
        // timeliness or activate an adapter. This ticket implements no adapter.
        false
    }
}
pub fn qualify(args: &[String]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err("BLOCKED: financekit_route_unverified; unsupported_arguments; route_only_no_account_probe".into());
    }
    // Public documentation is not evidence about this application's actual
    // developer account, binary, entitlement or distribution approval.
    let facts = Prerequisites::default();
    let mut reasons = vec!["financekit_route_unverified"];
    reasons.extend(facts.missing());
    reasons.extend([
        facts.limitation(),
        "personal_account_not_eligible_evidence",
        "statement_import_not_automation",
        "actual_coverage_cur057_unverified",
        "route_only_no_account_probe",
    ]);
    if !facts.automatic_capture_enabled() {
        reasons.push("automatic_capture_disabled");
    }
    Err(format!("BLOCKED: {}", reasons.join("; ")))
}
pub fn ticket() -> Result<Value, String> {
    let assertions = [
        "cur_044_1",
        "cur_044_3",
        "cur_044_4",
        "cur_044_no_account_probe",
    ];
    let list = crate::checks::cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-acquisition",
        "--test",
        "cur_044",
        "--",
        "--list",
    ])?;
    let actual: BTreeSet<_> = list
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    if actual != assertions.into_iter().collect() {
        return Err("BLOCKED: CUR044 required assertion inventory mismatch".into());
    }
    let output = crate::checks::cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-acquisition",
        "--test",
        "cur_044",
    ])?;
    if !output.contains("4 passed; 0 failed; 0 ignored") {
        return Err("BLOCKED: CUR044 incomplete synthetic contract execution".into());
    }
    Ok(json!({
        "ticket": "CUR-044", "assertions": assertions,
        "scope": "synthetic route-only fail-closed contract",
        "operational_financekit": "Blocked", "automatic_capture": false,
        "actual_coverage": "CUR-057 unverified"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_prerequisite_is_required_and_fixtures_never_activate() {
        let complete = Prerequisites {
            verified: [true; 5],
            owner_limitation_accepted: true,
        };
        assert!(complete.missing().is_empty());
        assert!(!complete.automatic_capture_enabled());
        for (index, reason) in PREREQUISITES.iter().enumerate() {
            let mut incomplete = complete;
            incomplete.verified[index] = false;
            assert_eq!(incomplete.missing(), vec![*reason]);
            assert!(!incomplete.automatic_capture_enabled());
        }
    }
    #[test]
    fn acceptance_is_independent_and_cannot_waive_route_requirements() {
        let unknown = Prerequisites::default();
        let accepted = Prerequisites {
            owner_limitation_accepted: true,
            ..unknown
        };
        assert_eq!(unknown.missing(), accepted.missing());
        assert_eq!(unknown.limitation(), "owner_limitation_unassessed");
        assert_eq!(accepted.limitation(), "owner_limitation_accepted");
        assert!(!accepted.automatic_capture_enabled());
    }
    #[test]
    fn extra_arguments_are_rejected_without_echo() {
        let error = qualify(&["private-sentinel".into()]).unwrap_err();
        assert!(error.contains("unsupported_arguments"));
        assert!(!error.contains("private-sentinel"));
    }
}
