//! Private questionnaire intake, not an activation or financial interpretation engine.
//! Only ordinal field paths and fixed vocabulary may leave this stdin boundary.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{self, Read},
};

#[derive(Default, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Answer {
    #[default]
    Unknown,
    Conflict,
    NotAssessed,
    Unresolved,
    Confirmed {
        value: String,
    },
}
impl Answer {
    fn state(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Conflict => "conflict",
            Self::NotAssessed => "not_assessed",
            Self::Unresolved => "unresolved",
            Self::Confirmed { .. } => "confirmed",
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Confirmed { value } => !value.trim().is_empty() && value.len() <= 4096,
            _ => true,
        }
    }
}
fn conflict() -> Answer {
    Answer::Conflict
}
fn not_assessed() -> Answer {
    Answer::NotAssessed
}
fn unresolved() -> Answer {
    Answer::Unresolved
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Cash,
    Card,
    Debt,
    Investment,
    ManualAsset,
    ManualLiability,
    AppleCard,
    Unknown,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    id: String,
    kind: Kind,
    #[serde(default)]
    institution: Answer,
    #[serde(default)]
    account: Answer,
    #[serde(default)]
    currencies: Answer,
    #[serde(default)]
    ownership: Answer,
    #[serde(default)]
    overlaps: Answer,
    #[serde(default)]
    exports: Answer,
    #[serde(default)]
    automatic_required: Answer,
    #[serde(default)]
    opening: Answer,
    #[serde(default)]
    inclusion: Answer,
}
impl Component {
    fn answers(&self) -> [(&'static str, &Answer); 9] {
        [
            ("institution", &self.institution),
            ("account", &self.account),
            ("currencies", &self.currencies),
            ("ownership", &self.ownership),
            ("overlaps", &self.overlaps),
            ("exports", &self.exports),
            ("automatic_required", &self.automatic_required),
            ("opening", &self.opening),
            ("inclusion", &self.inclusion),
        ]
    }
}
#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Route {
    Automatic,
    File,
    Manual,
    Unknown,
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Limitation {
    Unassessed,
    OwnerAccepted { detail: String },
}
impl Limitation {
    fn state(&self) -> &'static str {
        match self {
            Self::Unassessed => "unassessed",
            Self::OwnerAccepted { .. } => "owner_accepted",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    component_id: String,
    route: Route,
    limitation: Limitation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Meaning {
    #[serde(default)]
    timezone: Answer,
    #[serde(default)]
    reporting_currency: Answer,
    #[serde(default = "conflict")]
    source_precedence: Answer,
    #[serde(default = "not_assessed")]
    freshness_threshold: Answer,
    #[serde(default = "unresolved")]
    withholding: Answer,
    #[serde(default)]
    commitments: Answer,
}
impl Meaning {
    fn answers(&self) -> [(&'static str, &Answer); 6] {
        [
            ("timezone", &self.timezone),
            ("reporting_currency", &self.reporting_currency),
            ("source_precedence", &self.source_precedence),
            ("freshness_threshold", &self.freshness_threshold),
            ("withholding", &self.withholding),
            ("commitments", &self.commitments),
        ]
    }
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Profile {
    Synthetic,
    ActualHousehold,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema_version: u8,
    profile: Profile,
    inventory_manifest: Vec<String>,
    components: Vec<Component>,
    coverage: Vec<Coverage>,
    meaning: Meaning,
}
fn issue(field: String, state: &str, next_action: &str) -> Value {
    json!({"field":field,"state":state,"next_action":next_action})
}
fn assess(bytes: &[u8]) -> Result<(bool, Value), &'static str> {
    // serde's recursion limit and this byte bound apply before collection processing.
    let data: Inventory = serde_json::from_slice(bytes).map_err(|_| "Invalid inventory shape or value; use inventory-v1.json and retain explicit uncertainty.")?;
    if data.schema_version != 1 || data.components.is_empty() || data.components.len() > 1000 {
        return Err("Expected inventory version 1 with 1 to 1000 components.");
    }
    let manifest: BTreeSet<_> = data.inventory_manifest.iter().map(String::as_str).collect();
    let ids: BTreeSet<_> = data.components.iter().map(|c| c.id.as_str()).collect();
    let routes: BTreeSet<_> = data
        .coverage
        .iter()
        .map(|c| c.component_id.as_str())
        .collect();
    if manifest.len() != data.inventory_manifest.len()
        || ids.len() != data.components.len()
        || routes.len() != data.coverage.len()
        || manifest != ids
        || ids != routes
        || ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        return Err(
            "Inventory manifest, components and coverage must each contain every component exactly once with a local opaque identifier.",
        );
    }
    let mut unresolved_fields = Vec::new();
    let mut components = Vec::new();
    for (i, c) in data.components.iter().enumerate() {
        for (field, answer) in c.answers() {
            if !answer.valid() {
                return Err("Confirmed answers require nonblank text of at most 4096 bytes.");
            }
            if answer.state() != "confirmed" {
                unresolved_fields.push(issue(format!("components[{i}].{field}"),answer.state(),"Product owner: supply or explicitly resolve this component input privately; do not infer a value."));
            }
        }
        if let Answer::Confirmed { value } = &c.automatic_required
            && !matches!(value.as_str(), "yes" | "no")
        {
            return Err("automatic_required confirmed value must be yes or no.");
        }
        let (coverage_index, coverage) = data
            .coverage
            .iter()
            .enumerate()
            .find(|(_, r)| r.component_id == c.id)
            .ok_or("Missing coverage component.")?;
        match &coverage.limitation {
            Limitation::OwnerAccepted{detail} if detail.trim().is_empty() || detail.len()>4096 => return Err("An owner-accepted limitation requires nonblank scope/evidence text of at most 4096 bytes."),
            Limitation::Unassessed => unresolved_fields.push(issue(format!("coverage[{coverage_index}].limitation"),"unassessed","Product owner: explicitly assess and accept any route limitation; file/manual does not establish required automation.")),
            _=>{}
        }
        if coverage.route == Route::Unknown {
            unresolved_fields.push(issue(format!("coverage[{coverage_index}].route"),"unknown","Choose automatic, file, or manual acquisition after verifying feasibility; keep this component visible meanwhile."));
        }
        components.push(json!({"index":i,"kind":c.kind,"position":"unknown","route":coverage.route,"limitation":coverage.limitation.state()}));
    }
    for (field, answer) in data.meaning.answers() {
        if !answer.valid() {
            return Err("Confirmed meanings require nonblank text of at most 4096 bytes.");
        }
        if answer.state() != "confirmed" {
            unresolved_fields.push(issue(format!("meaning.{field}"),answer.state(),"Product owner: supply an explicit private decision and evidence, or retain the unresolved state."));
        }
    }
    Ok((
        data.profile == Profile::Synthetic,
        json!({
            "schema":"ProductionReadinessV1","schema_version":1,"inventory_status":"Passed",
            "components":components,"unresolved":unresolved_fields,
            "actual_household":{"status":"Blocked","required_input":"Verified private inventory and meaning decisions, plus separately qualified production prerequisites.","next_action":"Keep synthetic-only operation. Review private owner inputs and run the separate activation gates; this questionnaire never activates a dataset."},
            "limitations":["Syntax and coverage only: confirmed text records an owner answer, not verified financial meaning or consent.","No balances, credentials, provider access, deadlines or household activation are inferred.","Owner-accepted limitations cannot waive constitutional invariants or certify automatic coverage."]
        }),
    ))
}
/// No supplied paths, values, identifiers, or parse diagnostics enter logs or reports.
pub fn inspect_stdin() -> Result<(), String> {
    let mut bytes = Vec::new();
    let input = io::stdin().take(1_048_577).read_to_end(&mut bytes);
    let result = if input.is_err() || bytes.len() > 1_048_576 {
        Err("Inventory input unavailable or exceeds 1 MiB.")
    } else {
        assess(&bytes)
    };
    let (ok, report) = match result {
        Ok(result) => result,
        Err(message) => (
            false,
            json!({"schema":"ProductionReadinessV1","schema_version":1,"inventory_status":"Blocked","actual_household":{"status":"Blocked"},"unresolved":[issue("inventory".into(),"invalid",message)]}),
        ),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|_| "Cannot serialize readiness report")?
    );
    if ok {
        Ok(())
    } else {
        Err("BLOCKED: private household activation or input validation requires the actions in the redacted readiness report.".into())
    }
}
/// Synthetic contract qualification is deliberately separate from private activation.
pub fn qualify() -> Result<Value, String> {
    let assertions = ["cur_047_1", "cur_047_2", "cur_047_3"];
    let list = crate::checks::cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-service",
        "--test",
        "cur_047",
        "--",
        "--list",
    ])?;
    let found: BTreeSet<_> = list
        .lines()
        .filter_map(|s| s.strip_suffix(": test"))
        .collect();
    if found != assertions.into_iter().collect() {
        return Err(
            "BLOCKED: household-inputs requires exactly cur_047_1, cur_047_2 and cur_047_3".into(),
        );
    }
    let output = crate::checks::cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-service",
        "--test",
        "cur_047",
    ])?;
    if !output.contains("3 passed; 0 failed; 0 ignored") {
        return Err("BLOCKED: incomplete household-inputs execution".into());
    }
    use sha2::{Digest, Sha256};
    let mut inputs = Vec::new();
    for path in [
        "schemas/setup/inventory-v1.json",
        "fixtures/setup/synthetic-household/inventory.json",
    ] {
        let bytes = std::fs::read(crate::checks::root().join(path))
            .map_err(|_| "Missing synthetic setup contract input")?;
        inputs.push(json!({"path":path,"sha256":Sha256::digest(bytes).iter().map(|b|format!("{b:02x}")).collect::<String>()}));
    }
    Ok(
        json!({"assertions":assertions,"inputs":inputs,"actual_household":"Blocked","scope":"synthetic input contract only"}),
    )
}
