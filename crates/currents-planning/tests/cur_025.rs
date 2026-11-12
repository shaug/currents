//! CUR025 portable behavior through public APIs, with only synthetic records.
use currents_planning::{
    calendar::{CalendarEngine, ConversionPolicy, MAX_TIMEZONE_BYTES},
    commitments::{CommitmentEngine, resolve_occurrence},
};
use currents_types::*;
fn date(s: &str) -> Date {
    serde_json::from_value::<CivilDateV1>(serde_json::json!(s))
        .unwrap()
        .0
}
fn id(n: u32) -> RecordId {
    serde_json::from_value(serde_json::json!(format!(
        "00000000-0000-0000-0000-{n:012}"
    )))
    .unwrap()
}
fn fixtures() -> Vec<AuthoritativeRecordV1> {
    serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap()
}
fn calendar() -> CalendarEngine {
    let (zone, bytes) = jiff_tzdb::get("America/New_York").unwrap();
    CalendarEngine::new(zone, jiff_tzdb::VERSION.unwrap(), bytes).unwrap()
}
fn commitment() -> CommitmentV1 {
    let mut c = fixtures()
        .into_iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::Commitment(c) => Some(c),
            _ => None,
        })
        .unwrap();
    c.id = CommitmentId(id(25).0);
    c.timezone = "America/New_York".into();
    c.timezone_data_version = jiff_tzdb::VERSION.unwrap().into();
    *c.calendar_data = calendar().calendar_data();
    c.provenance.kind = DecisionKind::Planning;
    c.amounts.nominal = SourceValue::Known(Amount {
        coefficient: 10001,
        scale: 2,
        unit_id: UnitId(id(1).0),
    });
    c.legs = vec![PlannedLegV1 {
        id: id(26),
        account_id: AccountId(id(1).0),
        amounts: c.amounts.clone(),
        settlement: c.settlement.clone(),
    }];
    c.start = CivilDateV1(date("2024-01-31"));
    c.due_date = c.start;
    c.recurrence = RecurrenceV1 {
        cadence: RecurrenceCadenceV1::Monthly,
        interval: 1,
        month: None,
        day: Some(RecurrenceDayV1::Day(31)),
    };
    c
}
fn state(c: &CommitmentV1) -> DomainState {
    let mut s = DomainState::default();
    for r in fixtures() {
        match r {
            AuthoritativeRecordV1::Dataset(mut d) => {
                d.household_timezone = SourceValue::Known(c.timezone.clone());
                s.records.insert(id(10), AuthoritativeRecordV1::Dataset(d));
            }
            AuthoritativeRecordV1::AccountComponent(a) => {
                s.records
                    .insert(id(11), AuthoritativeRecordV1::AccountComponent(a));
            }
            _ => {}
        }
    }
    s
}
fn save(s: &mut DomainState, c: &CommitmentV1) {
    let plan = CommitmentEngine::new(calendar()).set(s, c).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    s.records.insert(RecordId(c.id.0), plan.records[0].clone());
}
fn key(c: &CommitmentV1, n: u64) -> OccurrenceKeyV1 {
    OccurrenceKeyV1 {
        commitment_id: c.id,
        series_generation: c.series_generation,
        ordinal: CommitSeq(n),
    }
}
fn pattern(prior: &CommitmentV1, from: u64, day: u8, not_before: &str) -> CommitmentV1 {
    let mut c = prior.clone();
    c.version += 1;
    c.series_generation += 1;
    c.recurrence.day = Some(RecurrenceDayV1::Day(day));
    c.supersession = Some(CommitmentSupersessionV1 {
        prior_version: prior.version,
        prior_generation: prior.series_generation,
        from_ordinal: CommitSeq(from),
        not_before: CivilDateV1(date(not_before)),
        remaps: vec![],
    });
    c
}
#[test]
fn cur_025_1() {
    let edges: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/planning/calendar/edges.json"
    ))
    .unwrap();
    for edge in edges.as_array().unwrap() {
        let mut c = commitment();
        c.start = CivilDateV1(date(edge["start"].as_str().unwrap()));
        c.due_date = c.start;
        let actual = calendar()
            .expand(&c, c.start.0, edge["months"].as_u64().unwrap() as u8)
            .unwrap();
        assert_eq!(
            actual
                .iter()
                .map(|o| o.due_date.0.to_string())
                .collect::<Vec<_>>(),
            edge["dates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
    }
    let engine = calendar();
    let mut c = commitment();
    let expanded = engine.expand(&c, c.start.0, 1).unwrap();
    assert_eq!(
        expanded.iter().map(|o| o.due_date.0).collect::<Vec<_>>(),
        vec![date("2024-01-31"), date("2024-02-29")]
    );
    c.start = CivilDateV1(date("2023-01-31"));
    c.due_date = c.start;
    assert_eq!(
        engine.expand(&c, c.start.0, 1).unwrap()[1].due_date.0,
        date("2023-02-28")
    );
    c.start = CivilDateV1(date("2024-02-29"));
    c.due_date = c.start;
    c.recurrence.cadence = RecurrenceCadenceV1::Annual;
    c.recurrence.day = Some(RecurrenceDayV1::Day(29));
    c.recurrence.month = Some(2);
    assert_eq!(
        engine.expand(&c, c.start.0, 12).unwrap()[1].due_date.0,
        date("2025-02-28")
    );
    c = commitment();
    c.start = CivilDateV1(date("2024-01-30"));
    c.due_date = c.start;
    c.recurrence.day = Some(RecurrenceDayV1::Day(30));
    assert_eq!(
        engine.expand(&c, c.start.0, 3).unwrap()[2].due_date.0,
        date("2024-03-30")
    );
    c.recurrence.day = Some(RecurrenceDayV1::LastDay(PlanningEmptyV1 {}));
    assert_eq!(
        engine.expand(&c, c.start.0, 3).unwrap()[2].due_date.0,
        date("2024-03-31")
    );
    c = commitment();
    c.start = CivilDateV1(date("2024-06-01"));
    c.due_date = c.start;
    c.recurrence = RecurrenceV1 {
        cadence: RecurrenceCadenceV1::Daily,
        interval: 2,
        month: None,
        day: None,
    };
    let daily = engine.expand(&c, c.start.0, 1).unwrap();
    assert_eq!(daily.len(), 16);
    assert_eq!(daily[0].settlement_range.earliest, c.start);
    assert_eq!(daily.last().unwrap().due_date.0, date("2024-07-01"));
    c.recurrence.cadence = RecurrenceCadenceV1::Weekly;
    c.recurrence.interval = 1;
    assert_eq!(engine.expand(&c, c.start.0, 1).unwrap().len(), 5);
    c.settlement = SettlementRuleV1::NextWeekday(PlanningEmptyV1 {});
    assert_eq!(
        engine.expand(&c, c.start.0, 1).unwrap()[0]
            .settlement_range
            .earliest
            .0,
        date("2024-06-03")
    );
    c.settlement = SettlementRuleV1::PreviousWeekday(PlanningEmptyV1 {});
    assert_eq!(
        engine.expand(&c, c.start.0, 1).unwrap()[0]
            .settlement_range
            .earliest
            .0,
        date("2024-05-31")
    );
    c.end = Some(CivilDateV1(date("2024-06-08")));
    assert_eq!(engine.expand(&c, c.start.0, 1).unwrap().len(), 2);
    c.exceptions = vec![PlanningExceptionV1 {
        ordinal: CommitSeq(1),
        action: PlanningExceptionActionV1::Skip,
        due_date: None,
        legs: vec![],
        reason: "synthetic skip".into(),
    }];
    assert_eq!(engine.expand(&c, c.start.0, 1).unwrap().len(), 1);
    for months in [0, 2, 13] {
        assert!(engine.expand(&c, c.start.0, months).is_err());
    }
    c.recurrence.interval = 0;
    assert!(engine.expand(&c, c.start.0, 1).is_err());
    c = commitment();
    c.recurrence_version = 2;
    assert_eq!(
        engine.expand(&c, c.start.0, 1).unwrap_err().class,
        ErrorClass::UnsupportedVersion
    );
}
#[test]
fn cur_025_2() {
    let engine = calendar();
    let c = commitment();
    let gap = "2024-03-10T02:30:00".parse().unwrap();
    let overlap = "2024-11-03T01:30:00".parse().unwrap();
    assert!(engine.resolve_local(gap, ConversionPolicy::Reject).is_err());
    assert!(
        engine
            .resolve_local(overlap, ConversionPolicy::Reject)
            .is_err()
    );
    for local in [gap, overlap] {
        let earlier = engine
            .resolve_local(local, ConversionPolicy::Earlier)
            .unwrap();
        let later = engine
            .resolve_local(local, ConversionPolicy::Later)
            .unwrap();
        assert_eq!((later - earlier).whole_seconds(), 3600);
    }
    let saved = engine.expand(&c, c.start.0, 12).unwrap();
    let wire = serde_json::to_string(&saved).unwrap();
    assert!(!wire.contains("T00:00"));
    let (_, bytes) = jiff_tzdb::get(&c.timezone).unwrap();
    assert!(CalendarEngine::new(&c.timezone, "future-tzdb", bytes).is_err());
    let mut historical = c.calendar_data.clone();
    historical.timezone_data_version = "previous-accepted-release".into();
    let older = CalendarEngine::from_saved(&historical, bytes).unwrap();
    let mut historical_commitment = c.clone();
    historical_commitment.timezone_data_version = historical.timezone_data_version.clone();
    historical_commitment.calendar_data = historical;
    assert_eq!(
        older.expand(&historical_commitment, c.start.0, 12).unwrap(),
        saved
    );
    let newer = CalendarEngine::from_saved(&historical_commitment.calendar_data, bytes).unwrap();
    assert_eq!(
        newer.expand(&c, c.start.0, 12).unwrap_err().class,
        ErrorClass::UnsupportedVersion
    );
    let restored: Vec<OccurrenceV1> = serde_json::from_str(&wire).unwrap();
    assert_eq!(restored, engine.expand(&c, c.start.0, 12).unwrap());
    let mut wrong = c.clone();
    wrong.calendar_data.timezone_data.digest = Digest([42; 32]);
    assert!(engine.expand(&wrong, c.start.0, 1).is_err());
    wrong = c.clone();
    wrong.timezone = "UTC".into();
    assert!(engine.expand(&wrong, c.start.0, 1).is_err());
    assert!(CalendarEngine::new("America/New_York", "synthetic", b"not tzif").is_err());
    assert!(
        CalendarEngine::new(
            "America/New_York",
            "synthetic",
            &vec![0; MAX_TIMEZONE_BYTES + 1]
        )
        .is_err()
    );
}
#[test]
fn cur_025_3() {
    supersession_boundary_regressions();
    supersession_boundary_edge_controls();
    let mut c = commitment();
    let mut s = state(&c);
    save(&mut s, &c);
    let mut history = vec![c.clone()];
    let old = calendar().expand(&c, c.start.0, 1).unwrap();
    c.version += 1;
    c.amounts.nominal = SourceValue::Known(Amount {
        coefficient: 20001,
        scale: 2,
        unit_id: UnitId(id(1).0),
    });
    c.legs[0].amounts = c.amounts.clone();
    c.due_date = CivilDateV1(date("2024-02-01"));
    save(&mut s, &c);
    history.push(c.clone());
    let edited = calendar().expand(&c, c.start.0, 3).unwrap();
    assert_eq!(old[0].key, edited[0].key);
    assert_eq!(edited[0].version, 2);
    let mut next = pattern(&c, 0, 15, "2024-02-29");
    next.start = CivilDateV1(date("2024-03-01"));
    next.due_date = next.start;
    let next_key = key(&next, 0);
    next.supersession
        .as_mut()
        .unwrap()
        .remaps
        .push(OccurrenceRemapV1 {
            old: key(&c, 0),
            new: Some(next_key),
        });
    save(&mut s, &next);
    history.push(next.clone());
    assert_eq!(
        resolve_occurrence(&history, &key(&c, 0)).unwrap(),
        Some(key(&next, 0))
    );
    assert_eq!(resolve_occurrence(&history, &key(&c, 1)).unwrap(), None);
    // A later amount/date edit retains the generation's original transition.
    next.version += 1;
    next.legs[0].amounts.nominal = SourceValue::Known(Amount {
        coefficient: 30001,
        scale: 2,
        unit_id: UnitId(id(1).0),
    });
    save(&mut s, &next);
    history.push(next.clone());
    let mut third = pattern(&next, 0, 20, "2024-03-15");
    third.start = CivilDateV1(date("2024-04-01"));
    third.due_date = third.start;
    let third_key = key(&third, 0);
    third
        .supersession
        .as_mut()
        .unwrap()
        .remaps
        .push(OccurrenceRemapV1 {
            old: key(&next, 0),
            new: Some(third_key),
        });
    save(&mut s, &third);
    history.push(third.clone());
    assert_eq!(
        resolve_occurrence(&history, &key(&c, 0)).unwrap(),
        Some(key(&third, 0))
    );
    let active = CommitmentEngine::new(calendar())
        .expand_history(&history, date("2024-01-01"), 12)
        .unwrap();
    assert!(active.iter().all(|o| o.key.series_generation == 3));
    assert_eq!(active.len(), 9);

    // A once-valid key becomes inactive when a later same-generation revision
    // removes it. Direct keys and one- or two-transition remaps agree with the
    // active expansion, without rewriting the accepted earlier snapshot.
    for (prefix_len, later_start, cutoff) in [
        (4, "2024-04-01", "2024-03-15"),
        (5, "2024-05-01", "2024-04-20"),
        (2, "2024-03-01", "2024-02-29"),
    ] {
        let prefix = &history[..prefix_len];
        let latest = prefix.last().unwrap();
        let target = key(latest, 0);
        let sources: Vec<_> = [key(&c, 0), key(&next, 0), key(&third, 0)]
            .into_iter()
            .filter(|k| k.series_generation <= latest.series_generation)
            .collect();
        let planner = CommitmentEngine::new(calendar());
        let saved = planner
            .expand_history(prefix, date("2024-01-01"), 12)
            .unwrap();
        assert!(saved.iter().any(|o| o.key == target));
        for removal in ["cancel", "skip", "end", "start"] {
            let mut removed = latest.clone();
            removed.version += 1;
            match removal {
                "cancel" | "skip" => removed.exceptions.push(PlanningExceptionV1 {
                    ordinal: CommitSeq(0),
                    action: if removal == "cancel" {
                        PlanningExceptionActionV1::Cancel
                    } else {
                        PlanningExceptionActionV1::Skip
                    },
                    due_date: None,
                    legs: vec![],
                    reason: "Synthetic owner removed the expectation".into(),
                }),
                "end" => removed.end = Some(removed.start),
                "start" => removed.start = CivilDateV1(date(later_start)),
                _ => unreachable!(),
            }
            let mut accepted = state(&removed);
            for revision in prefix {
                save(&mut accepted, revision);
            }
            save(&mut accepted, &removed);
            let mut changed = prefix.to_vec();
            changed.push(removed.clone());
            let remaining = planner
                .expand_history(&changed, date("2024-01-01"), 12)
                .unwrap();
            assert!(!remaining.iter().any(|o| o.key == target));
            for source in &sources {
                assert_eq!(
                    resolve_occurrence(prefix, source).unwrap(),
                    Some(target.clone())
                );
                assert_eq!(
                    resolve_occurrence(&changed, source).unwrap(),
                    None,
                    "{removal} must retire generation {} through source {source:?}",
                    latest.series_generation
                );
            }
            for occurrence in &remaining {
                assert_eq!(
                    resolve_occurrence(&changed, &occurrence.key).unwrap(),
                    Some(occurrence.key.clone())
                );
            }
            assert_eq!(
                planner
                    .expand_history(prefix, date("2024-01-01"), 12)
                    .unwrap(),
                saved
            );
            // A subsequent pattern change cannot resurrect the removed key or
            // a match that reached it through an earlier remap.
            let mut later = pattern(&removed, 0, 27, cutoff);
            later.start = CivilDateV1(date("2024-06-01"));
            later.due_date = later.start;
            later.end = None;
            later.exceptions.clear();
            save(&mut accepted, &later);
            changed.push(later.clone());
            for source in &sources {
                assert_eq!(resolve_occurrence(&changed, source).unwrap(), None);
            }
            assert_eq!(
                resolve_occurrence(&changed, &key(&later, 0)).unwrap(),
                Some(key(&later, 0))
            );
        }
    }
    // Inactive historical keys are not interchangeable with fabricated keys
    // that never denoted an occurrence in any accepted revision.
    let mut foreign = key(&third, 0);
    foreign.commitment_id = CommitmentId(id(999).0);
    let mut unknown_generation = key(&third, 0);
    unknown_generation.series_generation = 999;
    for invalid in [foreign, unknown_generation, key(&third, u64::MAX)] {
        assert_eq!(
            resolve_occurrence(&history, &invalid).unwrap_err().class,
            ErrorClass::InvalidReference
        );
    }
    for action in [
        PlanningExceptionActionV1::Cancel,
        PlanningExceptionActionV1::Skip,
    ] {
        let mut never_active = commitment();
        never_active.exceptions.push(PlanningExceptionV1 {
            ordinal: CommitSeq(0),
            action,
            due_date: None,
            legs: vec![],
            reason: "Synthetic occurrence absent from its first revision".into(),
        });
        let mut accepted = state(&never_active);
        save(&mut accepted, &never_active);
        assert_eq!(
            resolve_occurrence(std::slice::from_ref(&never_active), &key(&never_active, 0))
                .unwrap_err()
                .class,
            ErrorClass::InvalidReference
        );
    }
    let mut bounded = commitment();
    bounded.end = Some(bounded.start);
    save(&mut state(&bounded), &bounded);
    assert_eq!(
        resolve_occurrence(std::slice::from_ref(&bounded), &key(&bounded, 1))
            .unwrap_err()
            .class,
        ErrorClass::InvalidReference
    );
    let mut valid_transition = pattern(&third, 0, 21, "2024-04-20");
    // The prior series' ordinal zero is April 20, despite its April 1 anchor.
    // Satisfy that cutoff so each variant reaches the intended remap guard.
    valid_transition.start = CivilDateV1(date("2024-04-20"));
    valid_transition.due_date = valid_transition.start;
    valid_transition.supersession.as_mut().unwrap().remaps = vec![
        OccurrenceRemapV1 {
            old: key(&third, 0),
            new: Some(key(&valid_transition, 0)),
        },
        OccurrenceRemapV1 {
            old: key(&third, 1),
            new: Some(key(&valid_transition, 1)),
        },
    ];
    assert!(
        CommitmentEngine::new(calendar())
            .set(&s, &valid_transition)
            .is_ok(),
        "distinct remap targets in the new generation must be accepted"
    );
    let mut duplicate = valid_transition.clone();
    duplicate.supersession.as_mut().unwrap().remaps[1].new = Some(key(&valid_transition, 0));
    assert!(
        CommitmentEngine::new(calendar())
            .set(&s, &duplicate)
            .is_err(),
        "duplicate remap targets must be rejected"
    );
    let mut wrong_generation = valid_transition.clone();
    wrong_generation.supersession.as_mut().unwrap().remaps[0]
        .new
        .as_mut()
        .unwrap()
        .series_generation = 999;
    assert!(
        CommitmentEngine::new(calendar())
            .set(&s, &wrong_generation)
            .is_err(),
        "a remap target in the wrong generation must be rejected"
    );
    let mut absent_transition = duplicate.clone();
    absent_transition.supersession = None;
    assert!(
        CommitmentEngine::new(calendar())
            .set(&s, &absent_transition)
            .is_err()
    );
    let mut invalid_version = third.clone();
    invalid_version.version += 2;
    assert!(
        CommitmentEngine::new(calendar())
            .set(&s, &invalid_version)
            .is_err()
    );
    let mut stale = c.clone();
    stale.version = 1;
    assert!(CommitmentEngine::new(calendar()).set(&s, &stale).is_err());
}
#[test]
fn cur_025_4() {
    let mut c = commitment();
    let mut s = state(&c);
    let planner = CommitmentEngine::new(calendar());
    assert!(matches!(
        planner.set(&s, &c).unwrap().accounting,
        AccountingIntent::NoEffect(_)
    ));
    save(&mut s, &c);
    for future in ["2025-01-01", "2035-01-01"] {
        assert!(calendar().expand(&c, date(future), 12).is_ok());
        assert_eq!(
            s.records.get(&RecordId(c.id.0)),
            Some(&AuthoritativeRecordV1::Commitment(c.clone()))
        );
    }
    c.provenance.authority = AuthorityKind::Source;
    assert!(planner.set(&s, &c).is_err());
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("currents-accounting"));
    assert!(!manifest.contains("currents-storage"));
    assert!(!manifest.contains("sqlx"));
    s = state(&commitment());
    s.records.remove(&id(10));
    assert!(planner.set(&s, &commitment()).is_err());
}

// These assertions would fail if same-generation admission or history validation
// forgot the predecessor boundary. Explicit dates are independent test oracles.
fn supersession_boundary_regressions() {
    let planner = CommitmentEngine::new(calendar());
    for shifted in [false, true] {
        let mut first = commitment();
        first.start = CivilDateV1(date("2024-01-01"));
        first.due_date = first.start;
        first.recurrence = RecurrenceV1 {
            cadence: RecurrenceCadenceV1::Daily,
            interval: 1,
            month: None,
            day: None,
        };
        if shifted {
            first.exceptions.push(PlanningExceptionV1 {
                ordinal: CommitSeq(0),
                action: PlanningExceptionActionV1::Replace,
                due_date: Some(CivilDateV1(date("2024-01-15"))),
                legs: vec![],
                reason: "Synthetic surviving pre-cutoff date shift".into(),
            });
        }
        let mut second = first.clone();
        second.version = 2;
        second.series_generation = 2;
        second.recurrence.cadence = RecurrenceCadenceV1::Weekly;
        second.start = CivilDateV1(date("2024-01-20"));
        second.due_date = second.start;
        second.exceptions.clear();
        second.supersession = Some(CommitmentSupersessionV1 {
            prior_version: 1,
            prior_generation: 1,
            from_ordinal: CommitSeq(10),
            not_before: CivilDateV1(date(if shifted { "2024-01-16" } else { "2024-01-11" })),
            remaps: vec![],
        });
        if !shifted {
            let wire: serde_json::Value = serde_json::from_str(include_str!(
                "../../../fixtures/planning/calendar/supersession.json"
            ))
            .unwrap();
            let saved: CommitmentSupersessionV1 = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(second.supersession.as_ref(), Some(&saved));
            assert_eq!(serde_json::to_value(&saved).unwrap(), wire);
            let mut missing_bound = wire;
            missing_bound.as_object_mut().unwrap().remove("not_before");
            assert!(serde_json::from_value::<CommitmentSupersessionV1>(missing_bound).is_err());
        }
        let mut accepted = state(&first);
        save(&mut accepted, &first);
        for forged_floor in ["2024-01-01", "2024-01-20"] {
            let mut forged = second.clone();
            forged.supersession.as_mut().unwrap().not_before = CivilDateV1(date(forged_floor));
            assert!(
                planner.set(&accepted, &forged).is_err(),
                "transition must attest the exact derived floor, not merely a stricter or weaker one"
            );
        }
        save(&mut accepted, &second);
        let mut history = vec![first.clone(), second.clone()];
        let saved = planner.expand_history(&history, first.start.0, 1).unwrap();
        let saved_wire = serde_json::to_string(&saved).unwrap();
        let lower = if shifted { "2024-01-16" } else { "2024-01-11" };
        let mut safe = second.clone();
        safe.version += 1;
        safe.start = CivilDateV1(date(lower));
        safe.due_date = safe.start;
        safe.legs[0].amounts.nominal = SourceValue::Known(Amount {
            coefficient: 42001,
            scale: 2,
            unit_id: UnitId(id(1).0),
        });
        let mut rewritten_boundary = safe.clone();
        rewritten_boundary.supersession.as_mut().unwrap().not_before = first.start;
        assert!(planner.set(&accepted, &rewritten_boundary).is_err());
        save(&mut accepted, &safe);
        history.push(safe.clone());
        let edited = planner.expand_history(&history, first.start.0, 1).unwrap();
        let moved = edited.iter().find(|o| o.key == key(&second, 0)).unwrap();
        assert_eq!(moved.due_date, safe.start);
        assert_eq!(moved.version, 3);
        assert_eq!(safe.recurrence_version, first.recurrence_version);
        assert_eq!(
            resolve_occurrence(&history, &key(&second, 0)).unwrap(),
            Some(moved.key.clone())
        );
        assert_eq!(
            serde_json::to_string(
                &planner
                    .expand_history(&history[..2], first.start.0, 1)
                    .unwrap()
            )
            .unwrap(),
            saved_wire
        );
        let mut unsafe_edit = safe.clone();
        unsafe_edit.version += 1;
        unsafe_edit.start = CivilDateV1(date(if shifted { "2024-01-15" } else { "2024-01-01" }));
        unsafe_edit.due_date = unsafe_edit.start;
        assert!(
            planner.set(&accepted, &unsafe_edit).is_err(),
            "later edit must preserve the predecessor boundary (shifted={shifted})"
        );
        let mut invalid_history = history.clone();
        invalid_history.push(unsafe_edit);
        assert!(
            planner
                .expand_history(&invalid_history, first.start.0, 1)
                .is_err()
        );
        assert!(resolve_occurrence(&invalid_history, &key(&first, 0)).is_err());

        // A legal earlier anchor can leave ordinal zero inactive. The following
        // transition must inherit the earlier generation's boundary, not reopen it.
        safe.version += 1;
        safe.due_date = first.due_date;
        save(&mut accepted, &safe);
        history.push(safe.clone());
        let mut third = safe.clone();
        third.version += 1;
        third.series_generation += 1;
        third.recurrence.cadence = RecurrenceCadenceV1::Daily;
        third.recurrence.interval = 2;
        third.due_date = third.start;
        third.supersession = Some(CommitmentSupersessionV1 {
            prior_version: safe.version,
            prior_generation: safe.series_generation,
            from_ordinal: CommitSeq(0),
            not_before: CivilDateV1(date(lower)),
            remaps: vec![],
        });
        let mut lost_inheritance = third.clone();
        lost_inheritance.supersession.as_mut().unwrap().not_before = first.start;
        assert!(planner.set(&accepted, &lost_inheritance).is_err());
        save(&mut accepted, &third);
        history.push(third.clone());
        assert_eq!(
            resolve_occurrence(&history, &key(&first, 0)).unwrap(),
            Some(key(&first, 0))
        );
        let active = planner.expand_history(&history, first.start.0, 1).unwrap();
        assert!(!active.iter().any(|o| o.key.series_generation == 2));
        assert!(
            active
                .iter()
                .filter(|o| o.key.series_generation == 3)
                .all(|o| o.due_date.0 >= date(lower))
        );
        third.version += 1;
        third.start = CivilDateV1(date("2024-01-01"));
        third.due_date = third.start;
        assert!(
            planner.set(&accepted, &third).is_err(),
            "successive transitions must not forget inherited boundary"
        );
        history.push(third);
        assert!(planner.expand_history(&history, first.start.0, 1).is_err());
        assert!(resolve_occurrence(&history, &key(&first, 0)).is_err());
    }
}

fn supersession_boundary_edge_controls() {
    let planner = CommitmentEngine::new(calendar());
    let mut first = commitment();
    first.start = CivilDateV1(date("9999-10-01"));
    first.due_date = first.start;
    first.recurrence = RecurrenceV1 {
        cadence: RecurrenceCadenceV1::Daily,
        interval: 1,
        month: None,
        day: None,
    };
    first.exceptions = vec![PlanningExceptionV1 {
        ordinal: CommitSeq(0),
        action: PlanningExceptionActionV1::Replace,
        due_date: Some(CivilDateV1(date("9999-12-31"))),
        legs: vec![],
        reason: "Synthetic final representable date".into(),
    }];
    let mut next = first.clone();
    next.version = 2;
    next.series_generation = 2;
    next.recurrence.cadence = RecurrenceCadenceV1::Weekly;
    next.start = CivilDateV1(date("9999-10-11"));
    next.due_date = next.start;
    next.exceptions.clear();
    next.supersession = Some(CommitmentSupersessionV1 {
        prior_version: 1,
        prior_generation: 1,
        from_ordinal: CommitSeq(10),
        not_before: next.start,
        remaps: vec![],
    });
    // Only active, surviving pre-cutoff exceptions contribute. Ignoring either
    // liveness or the ordinal cutoff would incorrectly reject these controls.
    for inactive in ["skip", "cancel", "end", "before-start", "retired"] {
        let mut prior = first.clone();
        match inactive {
            "skip" => prior.exceptions[0].action = PlanningExceptionActionV1::Skip,
            "cancel" => prior.exceptions[0].action = PlanningExceptionActionV1::Cancel,
            "end" => prior.end = Some(CivilDateV1(date("9999-10-10"))),
            "before-start" => prior.exceptions[0].due_date = Some(CivilDateV1(date("9999-09-30"))),
            "retired" => prior.exceptions[0].ordinal = CommitSeq(10),
            _ => unreachable!(),
        }
        let mut accepted = state(&prior);
        save(&mut accepted, &prior);
        save(&mut accepted, &next);
        assert!(
            planner
                .expand_history(&[prior, next.clone()], first.start.0, 1)
                .is_ok(),
            "{inactive} is not a surviving late expectation"
        );
    }
    let mut accepted = state(&first);
    save(&mut accepted, &first);
    assert_eq!(
        planner.set(&accepted, &next).unwrap_err().class,
        ErrorClass::PrecisionOrOverflow
    );
    let invalid = vec![first.clone(), next.clone()];
    assert_eq!(
        planner
            .expand_history(&invalid, first.start.0, 1)
            .unwrap_err()
            .class,
        ErrorClass::PrecisionOrOverflow
    );
    assert_eq!(
        resolve_occurrence(&invalid, &key(&first, 0))
            .unwrap_err()
            .class,
        ErrorClass::PrecisionOrOverflow
    );

    // An inclusive floor at MAX is representable. Only an active predecessor
    // at MAX would require an impossible successor. No wrap/saturation is valid.
    first.exceptions[0].due_date = Some(CivilDateV1(date("9999-12-30")));
    next.start = CivilDateV1(date("9999-12-31"));
    next.due_date = next.start;
    next.supersession.as_mut().unwrap().from_ordinal = CommitSeq(91);
    next.supersession.as_mut().unwrap().not_before = next.start;
    let history = vec![first.clone(), next.clone()];
    assert_eq!(
        resolve_occurrence(&history, &key(&next, 0)).unwrap(),
        Some(key(&next, 0))
    );
    assert!(planner.expand_history(&history, first.start.0, 1).is_ok());
    // Admission's existing one-month horizon cannot extend beyond CivilDate MAX.
    let mut accepted = state(&first);
    save(&mut accepted, &first);
    assert_eq!(
        planner.set(&accepted, &next).unwrap_err().class,
        ErrorClass::PrecisionOrOverflow
    );
}
