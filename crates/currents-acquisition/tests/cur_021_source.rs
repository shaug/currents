//! CUR-021 synthetic holdings snapshots through the public parser and
//! evidence validation. Every identity and amount is invented.
use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
use currents_domain::evidence::ValidatedEvidenceBatchV1;
use currents_types::*;

fn id(n: u32) -> RecordId {
    serde_json::from_value(serde_json::json!(format!(
        "00000000-0000-0000-0000-{n:012}"
    )))
    .unwrap()
}

fn unit(n: u32, code: &str, exponent: u8, kind: UnitKind) -> UnitDefinition {
    UnitDefinition {
        id: UnitId(id(n).0),
        exponent,
        native_code: code.into(),
        kind,
    }
}

/// SECA: native code `SCA`, exponent 4.
fn seca() -> UnitDefinition {
    unit(6001, "SCA", 4, UnitKind::Security)
}

fn context(rows: u32) -> ImportContextV1 {
    ImportContextV1 {
        receipt_id: id(7000),
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
                observation: id(7001 + 3 * i),
                product: id(7002 + 3 * i),
                facet: id(7003 + 3 * i),
            })
            .collect(),
    }
}

/// `source` parsed by the holdings profile, which names SECA and SECB.
fn parse(source: &serde_json::Value) -> Result<CaptureBatchV1, EngineError> {
    let rows = source["positions"].as_array().map_or(0, Vec::len);
    let security = |n: u32, identifier: &str, unit: UnitDefinition| HoldingsSecurityV1 {
        identifier: SecuritySourceIdV1 {
            source: "synthetic-broker".into(),
            identifier: identifier.into(),
        },
        security: id(n),
        unit,
    };
    let profile = HoldingsProfileV1 {
        schema_version: 1,
        parser_version: "1".into(),
        currency: unit(1, "USD", 2, UnitKind::Currency),
        securities: vec![
            security(6000, "SYN-0001", seca()),
            security(6010, "SYN-0002", unit(6011, "SCB", 4, UnitKind::Security)),
        ],
        snapshot: id(7090),
    };
    let bytes = serde_json::to_vec(source).unwrap();
    let context = context(u32::try_from(rows).unwrap());
    FileParser::parse_holdings(
        BoundedEvidence::new("holdings.json", bytes, context)?,
        profile,
    )
}

#[test]
fn cur_021_snapshot_rejects_bad_input() {
    let original: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/investments/quantity/holdings-x-complete-2026-11-30.json"
    ))
    .unwrap();
    let batch = parse(&original).unwrap();
    assert!(ValidatedEvidenceBatchV1::new(batch).is_ok());

    let mut twice = original.clone();
    let row = twice["positions"][0].clone();
    twice["positions"].as_array_mut().unwrap().push(row);
    let mut precise = original.clone();
    precise["positions"][0]["quantity"] = "6.00001".into();
    let mut missing = original.clone();
    missing.as_object_mut().unwrap().remove("completeness");
    let mut unknown = original;
    unknown["completeness"] = "Total".into();
    for bad in [twice, missing, unknown] {
        assert_eq!(
            parse(&bad).err().unwrap().class,
            ErrorClass::InvalidReference
        );
    }
    assert_eq!(
        parse(&precise).err().unwrap().class,
        ErrorClass::PrecisionOrOverflow
    );

    // The ordinary CSV profile still parses SECA movement rows.
    let csv = include_bytes!("../../../fixtures/investments/quantity/quantity-q.csv");
    let profile = ParserProfileV1 {
        schema_version: 1,
        format: ImportFormatV1::SyntheticCsv,
        parser_version: "1".into(),
        unit: seca(),
    };
    let input = BoundedEvidence::new("quantity-q.csv", csv.to_vec(), context(3)).unwrap();
    let batch = FileParser::parse(input, profile.clone()).unwrap();
    let quantities: Vec<_> = batch
        .products
        .iter()
        .map(|p| p.product.facets[0].amount.clone())
        .collect();
    let seca = |n: i128| SourceValue::Known(Amount::new(n, 0, seca().id).unwrap());
    assert_eq!(quantities, vec![seca(10), seca(-4), seca(10)]);
    assert!(
        batch
            .observations
            .iter()
            .all(|o| o.semantic_kind == ObservationKind::Movement)
    );
    assert!(ValidatedEvidenceBatchV1::new(batch).is_ok());
    // That profile does not read holdings.
    let holdings = ParserProfileV1 {
        format: ImportFormatV1::SyntheticHoldings,
        ..profile
    };
    let input = BoundedEvidence::new("quantity-q.csv", csv.to_vec(), context(3)).unwrap();
    assert!(FileParser::parse(input, holdings).is_err());
}
