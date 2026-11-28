//! CUR-021 security and position declarations, using synthetic identities and
//! exact public interfaces.
use currents_domain::{
    ReplayEngine, associations::IdentityEngine, catalog::CatalogEngine,
    investments::InvestmentEngine, recognition::RecognitionEngine,
};
use currents_types::*;
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
