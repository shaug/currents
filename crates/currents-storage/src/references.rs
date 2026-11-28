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
    "DatasetPolicyRevision",
    "DateConversionPolicy",
    "FrozenAccountingManifest",
    "UnitCatalog",
    "AccountComponent",
    "AccountCatalogRevision",
    "ActivityAdoption",
    "PendingExpiry",
    "AccountAlias",
    "OwnershipPerimeter",
    "ImportReceipt",
    "ObservationVersion",
    "AddressedParseProduct",
    "FinancialEvent",
    "FinancialLeg",
    "Association",
    "AssociationRevision",
    "RelationshipCapture",
    "RelationshipParseRevision",
    "Decision",
    "Recognition",
    "RecognitionCorrectionRevision",
    "AccountingSegment",
    "SegmentReemission",
    "ReconciliationClaim",
    "ReconciliationDisposition",
    "Organization",
    "Rule",
    "Review",
    "Commitment",
    "Scenario",
    "ForecastRun",
    "Security",
    "Position",
    "Trade",
    "HoldingsSnapshot",
];
const EVIDENCE: &[&str] = &[
    "ObservationVersion",
    "ImportReceipt",
    "AddressedParseProduct",
    "RelationshipCapture",
    "RelationshipParseRevision",
];

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
    fn adopted_source(&mut self, source: &Value) {
        self.reference(&["ObservationVersion"], &source["observation"], None);
        self.reference(&["AddressedParseProduct"], &source["parse_product"], None);
        self.reference(
            &["Facet"],
            &scoped(&source["observation"], &source["facet"]),
            None,
        );
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
    fn commitment(&mut self, v: &Value) -> Result<(), EngineError> {
        self.content(&v["calendar_data"]["timezone_data"])?;
        if !v["supersession"].is_null() {
            self.reference(
                &["Commitment"],
                &v["id"],
                v["supersession"]["prior_version"].as_i64(),
            );
        }
        self.planned_legs(&v["legs"], &v["id"]);
        self.settlement(&v["settlement"]);
        self.interpretation(&v["provenance"]);
        if let Some(ex) = v["exceptions"].as_array() {
            for e in ex {
                self.planned_legs(&e["legs"], &v["id"]);
            }
        }
        Ok(())
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
/// Describe `item`, one of a commit's `items`. Each pending adoption is
/// consumed once. A leg correction's new adoption takes over the consumption
/// of the adoption it retires: in the commit of the `ReplacedLegs` revision
/// that names it, it refers to the existing consumption instead of defining
/// it again.
pub(crate) fn describe_in(
    items: &[AuthoritativeRecordV1],
    item: &AuthoritativeRecordV1,
) -> Result<Metadata, EngineError> {
    let mut m = describe(item)?;
    let AuthoritativeRecordV1::ActivityAdoption(a) = item else {
        return Ok(m);
    };
    let replaces = |record: &AuthoritativeRecordV1| {
        matches!(record, AuthoritativeRecordV1::RecognitionCorrectionRevision(r)
            if matches!(r.remap_disposition,
                RecognitionCorrectionRemapV1::ReplacedLegs { replacement_adoption, .. }
                    if replacement_adoption == a.id))
    };
    if a.prior_pending.is_some() && items.iter().any(replaces) {
        let at = m
            .definitions
            .iter()
            .position(|d| d.kind == "PendingConsumption")
            .ok_or_else(EngineError::invalid)?;
        let consumption = m.definitions.remove(at);
        m.references.push(Reference {
            kinds: vec![consumption.kind],
            id: consumption.id,
            version: Some(consumption.version),
        });
    }
    Ok(m)
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
        AuthoritativeRecordV1::AccountCatalogRevision(r) => {
            if r.schema_version != 1 {
                return Err(EngineError::version());
            }
            if r.id.0.is_nil()
                || r.id == r.component_id
                || r.prior_revision == Some(r.id)
                || r.component_id != r.catalog.component_id
                || r.account_id != r.catalog.account_id
            {
                return Err(EngineError::invalid());
            }
            m.reference(&["AccountComponent"], &v["component_id"], None);
            m.define(
                "CatalogComponentRevision",
                &scoped(&v["component_id"], &v["id"]),
                0,
                false,
            );
            m.reference(
                &["CatalogComponentRevision"],
                &scoped(&v["component_id"], &v["prior_revision"]),
                None,
            );
        }
        AuthoritativeRecordV1::RelationshipParseRevision(p) => {
            if p.schema_version != 1 || p.correction.schema_version != 1 {
                return Err(EngineError::version());
            }
            RelationshipSourceRowV1::validate_all(std::slice::from_ref(&p.row))?;
            if p.id.0.is_nil()
                || p.id == p.receipt_id
                || p.id == p.capture
                || p.profile.media_type != RELATIONSHIP_CORRECTION_MEDIA
                || p.profile.byte_count.0 > 1_048_576
            {
                return Err(EngineError::invalid());
            }
            m.reference(&["ImportReceipt"], &v["receipt_id"], None);
            m.reference(&["RelationshipCapture"], &v["capture"], None);
            if p.prior_parse.is_some() {
                m.reference(&["RelationshipParseRevision"], &v["prior_parse"], None);
            }
            m.content(&v["profile"])?;
        }
        AuthoritativeRecordV1::RelationshipCapture(capture) => {
            if capture.schema_version != 1 {
                return Err(EngineError::version());
            }
            RelationshipSourceRowV1::validate_all(&capture.rows)?;
            if capture.id.0.is_nil()
                || capture.id == capture.receipt_id
                || capture.raw.media_type != RELATIONSHIP_SOURCE_MEDIA
                || capture.raw.byte_count.0 > 1_048_576
            {
                return Err(EngineError::invalid());
            }
            m.reference(&["ImportReceipt"], &v["receipt_id"], None);
            m.content(&v["raw"])?;
        }
        AuthoritativeRecordV1::AssociationRevision(revision) => {
            if revision.schema_version != 1 {
                return Err(EngineError::version());
            }
            if revision.id.0.is_nil()
                || revision.root.0.is_nil()
                || revision.legs.is_empty()
                || revision.legs.len() > 1024
                || revision.captured_allocations.len() > 16384
                || revision.recognitions.len() > 1024
                || revision.superseded_adoptions.len() > 1024
                || revision.residuals.len() > 1024
                || revision.allocations.len() > 16384
                || revision.retired_recognitions.len() > 16384
            {
                return Err(EngineError::invalid());
            }
            match revision.prior_revision {
                None if revision.id == revision.root => {
                    m.define("AssociationRoot", &v["root"], 0, false);
                    m.define(
                        "AssociationRootKind",
                        &scoped(&v["root"], &v["kind"]),
                        0,
                        false,
                    );
                }
                Some(prior) if prior != revision.id && revision.id != revision.root => {
                    m.reference(
                        &["AssociationFamilyRevision"],
                        &scoped(&v["root"], &v["prior_revision"]),
                        None,
                    );
                    m.define("AssociationSuccessor", &v["prior_revision"], 0, false);
                }
                _ => return Err(EngineError::invalid()),
            }
            m.reference(&["AssociationRoot"], &v["root"], None);
            m.reference(
                &["AssociationRootKind"],
                &scoped(&v["root"], &v["kind"]),
                None,
            );
            m.define(
                "AssociationFamilyRevision",
                &scoped(&v["root"], &v["id"]),
                0,
                false,
            );
            m.many(&["FinancialLeg"], &v["legs"]);
            m.reference(&["FinancialLeg"], &v["survivor"], None);
            m.many(&["Recognition"], &v["retired_recognitions"]);
            m.many(&["Recognition"], &v["recognitions"]);
            m.many(&["ActivityAdoption"], &v["superseded_adoptions"]);
            for edge in v["captured_allocations"]
                .as_array()
                .ok_or_else(EngineError::invalid)?
            {
                m.reference(&["FinancialLeg"], &edge["leg"], None);
                m.reference(&["RelationshipCapture"], &edge["source"]["capture"], None);
                if !edge["source"]["parse_revision"].is_null() {
                    m.reference(
                        &["RelationshipParseRevision"],
                        &edge["source"]["parse_revision"],
                        None,
                    );
                }
            }

            for edge in v["allocations"]
                .as_array()
                .ok_or_else(EngineError::invalid)?
            {
                m.reference(&["FinancialLeg"], &edge["leg"], None);
                m.adopted_source(&edge["source"]);
            }
            m.policy(&v["governing_policy"]);
            m.interpretation(&v["interpretation"]);
        }
        AuthoritativeRecordV1::PendingExpiry(expiry) => {
            if expiry.schema_version != 1 {
                return Err(EngineError::version());
            }
            if expiry.id.0.is_nil()
                || expiry.pending_adoption.0.is_nil()
                || expiry.id == expiry.pending_adoption
            {
                return Err(EngineError::invalid());
            }
            m.reference(&["PendingAdoption"], &v["pending_adoption"], None);
            m.define("PendingConsumption", &v["pending_adoption"], 0, false);
            m.policy(&v["governing_policy"]);
            m.interpretation(&v["interpretation"]);
        }
        AuthoritativeRecordV1::ActivityAdoption(a) => {
            if a.schema_version != 1 {
                return Err(EngineError::version());
            }
            if a.id.0.is_nil()
                || a.legs.is_empty()
                || a.legs.len() > 1024
                || a.facets.len() > 16384
                || a.legs
                    .iter()
                    .any(|leg| !a.facets.iter().any(|f| f.leg_id == *leg))
                || a.facets.iter().any(|f| !a.legs.contains(&f.leg_id))
            {
                return Err(EngineError::invalid());
            }
            match a.status {
                ActivityStatusV1::Pending => {
                    if a.recognition.is_some()
                        || a.prior_pending.is_some()
                        || a.opening.is_some()
                        || a.unresolved_account.is_some()
                    {
                        return Err(EngineError::invalid());
                    }
                    m.define("PendingAdoption", &v["id"], 0, false);
                }
                ActivityStatusV1::Posted => {
                    if a.recognition.is_none() {
                        return Err(EngineError::invalid());
                    }
                    m.reference(&["Recognition"], &v["recognition"], None);
                    if a.prior_pending.is_some() {
                        m.reference(&["PendingAdoption"], &v["prior_pending"], None);
                        m.define("PendingConsumption", &v["prior_pending"], 0, false);
                    }
                }
            }
            m.reference(&["FinancialEvent"], &v["event_id"], None);
            m.many(&["FinancialLeg"], &v["legs"]);
            for leg in v["legs"].as_array().ok_or_else(EngineError::invalid)? {
                m.define("AdoptedFinancialLeg", leg, 0, false);
            }
            for facet in v["facets"].as_array().ok_or_else(EngineError::invalid)? {
                m.adopted_source(&facet["source"]);
            }
            m.policy(&v["governing_policy"]);
            m.interpretation(&v["interpretation"]);
            m.reference(&["Account"], &v["unresolved_account"], None);
            if let Some(opening) = &a.opening {
                if a.prior_pending.is_some() || opening.included_activity.len() > 4096 {
                    return Err(EngineError::invalid());
                }
                let opening = &v["opening"];
                m.reference(&["AccountingSegment"], &opening["segment_id"], None);
                m.reference(&["AccountComponent"], &opening["component_id"], None);
                m.adopted_source(&opening["evidence"]);
                for source in opening["included_activity"]
                    .as_array()
                    .ok_or_else(EngineError::invalid)?
                {
                    m.adopted_source(source);
                }
            }
        }
        AuthoritativeRecordV1::DateConversionPolicy(p) => {
            if p.version == 0 {
                return Err(EngineError::version());
            }
        }
        AuthoritativeRecordV1::FrozenAccountingManifest(manifest) => {
            if manifest.schema_version != 1
                || manifest.native_semantics != 1
                || manifest.instructions.byte_count.0 > 67_108_864
                || manifest.result_expectations.native_operation_count.0 > 20_000
            {
                return Err(EngineError::invalid());
            }
            m.content(&v["instructions"])?;
            m.prefixes.push(manifest.preconditions.through.0);
        }
        AuthoritativeRecordV1::Dataset(_) => {
            m.reference(&["UnitCatalog"], &v["unit_catalog"], None);
            m.reference(&["Unit"], &v["reporting_currency"], None);
        }
        AuthoritativeRecordV1::DatasetPolicy(_) => {
            m.reference(&["DatasetPolicy"], &v["opening_policy"], None);
            m.reference(&["DatasetPolicy"], &v["withholding_policy"], None);
        }
        AuthoritativeRecordV1::DatasetPolicyRevision(_) => {
            m.reference(&["DatasetPolicy"], &v["policy_id"], None);
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
        AuthoritativeRecordV1::AddressedParseProduct(product) => {
            product.validate()?;
            let v = &v["product"];
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
            m.reference(
                &["DateConversionPolicy"],
                &v["date_policy"]["id"],
                v["date_policy"]["version"].as_i64(),
            );
            for posting in v["postings"].as_array().ok_or_else(EngineError::invalid)? {
                m.define("Posting", &posting["id"], 0, false);
            }
        }
        AuthoritativeRecordV1::RecognitionCorrectionRevision(r) => {
            if r.schema_version != r.remap_disposition.schema_version() {
                return Err(EngineError::version());
            }
            if let RecognitionCorrectionRemapV1::ReinterpretedStableLegs(_) = &r.remap_disposition {
                let new = &v["remap_disposition"]["ReinterpretedStableLegs"];
                m.policy(&new["governing_policy"]);
                m.reference(&["Account"], &new["unresolved_account"], None);
                m.many(&["Account"], &new["changed_accounts"]);
            }
            if let RecognitionCorrectionRemapV1::ReplacedLegs {
                retired_adoption,
                replacement_adoption,
            } = &r.remap_disposition
            {
                if retired_adoption == replacement_adoption {
                    return Err(EngineError::invalid());
                }
                let replaced = &v["remap_disposition"]["ReplacedLegs"];
                m.reference(&["ActivityAdoption"], &replaced["retired_adoption"], None);
                m.reference(
                    &["ActivityAdoption"],
                    &replaced["replacement_adoption"],
                    None,
                );
                m.define("ReplacedAdoption", &replaced["retired_adoption"], 0, false);
            }
            if r.prior_recognition == r.replacement_recognition || r.reason.trim().is_empty() {
                return Err(EngineError::invalid());
            }
            m.reference(&["Recognition"], &v["prior_recognition"], None);
            m.reference(&["Recognition"], &v["replacement_recognition"], None);
            m.define(
                "RecognitionCorrectionSuccessor",
                &v["prior_recognition"],
                0,
                false,
            );
        }
        AuthoritativeRecordV1::AccountingSegment(_) => {
            m.reference(&["AccountComponent"], &v["component"], None);
            m.reference(&["AccountingSegment"], &v["prior_segment"], None);
            m.reference(&["Recognition"], &v["opening"], None);
            m.many(&["Recognition"], &v["included_activity"]);
            m.many(&["Recognition"], &v["replacement_closure"]);
            m.many(&["Account"], &v["approved_accounts"]);
        }
        AuthoritativeRecordV1::SegmentReemission(r) => {
            if r.schema_version != 1 {
                return Err(EngineError::version());
            }
            if r.original == r.replacement {
                return Err(EngineError::invalid());
            }
            m.reference(&["AccountingSegment"], &v["segment"], None);
            m.reference(&["Recognition"], &v["original"], None);
            m.reference(&["Recognition"], &v["replacement"], None);
            m.define("SegmentReemissionOriginal", &v["original"], 0, false);
        }
        AuthoritativeRecordV1::ReconciliationClaim(_) => {
            m.many(&["Account"], &v["scope"]);
            m.many(EVIDENCE, &v["evidence_dependencies"]);
            m.many(&["Decision"], &v["decision_dependencies"]);
            m.many(
                &[
                    "Recognition",
                    "FinancialLeg",
                    "AssociationRevision",
                    "AccountingSegment",
                ],
                &v["accounting_dependencies"],
            );
            m.reference(&["Unit"], &v["unit"], None);
            m.many(RECORDS, &v["exclusions"]);
            m.allocations(&v["matched_allocations"]);
        }
        AuthoritativeRecordV1::Security(security) => {
            if security.schema_version != 1 {
                return Err(EngineError::version());
            }
            match security.prior_revision {
                None if security.id == security.root => {}
                Some(prior) if prior != security.id && security.id != security.root => {
                    m.reference(&["Security"], &v["root"], None);
                    m.reference(&["Security"], &v["prior_revision"], None);
                    m.define("SecuritySuccessor", &v["prior_revision"], 0, false);
                }
                _ => return Err(EngineError::invalid()),
            }
            m.reference(&["Unit"], &v["unit_id"], None);
        }
        AuthoritativeRecordV1::Position(position) => {
            if position.schema_version != 1 {
                return Err(EngineError::version());
            }
            m.reference(&["Security"], &v["security"], None);
            m.reference(&["Account"], &v["quantity_account"], None);
            m.reference(&["Account"], &v["carrying_account"], None);
        }
        AuthoritativeRecordV1::Trade(trade) => {
            if trade.schema_version != 1 {
                return Err(EngineError::version());
            }
            m.reference(&["Position"], &v["position"], None);
            m.reference(&["FinancialEvent"], &v["event_id"], None);
            m.reference(&["Recognition"], &v["recognition"], None);
            for side in [&v["cash"], &v["quantity"]] {
                if side["state"] == "Known" {
                    let facet = &side["value"];
                    m.reference(&["FinancialLeg"], &facet["leg_id"], None);
                    m.define("AdoptedFinancialLeg", &facet["leg_id"], 0, false);
                    m.adopted_source(&facet["source"]);
                }
            }
            m.policy(&v["governing_policy"]);
            m.interpretation(&v["interpretation"]);
        }
        AuthoritativeRecordV1::HoldingsSnapshot(snapshot) => {
            if snapshot.schema_version != 1 {
                return Err(EngineError::version());
            }
            m.reference(&["ImportReceipt"], &v["receipt"], None);
            m.many(&["Account"], &v["scope"]);
            m.reference(&["Unit"], &v["currency"], None);
            for holding in v["holdings"].as_array().ok_or_else(EngineError::invalid)? {
                m.reference(&["ObservationVersion"], &holding["observation"], None);
                m.reference(&["Security"], &holding["security"], None);
            }
        }
        AuthoritativeRecordV1::ReconciliationDisposition(_) => {
            m.reference(&["ReconciliationClaim"], &v["claim"], None);
        }
        AuthoritativeRecordV1::Organization(_) => {
            m.many(RECORDS, &v["targets"]);
            m.reference(&["Organization"], &v["parent"], None);
            m.reference(&["Organization"], &v["supersedes"], None);
            m.allocations(&v["allocations"]);
        }
        AuthoritativeRecordV1::Rule(_) => m.many(&["Account"], &v["accounts"]),
        AuthoritativeRecordV1::Review(_) => m.many(RECORDS, &v["targets"]),
        AuthoritativeRecordV1::Commitment(_) => m.commitment(v)?,
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
                    m.commitment(c)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_revision_references_its_dataset_policy_family() {
        let revision: DatasetPolicyRevisionV1 = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000901",
            "policy_id": "00000000-0000-0000-0000-000000000700",
            "version": 1,
            "prior_version": null,
            "effective_scope": {"start": "2026-11-01", "end": null, "basis": "Effective"},
            "ownership_perimeter": null,
            "source_priority": null,
            "freshness_thresholds": null,
            "timezone": null,
            "reporting_currency": null
        }))
        .unwrap();
        let m = describe(&AuthoritativeRecordV1::DatasetPolicyRevision(revision)).unwrap();
        assert!(m.references.iter().any(
            |r| r.kinds == ["DatasetPolicy"] && r.id == "00000000-0000-0000-0000-000000000700"
        ));
    }
}
