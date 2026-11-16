//! Explicit synthetic source grammar. Descriptions carry no financial authority.
use crate::*;
choices!(RelationshipFacetRoleV1 {
    Movement,
    GrossPay,
    Withholding,
    Retirement,
    HealthSavings,
    NetPay,
    Fee
});
choices!(RelationshipSourceStatusV1 {
    Pending,
    Posted,
    Expired,
    Tombstone
});
dto!(RelationshipSourceFacetV1 {
    lineage: String,
    role: RelationshipFacetRoleV1,
    group: String,
    amount: String,
    unit: String
});
dto!(RelationshipSourceRowV1 {
    source_record: String,
    source_revision: String,
    prior_revision: Option<String>,
    status: RelationshipSourceStatusV1,
    effective: String,
    facets: Vec<RelationshipSourceFacetV1>
});

choices!(FacetAllocationModeV1 {
    Additive,
    Corroborating
});
dto!(QuantifiedFacetAllocationV1 {
    source: SourceFacetBindingV1,
    leg: LegId,
    allocated: Amount,
    mode: FacetAllocationModeV1
});
/// Nonauthoritative arithmetic result, never a persisted ownership decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceAllocationResidualV1 {
    pub source_root: RecordId,
    pub facet: RecordId,
    pub remaining: Amount,
}

// Immutable complete-binding snapshot. C04 family and financial admission are
// integrated separately before publication; this DTO alone grants no authority.
dto!(AssociationRevisionV1 {
    id: RecordId,
    schema_version: u32,
    root: RecordId,
    prior_revision: Option<RecordId>,
    kind: RelationshipKind,
    legs: Vec<LegId>,
    allocations: Vec<QuantifiedFacetAllocationV1>,
    captured_allocations: Vec<CapturedFacetAllocationV1>,
    recognitions: Vec<RecognitionId>,
    residuals: Vec<Amount>,
    superseded_adoptions: Vec<RecordId>,
    survivor: Option<LegId>,
    retired_recognitions: Vec<RecognitionId>,
    interpretation: InterpretationV1,
    governing_policy: PolicyReferenceV1
});

fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn decimal(value: &str) -> Result<(), EngineError> {
    if value.is_empty() || value.len() > 58 {
        return Err(EngineError::precision());
    }
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let mut parts = unsigned.split('.');
    let whole = parts.next().ok_or_else(EngineError::precision)?;
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction
            .is_some_and(|f| f.is_empty() || f.len() > 18 || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(EngineError::precision());
    }
    let coefficient = value
        .replace('.', "")
        .parse::<i128>()
        .map_err(|_| EngineError::precision())?;
    if coefficient == i128::MIN {
        return Err(EngineError::precision());
    }
    Ok(())
}

impl RelationshipSourceRowV1 {
    /// Shared pure DTO bounds; no source precedence or financial interpretation.
    pub fn validate_all(rows: &[Self]) -> Result<(), EngineError> {
        use std::collections::BTreeSet;
        if rows.is_empty() || rows.len() > 1024 {
            return Err(EngineError::invalid());
        }
        let mut versions = BTreeSet::new();
        let mut total = 0usize;
        let date_format = ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
            .map_err(|_| EngineError::invalid())?;
        for row in rows {
            if !key(&row.source_record)
                || !key(&row.source_revision)
                || row
                    .prior_revision
                    .as_ref()
                    .is_some_and(|p| !key(p) || *p == row.source_revision)
                || !versions.insert((&row.source_record, &row.source_revision))
                || row.facets.len() > 256
                || ::time::Date::parse(&row.effective, &date_format).is_err()
            {
                return Err(EngineError::invalid());
            }
            total = total
                .checked_add(row.facets.len())
                .ok_or_else(EngineError::invalid)?;
            if total > 16384 {
                return Err(EngineError::invalid());
            }
            match row.status {
                RelationshipSourceStatusV1::Pending | RelationshipSourceStatusV1::Posted
                    if !row.facets.is_empty() => {}
                RelationshipSourceStatusV1::Expired | RelationshipSourceStatusV1::Tombstone
                    if row.facets.is_empty() && row.prior_revision.is_some() => {}
                _ => return Err(EngineError::invalid()),
            }
            let mut facets = BTreeSet::new();
            for facet in &row.facets {
                if !key(&facet.lineage)
                    || !key(&facet.group)
                    || !facets.insert(&facet.lineage)
                    || facet.unit.len() != 3
                    || !facet.unit.bytes().all(|b| b.is_ascii_uppercase())
                {
                    return Err(EngineError::invalid());
                }
                decimal(&facet.amount)?;
            }
        }
        Ok(())
    }
}

// Descriptive retained capsule. It is not an adopted economic binding.
dto!(RelationshipCaptureV1 {
    id: RecordId,
    schema_version: u32,
    receipt_id: RecordId,
    source: SourceIdentityV1,
    original_capture_time: RecordedInstant,
    raw: ContentReferenceV1,
    rows: Vec<RelationshipSourceRowV1>
});
#[derive(Clone)]
pub struct RelationshipCaptureBatchV1 {
    pub receipt: ImportReceiptV1,
    pub capture: RelationshipCaptureV1,
    pub contents: Vec<EvidenceContentV1>,
}
pub const RELATIONSHIP_SOURCE_MEDIA: &str =
    "application/vnd.currents.synthetic-relationships.v1+json";

dto!(CapturedFacetBindingV1 {
    capture: RecordId,
    source_record: String,
    source_revision: String,
    facet: String
});
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCapturedFacetV1 {
    pub fact_key: String,
    pub amount: Amount,
    pub role: RelationshipFacetRoleV1,
    pub group: String,
    pub effective: TemporalValue,
    pub status: RelationshipSourceStatusV1,
}
impl RelationshipSourceFacetV1 {
    pub fn exact_amount(&self, unit: &UnitDefinition) -> Result<Amount, EngineError> {
        unit.validate()?;
        decimal(&self.amount)?;
        if unit.native_code != self.unit {
            return Err(EngineError::invalid());
        }
        let scale = u8::try_from(self.amount.split('.').nth(1).map_or(0, str::len))
            .map_err(|_| EngineError::precision())?;
        let coefficient = self
            .amount
            .replace('.', "")
            .parse::<i128>()
            .map_err(|_| EngineError::precision())?;
        let amount = Amount::new(coefficient, scale, unit.id)?;
        amount.checked_rescale(unit.exponent)?;
        amount.normalized()
    }
}

dto!(CapturedFacetAllocationV1 {
    source: CapturedFacetBindingV1,
    leg: LegId,
    allocated: Amount,
    mode: FacetAllocationModeV1
});
dto!(CompleteAssociationRequestV1 { revision:AssociationRevisionV1, events:Vec<FinancialEventV1>, legs:Vec<FinancialLegV1>, recognitions:Vec<RecognitionV1> });
