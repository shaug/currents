//! CUR-021 security and position declarations and trades, using synthetic
//! identities and exact public interfaces.
use currents_accounting::AccountingAdapter;
use currents_domain::{
    ReplayEngine,
    associations::IdentityEngine,
    catalog::CatalogEngine,
    correction::CorrectionEngine,
    investments::InvestmentEngine,
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
