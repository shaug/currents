//! Exact governing policy projection from already validated ordered history.
//! This index is derived, never a new authority or a latest-policy fallback.
use currents_types::*;
use std::collections::BTreeMap;
pub struct PolicyHistory;
impl PolicyHistory {
    /// Return a prospective index without changing the accepted prefix. The
    /// replay caller publishes it only with the rest of its validated commit.
    pub fn project(
        state: &DomainState,
        records: &[AuthoritativeRecordV1],
    ) -> Result<BTreeMap<(RecordId, u32), DatasetPolicyV1>, EngineError> {
        for ((id, version), policy) in &state.dataset_policies {
            if *id != policy.id || *version != policy.version || id.0.is_nil() {
                return Err(EngineError::invalid());
            }
            if *version == 0 {
                return Err(EngineError::version());
            }
        }
        let mut prospective = state.dataset_policies.clone();
        for record in records {
            if let AuthoritativeRecordV1::DatasetPolicy(policy) = record {
                if policy.id.0.is_nil() {
                    return Err(EngineError::invalid());
                }
                if policy.version == 0 {
                    return Err(EngineError::version());
                }
                let key = (policy.id, policy.version);
                if prospective.get(&key).is_some_and(|old| old != policy) {
                    return Err(EngineError::invalid());
                }
                prospective.insert(key, policy.clone());
            }
        }
        Ok(prospective)
    }
}
