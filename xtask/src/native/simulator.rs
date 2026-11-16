//! Deterministic selection from actual captured simulator inventories; never creates devices.
use serde_json::Value;
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("BLOCKED: simulator inventory: {message}"))
    }
}
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub(super) fn select(runtime: &str, runtimes: &Value, devices: &Value) -> Result<String> {
    let expected = format!(
        "com.apple.CoreSimulator.SimRuntime.iOS-{}",
        runtime.replace('.', "-")
    );
    let rows = runtimes["runtimes"]
        .as_array()
        .ok_or("BLOCKED: missing simulator runtimes")?;
    let mut identifiers = BTreeSet::new();
    let mut found = false;
    for row in rows {
        let id = row["identifier"]
            .as_str()
            .ok_or("BLOCKED: missing runtime identifier")?;
        require(
            !id.is_empty() && identifiers.insert(id),
            "duplicate or empty runtime identifier",
        )?;
        require(
            row["version"].as_str().is_some() && row["isAvailable"].is_boolean(),
            "malformed runtime row",
        )?;
        if id.starts_with("com.apple.CoreSimulator.SimRuntime.iOS-") && row["version"] == runtime {
            require(id == expected, "runtime identifier and version disagree")?;
        }
        if id == expected {
            require(
                row["version"] == runtime && row["isAvailable"] == true,
                "locked runtime unavailable or mismatched",
            )?;
            found = true;
        }
    }
    require(found, "requested iOS runtime missing")?;
    let groups = devices["devices"]
        .as_object()
        .ok_or("BLOCKED: missing simulator devices")?;
    let mut ids = BTreeSet::new();
    let mut candidates = BTreeSet::new();
    for (group, rows) in groups {
        let rows = rows
            .as_array()
            .ok_or("BLOCKED: malformed simulator device group")?;
        for row in rows {
            let id = row["udid"]
                .as_str()
                .ok_or("BLOCKED: missing simulator UUID")?;
            require(
                uuid(id) && ids.insert(id.to_ascii_uppercase()),
                "invalid or duplicate simulator UUID",
            )?;
            let available = row["isAvailable"]
                .as_bool()
                .ok_or("BLOCKED: missing simulator availability")?;
            if group == &expected && available {
                candidates.insert(id);
            }
        }
    }
    candidates
        .first()
        .map(|id| (*id).to_owned())
        .ok_or("BLOCKED: no available simulator in requested runtime".into())
}
