use currents_types::*;
#[test]
fn cur_015_3_parser_binding_preserves_v1_and_addresses_correction() {
    let old = serde_json::json!({"capture":"00000000-0000-0000-0000-000000000001", "source_record":"r", "source_revision":"v1", "facet":"movement"});
    let binding: CapturedFacetBindingV1 = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(binding).unwrap(), old);
    let mut corrected = old;
    corrected["parse_revision"] = serde_json::json!("00000000-0000-0000-0000-000000000002");
    let result = serde_json::from_value::<CapturedFacetBindingV1>(corrected.clone());
    assert!(
        result.is_ok(),
        "explicit corrected interpretation must be addressable: {result:?}"
    );
    assert_eq!(serde_json::to_value(result.unwrap()).unwrap(), corrected);
}
