//! Synthetic pure-domain fixtures. Service tests persist real content and declare actual native accounts.
use currents_domain::catalog::CatalogEngine;
use currents_types::*;
use serde_json::json;
pub fn id(n: u32) -> RecordId {
    serde_json::from_value(json!(format!("00000000-0000-0000-0000-{n:012}"))).unwrap()
}

pub fn date(day: &str) -> TemporalValue {
    serde_json::from_value(json!({"kind":"Date","civil":day,"original_text":day})).unwrap()
}

pub fn state() -> DomainState {
    let mut state = DomainState::default();
    let mut dataset = serde_json::from_str::<Vec<AuthoritativeRecordV1>>(include_str!(
        "../../../../fixtures/contracts/v1/records.json"
    ))
    .unwrap()
    .into_iter()
    .find_map(|r| match r {
        AuthoritativeRecordV1::Dataset(d) => Some(d),
        _ => None,
    })
    .unwrap();
    dataset.id = DatasetId(id(100).0);
    dataset.unit_catalog = id(101);
    dataset.ownership_perimeter = vec![];
    state
        .records
        .insert(id(100), AuthoritativeRecordV1::Dataset(dataset));
    state.records.insert(
        id(101),
        AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
            id: id(101),
            version: 1,
            units: vec![UnitDefinition {
                id: UnitId(id(1).0),
                exponent: 2,
                native_code: "USD".into(),
                kind: UnitKind::Currency,
            }],
        }),
    );
    state
}

pub fn catalog() -> CatalogRequestV1 {
    CatalogRequestV1 {
        schema_version: 1,
        revision_id: id(12),
        prior_revision: None,
        catalog: SetAccountCatalogV1 {
            account_id: AccountId(id(10).0),
            component_id: id(11),
            account_class: AccountClass::Asset,
            unit_id: UnitId(id(1).0),
            normal_sign: NormalSign::Debit,
            aliases: vec![],
            lifecycle: Lifecycle::Open,
            display_name: "Synthetic checking".into(),
        },
        ownership: OwnershipV1 {
            account_id: AccountId(id(10).0),
            numerator: CommitSeq(1),
            denominator: CommitSeq(1),
        },
        origin: AccountOriginV1::External,
        overlaps: vec![],
    }
}

pub fn apply(state: &mut DomainState, plan: &DecisionPlan) {
    for record in &plan.records {
        let value = serde_json::to_value(record).unwrap();
        let id = serde_json::from_value(value["value"]["id"].clone()).unwrap();
        state.records.insert(id, record.clone());
    }
    state.through.0 += 1;
    if let Some(index) = &mut state.accounting {
        index.prefix.through = state.through;
    }
}

pub fn amount(value: i128) -> Amount {
    Amount::new(value, 0, UnitId(id(1).0)).unwrap()
}

pub fn interpretation() -> InterpretationV1 {
    InterpretationV1 {
        kind: DecisionKind::Interpretation,
        authority: AuthorityKind::Owner,
        evidence: vec![id(200)],
        reason: "Explicit synthetic adoption".into(),
    }
}

pub fn evidence(
    state: &mut DomainState,
    n: u32,
    value: i128,
    prior: Option<RecordId>,
) -> SourceFacetBindingV1 {
    let source = SourceIdentityV1 {
        source: "synthetic".into(),
        connection_id: id(102),
        device_id: None,
    };
    let observation = ObservationVersionV1 {
        id: id(n),
        source,
        original_ids: vec!["fixture-only".into()],
        sanitized_representation: "synthetic".into(),
        reported_fields: vec![],
        temporal: date("2026-11-01"),
        semantic_kind: ObservationKind::Movement,
        prior_version: prior,
        tombstone_of: None,
        content_digest: Digest([0; 32]),
    };
    state.records.insert(
        id(n),
        AuthoritativeRecordV1::ObservationVersion(observation),
    );
    let binding = SourceFacetBindingV1 {
        observation: id(n),
        parse_product: id(n + 1),
        facet: id(900),
        amount: amount(value),
    };
    state.records.insert(
        id(n + 1),
        AuthoritativeRecordV1::AddressedParseProduct(AddressedParseProductV1 {
            id: id(n + 1),
            schema_version: 1,
            product: ParseProductV1 {
                source_version_id: id(n),
                parser_id: "synthetic".into(),
                parser_version: "1".into(),
                facets: vec![MovementFacetV1 {
                    lineage_key: binding.facet,
                    kind: ObservationKind::Movement,
                    amount: SourceValue::Known(binding.amount),
                    relationships: vec![],
                    temporal: date("2026-11-01"),
                }],
            },
            balance_kinds: vec![],
        }),
    );
    binding
}

pub fn activity_state() -> DomainState {
    let mut state = state();
    let plan = CatalogEngine::set_complete(&state, &catalog()).unwrap();
    apply(&mut state, &plan);
    let mut policy = serde_json::from_str::<Vec<AuthoritativeRecordV1>>(include_str!(
        "../../../../fixtures/contracts/v1/records.json"
    ))
    .unwrap()
    .into_iter()
    .find_map(|r| match r {
        AuthoritativeRecordV1::DatasetPolicy(p) => Some(p),
        _ => None,
    })
    .unwrap();
    policy.id = id(104);
    let record = AuthoritativeRecordV1::DatasetPolicy(policy);
    state.dataset_policies = currents_domain::policy_history::PolicyHistory::project(
        &state,
        std::slice::from_ref(&record),
    )
    .unwrap();
    state.records.insert(id(104), record);
    state
}

pub fn pending(binding: SourceFacetBindingV1, n: u32) -> ActivityRequestV1 {
    let mut adopted_interpretation = interpretation();
    adopted_interpretation.evidence = vec![binding.observation];
    let leg = FinancialLegV1 {
        id: LegId(id(n + 1).0),
        event_id: id(n + 2),
        account_id: AccountId(id(10).0),
        movement: binding.amount,
        allocations: vec![EvidenceAllocationV1 {
            observation: binding.observation,
            facet: binding.facet,
            amount: binding.amount,
        }],
        effective: date("2026-11-01"),
        posting: date("2026-11-01"),
        relationships: vec![],
        unresolved_residual: SourceValue::Absent,
        interpretation: SourceValue::Known(adopted_interpretation.clone()),
    };
    ActivityRequestV1 {
        schema_version: 1,
        adoption_id: id(n),
        event: FinancialEventV1 {
            id: id(n + 2),
            legs: vec![leg.id],
            relationships: vec![],
            interpretation: SourceValue::Known(adopted_interpretation.clone()),
        },
        legs: vec![AdoptedLegV1 {
            leg,
            sources: vec![binding],
        }],
        status: ActivityStatusV1::Pending,
        prior_pending: None,
        interpretation: adopted_interpretation,
        governing_policy: PolicyReferenceV1 {
            id: id(104),
            version: 1,
        },
        financial: None,
    }
}

pub fn monetary_state() -> DomainState {
    let mut state = activity_state();
    for (n, role, class, sign) in [
        (
            30,
            ControlRoleV1::ExpenseFlow,
            AccountClass::Expense,
            NormalSign::Debit,
        ),
        (
            40,
            ControlRoleV1::UnresolvedEquity,
            AccountClass::Equity,
            NormalSign::Credit,
        ),
        (
            50,
            ControlRoleV1::OpeningEquity,
            AccountClass::Equity,
            NormalSign::Credit,
        ),
    ] {
        let mut request = catalog();
        request.revision_id = id(n + 2);
        request.catalog.component_id = id(n + 1);
        request.catalog.account_id = AccountId(id(n).0);
        request.ownership.account_id = request.catalog.account_id;
        request.catalog.account_class = class;
        request.catalog.normal_sign = sign;
        request.origin = AccountOriginV1::Internal(InternalAccountV1 {
            role,
            created_at: date("2026-01-01"),
        });
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        apply(&mut state, &plan);
    }
    let policy = DateConversionPolicyV1 {
        id: id(103),
        version: 1,
        conversion: DateConversionV1::CivilOnly,
        bookkeeping: BookkeepingFallbackV1::RequirePosting,
    };
    state.records.insert(
        id(103),
        AuthoritativeRecordV1::DateConversionPolicy(policy.clone()),
    );
    let mut index = AccountingIndexV1::empty(NativeHead {
        semantics_version: 1,
        checkpoint: Digest([0; 32]),
    });
    index.prefix.through = state.through;
    index.calendar_declared = true;
    index.date_policies.insert((id(103), 1), policy);
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::UnitCatalog(c) => {
                for unit in &c.units {
                    index.units.insert(unit.id, unit.clone());
                }
            }
            AuthoritativeRecordV1::AccountComponent(c) => {
                index.accounts.insert(
                    c.account_id,
                    NativeAccountMappingV1 {
                        account_id: c.account_id,
                        unit_id: c.unit_id,
                        account_class: c.account_class,
                        path: format!("fixture/{}", c.account_id.0),
                    },
                );
            }
            _ => {}
        }
    }
    state.accounting = Some(index);
    state
}

/// The activity recognition that posted `request` derives: its postings,
/// its legs' allocations, and a scope over its civil effective dates.
pub fn recognized(request: &ActivityRequestV1) -> RecognitionV1 {
    let terms = request.financial.as_ref().unwrap();
    let days: Vec<Date> = terms
        .postings
        .iter()
        .filter_map(|p| match p.effective {
            TemporalValue::Date { civil, .. } => Some(civil),
            _ => None,
        })
        .collect();
    RecognitionV1 {
        id: terms.recognition_id,
        lineage: terms.lineage,
        postings: terms.postings.clone(),
        kind: RecognitionKind::Activity,
        evidence_allocations: request
            .legs
            .iter()
            .flat_map(|l| l.leg.allocations.clone())
            .collect(),
        leg_ids: request.event.legs.clone(),
        scope: DateScope {
            start: days.iter().min().copied(),
            end: days.iter().max().copied(),
            basis: DateBasis::Effective,
        },
        bookkeeping_date: terms.bookkeeping_date.clone(),
        date_policy: terms.date_policy.clone(),
    }
}

pub fn posted(binding: SourceFacetBindingV1, n: u32, counter: u32) -> ActivityRequestV1 {
    let mut request = pending(binding.clone(), n);
    request.status = ActivityStatusV1::Posted;
    request.financial = Some(RecognitionTermsV1 {
        recognition_id: RecognitionId(id(n + 3).0),
        lineage: id(n + 4),
        postings: vec![
            PostingV1 {
                id: id(n + 5),
                account_id: AccountId(id(10).0),
                amount: binding.amount,
                effective: date("2026-11-01"),
                posting: date("2026-11-01"),
            },
            PostingV1 {
                id: id(n + 6),
                account_id: AccountId(id(counter).0),
                amount: binding.amount.checked_neg().unwrap(),
                effective: date("2026-11-01"),
                posting: date("2026-11-01"),
            },
        ],
        bookkeeping_date: date("2026-11-01"),
        date_policy: PolicyReferenceV1 {
            id: id(103),
            version: 1,
        },
        unresolved_account: None,
        opening: None,
    });
    request
}

pub fn supported_opening(state: &mut DomainState) -> ActivityRequestV1 {
    let balance = evidence(state, 250, 3300, None);
    let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
        state.records.get_mut(&id(250))
    else {
        panic!()
    };
    observation.semantic_kind = ObservationKind::Balance;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get_mut(&id(251))
    else {
        panic!()
    };
    product.product.facets[0].kind = ObservationKind::Balance;
    let included = evidence(state, 260, -96, None);
    let mut request = posted(balance.clone(), 700, 50);
    request.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: id(707),
        component_id: id(11),
        amount: balance.amount,
        cutoff: date("2026-11-01"),
        evidence: balance,
        included_activity: vec![included],
    });
    request
}
