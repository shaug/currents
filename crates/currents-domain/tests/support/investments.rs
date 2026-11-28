//! Synthetic securities and investment accounts for CUR-021.
//!
//! Unit USD has exponent 2. Security SECA has source identifier `SYN-0001`
//! from source `synthetic-broker`, display symbol `SYNA` and quantity
//! exponent 4. Brokerage account X holds cash account B (external, USD,
//! the recognition fixture's account 10); X itself is not a record.
//! Position P links SECA, quantity account Q and carrying account K. Fee
//! expense E is the recognition fixture's expense account 30.
use super::fixture::*;
use currents_accounting::AccountingAdapter;
use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
use currents_domain::{
    ReplayEngine, associations::IdentityEngine, catalog::CatalogEngine,
    investments::InvestmentEngine, recognition::RecognitionEngine,
};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub const BROKER: &str = "synthetic-broker";
pub const OTHER_SOURCE: &str = "synthetic-aggregator";
pub const SECA: u32 = 6000;
pub const SECA_UNIT: u32 = 6001;
pub const SECB: u32 = 6010;
pub const SECB_UNIT: u32 = 6011;
/// External quantity account Q, in SECA.
pub const Q: u32 = 6100;
/// Carrying-amount account K, in USD.
pub const K: u32 = 6110;
/// Quantity control in SECA.
pub const QUANTITY_CONTROL: u32 = 6120;
/// Disposal clearing in USD.
pub const DISPOSAL_CLEARING: u32 = 6130;
/// An external account in USD.
pub const USD_HOLDING: u32 = 6140;
/// An external quantity account in SECB.
pub const QB: u32 = 6150;
/// Cash account B, in USD.
pub const B: u32 = 10;
/// Fee expense E, in USD.
pub const FEE_EXPENSE: u32 = 30;
/// Position P.
pub const P: u32 = 6200;

pub fn unit_id(n: u32) -> UnitId {
    UnitId(id(n).0)
}

pub fn account_id(n: u32) -> AccountId {
    AccountId(id(n).0)
}

pub fn source_id(source: &str, identifier: &str) -> SecuritySourceIdV1 {
    SecuritySourceIdV1 {
        source: source.into(),
        identifier: identifier.into(),
    }
}

/// A new security with its own unit, declared by catalog record `n + 2`.
pub fn declaration(n: u32, code: &str, identifier: &str, exponent: u8) -> SecurityDeclarationV1 {
    SecurityDeclarationV1 {
        security: SecurityV1 {
            id: id(n),
            schema_version: 1,
            root: id(n),
            prior_revision: None,
            unit_id: unit_id(n + 1),
            source_identifiers: vec![source_id(BROKER, identifier)],
            display_symbol: Some("SYNA".into()),
            local_mappings: vec![],
        },
        unit: UnitDefinition {
            id: unit_id(n + 1),
            exponent,
            native_code: code.into(),
            kind: UnitKind::Security,
        },
        unit_catalog: id(n + 2),
    }
}

pub fn seca() -> SecurityDeclarationV1 {
    declaration(SECA, "SCA", "SYN-0001", 4)
}

pub fn secb() -> SecurityDeclarationV1 {
    declaration(SECB, "SCB", "SYN-0002", 4)
}

/// An account `n` in unit `unit`, with component `n + 1` and revision `n + 2`.
pub fn account(n: u32, unit: UnitId, origin: AccountOriginV1) -> CatalogRequestV1 {
    let (class, sign) = match &origin {
        AccountOriginV1::External => (AccountClass::Asset, NormalSign::Debit),
        AccountOriginV1::Internal(control) => match control.role {
            ControlRoleV1::IncomeFlow => (AccountClass::Income, NormalSign::Credit),
            ControlRoleV1::ExpenseFlow => (AccountClass::Expense, NormalSign::Debit),
            ControlRoleV1::Clearing
            | ControlRoleV1::TaxPrepayment
            | ControlRoleV1::InvestmentCarrying => (AccountClass::Asset, NormalSign::Debit),
            ControlRoleV1::UnresolvedEquity
            | ControlRoleV1::OpeningEquity
            | ControlRoleV1::QuantityControl
            | ControlRoleV1::DisposalClearing => (AccountClass::Equity, NormalSign::Credit),
        },
    };
    let mut request = catalog();
    request.revision_id = id(n + 2);
    request.catalog.component_id = id(n + 1);
    request.catalog.account_id = account_id(n);
    request.catalog.account_class = class;
    request.catalog.normal_sign = sign;
    request.catalog.unit_id = unit;
    request.catalog.display_name = format!("Synthetic account {n}");
    request.ownership.account_id = account_id(n);
    request.origin = origin;
    request
}

pub fn control(role: ControlRoleV1) -> AccountOriginV1 {
    AccountOriginV1::Internal(InternalAccountV1 {
        role,
        created_at: date("2026-01-01"),
    })
}

/// Apply `plan` and declare its metadata in the native index, as a
/// committed metadata declaration would.
pub fn commit_metadata(state: &mut DomainState, plan: &DecisionPlan) {
    apply(state, plan);
    let AccountingIntent::MetadataDeclaration(declared) = &plan.accounting else {
        return;
    };
    let index = state.accounting.as_mut().unwrap();
    for unit in &declared.units {
        index.units.insert(unit.id, unit.clone());
    }
    for account in &declared.accounts {
        index.accounts.insert(
            account.account_id,
            NativeAccountMappingV1 {
                account_id: account.account_id,
                unit_id: account.unit_id,
                account_class: account.account_class,
                path: format!("fixture/{}", account.account_id.0),
            },
        );
    }
}

/// The recognition fixture's monetary state with SECA declared.
pub fn with_seca() -> DomainState {
    let mut state = monetary_state();
    let plan = InvestmentEngine::declare_security(&state, &seca()).unwrap();
    commit_metadata(&mut state, &plan);
    state
}

/// SECA, quantity account Q, carrying account K, quantity control and
/// disposal clearing, each committed.
pub fn world() -> DomainState {
    let mut state = with_seca();
    for request in [
        account(Q, unit_id(SECA_UNIT), AccountOriginV1::External),
        account(K, unit_id(1), control(ControlRoleV1::InvestmentCarrying)),
        account(
            QUANTITY_CONTROL,
            unit_id(SECA_UNIT),
            control(ControlRoleV1::QuantityControl),
        ),
        account(
            DISPOSAL_CLEARING,
            unit_id(1),
            control(ControlRoleV1::DisposalClearing),
        ),
    ] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        commit_metadata(&mut state, &plan);
    }
    state
}

pub fn position(n: u32, security: u32, quantity: u32, carrying: u32) -> PositionV1 {
    PositionV1 {
        id: id(n),
        schema_version: 1,
        security: id(security),
        quantity_account: account_id(quantity),
        carrying_account: account_id(carrying),
    }
}

/// `amount` whole units of `unit`.
pub fn units(amount: i128, unit: u32) -> Amount {
    Amount::new(amount, 0, unit_id(unit)).unwrap()
}

/// Source evidence `n` for a movement of `amount`, in its own unit.
pub fn quantity_evidence(state: &mut DomainState, n: u32, amount: Amount) -> SourceFacetBindingV1 {
    let mut binding = evidence(state, n, 1, None);
    binding.amount = amount;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get_mut(&binding.parse_product)
    else {
        panic!("evidence product")
    };
    product.product.facets[0].amount = SourceValue::Known(amount);
    binding
}

/// USD as the recognition fixture declares it.
pub fn usd() -> UnitDefinition {
    UnitDefinition {
        id: unit_id(1),
        exponent: 2,
        native_code: "USD".into(),
        kind: UnitKind::Currency,
    }
}

/// Each row of `csv`, imported through the `SyntheticCsv` profile in `unit`
/// and placed in `state`, by original ID. Receipt `n` precedes the rows;
/// row `i` has observation `n + 1 + 3i`, then its product and facet.
pub fn import(
    state: &mut DomainState,
    csv: &str,
    unit: UnitDefinition,
    n: u32,
) -> BTreeMap<String, SourceFacetBindingV1> {
    let rows = u32::try_from(csv.lines().count() - 1).unwrap();
    let context = ImportContextV1 {
        receipt_id: id(n),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(102),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-11T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-12T13:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: (0..rows)
            .map(|i| ImportRowIdsV1 {
                observation: id(n + 1 + 3 * i),
                product: id(n + 2 + 3 * i),
                facet: id(n + 3 + 3 * i),
            })
            .collect(),
    };
    let profile = ParserProfileV1 {
        schema_version: 1,
        format: ImportFormatV1::SyntheticCsv,
        parser_version: "1".into(),
        unit,
    };
    let input = BoundedEvidence::new("synthetic.csv", csv.as_bytes().to_vec(), context).unwrap();
    let batch = FileParser::parse(input, profile).unwrap();
    state.records.insert(
        batch.receipt.id,
        AuthoritativeRecordV1::ImportReceipt(batch.receipt.clone()),
    );
    let mut bindings = BTreeMap::new();
    for (observation, product) in batch.observations.into_iter().zip(batch.products) {
        let facet = &product.product.facets[0];
        let SourceValue::Known(amount) = facet.amount else {
            panic!("a known amount")
        };
        // The parser lists the file and row identities, then the row's own.
        bindings.insert(
            observation.original_ids.last().unwrap().clone(),
            SourceFacetBindingV1 {
                observation: observation.id,
                parse_product: product.id,
                facet: facet.lineage_key,
                amount,
            },
        );
        state.records.insert(
            observation.id,
            AuthoritativeRecordV1::ObservationVersion(observation),
        );
        state.records.insert(
            product.id,
            AuthoritativeRecordV1::AddressedParseProduct(product),
        );
    }
    bindings
}

/// The imported trade rows by original ID: USD rows on B, SECA rows on Q.
pub struct Rows {
    pub cash: BTreeMap<String, SourceFacetBindingV1>,
    pub quantity: BTreeMap<String, SourceFacetBindingV1>,
}

/// `world()` with position P declared and the synthetic trade rows imported.
pub fn trading() -> (DomainState, Rows) {
    let mut state = world();
    let plan = InvestmentEngine::declare_position(&state, &position(P, SECA, Q, K)).unwrap();
    commit_metadata(&mut state, &plan);
    // Frozen entries name each account by its native path.
    for mapping in state.accounting.as_mut().unwrap().accounts.values_mut() {
        mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
    }
    let cash = import(
        &mut state,
        include_str!("../../../../fixtures/investments/quantity/cash-b.csv"),
        usd(),
        8000,
    );
    let quantity = import(
        &mut state,
        include_str!("../../../../fixtures/investments/quantity/quantity-q.csv"),
        seca().unit,
        8100,
    );
    (state, Rows { cash, quantity })
}

/// The date the imported row `binding` reports.
pub fn row_date(state: &DomainState, binding: &SourceFacetBindingV1) -> TemporalValue {
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get(&binding.parse_product)
    else {
        panic!("an imported row")
    };
    product.product.facets[0].temporal.clone()
}

/// Posting `n` of `amount` to account `account` on `date`.
fn posting(n: u32, account: u32, amount: Amount, date: &TemporalValue) -> PostingV1 {
    PostingV1 {
        id: id(n),
        account_id: account_id(account),
        amount,
        effective: date.clone(),
        posting: date.clone(),
    }
}

/// Trade request `n` on position P with a fee of `fee` whole USD. Its IDs
/// start at `n`: the trade, its event, the cash leg, the quantity leg, the
/// recognition, its lineage, then its postings. A known cash side posts B
/// against K for a buy, or against E and disposal clearing for a sale. A
/// known quantity side posts Q against quantity control.
pub fn trade(
    state: &DomainState,
    n: u32,
    side: TradeSideV1,
    cash: Option<&SourceFacetBindingV1>,
    quantity: Option<&SourceFacetBindingV1>,
    fee: i128,
) -> TradeRequestV1 {
    let mut adopted = interpretation();
    adopted.evidence = cash
        .iter()
        .chain(&quantity)
        .map(|b| b.observation)
        .collect();
    let leg = |k: u32, binding: &SourceFacetBindingV1, account: u32| AdoptedLegV1 {
        leg: FinancialLegV1 {
            id: LegId(id(n + k).0),
            event_id: id(n + 1),
            account_id: account_id(account),
            movement: binding.amount,
            allocations: vec![EvidenceAllocationV1 {
                observation: binding.observation,
                facet: binding.facet,
                amount: binding.amount,
            }],
            effective: row_date(state, binding),
            posting: row_date(state, binding),
            relationships: vec![],
            unresolved_residual: SourceValue::Absent,
            interpretation: SourceValue::Known(adopted.clone()),
        },
        sources: vec![binding.clone()],
    };
    let cash = cash.map(|b| leg(2, b, B));
    let quantity = quantity.map(|b| leg(3, b, Q));
    let fee = amount(fee);
    let mut postings = Vec::new();
    let mut post = |account: u32, amount: Amount, date: &TemporalValue| {
        let n = n + 6 + u32::try_from(postings.len()).unwrap();
        postings.push(posting(n, account, amount, date));
    };
    if let Some(cash) = &cash {
        let (movement, date) = (cash.leg.movement, &cash.leg.effective);
        post(B, movement, date);
        match side {
            TradeSideV1::Buy => post(K, movement.checked_neg().unwrap(), date),
            TradeSideV1::Sell => {
                if fee.coefficient != 0 {
                    post(FEE_EXPENSE, fee, date);
                }
                let gross = movement.checked_add(&fee).unwrap();
                post(DISPOSAL_CLEARING, gross.checked_neg().unwrap(), date);
            }
        }
    }
    if let Some(quantity) = &quantity {
        let (movement, date) = (quantity.leg.movement, &quantity.leg.effective);
        post(Q, movement, date);
        post(QUANTITY_CONTROL, movement.checked_neg().unwrap(), date);
    }
    let bookkeeping = cash
        .iter()
        .chain(&quantity)
        .map(|l| l.leg.effective.clone())
        .next()
        .unwrap_or_else(|| date("2026-11-03"));
    let known = |leg: Option<AdoptedLegV1>| leg.map_or(SourceValue::Missing, SourceValue::Known);
    TradeRequestV1 {
        schema_version: 1,
        id: id(n),
        position: id(P),
        side,
        event: FinancialEventV1 {
            id: id(n + 1),
            legs: cash.iter().chain(&quantity).map(|l| l.leg.id).collect(),
            relationships: vec![],
            interpretation: SourceValue::Known(adopted.clone()),
        },
        cash: known(cash),
        quantity: known(quantity),
        fee,
        terms: RecognitionTermsV1 {
            recognition_id: RecognitionId(id(n + 4).0),
            lineage: id(n + 5),
            postings,
            bookkeeping_date: bookkeeping,
            date_policy: PolicyReferenceV1 {
                id: id(103),
                version: 1,
            },
            unresolved_account: None,
            opening: None,
        },
        interpretation: adopted,
        governing_policy: PolicyReferenceV1 {
            id: id(104),
            version: 1,
        },
    }
}

/// One proposed commit with the exact frozen operations its manifest names.
#[derive(Clone)]
pub struct Candidate {
    pub commit: DomainCommitV1,
    pub bytes: Vec<u8>,
    pub batch: AccountingBatchV1,
}

/// The commit after `state` that holds `records` and the manifest of `batch`.
pub fn candidate(
    state: &DomainState,
    mut records: Vec<AuthoritativeRecordV1>,
    batch: AccountingBatchV1,
) -> Candidate {
    let seq = CommitSeq(state.through.0 + 1);
    let n = u32::try_from(seq.0).unwrap();
    let manifest = AccountingAdapter::manifest(id(9500 + n), &batch).unwrap();
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(manifest));
    let mut commit = DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![],
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest([u8::try_from(n).unwrap(); 32]),
        },
        command_result: CommandResultV1 {
            command_id: CommandId(id(9600 + n).0),
            digest: Digest([9; 32]),
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
        digest: Digest([0; 32]),
    };
    commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&commit).unwrap()).into());
    let bytes = AccountingAdapter::instruction_bytes(&batch).unwrap();
    Candidate {
        commit,
        bytes,
        batch,
    }
}

/// `plan` frozen against `state` and proposed as the next commit.
pub fn planned(state: &DomainState, plan: &DecisionPlan) -> Candidate {
    let batch = AccountingAdapter::freeze(&plan.accounting, state).unwrap();
    candidate(state, plan.records.clone(), batch)
}

/// Replay one commit as the service does: domain re-admission, the native
/// index and its correspondence, then the audit of the whole projection.
pub fn admit(state: &DomainState, c: &Candidate) -> Result<DomainState, EngineError> {
    let mut projection = state.clone();
    ReplayEngine::apply(&mut projection, &c.commit)?;
    let mut native = state.clone();
    AccountingAdapter::advance_index(&mut native, &c.commit, &c.bytes)?;
    let index = native.accounting.ok_or_else(EngineError::invalid)?;
    AccountingAdapter::verify_correspondence(&c.commit, Some(&c.batch), &index)?;
    projection.accounting = Some(index);
    RecognitionEngine::audit(&projection)?;
    IdentityEngine::audit(&projection)?;
    Ok(projection)
}

/// Replay refuses `c`, and `state` stays at its prior head.
pub fn rejected(state: &DomainState, c: &Candidate) {
    let before = (state.through, state.domain_digest, state.records.clone());
    assert!(admit(state, c).is_err());
    assert_eq!(
        (state.through, state.domain_digest, state.records.clone()),
        before
    );
}

/// `batch` with changed operations, its counts and digests bound again so
/// only the operations differ.
pub fn rebound(mut batch: AccountingBatchV1) -> AccountingBatchV1 {
    let count = CommitSeq(batch.operations.len() as u64);
    batch.result_expectations = AccountingExpectationsV1 {
        native_operation_count: count,
        financial_entry_count: count,
    };
    batch.decision_digest = AccountingAdapter::prebinding_digest(&batch).unwrap();
    for op in &mut batch.operations {
        if let FrozenNativeOperationV1::Entry(e) = op {
            e.generated_document = batch.decision_digest;
        }
    }
    batch
}

/// Native balances: every frozen posting of every recognition not reversed,
/// normalized.
pub fn balances(state: &DomainState) -> BTreeMap<AccountId, Amount> {
    let index = state.accounting.as_ref().unwrap();
    let mut totals = BTreeMap::new();
    for (id, entries) in &index.recognitions {
        if index.reversed.contains(id) {
            continue;
        }
        for posting in entries.iter().flat_map(|e| &e.postings) {
            let posting = &posting.original;
            let zero = Amount::new(0, 0, posting.amount.unit_id).unwrap();
            let total = totals.entry(posting.account_id).or_insert(zero);
            *total = total
                .checked_add(&posting.amount)
                .unwrap()
                .normalized()
                .unwrap();
        }
    }
    totals
}

/// `value` whole units of `unit`, normalized as `balances` reports it.
pub fn balance(value: i128, unit: u32) -> Amount {
    units(value, unit).normalized().unwrap()
}
