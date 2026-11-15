//! CUR025 synthetic durable C03 planning qualification.
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
use currents_accounting::AccountingAdapter;
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_planning::{
    calendar::CalendarEngine,
    commitments::{CommitmentEngine, resolve_occurrence},
};
use currents_protocol::*;
use currents_service as service_api;
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;
use service_api::*;
use std::{path::Path, sync::Arc};
use support::{Fixture, id};

fn date(s: &str) -> CivilDateV1 {
    serde_json::from_value(serde_json::json!(s)).unwrap()
}
fn fixtures() -> Vec<AuthoritativeRecordV1> {
    serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap()
}
async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x10; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn content(store: &EncryptedStore, through: CommitSeq, digest: Digest) -> Vec<u8> {
    let mut stream = store
        .history()
        .stream_content(through, digest)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        bytes.extend(chunk);
    }
    bytes
}
fn key(c: &CommitmentV1, generation: u32, ordinal: u64) -> OccurrenceKeyV1 {
    OccurrenceKeyV1 {
        commitment_id: c.id,
        series_generation: generation,
        ordinal: CommitSeq(ordinal),
    }
}

// Exact-prefix immutable test snapshot, refreshed after each accepted commit.
// Deliberately not a new production host, C06 replay implementation or background
// scheduler. There is no storage capability inside CommitmentEngine.
struct PlanningFixture {
    state: DomainState,
    engine: CommitmentEngine,
}
impl CommandPlanner for PlanningFixture {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<CommandPlan, EngineError> {
        if self.state.accounting.as_ref().map(|s| &s.prefix) != Some(prefix) {
            return Err(EngineError::invalid());
        }
        let OperationV1::SetCommitment(c) = &body.operation else {
            return Err(EngineError::version());
        };
        let decision_plan = self.engine.set(&self.state, c)?;
        assert!(matches!(
            decision_plan.accounting,
            AccountingIntent::NoEffect(_)
        ));
        let frozen_batch = AccountingAdapter::freeze(&decision_plan.accounting, &self.state)?;
        assert!(frozen_batch.operations.is_empty());
        assert_eq!(
            frozen_batch.result_expectations.financial_entry_count,
            CommitSeq(0)
        );
        Ok(CommandPlan {
            effects: EffectsV1 {
                records: vec![RecordId(c.id.0)],
                accounts: c.legs.iter().map(|l| l.account_id).collect(),
                amounts: vec![],
                invalidations: decision_plan.invalidations.clone(),
            },
            approval_scope: ApprovalScopeV1 {
                accounts: c.legs.iter().map(|l| l.account_id).collect(),
                operation: DecisionKind::Planning,
                fields: vec!["commitment".into()],
            },
            decision_plan,
            frozen_batch,
            warnings: vec![],
            decision_id: RecordId(body.command_id.0),
            manifest_id: id(body.command_id.0.as_u128() + 200_000),
            policy: PolicyReferenceV1 {
                id: id(700),
                version: 1,
            },
        })
    }
    fn query(&self, _: &QueryV1, _: &AccountingPrefixV1) -> Result<QueryResponseV1, EngineError> {
        Err(EngineError::version())
    }
}

async fn state_at_tip(
    store: &EncryptedStore,
    empty: &NativeHead,
) -> (DomainState, Vec<CommitmentV1>) {
    let tip = store.writer().audited_prefix().await.unwrap();
    let commits = store.history().read_prefix(tip.through).await.unwrap();
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty.clone())),
        ..DomainState::default()
    };
    let mut revisions = vec![];
    for commit in commits {
        let mut instructions = vec![];
        for record in &commit.records {
            if let AuthoritativeRecordV1::FrozenAccountingManifest(m) = record {
                instructions = content(store, commit.seq, m.instructions.digest).await;
                let batch = AccountingAdapter::from_manifest(m, &instructions).unwrap();
                assert_eq!(
                    batch.result_expectations.financial_entry_count,
                    CommitSeq(0)
                );
                assert!(
                    !batch
                        .operations
                        .iter()
                        .any(|op| matches!(op, FrozenNativeOperationV1::Entry(_)))
                );
            }
            if let AuthoritativeRecordV1::Commitment(c) = record {
                revisions.push(c.clone());
            }
        }
        AccountingAdapter::advance_index(&mut state, &commit, &instructions).unwrap();
        // Fixture-only latest-record view. Immutable revisions stay in history.
        for record in commit.records {
            let json = serde_json::to_value(&record).unwrap();
            if let Ok(id) = serde_json::from_value::<RecordId>(json["value"]["id"].clone()) {
                state.records.insert(id, record);
            }
        }
    }
    assert_eq!(state.accounting.as_ref().unwrap().prefix, tip);
    (state, revisions)
}

#[tokio::test]
async fn cur_025_service_durable_revisions_and_no_recognition() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("commitments.db");
    let mut f = Fixture::at_path(path.clone()).await;
    let empty_native = f.prefix().await.native_head;
    let catalog = f.body(10);
    let r = f
        .service
        .submit(f.ctx(), catalog.clone(), f.approve(&catalog))
        .await
        .unwrap();
    assert_eq!(r.state, CommandState::Committed);
    let metadata_native = f.prefix().await.native_head;
    f.service.close().await.unwrap();

    // Synthetic trusted-host acquisition, before opening commitment Service.
    // The receipt makes TZif content part of accepted authority, not staging-only.
    let (zone, tzif) = jiff_tzdb::get("America/New_York").unwrap();
    let version = jiff_tzdb::VERSION.unwrap();
    let calendar = CalendarEngine::new(zone, version, tzif).unwrap();
    let calendar_data = calendar.calendar_data();
    let store = open(&path).await;
    let staged = store.stage(tzif).await.unwrap();
    assert_eq!(staged.digest(), calendar_data.timezone_data.digest);
    let mut dataset = fixtures()
        .into_iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::Dataset(d) => Some(d),
            _ => None,
        })
        .unwrap();
    dataset.id = DatasetId(id(1000).0);
    dataset.household_timezone = SourceValue::Known(zone.into());
    let catalog_records = store.history().read_prefix(CommitSeq(1)).await.unwrap();
    let unit_catalog = catalog_records[0]
        .records
        .iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::UnitCatalog(u) => Some(u),
            _ => None,
        })
        .unwrap();
    dataset.unit_catalog = unit_catalog.id;
    dataset.reporting_currency = unit_catalog.units[0].id;
    dataset.commit_head = CommitSeq(1);
    dataset.ownership_perimeter = vec![];
    dataset.policy_versions.timezone_data = version.into();
    let unit = dataset.reporting_currency;
    let account = catalog_records[0]
        .records
        .iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::AccountComponent(a) => Some(a.account_id),
            _ => None,
        })
        .unwrap();
    let mut receipt = fixtures()
        .into_iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::ImportReceipt(v) => Some(v),
            _ => None,
        })
        .unwrap();
    receipt.id = id(50_001);
    receipt.contents = vec![calendar_data.timezone_data.clone()];
    receipt.requested_coverage.clear();
    receipt.reported_coverage.clear();
    receipt.result = ImportResult::Complete;
    receipt.complete = true;
    receipt.truncated = false;
    let seed = store
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(1),
            records: vec![
                AuthoritativeRecordV1::Dataset(dataset),
                AuthoritativeRecordV1::ImportReceipt(receipt),
            ],
            decisions: vec![],
            staged: vec![staged],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(id(50_002).0),
                digest: Digest([25; 32]),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
        })
        .await
        .unwrap();
    assert_eq!(seed.seq, CommitSeq(2));
    assert_eq!(seed.native_head_after, metadata_native);
    assert_eq!(
        content(&store, seed.seq, calendar_data.timezone_data.digest).await,
        tzif
    );
    store.close().await.unwrap();

    let mut c = fixtures()
        .into_iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::Commitment(c) => Some(c),
            _ => None,
        })
        .unwrap();
    c.id = CommitmentId(id(51_000).0);
    c.version = 1;
    c.series_generation = 1;
    c.supersession = None;
    c.timezone = zone.into();
    c.timezone_data_version = version.into();
    c.calendar_version = 1;
    *c.calendar_data = calendar_data.clone();
    c.recurrence_version = 1;
    c.start = date("2024-01-01");
    c.due_date = c.start;
    c.end = None;
    c.exceptions.clear();
    c.recurrence = RecurrenceV1 {
        cadence: RecurrenceCadenceV1::Daily,
        interval: 1,
        month: None,
        day: None,
    };
    c.settlement = SettlementRuleV1::SameDay(PlanningEmptyV1 {});
    c.amounts = PlanningAmountsV1 {
        nominal: SourceValue::Known(Amount {
            coefficient: 101,
            scale: 2,
            unit_id: unit,
        }),
        low: SourceValue::Unknown,
        base: SourceValue::Unknown,
        high: SourceValue::Unknown,
    };
    c.legs = vec![PlannedLegV1 {
        id: id(51_001),
        account_id: account,
        amounts: c.amounts.clone(),
        settlement: c.settlement.clone(),
    }];
    c.provenance = InterpretationV1 {
        kind: DecisionKind::Planning,
        authority: AuthorityKind::Owner,
        evidence: vec![],
        reason: "Synthetic owner confirmed obligation".into(),
    };
    let first = c.clone();
    let before = calendar.expand(&first, first.start.0, 1).unwrap();
    // Amount/date-only revision retains key identity and generation.
    c.version = 2;
    c.due_date = date("2024-01-02");
    c.amounts.nominal = SourceValue::Known(Amount {
        coefficient: 251,
        scale: 2,
        unit_id: unit,
    });
    c.legs[0].amounts = c.amounts.clone();
    let edited = c.clone();
    let after = calendar.expand(&edited, edited.start.0, 1).unwrap();
    assert_eq!(before[0].key, after[0].key);
    assert_ne!(before[0].due_date, after[0].due_date);
    assert_ne!(before[0].legs[0].amounts, after[0].legs[0].amounts);
    assert_eq!(after[0].version, 2);

    // First transition: keep ordinal 0, map ordinal 1, explicitly retire 2,
    // and retire unlisted future keys at/after the cutoff.
    c.version = 3;
    c.series_generation = 2;
    c.start = date("2024-01-03");
    c.due_date = c.start;
    c.recurrence.cadence = RecurrenceCadenceV1::Weekly;
    c.supersession = Some(CommitmentSupersessionV1 {
        prior_version: 2,
        prior_generation: 1,
        from_ordinal: CommitSeq(1),
        not_before: date("2024-01-03"),
        remaps: vec![
            OccurrenceRemapV1 {
                old: key(&c, 1, 1),
                new: Some(key(&c, 2, 0)),
            },
            OccurrenceRemapV1 {
                old: key(&c, 1, 2),
                new: None,
            },
        ],
    });
    let transition_one = c.clone();
    // Second transition chains the first remap into generation 3.
    c.version = 4;
    c.series_generation = 3;
    c.recurrence.interval = 2;
    c.supersession = Some(CommitmentSupersessionV1 {
        prior_version: 3,
        prior_generation: 2,
        from_ordinal: CommitSeq(0),
        not_before: date("2024-01-03"),
        remaps: vec![
            OccurrenceRemapV1 {
                old: key(&c, 2, 0),
                new: Some(key(&c, 3, 0)),
            },
            OccurrenceRemapV1 {
                old: key(&c, 2, 1),
                new: None,
            },
        ],
    });
    let mut expected = vec![first, edited, transition_one, c.clone()];
    // Accept each way of removing the chained remap target through real C03
    // commands. Every revision is checked again after closing and reopening.
    c.version = 5;
    c.exceptions.push(PlanningExceptionV1 {
        ordinal: CommitSeq(0),
        action: PlanningExceptionActionV1::Cancel,
        due_date: None,
        legs: vec![],
        reason: "Synthetic owner canceled the mapped expectation".into(),
    });
    expected.push(c.clone());
    c.version = 6;
    c.exceptions[0].action = PlanningExceptionActionV1::Skip;
    expected.push(c.clone());
    c.version = 7;
    c.exceptions.clear();
    c.start = date("2024-01-04");
    expected.push(c.clone());
    c.version = 8;
    c.start = date("2024-01-03");
    c.due_date = date("2024-01-05");
    c.end = Some(date("2024-01-04"));
    expected.push(c.clone());
    let mut submitted = vec![];
    for (i, revision) in expected.iter().enumerate() {
        // Fresh process-like reopen and durable state load for EVERY revision.
        let store = open(&path).await;
        let (state, history) = state_at_tip(&store, &empty_native).await;
        assert_eq!(history, expected[..i]);
        let retained = content(&store, state.through, calendar_data.timezone_data.digest).await;
        let restored = CalendarEngine::from_saved(&calendar_data, &retained).unwrap();
        assert_eq!(restored.calendar_data(), calendar_data);
        let engine = CommitmentEngine::new(restored);
        if i > 4 {
            for source in [key(&c, 1, 1), key(&c, 2, 0), key(&c, 3, 0)] {
                assert_eq!(resolve_occurrence(&history, &source).unwrap(), None);
            }
            assert!(
                !engine
                    .expand_history(&history, date("2024-01-01").0, 1)
                    .unwrap()
                    .iter()
                    .any(|o| o.key == key(&c, 3, 0))
            );
        }
        let head = state.through;
        f.host.reconnect();
        f.service = Service::new(
            store,
            DatasetId(id(1000).0),
            InstanceId(id(999).0),
            ServiceProfile::Synthetic,
            f.host.clone(),
            Arc::new(PlanningFixture { state, engine }),
            f.clock.clone(),
        )
        .await
        .unwrap();
        f.service.recover().await.unwrap();
        let mut body = f.body(60_000 + i as u128);
        body.expected_head = head;
        body.operation = OperationV1::SetCommitment(revision.clone());
        let auth = f.approve(&body);
        let result = f.service.submit(f.ctx(), body.clone(), auth).await.unwrap();
        assert_eq!(result.state, CommandState::Committed);
        assert_eq!(result.commit, Some(CommitSeq(3 + i as u64)));
        submitted.push((body, result));
        f.service.close().await.unwrap();
    }
    let store = open(&path).await;
    let (state, history) = state_at_tip(&store, &empty_native).await;
    assert_eq!(history, expected);
    let retained = content(&store, state.through, calendar_data.timezone_data.digest).await;
    let engine = CommitmentEngine::new(CalendarEngine::new(zone, version, &retained).unwrap());
    assert_eq!(
        resolve_occurrence(&history[..4], &key(&c, 1, 1)).unwrap(),
        Some(key(&c, 3, 0))
    );
    for source in [key(&c, 1, 1), key(&c, 2, 0), key(&c, 3, 0)] {
        assert_eq!(resolve_occurrence(&history, &source).unwrap(), None);
    }
    assert_eq!(resolve_occurrence(&history, &key(&c, 1, 2)).unwrap(), None);
    assert_eq!(resolve_occurrence(&history, &key(&c, 2, 1)).unwrap(), None);
    assert_eq!(
        resolve_occurrence(&history, &key(&c, 1, 0)).unwrap(),
        Some(key(&c, 1, 0))
    );
    let active = engine
        .expand_history(&history, date("2024-01-01").0, 1)
        .unwrap();
    let before_removal = engine
        .expand_history(&history[..4], date("2024-01-01").0, 1)
        .unwrap();
    assert_eq!(
        before_removal
            .iter()
            .filter(|o| o.key == key(&c, 3, 0))
            .count(),
        1
    );
    assert!(!active.iter().any(|o| o.key == key(&c, 3, 0)));
    assert!(!active.iter().any(|o| o.key.series_generation == 2));
    assert!(
        active
            .iter()
            .all(|o| o.key.series_generation != 1 || o.key.ordinal == CommitSeq(0))
    );
    let snapshot = active.clone();
    let tip = store.writer().audited_prefix().await.unwrap();
    assert_eq!(tip.native_head, metadata_native);
    f.host.reconnect();
    f.service = Service::new(
        store,
        DatasetId(id(1000).0),
        InstanceId(id(999).0),
        ServiceProfile::Synthetic,
        f.host.clone(),
        Arc::new(PlanningFixture { state, engine }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    f.service.recover().await.unwrap();
    for (body, result) in submitted {
        assert_eq!(
            f.service
                .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
                .await
                .unwrap(),
            result
        );
    }
    // Obligations are already overdue relative to fixture clock. Advance within
    // its one-hour auth TTL, drain real service, close, independently audit.
    f.clock.advance(60);
    f.service.drain().await.unwrap();
    f.service.close().await.unwrap();
    let store = open(&path).await;
    assert_eq!(store.writer().audited_prefix().await.unwrap(), tip);
    let (_, history) = state_at_tip(&store, &empty_native).await;
    let retained = content(&store, tip.through, calendar_data.timezone_data.digest).await;
    // A different available tzdb version cannot silently substitute for saved data.
    assert!(
        CalendarEngine::new(zone, "synthetic-upgrade-not-the-pinned-version", &retained).is_err()
    );
    let restored = CalendarEngine::new(zone, version, &retained).unwrap();
    assert_eq!(
        CommitmentEngine::new(restored)
            .expand_history(&history, date("2024-01-01").0, 1)
            .unwrap(),
        snapshot
    );
    store.close().await.unwrap();
    durable_boundary_regressions(
        &mut f,
        &path,
        &empty_native,
        &metadata_native,
        &calendar_data,
        &c,
    )
    .await;
}

// Uses actual C03 commands and retained C02/C03 history, not an in-memory save.
async fn durable_boundary_regressions(
    f: &mut Fixture,
    path: &Path,
    empty_native: &NativeHead,
    metadata_native: &NativeHead,
    calendar_data: &ForecastCalendarDataV1,
    template: &CommitmentV1,
) {
    for (case, initial, shifted, transition, lower, unsafe_date) in [
        (
            0,
            "2024-01-01",
            "2024-01-15",
            "2024-01-20",
            "2024-01-16",
            "2024-01-15",
        ),
        (
            1,
            "9999-11-01",
            "9999-11-15",
            "9999-11-20",
            "9999-11-16",
            "9999-11-15",
        ),
    ] {
        let mut first = template.clone();
        first.id = CommitmentId(id(80_000 + case).0);
        first.version = 1;
        first.series_generation = 1;
        first.supersession = None;
        first.start = date(initial);
        first.due_date = first.start;
        first.end = None;
        first.recurrence.cadence = RecurrenceCadenceV1::Daily;
        first.recurrence.interval = 1;
        first.exceptions = vec![PlanningExceptionV1 {
            ordinal: CommitSeq(0),
            action: PlanningExceptionActionV1::Replace,
            due_date: Some(date(shifted)),
            legs: vec![],
            reason: "Synthetic retained pre-cutoff shift".into(),
        }];
        let mut second = first.clone();
        second.version = 2;
        second.series_generation = 2;
        second.recurrence.cadence = RecurrenceCadenceV1::Weekly;
        second.start = date(transition);
        second.due_date = second.start;
        second.exceptions.clear();
        second.supersession = Some(CommitmentSupersessionV1 {
            prior_version: 1,
            prior_generation: 1,
            from_ordinal: CommitSeq(10),
            not_before: date(lower),
            remaps: vec![],
        });
        let mut safe = second.clone();
        safe.version += 1;
        safe.start = date(lower);
        safe.due_date = safe.start;
        let mut earlier_anchor = safe.clone();
        earlier_anchor.version += 1;
        earlier_anchor.due_date = first.due_date;
        let mut third = earlier_anchor.clone();
        third.version += 1;
        third.series_generation += 1;
        third.recurrence.cadence = RecurrenceCadenceV1::Daily;
        third.recurrence.interval = 2;
        third.due_date = third.start;
        third.supersession = Some(CommitmentSupersessionV1 {
            prior_version: earlier_anchor.version,
            prior_generation: earlier_anchor.series_generation,
            from_ordinal: CommitSeq(0),
            not_before: date(lower),
            remaps: vec![],
        });
        let expected = [first.clone(), second, safe, earlier_anchor, third];
        let mut saved = None;
        // A final read-only iteration also tests unsafe editing after transition 2.
        for i in 0..=expected.len() {
            let store = open(path).await;
            let (state, all_history) = state_at_tip(&store, empty_native).await;
            let history: Vec<_> = all_history
                .into_iter()
                .filter(|c| c.id == first.id)
                .collect();
            assert_eq!(history, expected[..i]);
            assert_eq!(
                &store.writer().audited_prefix().await.unwrap().native_head,
                metadata_native
            );
            let bytes = content(&store, state.through, calendar_data.timezone_data.digest).await;
            let engine =
                CommitmentEngine::new(CalendarEngine::from_saved(calendar_data, &bytes).unwrap());
            if i >= 2 {
                let original = engine
                    .expand_history(&history[..2], first.start.0, 1)
                    .unwrap();
                if let Some(saved) = &saved {
                    assert_eq!(&original, saved);
                } else {
                    saved = Some(original);
                }
                let active = engine.expand_history(&history, first.start.0, 1).unwrap();
                assert_eq!(
                    resolve_occurrence(&history, &key(&first, 1, 0)).unwrap(),
                    Some(key(&first, 1, 0))
                );
                assert!(
                    active
                        .iter()
                        .any(|o| o.key == key(&first, 1, 0) && o.due_date == date(shifted))
                );
                assert!(
                    active
                        .iter()
                        .filter(|o| o.key.series_generation > 1)
                        .all(|o| o.due_date.0 >= date(lower).0)
                );
                if i == 3 {
                    let moved = active.iter().find(|o| o.key == key(&first, 2, 0)).unwrap();
                    assert_eq!(moved.due_date, date(lower));
                    assert_eq!(moved.version, 3);
                }
                if i == 5 {
                    assert!(!active.iter().any(|o| o.key.series_generation == 2));
                }
            }
            let head = state.through;
            f.host.reconnect();
            f.service = Service::new(
                store,
                DatasetId(id(1000).0),
                InstanceId(id(999).0),
                ServiceProfile::Synthetic,
                f.host.clone(),
                Arc::new(PlanningFixture { state, engine }),
                f.clock.clone(),
            )
            .await
            .unwrap();
            f.service.recover().await.unwrap();
            if i >= 2 {
                let mut unsafe_edit = history.last().unwrap().clone();
                unsafe_edit.version += 1;
                unsafe_edit.start = date(unsafe_date);
                unsafe_edit.due_date = unsafe_edit.start;
                let mut body = f.body(90_000 + case * 100 + i as u128);
                body.expected_head = head;
                body.operation = OperationV1::SetCommitment(unsafe_edit.clone());
                // C03 prepares the decision during mandatory preview. An invalid
                // plan must fail there, before any approval or submit exists.
                assert_eq!(
                    f.service.preview(f.ctx(), body).unwrap_err().class,
                    ErrorClass::InvalidReference,
                    "C03 must reject unsafe boundary edit after durable reopen, case={case}, revision={i}"
                );
                assert_eq!(f.prefix().await.through, head);
                let mut invalid_history = history.clone();
                invalid_history.push(unsafe_edit);
                let engine = CommitmentEngine::new(
                    CalendarEngine::from_saved(calendar_data, &bytes).unwrap(),
                );
                assert!(
                    engine
                        .expand_history(&invalid_history, first.start.0, 1)
                        .is_err()
                );
                assert!(resolve_occurrence(&invalid_history, &key(&first, 1, 0)).is_err());
            }
            if let Some(revision) = expected.get(i) {
                let mut body = f.body(100_000 + case * 100 + i as u128);
                body.expected_head = head;
                body.operation = OperationV1::SetCommitment(revision.clone());
                let auth = f.approve(&body);
                let result = f.service.submit(f.ctx(), body, auth).await.unwrap();
                assert_eq!(result.state, CommandState::Committed);
            }
            f.service.close().await.unwrap();
        }
    }
}
