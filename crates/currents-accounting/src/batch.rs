//! Pure deterministic C05 freezing. Execution/authority remains C04.
use crate::{PreparedNativeOperation, catalog, dates, numbers, templates};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SERIALIZED_BYTES: usize = MAX_BYTES + 1_048_576;
const MEDIA: &str = "application/vnd.currents.frozen-operations-v1+json";
pub struct AccountingAdapter;
struct SizeLimit(usize);
impl std::io::Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_SERIALIZED_BYTES)
            .ok_or_else(|| std::io::Error::other("frozen instruction bound"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_size<T: serde::Serialize>(v: &T) -> Result<(), EngineError> {
    serde_json::to_writer(SizeLimit(0), v).map_err(|_| EngineError::invalid())
}
fn canonical<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, EngineError> {
    bounded_size(v)?;
    serde_json_canonicalizer::to_vec(v).map_err(|_| EngineError::invalid())
}
fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
impl AccountingAdapter {
    pub fn freeze(
        intent: &AccountingIntent,
        state: &DomainState,
    ) -> Result<AccountingBatchV1, EngineError> {
        bounded_size(intent)?;
        let index = state.accounting.as_ref().ok_or_else(EngineError::invalid)?;
        if index.prefix.through != state.through || index.prefix.native_head.semantics_version != 1
        {
            return Err(EngineError::invalid());
        }
        let mut operations = vec![];
        match intent {
            AccountingIntent::NoEffect(_) => {}
            AccountingIntent::MetadataDeclaration(v) => {
                if !index.calendar_declared {
                    operations.push(FrozenNativeOperationV1::Calendar(NativeCalendarV1 {
                        start: "0001-01-01".into(),
                        end: "9999-12-31".into(),
                    }));
                    operations.push(FrozenNativeOperationV1::Policy(NativePolicyV1 {
                        effective: "0001-01-01".into(),
                    }));
                }
                let mut units = index.units.clone();
                let mut accounts = index.accounts.clone();
                for unit in &v.units {
                    catalog::check_unit(unit, &units)?;
                    units.entry(unit.id).or_insert_with(|| {
                        operations.push(FrozenNativeOperationV1::Unit(unit.clone()));
                        unit.clone()
                    });
                }
                for account in &v.accounts {
                    let mapped = catalog::mapping(account)?;
                    if !units.contains_key(&account.unit_id) {
                        return Err(EngineError::invalid());
                    }
                    if let Some(old) = accounts.get(&account.account_id) {
                        if old != &mapped {
                            return Err(EngineError::version());
                        }
                    } else {
                        accounts.insert(account.account_id, mapped.clone());
                        operations.push(FrozenNativeOperationV1::Account(mapped));
                    }
                }
            }
            AccountingIntent::Recognition(r) => {
                operations.push(FrozenNativeOperationV1::Entry(Self::recognition(r, state)?))
            }
            AccountingIntent::Correction(c) => {
                if index.reversed.contains(&c.active_recognition) {
                    return Err(EngineError::invalid());
                }
                let original = index
                    .recognitions
                    .get(&c.active_recognition)
                    .ok_or_else(EngineError::invalid)?;
                let correction = digest(&canonical(c)?);
                for (position, old) in original.iter().enumerate() {
                    let mut reversal = old.clone();
                    reversal.key = format!(
                        "reverse/{}/{position}",
                        correction
                            .0
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    );
                    reversal.reverses = Some(old.key.clone());
                    reversal.generated_document = Digest([0; 32]);
                    for p in &mut reversal.postings {
                        p.original.amount = p.original.amount.checked_neg()?;
                        p.native_coefficient =
                            numbers::native_amount(&p.original.amount, &p.unit)?.to_string();
                    }
                    operations.push(FrozenNativeOperationV1::Entry(reversal));
                }
                for r in &c.replacement {
                    operations.push(FrozenNativeOperationV1::Entry(Self::recognition(r, state)?));
                }
            }
        }
        let financial = operations
            .iter()
            .filter(|op| matches!(op, FrozenNativeOperationV1::Entry(_)))
            .count();
        let mut batch = AccountingBatchV1 {
            schema_version: 1,
            native_semantics: 1,
            result_expectations: AccountingExpectationsV1 {
                native_operation_count: CommitSeq(operations.len() as u64),
                financial_entry_count: CommitSeq(financial as u64),
            },
            operations,
            preconditions: index.prefix.clone(),
            decision_digest: Digest([0; 32]),
        };
        batch.decision_digest = Self::prebinding_digest(&batch)?;
        for op in &mut batch.operations {
            if let FrozenNativeOperationV1::Entry(e) = op {
                e.generated_document = batch.decision_digest;
            }
        }
        Self::validate_limits(&batch)?;
        Ok(batch)
    }
    fn recognition(r: &RecognitionV1, state: &DomainState) -> Result<FrozenEntryV1, EngineError> {
        let index = state.accounting.as_ref().ok_or_else(EngineError::invalid)?;
        if !index.calendar_declared || index.recognitions.contains_key(&r.id) {
            return Err(EngineError::invalid());
        }
        let mut policies = state.records.values().filter_map(|record| match record {
            AuthoritativeRecordV1::DateConversionPolicy(p)
                if p.id == r.date_policy.id && p.version == r.date_policy.version =>
            {
                Some(p)
            }
            _ => None,
        });
        let cached = index
            .date_policies
            .get(&(r.date_policy.id, r.date_policy.version));
        let record = policies.next();
        if policies.next().is_some() || (cached.is_some() && record.is_some() && cached != record) {
            return Err(EngineError::invalid());
        }
        let policy = cached.or(record).ok_or_else(EngineError::invalid)?.clone();
        let (booking_date, value_date) = dates::mapping(r, &policy)?;
        let mut postings = vec![];
        for p in &r.postings {
            let mapping = index
                .accounts
                .get(&p.account_id)
                .ok_or_else(EngineError::invalid)?;
            let unit = index
                .units
                .get(&p.amount.unit_id)
                .ok_or_else(EngineError::invalid)?;
            if mapping.unit_id != unit.id {
                return Err(EngineError::invalid());
            }
            let mut original = p.clone();
            original.amount = original.amount.normalized()?;
            postings.push(FrozenPostingV1 {
                original,
                native_coefficient: numbers::native_amount(&p.amount, unit)?.to_string(),
                path: mapping.path.clone(),
                unit: unit.clone(),
            });
        }
        Ok(FrozenEntryV1 {
            recognition_id: r.id,
            key: format!("recognition/{}", r.id.0),
            booking_date,
            value_date,
            postings,
            date_policy: policy,
            generated_document: Digest([0; 32]),
            reverses: None,
        })
    }
    pub fn prebinding_digest(batch: &AccountingBatchV1) -> Result<Digest, EngineError> {
        let mut operations =
            serde_json::to_value(&batch.operations).map_err(|_| EngineError::invalid())?;
        for op in operations.as_array_mut().ok_or_else(EngineError::invalid)? {
            if op["kind"] == "Entry" {
                op["value"]
                    .as_object_mut()
                    .ok_or_else(EngineError::invalid)?
                    .remove("generated_document");
            }
        }
        Ok(digest(&canonical(
            &serde_json::json!({"domain":"currents/accounting-prebinding/v1","schema_version":batch.schema_version,"native_semantics":batch.native_semantics,"preconditions":batch.preconditions,"operations":operations}),
        )?))
    }
    pub fn instruction_bytes(batch: &AccountingBatchV1) -> Result<Vec<u8>, EngineError> {
        canonical(&batch.operations)
    }
    pub fn validate_limits(batch: &AccountingBatchV1) -> Result<(), EngineError> {
        if batch.schema_version != 1
            || batch.native_semantics != 1
            || batch.preconditions.native_head.semantics_version != 1
        {
            return Err(EngineError::version());
        }
        if batch.operations.len() > 20_000
            || batch
                .operations
                .iter()
                .any(|op| matches!(op,FrozenNativeOperationV1::Entry(e) if e.postings.len()>1024))
            || Self::instruction_bytes(batch)?.len() > MAX_BYTES
            || Self::prebinding_digest(batch)? != batch.decision_digest
        {
            return Err(EngineError::invalid());
        }
        let financial = batch
            .operations
            .iter()
            .filter(|op| matches!(op, FrozenNativeOperationV1::Entry(_)))
            .count();
        if batch.result_expectations.native_operation_count.0 != batch.operations.len() as u64
            || batch.result_expectations.financial_entry_count.0 != financial as u64
            || (financial > 0 && financial != batch.operations.len())
        {
            return Err(EngineError::invalid());
        }
        let mut keys = BTreeSet::new();
        let mut units = BTreeMap::new();
        let mut accounts = BTreeSet::new();
        for op in &batch.operations {
            match op {
                FrozenNativeOperationV1::Calendar(v) => {
                    if v.start != "0001-01-01" || v.end != "9999-12-31" {
                        return Err(EngineError::invalid());
                    }
                }
                FrozenNativeOperationV1::Policy(v) => {
                    if v.effective != "0001-01-01" {
                        return Err(EngineError::invalid());
                    }
                }
                FrozenNativeOperationV1::Unit(v) => {
                    catalog::check_unit(v, &units)?;
                    if units.insert(v.id, v.clone()).is_some() {
                        return Err(EngineError::invalid());
                    }
                }
                FrozenNativeOperationV1::Account(v) => {
                    if v.path != format!("Currents:A{}", v.account_id.0.simple())
                        || !accounts.insert(v.account_id)
                    {
                        return Err(EngineError::invalid());
                    }
                }
                FrozenNativeOperationV1::Entry(e) => {
                    if e.generated_document != batch.decision_digest
                        || e.postings.len() < 2
                        || e.postings.len() > 1024
                        || !keys.insert(&e.key)
                    {
                        return Err(EngineError::invalid());
                    }
                    let mut totals: BTreeMap<UnitId, Vec<i128>> = BTreeMap::new();
                    let mut posting_ids = BTreeSet::new();
                    for p in &e.postings {
                        catalog::check_unit(&p.unit, &units)?;
                        units.insert(p.unit.id, p.unit.clone());
                        if p.path != format!("Currents:A{}", p.original.account_id.0.simple())
                            || !posting_ids.insert(p.original.id)
                        {
                            return Err(EngineError::invalid());
                        }
                        let n = numbers::native_amount(&p.original.amount, &p.unit)?;
                        if n == 0 || p.native_coefficient != n.to_string() {
                            return Err(EngineError::precision());
                        }
                        totals.entry(p.unit.id).or_default().push(n);
                    }
                    for values in totals.values() {
                        if numbers::checked_sum(values)? != 0 {
                            return Err(EngineError::invalid());
                        }
                    }
                    let original = RecognitionV1 {
                        id: e.recognition_id,
                        lineage: RecordId(e.recognition_id.0),
                        postings: e.postings.iter().map(|p| p.original.clone()).collect(),
                        kind: RecognitionKind::Activity,
                        evidence_allocations: vec![],
                        leg_ids: vec![],
                        scope: DateScope {
                            start: None,
                            end: None,
                            basis: DateBasis::Effective,
                        },
                        bookkeeping_date: TemporalValue::Date {
                            civil: dates::parse(&e.booking_date)?,
                            original_text: String::new(),
                        },
                        date_policy: PolicyReferenceV1 {
                            id: e.date_policy.id,
                            version: e.date_policy.version,
                        },
                    };
                    let mapped = dates::mapping(&original, &e.date_policy)?;
                    if mapped != (e.booking_date.clone(), e.value_date.clone()) {
                        return Err(EngineError::invalid());
                    }
                    templates::entry(e)?;
                }
            }
        }
        Ok(())
    }
    pub fn prepare_operations(
        batch: &AccountingBatchV1,
    ) -> Result<Vec<PreparedNativeOperation>, EngineError> {
        Self::validate_limits(batch)?;
        batch.operations.iter().map(templates::prepare).collect()
    }
    pub fn manifest(
        id: RecordId,
        batch: &AccountingBatchV1,
    ) -> Result<FrozenAccountingManifestV1, EngineError> {
        Self::validate_limits(batch)?;
        let bytes = Self::instruction_bytes(batch)?;
        Ok(FrozenAccountingManifestV1 {
            id,
            schema_version: 1,
            native_semantics: 1,
            instructions: ContentReferenceV1 {
                digest: digest(&bytes),
                byte_count: CommitSeq(bytes.len() as u64),
                media_type: MEDIA.into(),
            },
            preconditions: batch.preconditions.clone(),
            decision_digest: batch.decision_digest,
            result_expectations: batch.result_expectations.clone(),
        })
    }
    pub fn from_manifest(
        manifest: &FrozenAccountingManifestV1,
        bytes: &[u8],
    ) -> Result<AccountingBatchV1, EngineError> {
        if bytes.len() > MAX_BYTES
            || manifest.instructions.byte_count.0 != bytes.len() as u64
            || manifest.instructions.digest != digest(bytes)
            || manifest.instructions.media_type != MEDIA
        {
            return Err(EngineError::invalid());
        }
        let operations = serde_json::from_slice(bytes).map_err(|_| EngineError::invalid())?;
        let batch = AccountingBatchV1 {
            schema_version: manifest.schema_version,
            native_semantics: manifest.native_semantics,
            operations,
            preconditions: manifest.preconditions.clone(),
            decision_digest: manifest.decision_digest,
            result_expectations: manifest.result_expectations.clone(),
        };
        if Self::instruction_bytes(&batch)? != bytes {
            return Err(EngineError::invalid());
        }
        Self::validate_limits(&batch)?;
        Ok(batch)
    }
    pub fn advance_index(
        state: &mut DomainState,
        commit: &DomainCommitV1,
        bytes: &[u8],
    ) -> Result<(), EngineError> {
        let mut index = state.accounting.clone().ok_or_else(EngineError::invalid)?;
        if state.through != index.prefix.through
            || commit.seq.0
                != state
                    .through
                    .0
                    .checked_add(1)
                    .ok_or_else(EngineError::invalid)?
            || commit.previous_digest != index.prefix.domain_digest
            || commit.native_head_after.semantics_version != 1
        {
            return Err(EngineError::invalid());
        }
        let mut preimage = commit.clone();
        preimage.digest = Digest([0; 32]);
        if digest(&canonical(&preimage)?) != commit.digest {
            return Err(EngineError::invalid());
        }
        let mut manifests = commit.records.iter().filter_map(|v| {
            if let AuthoritativeRecordV1::FrozenAccountingManifest(m) = v {
                Some(m)
            } else {
                None
            }
        });
        if let Some(m) = manifests.next() {
            if manifests.next().is_some() || m.preconditions != index.prefix {
                return Err(EngineError::invalid());
            }
            let batch = Self::from_manifest(m, bytes)?;
            if batch.operations.is_empty() && commit.native_head_after != index.prefix.native_head {
                return Err(EngineError::invalid());
            }
            for op in batch.operations {
                match op {
                    FrozenNativeOperationV1::Calendar(_) => {
                        if index.calendar_declared {
                            return Err(EngineError::invalid());
                        }
                        index.calendar_declared = true;
                    }
                    FrozenNativeOperationV1::Policy(_) => {}
                    FrozenNativeOperationV1::Unit(v) => {
                        catalog::check_unit(&v, &index.units)?;
                        index.units.insert(v.id, v);
                    }
                    FrozenNativeOperationV1::Account(v) => {
                        if !index.units.contains_key(&v.unit_id)
                            || index
                                .accounts
                                .get(&v.account_id)
                                .is_some_and(|old| old != &v)
                        {
                            return Err(EngineError::invalid());
                        }
                        index.accounts.insert(v.account_id, v);
                    }
                    FrozenNativeOperationV1::Entry(e) => {
                        let declared = index
                            .date_policies
                            .get(&(e.date_policy.id, e.date_policy.version))
                            .or_else(|| {
                                state.records.values().find_map(|v| match v {
                                    AuthoritativeRecordV1::DateConversionPolicy(p)
                                        if p.id == e.date_policy.id
                                            && p.version == e.date_policy.version =>
                                    {
                                        Some(p)
                                    }
                                    _ => None,
                                })
                            });
                        if declared != Some(&e.date_policy) {
                            return Err(EngineError::invalid());
                        }
                        for p in &e.postings {
                            if index.units.get(&p.unit.id) != Some(&p.unit)
                                || index
                                    .accounts
                                    .get(&p.original.account_id)
                                    .is_none_or(|a| a.path != p.path || a.unit_id != p.unit.id)
                            {
                                return Err(EngineError::invalid());
                            }
                        }
                        if let Some(key) = &e.reverses {
                            let originals = index
                                .recognitions
                                .get(&e.recognition_id)
                                .ok_or_else(EngineError::invalid)?;
                            let old = originals
                                .iter()
                                .find(|old| &old.key == key)
                                .ok_or_else(EngineError::invalid)?;
                            if e.booking_date != old.booking_date
                                || e.value_date != old.value_date
                                || e.date_policy != old.date_policy
                                || e.postings.len() != old.postings.len()
                            {
                                return Err(EngineError::invalid());
                            }
                            for (p, old) in e.postings.iter().zip(&old.postings) {
                                let mut expected = old.clone();
                                expected.original.amount =
                                    expected.original.amount.checked_neg()?;
                                expected.native_coefficient = numbers::native_amount(
                                    &expected.original.amount,
                                    &expected.unit,
                                )?
                                .to_string();
                                if p != &expected {
                                    return Err(EngineError::invalid());
                                }
                            }
                            if !index.reversed.insert(e.recognition_id) {
                                return Err(EngineError::invalid());
                            }
                        } else if index
                            .recognitions
                            .insert(e.recognition_id, vec![e])
                            .is_some()
                        {
                            return Err(EngineError::invalid());
                        }
                    }
                }
            }
        } else if !bytes.is_empty() || commit.native_head_after != index.prefix.native_head {
            return Err(EngineError::invalid());
        }
        for record in &commit.records {
            if let AuthoritativeRecordV1::DateConversionPolicy(p) = record {
                if p.version == 0
                    || index
                        .date_policies
                        .get(&(p.id, p.version))
                        .is_some_and(|old| old != p)
                {
                    return Err(EngineError::invalid());
                }
                index.date_policies.insert((p.id, p.version), p.clone());
            }
        }
        index.prefix = AccountingPrefixV1 {
            through: commit.seq,
            domain_digest: commit.digest,
            native_head: commit.native_head_after.clone(),
        };
        state.through = commit.seq;
        state.accounting = Some(index);
        Ok(())
    }
}
