//! CUR-021 security and position declarations, trades and holdings
//! snapshots, using synthetic identities and exact public interfaces.
use currents_accounting::AccountingAdapter;
use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
use currents_domain::{
    ReplayEngine,
    associations::IdentityEngine,
    catalog::CatalogEngine,
    correction::CorrectionEngine,
    evidence::{EvidenceEngine, ValidatedEvidenceBatchV1},
    investments::{InvestmentEngine, holdings::SnapshotUnmappedV1, quantity::PositionHoldingsV1},
    recognition::RecognitionEngine,
    segments::{ReemissionV1, SegmentEngine, SegmentReplacementV1},
};
use currents_types::*;
use std::collections::BTreeMap;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;
#[path = "support/associations.rs"]
#[allow(dead_code)]
mod association_fixtures;
#[path = "support/investments.rs"]
#[allow(dead_code)]
mod investments;
#[path = "support/transfers.rs"]
#[allow(dead_code)]
mod transfer_fixtures;
use investments::*;

fn mapping(identifier: SecuritySourceIdV1) -> SecurityMappingV1 {
    SecurityMappingV1 {
        identifier,
        reason: "Owner confirmed the same instrument".into(),
    }
}

/// The current revision of `root`, revised as record `n`.
fn revision(state: &DomainState, root: u32, n: u32) -> SecurityV1 {
    let mut next = InvestmentEngine::current(state, id(root)).unwrap();
    next.prior_revision = Some(next.id);
    next.id = id(n);
    next
}

#[test]
fn cur_021_security_identity_requires_source_identifier() {
    let state = monetary_state();
    let plan = InvestmentEngine::declare_security(&state, &seca()).unwrap();
    let AccountingIntent::MetadataDeclaration(declared) = &plan.accounting else {
        panic!("a security declaration is metadata only")
    };
    assert!(declared.accounts.is_empty());
    let [unit] = declared.units.as_slice() else {
        panic!("one security unit")
    };
    assert_eq!(unit.kind, UnitKind::Security);
    assert_eq!(unit.exponent, 4);
    assert_eq!(unit.id, unit_id(SECA_UNIT));
    let [
        AuthoritativeRecordV1::UnitCatalog(catalog),
        AuthoritativeRecordV1::Security(security),
    ] = plan.records.as_slice()
    else {
        panic!("a unit catalog and a security record")
    };
    assert_eq!(catalog.units, vec![unit.clone()]);
    assert_eq!(security.unit_id, unit.id);
    assert_eq!(
        security.source_identifiers,
        vec![source_id(BROKER, "SYN-0001")]
    );
    assert!(plan.invalidations.is_empty());

    // A symbol alone is not identity.
    let mut symbol_only = seca();
    symbol_only.security.source_identifiers.clear();
    assert!(InvestmentEngine::declare_security(&state, &symbol_only).is_err());
    // A security unit is a security unit.
    let mut currency = seca();
    currency.unit.kind = UnitKind::Currency;
    assert!(InvestmentEngine::declare_security(&state, &currency).is_err());

    // The same symbol with another identifier is another security.
    let mut state = with_seca();
    let plan = InvestmentEngine::declare_security(&state, &secb()).unwrap();
    let AccountingIntent::MetadataDeclaration(declared) = &plan.accounting else {
        panic!("metadata only")
    };
    assert_eq!(declared.units[0].id, unit_id(SECB_UNIT));
    assert_ne!(unit_id(SECB_UNIT), unit_id(SECA_UNIT));
    commit_metadata(&mut state, &plan);
    let second = InvestmentEngine::current(&state, id(SECB)).unwrap();
    assert_eq!(second.display_symbol.as_deref(), Some("SYNA"));
    assert_eq!(second.unit_id, unit_id(SECB_UNIT));

    // SYN-0001 already identifies SECA.
    let mut joined = revision(&state, SECB, SECB + 3);
    joined
        .local_mappings
        .push(mapping(source_id(BROKER, "SYN-0001")));
    assert!(InvestmentEngine::revise_security(&state, &joined).is_err());
    let again = declaration(6020, "SCC", "SYN-0001", 4);
    assert!(InvestmentEngine::declare_security(&state, &again).is_err());
}

#[test]
fn cur_021_local_mapping_joins_sources() {
    let mut state = with_seca();
    let alt = source_id(OTHER_SOURCE, "ALT-77");
    // The other source reports SYNA as its symbol, but a symbol is not identity.
    assert_eq!(InvestmentEngine::resolve(&state, &alt).unwrap(), None);
    assert_eq!(
        InvestmentEngine::resolve(&state, &source_id(OTHER_SOURCE, "SYNA")).unwrap(),
        None
    );

    let mut joined = revision(&state, SECA, SECA + 3);
    joined.local_mappings.push(mapping(alt.clone()));
    let plan = InvestmentEngine::revise_security(&state, &joined).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert_eq!(
        plan.records,
        vec![AuthoritativeRecordV1::Security(joined.clone())]
    );
    commit_metadata(&mut state, &plan);
    let found = InvestmentEngine::resolve(&state, &alt).unwrap().unwrap();
    assert_eq!(found, joined);
    assert_eq!(found.root, id(SECA));
    assert_eq!(found.unit_id, unit_id(SECA_UNIT));
    assert_eq!(
        InvestmentEngine::resolve(&state, &source_id(BROKER, "SYN-0001")).unwrap(),
        Some(joined)
    );

    // A revision keeps every identifier and mapping and extends the head.
    let mut dropped = revision(&state, SECA, SECA + 4);
    dropped.local_mappings.clear();
    assert!(InvestmentEngine::revise_security(&state, &dropped).is_err());
    let mut fork = revision(&state, SECA, SECA + 4);
    fork.prior_revision = Some(id(SECA));
    assert!(InvestmentEngine::revise_security(&state, &fork).is_err());
    let mut moved = revision(&state, SECA, SECA + 4);
    moved.unit_id = unit_id(1);
    assert!(InvestmentEngine::revise_security(&state, &moved).is_err());
}

#[test]
fn cur_021_control_roles_follow_unit_kind() {
    let state = with_seca();
    let usd = unit_id(1);
    let seca_unit = unit_id(SECA_UNIT);
    for (n, role, unit) in [
        (6300, ControlRoleV1::QuantityControl, seca_unit),
        (6310, ControlRoleV1::InvestmentCarrying, usd),
        (6320, ControlRoleV1::DisposalClearing, usd),
    ] {
        let plan = CatalogEngine::set_complete(&state, &account(n, unit, control(role))).unwrap();
        let AccountingIntent::MetadataDeclaration(declared) = plan.accounting else {
            panic!("{role:?} declares metadata")
        };
        assert_eq!(declared.accounts[0].unit_id, unit);
    }
    let mut refused = vec![
        (ControlRoleV1::QuantityControl, usd),
        (ControlRoleV1::InvestmentCarrying, seca_unit),
        (ControlRoleV1::DisposalClearing, seca_unit),
    ];
    for role in [
        ControlRoleV1::OpeningEquity,
        ControlRoleV1::UnresolvedEquity,
        ControlRoleV1::IncomeFlow,
        ControlRoleV1::ExpenseFlow,
        ControlRoleV1::Clearing,
        ControlRoleV1::TaxPrepayment,
    ] {
        refused.push((role, seca_unit));
        // Each existing role keeps declaring in a currency.
        let plan = CatalogEngine::set_complete(&state, &account(6330, usd, control(role)));
        assert!(plan.is_ok(), "{role:?} in USD");
    }
    for (role, unit) in refused {
        assert!(
            CatalogEngine::set_complete(&state, &account(6330, unit, control(role))).is_err(),
            "{role:?}"
        );
    }
    // An external account may hold a security quantity.
    assert!(
        CatalogEngine::set_complete(&state, &account(Q, seca_unit, AccountOriginV1::External))
            .is_ok()
    );
}

#[test]
fn cur_021_position_links_security_accounts() {
    let mut state = world();
    let plan = InvestmentEngine::declare_security(&state, &secb()).unwrap();
    commit_metadata(&mut state, &plan);
    for request in [
        account(USD_HOLDING, unit_id(1), AccountOriginV1::External),
        account(QB, unit_id(SECB_UNIT), AccountOriginV1::External),
    ] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        commit_metadata(&mut state, &plan);
    }

    let linked = position(6200, SECA, Q, K);
    let plan = InvestmentEngine::declare_position(&state, &linked).unwrap();
    assert_eq!(
        plan.records,
        vec![AuthoritativeRecordV1::Position(linked.clone())]
    );
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));

    // The quantity account is in USD, or holds another security.
    for bad in [
        position(6201, SECA, USD_HOLDING, K),
        position(6202, SECA, QB, K),
    ] {
        assert!(InvestmentEngine::declare_position(&state, &bad).is_err());
    }
    // The carrying account is not a carrying-amount control account.
    assert!(InvestmentEngine::declare_position(&state, &position(6203, SECA, Q, 10)).is_err());

    commit_metadata(&mut state, &plan);
    // Q already belongs to SECA's position.
    assert!(InvestmentEngine::declare_position(&state, &position(6204, SECA, Q, K)).is_err());
}

#[test]
fn cur_021_ordinary_adoption_refuses_security_postings() {
    let mut state = world();
    let binding = quantity_evidence(&mut state, 200, units(10, SECA_UNIT));
    let mut request = posted(binding.clone(), 300, 40);
    request.legs[0].leg.account_id = account_id(Q);
    let terms = request.financial.as_mut().unwrap();
    terms.postings[0].account_id = account_id(Q);
    terms.unresolved_account = Some(account_id(40));
    assert!(RecognitionEngine::adopt(&state, &request).is_err());
    // Nor against quantity control in the same unit.
    let mut controlled = request.clone();
    let terms = controlled.financial.as_mut().unwrap();
    terms.postings[1].account_id = account_id(QUANTITY_CONTROL);
    terms.unresolved_account = None;
    assert!(RecognitionEngine::adopt(&state, &controlled).is_err());
    // Nor as pending exposure.
    let mut exposure = pending(binding, 300);
    exposure.legs[0].leg.account_id = account_id(Q);
    assert!(RecognitionEngine::adopt(&state, &exposure).is_err());

    // An ordinary 96 USD purchase on B still adopts.
    let purchase = evidence(&mut state, 400, -96, None);
    let plan = RecognitionEngine::adopt(&state, &posted(purchase, 500, 30)).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::Recognition(_)));

    transfer_family_refuses_security_legs();
}

/// A CUR-015 transfer family with legs in SECA is refused.
fn transfer_family_refuses_security_legs() {
    use association_fixtures::*;
    use transfer_fixtures::*;
    let (mut state, _) = transfer_request();
    // A USD transfer family against clearing account 80 still admits.
    let rows = cross_account_rows();
    let capture = retained_relationships(&mut state, 7000, rows.clone());
    let family = clearing_family(capture, &rows, 7100, &[(10, 0), (60, 1)]);
    assert!(IdentityEngine::associate(&state, &family).is_ok());

    // The same family moving SECA between two quantity accounts, against a
    // SECA clearing account the catalog no longer declares.
    let plan = InvestmentEngine::declare_security(&state, &seca()).unwrap();
    commit_metadata(&mut state, &plan);
    const Q2: u32 = 6160;
    const SECA_CLEARING: u32 = 6170;
    for n in [Q, Q2] {
        let request = account(n, unit_id(SECA_UNIT), AccountOriginV1::External);
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        commit_metadata(&mut state, &plan);
    }
    let clearing = account(SECA_CLEARING, unit_id(1), control(ControlRoleV1::Clearing));
    let mut plan = CatalogEngine::set_complete(&state, &clearing).unwrap();
    for record in &mut plan.records {
        match record {
            AuthoritativeRecordV1::AccountComponent(c) => c.unit_id = unit_id(SECA_UNIT),
            AuthoritativeRecordV1::AccountCatalogRevision(r) => {
                r.catalog.unit_id = unit_id(SECA_UNIT)
            }
            _ => {}
        }
    }
    let AccountingIntent::MetadataDeclaration(declared) = &mut plan.accounting else {
        panic!("metadata")
    };
    declared.accounts[0].unit_id = unit_id(SECA_UNIT);
    commit_metadata(&mut state, &plan);
    let mut seca_rows = serde_json::to_value(&rows).unwrap();
    for row in seca_rows.as_array_mut().unwrap() {
        row["facets"][0]["unit"] = "SCA".into();
        row["source_record"] = format!("seca-{}", row["source_record"].as_str().unwrap()).into();
    }
    let seca_rows: Vec<RelationshipSourceRowV1> = serde_json::from_value(seca_rows).unwrap();
    let capture = retained_relationships(&mut state, 7300, seca_rows.clone());
    let mut family = clearing_family(capture, &seca_rows, 7400, &[(Q, 0), (Q2, 1)]);
    let in_seca = |a: &mut Amount| a.unit_id = unit_id(SECA_UNIT);
    for leg in &mut family.legs {
        in_seca(&mut leg.movement);
    }
    for edge in &mut family.revision.captured_allocations {
        in_seca(&mut edge.allocated);
    }
    for residual in &mut family.revision.residuals {
        in_seca(residual);
    }
    for recognition in &mut family.recognitions {
        for posting in &mut recognition.postings {
            in_seca(&mut posting.amount);
            if posting.account_id == account_id(80) {
                posting.account_id = account_id(SECA_CLEARING);
            }
        }
    }
    assert!(IdentityEngine::associate(&state, &family).is_err());
}

/// A committed commit after `state` that holds `records`.
fn commit(state: &DomainState, records: Vec<AuthoritativeRecordV1>) -> DomainCommitV1 {
    let seq = CommitSeq(state.through.0 + 1);
    DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![],
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest([0; 32]),
        },
        digest: Digest([u8::try_from(seq.0).unwrap(); 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(id(9000 + u32::try_from(seq.0).unwrap()).0),
            digest: Digest([88; 32]),
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
    }
}

/// Replay refuses `records` after `state` and keeps the prior head.
fn refused(state: &DomainState, records: Vec<AuthoritativeRecordV1>) {
    let mut copy = state.clone();
    assert!(ReplayEngine::apply(&mut copy, &commit(state, records)).is_err());
    assert_eq!(copy.through, state.through);
    assert_eq!(copy.domain_digest, state.domain_digest);
    assert_eq!(copy.records, state.records);
}

#[test]
fn cur_021_replay_readmits_declarations() {
    let mut state = monetary_state();
    let replay = |state: &mut DomainState, plan: DecisionPlan| {
        ReplayEngine::apply(state, &commit(state, plan.records)).unwrap();
    };
    let plan = InvestmentEngine::declare_security(&state, &seca()).unwrap();
    replay(&mut state, plan);
    let plan = InvestmentEngine::declare_security(&state, &secb()).unwrap();
    replay(&mut state, plan);
    let mut joined = revision(&state, SECA, SECA + 3);
    joined
        .local_mappings
        .push(mapping(source_id(OTHER_SOURCE, "ALT-77")));
    let plan = InvestmentEngine::revise_security(&state, &joined).unwrap();
    replay(&mut state, plan);
    for request in [
        account(Q, unit_id(SECA_UNIT), AccountOriginV1::External),
        account(K, unit_id(1), control(ControlRoleV1::InvestmentCarrying)),
        account(
            QUANTITY_CONTROL,
            unit_id(SECA_UNIT),
            control(ControlRoleV1::QuantityControl),
        ),
    ] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        replay(&mut state, plan);
    }
    let plan = InvestmentEngine::declare_position(&state, &position(6200, SECA, Q, K)).unwrap();
    replay(&mut state, plan);
    assert_eq!(
        InvestmentEngine::resolve(&state, &source_id(OTHER_SOURCE, "ALT-77")).unwrap(),
        Some(joined)
    );
    assert!(matches!(
        state.records.get(&id(6200)),
        Some(AuthoritativeRecordV1::Position(_))
    ));

    // A new security that names SECA's unit, declared by an earlier commit.
    let mut borrowed = declaration(6020, "SCC", "SYN-0003", 4);
    borrowed.security.unit_id = unit_id(SECA_UNIT);
    refused(
        &state,
        vec![AuthoritativeRecordV1::Security(borrowed.security.clone())],
    );
    // A unit catalog in the same commit that declares another unit.
    let own = declaration(6020, "SCC", "SYN-0003", 4);
    let plan = InvestmentEngine::declare_security(&state, &own).unwrap();
    let mut records = plan.records.clone();
    records[1] = AuthoritativeRecordV1::Security(borrowed.security);
    refused(&state, records);
    // A security unit with no security record.
    refused(&state, vec![plan.records[0].clone()]);
    // A forged position whose quantity account is in USD.
    refused(
        &state,
        vec![AuthoritativeRecordV1::Position(position(6201, SECB, 10, K))],
    );
    // The declaration itself still replays at this head.
    ReplayEngine::apply(&mut state.clone(), &commit(&state, plan.records)).unwrap();
}

fn civil(day: &str) -> Date {
    let TemporalValue::Date { civil, .. } = date(day) else {
        panic!("civil date")
    };
    civil
}

/// `request` planned, frozen and admitted after `state`.
fn traded(state: &DomainState, request: &TradeRequestV1) -> DomainState {
    let plan = InvestmentEngine::recognize(state, request).unwrap();
    admit(state, &planned(state, &plan)).unwrap()
}

/// The buy of 10 SECA for 1,000 USD on 2026-11-03, as trade 8200.
fn buy_of(state: &DomainState, rows: &Rows) -> TradeRequestV1 {
    trade(
        state,
        8200,
        TradeSideV1::Buy,
        Some(&rows.cash["buy"]),
        Some(&rows.quantity["buy"]),
        0,
    )
}

/// Each account whose balance differs between `before` and `after`, with
/// its change.
fn changes(before: &DomainState, after: &DomainState) -> BTreeMap<AccountId, Amount> {
    let (before, after) = (balances(before), balances(after));
    let mut changed = BTreeMap::new();
    for account in before.keys().chain(after.keys()) {
        let unit = before.get(account).or(after.get(account)).unwrap().unit_id;
        let zero = Amount::new(0, 0, unit).unwrap();
        let old = before.get(account).copied().unwrap_or(zero);
        let new = after.get(account).copied().unwrap_or(zero);
        let delta = new
            .checked_add(&old.checked_neg().unwrap())
            .unwrap()
            .normalized()
            .unwrap();
        if delta.coefficient != 0 {
            changed.insert(*account, delta);
        }
    }
    changed
}

/// Each `(account, whole units, unit)` as an expected change.
fn expect(entries: &[(u32, i128, u32)]) -> BTreeMap<AccountId, Amount> {
    entries
        .iter()
        .map(|(account, value, unit)| (account_id(*account), balance(*value, *unit)))
        .collect()
}

/// A correction that issues `original` again as recognition `n`, with
/// postings from `n + 1`.
fn reissued(state: &DomainState, original: RecognitionId, n: u32) -> CorrectionV1 {
    let Some(AuthoritativeRecordV1::Recognition(recognition)) =
        state.records.get(&RecordId(original.0))
    else {
        panic!("a recognition")
    };
    let mut again = recognition.clone();
    again.id = RecognitionId(id(n).0);
    for (k, posting) in again.postings.iter_mut().enumerate() {
        posting.id = id(n + 1 + u32::try_from(k).unwrap());
    }
    CorrectionV1 {
        active_recognition: original,
        replacement: vec![again],
        remapping: vec![],
        reason: "Synthetic identity correction".into(),
    }
}

#[test]
fn cur_021_1_buy_commits_cash_and_quantity() {
    let (state, rows) = trading();
    let buy = buy_of(&state, &rows);
    let plan = InvestmentEngine::recognize(&state, &buy).unwrap();
    let trades: Vec<_> = plan
        .records
        .iter()
        .filter_map(|r| match r {
            AuthoritativeRecordV1::Trade(t) => Some(t),
            _ => None,
        })
        .collect();
    let [record] = trades.as_slice() else {
        panic!("one trade record")
    };
    assert_eq!(record.position, id(P));
    assert_eq!(record.recognition, buy.terms.recognition_id);
    assert!(matches!(record.cash, SourceValue::Known(_)));
    assert!(matches!(record.quantity, SourceValue::Known(_)));

    // One native batch whose one entry balances each unit on its own.
    let batch = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    let [FrozenNativeOperationV1::Entry(entry)] = batch.operations.as_slice() else {
        panic!("one entry")
    };
    let posted: BTreeMap<_, _> = entry
        .postings
        .iter()
        .map(|p| {
            (
                p.original.account_id,
                p.original.amount.normalized().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        posted,
        expect(&[
            (B, -1000, 1),
            (K, 1000, 1),
            (Q, 10, SECA_UNIT),
            (QUANTITY_CONTROL, -10, SECA_UNIT),
        ])
    );
    for unit in [unit_id(1), unit_id(SECA_UNIT)] {
        let total = entry
            .postings
            .iter()
            .filter(|p| p.unit.id == unit)
            .try_fold(Amount::new(0, 0, unit).unwrap(), |sum, p| {
                sum.checked_add(&p.original.amount)
            })
            .unwrap();
        assert_eq!(total.coefficient, 0);
    }

    // Replay accepts the commit.
    let next = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(changes(&state, &next), posted);
    let holdings = InvestmentEngine::holdings(&next, id(P), civil("2026-11-30")).unwrap();
    assert_eq!(holdings.quantity, balance(10, SECA_UNIT));
    assert_eq!(holdings.carrying, balance(1000, 1));
    assert!(holdings.missing_quantity.is_empty() && holdings.missing_cash.is_empty());

    // Replay refuses frozen operations that hold only one side's postings.
    for unit in [unit_id(1), unit_id(SECA_UNIT)] {
        let mut one_side = batch.clone();
        let FrozenNativeOperationV1::Entry(entry) = &mut one_side.operations[0] else {
            panic!("an entry")
        };
        entry.postings.retain(|p| p.unit.id == unit);
        let c = candidate(&state, plan.records.clone(), rebound(one_side));
        rejected(&state, &c);
    }
}

#[test]
fn cur_021_1_sale_holds_proceeds_in_disposal_clearing() {
    let (state, rows) = trading();
    let bought = traded(&state, &buy_of(&state, &rows));
    let sale = trade(
        &bought,
        8300,
        TradeSideV1::Sell,
        Some(&rows.cash["sale"]),
        Some(&rows.quantity["sale"]),
        3,
    );
    let sold = traded(&bought, &sale);
    // Gross proceeds go to disposal clearing and the fee to expense once.
    // No income or gain account changes, and K keeps the buy's 1,000.
    assert_eq!(
        changes(&bought, &sold),
        expect(&[
            (B, 517, 1),
            (FEE_EXPENSE, 3, 1),
            (DISPOSAL_CLEARING, -520, 1),
            (Q, -4, SECA_UNIT),
            (QUANTITY_CONTROL, 4, SECA_UNIT),
        ])
    );
    assert_eq!(balances(&sold)[&account_id(K)], balance(1000, 1));
    let holdings = InvestmentEngine::holdings(&sold, id(P), civil("2026-11-30")).unwrap();
    assert_eq!(holdings.quantity, balance(6, SECA_UNIT));
    assert_eq!(holdings.carrying, balance(1000, 1));
    // Through the day before the sale, only the buy counts.
    let holdings = InvestmentEngine::holdings(&sold, id(P), civil("2026-11-09")).unwrap();
    assert_eq!(holdings.quantity, balance(10, SECA_UNIT));
}

#[test]
fn cur_021_1_buy_fee_capitalized_once() {
    let (state, rows) = trading();
    let buy = trade(
        &state,
        8200,
        TradeSideV1::Buy,
        Some(&rows.cash["buy-with-fee"]),
        Some(&rows.quantity["buy-with-fee"]),
        5,
    );
    let next = traded(&state, &buy);
    assert_eq!(
        changes(&state, &next),
        expect(&[
            (B, -1005, 1),
            (K, 1005, 1),
            (Q, 10, SECA_UNIT),
            (QUANTITY_CONTROL, -10, SECA_UNIT),
        ])
    );
    // A buy's fee is not also an expense.
    let mut expensed = buy.clone();
    expensed.terms.postings[1].amount = amount(1000);
    expensed.terms.postings.push(PostingV1 {
        id: id(8290),
        account_id: account_id(FEE_EXPENSE),
        amount: amount(5),
        effective: date("2026-11-03"),
        posting: date("2026-11-03"),
    });
    assert!(InvestmentEngine::recognize(&state, &expensed).is_err());
}

#[test]
fn cur_021_1_missing_side_stays_explicit() {
    let (state, rows) = trading();
    let through = civil("2026-11-30");

    let cash = &rows.cash["buy"];
    let cash_only = trade(&state, 8200, TradeSideV1::Buy, Some(cash), None, 0);
    assert_eq!(cash_only.quantity, SourceValue::Missing);
    let next = traded(&state, &cash_only);
    assert_eq!(
        changes(&state, &next),
        expect(&[(B, -1000, 1), (K, 1000, 1)])
    );
    let holdings = InvestmentEngine::holdings(&next, id(P), through).unwrap();
    assert_eq!(holdings.missing_quantity, vec![id(8200)]);
    assert!(holdings.missing_cash.is_empty());
    assert_eq!(holdings.quantity, balance(0, SECA_UNIT));
    assert_eq!(holdings.carrying, balance(1000, 1));

    let quantity = &rows.quantity["buy"];
    let quantity_only = trade(&state, 8200, TradeSideV1::Buy, None, Some(quantity), 0);
    assert_eq!(quantity_only.cash, SourceValue::Missing);
    let next = traded(&state, &quantity_only);
    assert_eq!(
        changes(&state, &next),
        expect(&[(Q, 10, SECA_UNIT), (QUANTITY_CONTROL, -10, SECA_UNIT)])
    );
    let holdings = InvestmentEngine::holdings(&next, id(P), through).unwrap();
    assert_eq!(holdings.missing_cash, vec![id(8200)]);
    assert!(holdings.missing_quantity.is_empty());
    assert_eq!(holdings.quantity, balance(10, SECA_UNIT));
    assert_eq!(holdings.carrying, balance(0, 1));

    let neither = trade(&state, 8200, TradeSideV1::Buy, None, None, 0);
    assert!(InvestmentEngine::recognize(&state, &neither).is_err());
}

#[test]
fn cur_021_1_no_sum_of_dollars_and_shares() {
    let (state, rows) = trading();
    let buy = buy_of(&state, &rows);
    assert!(InvestmentEngine::recognize(&state, &buy).is_ok());
    let refused = |request: TradeRequestV1| {
        assert!(InvestmentEngine::recognize(&state, &request).is_err());
    };
    // The postings are B, K, Q and quantity control. Here USD sums to
    // -1,000 and SECA to +1,000, and neither unit balances on its own.
    let mut mixed = buy.clone();
    let postings = &mut mixed.terms.postings;
    postings[2].amount = units(1000, SECA_UNIT);
    postings.remove(3);
    postings.remove(1);
    refused(mixed);
    // A SECA posting on K.
    let mut on_carrying = buy.clone();
    on_carrying.terms.postings[1].amount = units(10, SECA_UNIT);
    refused(on_carrying);
    // A USD posting on Q.
    let mut on_quantity = buy.clone();
    on_quantity.terms.postings[2].amount = amount(10);
    refused(on_quantity);
    // 0.00001 SECA exceeds exponent 4, and the engine never rounds it.
    let mut fine = buy.clone();
    let tiny = Amount::new(1, 5, unit_id(SECA_UNIT)).unwrap();
    let SourceValue::Known(leg) = &mut fine.quantity else {
        panic!("a quantity leg")
    };
    leg.leg.movement = tiny;
    leg.leg.allocations[0].amount = tiny;
    leg.sources[0].amount = tiny;
    fine.terms.postings[2].amount = tiny;
    fine.terms.postings[3].amount = tiny.checked_neg().unwrap();
    refused(fine);
}

#[test]
fn cur_021_corrections_refuse_trades() {
    let (state, rows) = trading();
    let buy = buy_of(&state, &rows);
    let mut state = traded(&state, &buy);
    let original = buy.terms.recognition_id;
    let request = reissued(&state, original, 8800);
    let policy = PolicyReferenceV1 {
        id: id(104),
        version: 1,
    };
    assert!(CorrectionEngine::correct(&state, &request).is_err());
    assert!(CorrectionEngine::correct_durable(&state, id(8850), &request).is_err());
    assert!(CorrectionEngine::reinterpret(&state, id(8850), &request, &policy, None).is_err());
    let adoption = posted(rows.cash["sale"].clone(), 8700, 30);
    assert!(CorrectionEngine::replace_legs(&state, id(8850), &request, &adoption).is_err());

    // A correction of an ordinary 96 USD purchase on B still has a plan.
    let purchase = evidence(&mut state, 400, -96, None);
    let plan = RecognitionEngine::adopt(&state, &posted(purchase, 500, 30)).unwrap();
    let state = admit(&state, &planned(&state, &plan)).unwrap();
    let ordinary = reissued(&state, RecognitionId(id(503).0), 8900);
    assert!(CorrectionEngine::correct(&state, &ordinary).is_ok());

    // A segment replacement of B whose closure includes the buy.
    let (mut state, rows) = trading();
    let opening = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    let state = admit(&state, &planned(&state, &plan)).unwrap();
    let state = traded(&state, &buy_of(&state, &rows));
    let again = reissued(&state, original, 8800);
    let replacement = SegmentReplacementV1 {
        prior_segment: id(707),
        opening,
        backfill: vec![],
        approved_accounts: vec![account_id(B)],
        transfers: vec![],
        reemissions: vec![ReemissionV1 {
            id: id(8960),
            original,
            replacement: again.replacement[0].clone(),
        }],
    };
    assert!(SegmentEngine::replace(&state, &replacement).is_err());
}

/// Quantity account QY of brokerage account Y, in SECA.
const QY: u32 = 6160;
/// Carrying accounts of positions PB and PY, in USD.
const KB: u32 = 6170;
const KY: u32 = 6180;
/// Quantity control in SECB.
const QB_CONTROL: u32 = 6190;
/// Position PB holds SECB on QB in X; position PY holds SECA on QY in Y.
const PB: u32 = 6210;
const PY: u32 = 6220;

/// A buy of `binding`'s quantity on `position` with no cash side, as trade
/// `n`, posting `account` against `control`.
fn quantity_buy(
    state: &DomainState,
    n: u32,
    position: u32,
    (account, control): (u32, u32),
    binding: &SourceFacetBindingV1,
) -> TradeRequestV1 {
    let mut request = trade(state, n, TradeSideV1::Buy, None, Some(binding), 0);
    request.position = id(position);
    let SourceValue::Known(leg) = &mut request.quantity else {
        panic!("a quantity side")
    };
    leg.leg.account_id = account_id(account);
    request.terms.postings[0].account_id = account_id(account);
    request.terms.postings[1].account_id = account_id(control);
    request
}

/// Brokerage account X holds SECA 6 on P and SECB 3 on PB; brokerage
/// account Y holds SECA 2 on PY. Trades recognize every quantity.
fn held() -> DomainState {
    let (mut state, rows) = trading();
    let plan = InvestmentEngine::declare_security(&state, &secb()).unwrap();
    commit_metadata(&mut state, &plan);
    for request in [
        account(QB, unit_id(SECB_UNIT), AccountOriginV1::External),
        account(
            QB_CONTROL,
            unit_id(SECB_UNIT),
            control(ControlRoleV1::QuantityControl),
        ),
        account(KB, unit_id(1), control(ControlRoleV1::InvestmentCarrying)),
        account(QY, unit_id(SECA_UNIT), AccountOriginV1::External),
        account(KY, unit_id(1), control(ControlRoleV1::InvestmentCarrying)),
    ] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        commit_metadata(&mut state, &plan);
    }
    for p in [position(PB, SECB, QB, KB), position(PY, SECA, QY, KY)] {
        let plan = InvestmentEngine::declare_position(&state, &p).unwrap();
        commit_metadata(&mut state, &plan);
    }
    for mapping in state.accounting.as_mut().unwrap().accounts.values_mut() {
        mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
    }
    let header = "original_id,amount,unit,effective,description\n";
    let b = format!("{header}buy-b,3,SCB,2026-11-04,synthetic buy of 3 SECB\n");
    let b = import(&mut state, &b, secb().unit, 8400);
    let y = format!("{header}buy-y,2,SCA,2026-11-05,synthetic buy of 2 SECA in Y\n");
    let y = import(&mut state, &y, seca().unit, 8500);
    let state = traded(&state, &buy_of(&state, &rows));
    let sale = trade(
        &state,
        8300,
        TradeSideV1::Sell,
        Some(&rows.cash["sale"]),
        Some(&rows.quantity["sale"]),
        3,
    );
    let state = traded(&state, &sale);
    let buy_b = quantity_buy(&state, 8600, PB, (QB, QB_CONTROL), &b["buy-b"]);
    let state = traded(&state, &buy_b);
    let buy_y = quantity_buy(&state, 8700, PY, (QY, QUANTITY_CONTROL), &y["buy-y"]);
    traded(&state, &buy_y)
}

/// `name` from the synthetic holdings fixtures, captured as receipt `n`
/// with its rows from `n + 1` and snapshot `n + 90`. The host names SECA
/// and SECB by their source identifiers.
fn snapshot_batch(name: &str, n: u32) -> CaptureBatchV1 {
    receipted_batch(name, n, n)
}

/// `snapshot_batch(name, n)` captured under receipt `receipt` instead.
fn receipted_batch(name: &str, n: u32, receipt: u32) -> CaptureBatchV1 {
    let path = format!(
        "{}/../../fixtures/investments/quantity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).unwrap();
    let source: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = u32::try_from(source["positions"].as_array().unwrap().len()).unwrap();
    let context = ImportContextV1 {
        receipt_id: id(receipt),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(102),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-12-01T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-12-01T13:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: (0..rows)
            .map(|i| ImportRowIdsV1 {
                observation: id(n + 1 + 3 * i),
                product: id(n + 2 + 3 * i),
                facet: id(n + 3 + 3 * i),
            })
            .collect(),
    };
    let security = |n: u32, identifier: &str, unit: UnitDefinition| HoldingsSecurityV1 {
        identifier: source_id(BROKER, identifier),
        security: id(n),
        unit,
    };
    let profile = HoldingsProfileV1 {
        schema_version: 1,
        parser_version: "1".into(),
        currency: usd(),
        securities: vec![
            security(SECA, "SYN-0001", seca().unit),
            security(SECB, "SYN-0002", secb().unit),
        ],
        snapshot: id(n + 90),
    };
    let input = BoundedEvidence::new("holdings.json", bytes, context).unwrap();
    FileParser::parse_holdings(input, profile).unwrap()
}

/// The plan that publishes `name` after `state` as receipt `n`.
fn snapshot_plan(state: &DomainState, name: &str, n: u32) -> DecisionPlan {
    let batch = ValidatedEvidenceBatchV1::new(snapshot_batch(name, n)).unwrap();
    EvidenceEngine::publish_holdings(state, batch).unwrap()
}

/// The snapshot record `plan` publishes.
fn snapshot_of(plan: &DecisionPlan) -> &HoldingsSnapshotV1 {
    let Some(AuthoritativeRecordV1::HoldingsSnapshot(snapshot)) = plan.records.last() else {
        panic!("a snapshot record")
    };
    snapshot
}

/// `name` published after `state` and admitted by replay, with no
/// accounting effect.
fn published(state: &DomainState, name: &str, n: u32) -> (HoldingsSnapshotV1, DomainState) {
    let plan = snapshot_plan(state, name, n);
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    let next = admit(state, &planned(state, &plan)).unwrap();
    assert!(changes(state, &next).is_empty());
    (snapshot_of(&plan).clone(), next)
}

fn holdings_of(state: &DomainState, position: u32, day: &str) -> PositionHoldingsV1 {
    InvestmentEngine::holdings(state, id(position), civil(day)).unwrap()
}

#[test]
fn cur_021_3_incomplete_snapshot_keeps_missing_security() {
    let state = held();
    let (snapshot, next) = published(&state, "holdings-x-partial.json", 7000);
    assert_eq!(snapshot.completeness, CoverageStatus::Partial);
    assert!(snapshot.absences.is_empty());
    let b = holdings_of(&next, PB, "2026-11-30");
    assert_eq!(b.quantity, balance(3, SECB_UNIT));
    assert_eq!((b.snapshot, b.absence), (None, None));
    let a = holdings_of(&next, P, "2026-11-30");
    assert_eq!(a.quantity, balance(6, SECA_UNIT));
    let listed = a.snapshot.unwrap();
    assert_eq!(listed.snapshot, id(7090));
    assert_eq!(listed.quantity, units(6, SECA_UNIT));
    assert_eq!(
        listed.market_value,
        Some(Amount::new(60000, 2, unit_id(1)).unwrap())
    );
}

#[test]
fn cur_021_3_complete_snapshot_records_absence_assertion() {
    let state = held();
    let (first, next) = published(&state, "holdings-x-complete-2026-11-30.json", 7000);
    let v1 = HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 1,
        prior: None,
    };
    assert_eq!(first.absences, vec![v1]);
    assert_eq!(first.scope, vec![account_id(Q), account_id(QB)]);
    assert_eq!(
        first.market_value,
        Some(Amount::new(60000, 2, unit_id(1)).unwrap())
    );
    let b = holdings_of(&next, PB, "2026-11-30");
    assert_eq!(b.quantity, balance(3, SECB_UNIT));
    assert_eq!(b.snapshot, None);
    assert_eq!(b.absence.unwrap().snapshot, first.id);
    // A later complete snapshot that again omits SECB adds version 2.
    let (second, last) = published(&next, "holdings-x-complete-2026-12-31.json", 7100);
    let v2 = HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 2,
        prior: Some(first.id),
    };
    assert_eq!(second.absences, vec![v2]);
    let b = holdings_of(&last, PB, "2026-12-31");
    assert_eq!(b.quantity, balance(3, SECB_UNIT));
    assert_eq!(b.absence.unwrap().version, 2);
    // Through the earlier day, version 1 is the current assertion.
    assert_eq!(
        holdings_of(&last, PB, "2026-11-30")
            .absence
            .unwrap()
            .version,
        1
    );
    // SECA is listed, so it has no absence.
    let a = holdings_of(&last, P, "2026-12-31");
    assert_eq!(a.quantity, balance(6, SECA_UNIT));
    assert_eq!(a.snapshot.unwrap().snapshot, second.id);
    assert_eq!(a.absence, None);
}

#[test]
fn cur_021_3_absence_follows_scope() {
    let state = held();
    let (snapshot, next) = published(&state, "holdings-y-complete.json", 7000);
    assert_eq!(snapshot.completeness, CoverageStatus::Complete);
    assert!(snapshot.absences.is_empty());
    assert_eq!(holdings_of(&next, PB, "2026-11-30").absence, None);
    // Only Y's position reads the snapshot.
    let y = holdings_of(&next, PY, "2026-11-30");
    assert_eq!(y.quantity, balance(2, SECA_UNIT));
    assert_eq!(y.snapshot.unwrap().quantity, units(2, SECA_UNIT));
    assert_eq!(holdings_of(&next, P, "2026-11-30").snapshot, None);
}

#[test]
fn cur_021_snapshot_keeps_unmapped_positions() {
    let state = held();
    let (snapshot, next) = published(&state, "holdings-x-unmapped.json", 7000);
    let [listed, unmapped] = snapshot.holdings.as_slice() else {
        panic!("two listed positions")
    };
    assert_eq!(listed.security, Some(id(SECA)));
    // SYN-9999 names no security, though its symbol is SECA's.
    assert_eq!((unmapped.security, unmapped.quantity), (None, None));
    // The unmapped position is retained as holding evidence.
    let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
        next.records.get(&unmapped.observation)
    else {
        panic!("a retained observation")
    };
    assert_eq!(observation.semantic_kind, ObservationKind::Holding);
    assert!(observation.sanitized_representation.contains("SYN-9999"));
    // It does not count toward SECA.
    let a = holdings_of(&next, P, "2026-11-30");
    assert_eq!(a.snapshot.unwrap().quantity, units(6, SECA_UNIT));
    assert_eq!(a.quantity, balance(6, SECA_UNIT));
}

#[test]
fn cur_021_3_unmapped_position_withholds_absences() {
    let state = held();
    let name = "holdings-x-complete-unmapped.json";
    let (snapshot, next) = published(&state, name, 7000);
    assert_eq!(snapshot.completeness, CoverageStatus::Complete);
    let [listed, unmapped] = snapshot.holdings.as_slice() else {
        panic!("two listed positions")
    };
    assert_eq!(listed.security, Some(id(SECA)));
    assert_eq!(unmapped.security, None);
    // SYN-9999 could be SECB, so the snapshot asserts no absence for PB.
    assert!(snapshot.absences.is_empty());
    let b = holdings_of(&next, PB, "2026-11-30");
    assert_eq!(b.quantity, balance(3, SECB_UNIT));
    assert_eq!((b.snapshot, b.absence), (None, None));
    // The read shows the unmapped position instead.
    let shown = SnapshotUnmappedV1 {
        snapshot: snapshot.id,
        as_of: civil("2026-11-30"),
        observations: vec![unmapped.observation],
    };
    assert_eq!(b.unmapped, Some(shown));

    // Replay refuses a commit whose snapshot asserts that absence.
    let plan = snapshot_plan(&state, name, 7000);
    let batch = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    let mut forged = plan.records.clone();
    let Some(AuthoritativeRecordV1::HoldingsSnapshot(draft)) = forged.last_mut() else {
        panic!("a snapshot record")
    };
    draft.absences.push(HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 1,
        prior: None,
    });
    rejected(&state, &candidate(&state, forged, batch));

    // A later complete snapshot with no unmapped position records the
    // absence as usual.
    let (later, last) = published(&next, "holdings-x-complete-2026-12-31.json", 7100);
    let v1 = HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 1,
        prior: None,
    };
    assert_eq!(later.absences, vec![v1]);
    let b = holdings_of(&last, PB, "2026-12-31");
    assert_eq!(b.absence.unwrap().snapshot, later.id);
    assert_eq!(b.unmapped, None);
}

#[test]
fn cur_021_3_absence_versions_follow_publication_order() {
    let state = held();
    let (december, next) = published(&state, "holdings-x-complete-2026-12-31.json", 7000);
    // The November statement is imported late.
    let (november, last) = published(&next, "holdings-x-complete-2026-11-30.json", 7100);
    let v1 = HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 1,
        prior: None,
    };
    assert_eq!(december.absences, vec![v1]);
    // The version counts publications, so November's is version 2.
    let v2 = HoldingAbsenceV1 {
        position: id(PB),
        security: id(SECB),
        version: 2,
        prior: Some(december.id),
    };
    assert_eq!(november.absences, vec![v2]);
    // The current absence has the latest as-of date.
    let current = holdings_of(&last, PB, "2026-12-31").absence.unwrap();
    assert_eq!(
        (current.snapshot, current.as_of, current.version),
        (december.id, civil("2026-12-31"), 1)
    );
    let current = holdings_of(&last, PB, "2026-11-30").absence.unwrap();
    assert_eq!(
        (current.snapshot, current.as_of, current.version),
        (november.id, civil("2026-11-30"), 2)
    );
}

#[test]
fn cur_021_snapshot_replay() {
    let state = held();
    let mut next = state.clone();
    for (name, n) in [
        ("holdings-x-partial.json", 7000),
        ("holdings-x-complete-2026-11-30.json", 7100),
        ("holdings-y-complete.json", 7200),
        ("holdings-x-unmapped.json", 7300),
    ] {
        next = published(&next, name, n).1;
    }
    assert_eq!(next.through.0, state.through.0 + 4);

    // A commit whose absence names SECA, which the snapshot lists.
    let plan = snapshot_plan(&next, "holdings-x-complete-2026-12-31.json", 7400);
    let batch = AccountingAdapter::freeze(&plan.accounting, &next).unwrap();
    let mut forged = plan.records.clone();
    let Some(AuthoritativeRecordV1::HoldingsSnapshot(snapshot)) = forged.last_mut() else {
        panic!("a snapshot record")
    };
    snapshot.absences.push(HoldingAbsenceV1 {
        position: id(P),
        security: id(SECA),
        version: 1,
        prior: None,
    });
    rejected(&next, &candidate(&next, forged, batch.clone()));
    // The commit as planned still replays at that head.
    admit(&next, &candidate(&next, plan.records, batch)).unwrap();
    // A snapshot under an already published receipt has no plan.
    let name = "holdings-x-complete-2026-12-31.json";
    let reused = ValidatedEvidenceBatchV1::new(receipted_batch(name, 7400, 7000)).unwrap();
    assert!(EvidenceEngine::publish_holdings(&next, reused).is_err());
}
