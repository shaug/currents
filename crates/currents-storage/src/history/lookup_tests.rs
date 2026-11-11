//! Stored-record integrity remains independent of caller digest matching.
use super::*;

fn commit() -> DomainCommitV1 {
    DomainCommitV1 {
        seq: CommitSeq(1),
        previous_digest: Digest::default(),
        records: vec![],
        decisions: vec![],
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest::default(),
        },
        command_result: CommandResultV1 {
            command_id: CommandId(uuid::Uuid::from_u128(1)),
            digest: Digest([1; 32]),
            state: CommandState::Committed,
            commit: Some(CommitSeq(1)),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
        digest: Digest::default(),
    }
}
fn seal(mut commit: DomainCommitV1) -> Vec<u8> {
    commit.digest = Digest::default();
    commit.digest = hash(&canonical(&commit).unwrap());
    canonical(&commit).unwrap()
}
#[test]
fn record_integrity_precedes_caller_digest() {
    let commit = commit();
    let command = commit.command_result.command_id;
    let bytes = seal(commit.clone());
    let decoded = checked_record(&bytes, command).unwrap();
    assert_eq!(
        checked_command(&bytes, command, commit.command_result.digest).unwrap(),
        decoded
    );
    let wrong_digest = Digest([99; 32]);
    assert_eq!(
        checked_command(&bytes, command, wrong_digest)
            .unwrap_err()
            .class,
        EngineError::invalid().class
    );
    let mut pending = commit.clone();
    pending.command_result.state = CommandState::Pending;
    let mut mismatched_commit = commit;
    mismatched_commit.command_result.commit = Some(CommitSeq(2));
    let mut bad_hash = decoded.clone();
    bad_hash.digest = Digest([99; 32]);
    for (bytes, id) in [
        (b"{}".to_vec(), command),
        (serde_json::to_vec_pretty(&decoded).unwrap(), command),
        (bytes, CommandId(uuid::Uuid::from_u128(2))),
        (seal(pending), command),
        (seal(mismatched_commit), command),
        (canonical(&bad_hash).unwrap(), command),
    ] {
        assert_eq!(
            checked_record(&bytes, id).unwrap_err().class,
            ErrorClass::IntegrityFailure
        );
        assert_eq!(
            checked_command(&bytes, id, wrong_digest).unwrap_err().class,
            ErrorClass::IntegrityFailure
        );
    }
}
