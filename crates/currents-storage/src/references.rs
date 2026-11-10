//! Role-specific identity/reference index. Provenance UUIDs are not record FKs.
use currents_types::*;
use serde_json::Value;

pub(crate) struct Definition {
    pub kind: String,
    pub id: String,
    pub version: i64,
    pub family: bool,
}
#[derive(Clone)]
pub(crate) struct Reference {
    pub kinds: Vec<String>,
    pub id: String,
    pub version: Option<i64>,
}
pub(crate) struct Content {
    pub digest: Digest,
    pub size: Option<i64>,
}
pub(crate) struct Metadata {
    pub kind: String,
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
    pub contents: Vec<Content>,
    unsupported: bool,
    pub prefixes: Vec<u64>,
}
const RECORDS: &[&str] = &[
    "DatasetPolicy",
    "UnitCatalog",
    "AccountComponent",
    "AccountAlias",
    "OwnershipPerimeter",
    "ImportReceipt",
    "ObservationVersion",
    "FinancialEvent",
    "FinancialLeg",
    "Association",
    "Decision",
    "Recognition",
    "AccountingSegment",
    "ReconciliationClaim",
    "Organization",
    "Rule",
    "Review",
    "Commitment",
    "Scenario",
    "ForecastRun",
];
const EVIDENCE: &[&str] = &["ObservationVersion", "ImportReceipt"];

/// Result effects use family existence, not changed-record or latest-version semantics.
/// The canonical commit carries them directly; they are not synthetic item rows.
pub(crate) fn command_effects(effects: &EffectsV1) -> impl Iterator<Item = Reference> + '_ {
    effects
        .records
        .iter()
        .chain(&effects.invalidations)
        .map(|id| Reference {
            kinds: RECORDS.iter().map(|kind| (*kind).into()).collect(),
            id: id.0.to_string(),
            version: None,
        })
        .chain(effects.accounts.iter().map(|id| Reference {
            kinds: vec!["Account".into()],
            id: id.0.to_string(),
            version: None,
        }))
        .chain(effects.amounts.iter().map(|amount| Reference {
            kinds: vec!["Unit".into()],
            id: amount.unit_id.0.to_string(),
            version: None,
        }))
}

impl Metadata {
    fn define(&mut self, kind: &str, id: &Value, version: i64, family: bool) {
        if let Some(id) = id.as_str() {
            self.definitions.push(Definition {
                kind: kind.into(),
                id: id.into(),
                version,
                family,
            });
        }
    }
    fn reference(&mut self, kinds: &[&str], id: &Value, version: Option<i64>) {
        if let Some(id) = id.as_str() {
            self.references.push(Reference {
                kinds: kinds.iter().map(|s| (*s).into()).collect(),
                id: id.into(),
                version,
            });
        }
    }
    fn many(&mut self, kinds: &[&str], v: &Value) {
        if let Some(values) = v.as_array() {
            for id in values {
                self.reference(kinds, id, None);
            }
        }
    }
    fn content(&mut self, v: &Value) -> Result<(), EngineError> {
        let r: ContentReferenceV1 =
            serde_json::from_value(v.clone()).map_err(|_| EngineError::invalid())?;
        self.contents.push(Content {
            digest: r.digest,
            size: Some(i64::try_from(r.byte_count.0).map_err(|_| EngineError::invalid())?),
        });
        Ok(())
    }
    fn policy(&mut self, v: &Value) {
        self.reference(&["DatasetPolicy"], &v["id"], v["version"].as_i64());
    }
    fn allocations(&mut self, v: &Value) {
        if let Some(values) = v.as_array() {
            for a in values {
                self.allocation(a);
            }
        }
    }
    fn allocation(&mut self, a: &Value) {
        self.reference(&["ObservationVersion"], &a["observation"], None);
        self.reference(&["Facet"], &scoped(&a["observation"], &a["facet"]), None);
    }
    fn interpretation(&mut self, v: &Value) {
        self.many(EVIDENCE, &v["evidence"]);
        if v["state"] == "Known" {
            self.many(EVIDENCE, &v["value"]["evidence"]);
        }
    }
    // Amount and account roles are stable across nested record values. Source identities,
    // actor/grant UUIDs and opaque text are deliberately never traversed as references.
    fn values(&mut self, v: &Value) {
        match v {
            Value::Object(map) => {
                if map.contains_key("through_commit") && map.contains_key("dataset_id") {
                    self.reference(&["Dataset"], &v["dataset_id"], None);
                    self.reference(
                        &["DatasetCatalog"],
                        &v["dataset_id"],
                        v["unit_catalog_version"].as_i64(),
                    );
                    if let Ok(commit) =
                        serde_json::from_value::<CommitSeq>(v["through_commit"].clone())
                    {
                        self.prefixes.push(commit.0);
                    }
                }
                if map.contains_key("coefficient") && map.contains_key("scale") {
                    self.reference(&["Unit"], &v["unit_id"], None);
                }
                for (k, x) in map {
                    if k == "account_id" || k == "account" {
                        self.reference(&["Account"], x, None);
                    }
                    self.values(x);
                }
            }
            Value::Array(v) => {
                for x in v {
                    self.values(x)
                }
            }
            _ => {}
        }
    }
    fn planned_legs(&mut self, v: &Value, commitment: &Value) {
        if let Some(legs) = v.as_array() {
            for leg in legs {
                self.define("PlannedLeg", &scoped(commitment, &leg["id"]), 0, true);
                self.settlement(&leg["settlement"]);
            }
        }
    }
    fn settlement(&mut self, v: &Value) {
        if v["kind"] == "HolidayCalendar" {
            // No versioned calendar definition exists in C02 yet.
            self.unsupported = true;
        }
    }
    fn commitment(&mut self, v: &Value) {
        self.planned_legs(&v["legs"], &v["id"]);
        self.settlement(&v["settlement"]);
        self.interpretation(&v["provenance"]);
        if let Some(ex) = v["exceptions"].as_array() {
            for e in ex {
                self.planned_legs(&e["legs"], &v["id"]);
            }
        }
    }
    fn occurrence_match(&mut self, v: &Value) {
        self.reference(&["Commitment"], &v["occurrence"]["commitment_id"], None);
        self.reference(
            &["PlannedLeg"],
            &scoped(&v["occurrence"]["commitment_id"], &v["planned_leg"]),
            None,
        );
        self.reference(&["FinancialLeg"], &v["economic_leg"], None);
    }
}
pub(crate) fn describe(record: &AuthoritativeRecordV1) -> Result<Metadata, EngineError> {
    let json = serde_json::to_value(record).map_err(|_| EngineError::invalid())?;
    let kind = json["kind"].as_str().ok_or_else(EngineError::invalid)?;
    let v = &json["value"];
    let mut m = Metadata {
        kind: kind.into(),
        definitions: vec![],
        references: vec![],
        contents: vec![],
        unsupported: false,
        prefixes: vec![],
    };
    if !v["id"].is_null() {
        m.define(kind, &v["id"], v["version"].as_i64().unwrap_or(0), false);
    }
    m.values(v);
    // Matching the enum, not only strings, makes a newly added variant a compile failure.
    match record {
        AuthoritativeRecordV1::Dataset(_) => {
            m.reference(&["UnitCatalog"], &v["unit_catalog"], None);
            m.reference(&["Unit"], &v["reporting_currency"], None);
        }
        AuthoritativeRecordV1::DatasetPolicy(_) => {
            m.reference(&["DatasetPolicy"], &v["opening_policy"], None);
            m.reference(&["DatasetPolicy"], &v["withholding_policy"], None);
        }
        AuthoritativeRecordV1::UnitCatalog(_) => {
            for u in v["units"].as_array().ok_or_else(EngineError::invalid)? {
                m.define("Unit", &u["id"], 0, true);
            }
        }
        AuthoritativeRecordV1::AccountComponent(_) => {
            m.define("Account", &v["account_id"], 0, true);
            m.reference(&["Unit"], &v["unit_id"], None);
            m.many(&["AccountComponent"], &v["overlapping_components"]);
        }
        AuthoritativeRecordV1::AccountAlias(_) | AuthoritativeRecordV1::OwnershipPerimeter(_) => {}
        AuthoritativeRecordV1::ImportReceipt(receipt) => {
            if receipt.complete && (receipt.truncated || receipt.result != ImportResult::Complete) {
                return Err(EngineError::invalid());
            }
            for c in v["contents"].as_array().ok_or_else(EngineError::invalid)? {
                m.content(c)?;
            }
        }
        AuthoritativeRecordV1::ObservationVersion(_) => {
            m.reference(&["ObservationVersion"], &v["prior_version"], None);
            m.reference(&["ObservationVersion"], &v["tombstone_of"], None);
            m.contents.push(Content {
                digest: serde_json::from_value(v["content_digest"].clone())
                    .map_err(|_| EngineError::invalid())?,
                size: None,
            });
        }
        AuthoritativeRecordV1::ParseProduct(_) => {
            m.reference(&["ObservationVersion"], &v["source_version_id"], None);
            for facet in v["facets"].as_array().ok_or_else(EngineError::invalid)? {
                m.define(
                    "Facet",
                    &scoped(&v["source_version_id"], &facet["lineage_key"]),
                    0,
                    true,
                );
                m.many(&["Association"], &facet["relationships"]);
            }
        }
        AuthoritativeRecordV1::FinancialEvent(_) => {
            m.many(&["FinancialLeg"], &v["legs"]);
            m.many(&["Association"], &v["relationships"]);
            m.interpretation(&v["interpretation"]);
        }
        AuthoritativeRecordV1::FinancialLeg(_) => {
            m.reference(&["FinancialEvent"], &v["event_id"], None);
            m.many(&["Association"], &v["relationships"]);
            m.allocations(&v["allocations"]);
            m.interpretation(&v["interpretation"]);
        }
        AuthoritativeRecordV1::Association(_) => {
            m.many(&["FinancialLeg"], &v["legs"]);
            m.reference(&["FinancialLeg"], &v["survivor"], None);
            m.many(&["Recognition"], &v["corrections"]);
            m.allocations(&v["allocations"]);
        }
        AuthoritativeRecordV1::Allocation(_) => m.allocation(v),
        AuthoritativeRecordV1::Decision(_) => {
            m.many(RECORDS, &v["references"]);
            m.many(RECORDS, &v["adopted_result"]);
            m.many(&["Decision"], &v["corrections"]);
            m.many(&["Decision"], &v["supersedes"]);
            m.policy(&v["policy"]);
        }
        AuthoritativeRecordV1::Recognition(_) => {
            m.many(&["FinancialLeg"], &v["leg_ids"]);
            m.allocations(&v["evidence_allocations"]);
            // C02/C08 have not defined an addressable date-policy payload.
            m.unsupported = true;
            for posting in v["postings"].as_array().ok_or_else(EngineError::invalid)? {
                m.define("Posting", &posting["id"], 0, false);
            }
        }
        AuthoritativeRecordV1::AccountingSegment(_) => {
            m.reference(&["AccountComponent"], &v["component"], None);
            m.reference(&["AccountingSegment"], &v["prior_segment"], None);
            m.reference(&["Recognition"], &v["opening"], None);
            m.many(&["Recognition"], &v["included_activity"]);
            m.many(&["Recognition"], &v["replacement_closure"]);
            m.many(&["Account"], &v["approved_accounts"]);
        }
        AuthoritativeRecordV1::ReconciliationClaim(_) => {
            m.many(&["Account"], &v["scope"]);
            m.many(EVIDENCE, &v["evidence_dependencies"]);
            m.many(&["Decision"], &v["decision_dependencies"]);
            m.many(RECORDS, &v["exclusions"]);
            m.allocations(&v["matched_allocations"]);
        }
        AuthoritativeRecordV1::Organization(_) => {
            m.many(RECORDS, &v["targets"]);
            m.reference(&["Organization"], &v["parent"], None);
            m.reference(&["Organization"], &v["supersedes"], None);
            m.allocations(&v["allocations"]);
        }
        AuthoritativeRecordV1::Rule(_) => m.many(&["Account"], &v["accounts"]),
        AuthoritativeRecordV1::Review(_) => m.many(RECORDS, &v["targets"]),
        AuthoritativeRecordV1::Commitment(_) => m.commitment(v),
        AuthoritativeRecordV1::OccurrenceMatch(_) => m.occurrence_match(v),
        AuthoritativeRecordV1::Scenario(_) => {
            m.many(&["Commitment"], &v["commitments"]);
            m.reference(&["Dataset"], &v["basis"]["dataset_id"], None);
        }
        AuthoritativeRecordV1::ForecastRun(_) => {
            let req = &v["request"];
            m.reference(&["Dataset"], &req["basis"]["dataset_id"], None);
            m.reference(&["Scenario"], &req["scenario"], None);
            if let Some(versions) = req["commitment_versions"].as_array() {
                for x in versions {
                    m.reference(&["Commitment"], &x["id"], x["version"].as_i64());
                }
            }
            if let Some(versions) = req["assumption_versions"].as_array() {
                for x in versions {
                    m.reference(
                        &["Assumption"],
                        &scoped(&v["id"], &x["id"]),
                        x["version"].as_i64(),
                    );
                }
            }
            let expanded = &v["expanded_inputs"];
            if let Some(cs) = expanded["commitments"].as_array() {
                for c in cs {
                    m.reference(&["Commitment"], &c["id"], c["version"].as_i64());
                    m.commitment(c);
                }
            }
            if let Some(cs) = expanded["occurrences"].as_array() {
                for c in cs {
                    m.reference(&["Commitment"], &c["key"]["commitment_id"], None);
                    m.planned_legs(&c["legs"], &c["key"]["commitment_id"]);
                }
            }
            if let Some(a) = expanded["assumptions"].as_array() {
                for x in a {
                    m.define(
                        "Assumption",
                        &scoped(&v["id"], &x["id"]),
                        x["version"].as_i64().unwrap_or(0),
                        false,
                    );
                    m.reference(&["Organization"], &x["category"], None);
                    m.many(&["FinancialLeg"], &x["exclusions"]);
                    m.interpretation(&x["provenance"]);
                }
            }
            if let Some(ps) = expanded["starting_positions"].as_array() {
                for p in ps {
                    m.many(RECORDS, &p["selected_basis"]);
                    if let Some(ms) = p["inclusion_map"].as_array() {
                        for x in ms {
                            m.occurrence_match(x);
                        }
                    }
                }
            }
            if let Some(ms) = expanded["matches"].as_array() {
                for x in ms {
                    m.occurrence_match(x);
                }
            }
            if let Some(outputs) = v["outputs"].as_array() {
                for output in outputs {
                    m.many(&["Account"], &output["unknown_accounts"]);
                    m.many(&["Account"], &output["omitted_accounts"]);
                    m.many(RECORDS, &output["matching_conflicts"]);
                    if let Some(matches) = output["inclusion_maps"].as_array() {
                        for item in matches {
                            m.occurrence_match(item);
                        }
                    }
                }
            }
            m.content(&v["calendar_data"]["timezone_data"])?;
        }
    }
    if m.unsupported {
        return Err(EngineError::version());
    }
    Ok(m)
}

fn scoped(parent: &Value, id: &Value) -> Value {
    match (parent.as_str(), id.as_str()) {
        (Some(parent), Some(id)) => Value::String(format!("{parent}/{id}")),
        _ => Value::Null,
    }
}
