# S5: Reinterpret adopted activity

`reinterpretation.json` is synthetic input for
`cur_016_reinterpretation_encrypted_commit` in
`crates/currents-service/tests/cur_016.rs`.

An unexplained 3,300 USD deposit to account A on 2026-11-02 is adopted as
debit A, credit unresolved equity. The owner then corrects its interpretation
to wages income. The correction reverses the old recognition at its own dates
and adds debit A, credit wages with the same dates. The deposit's leg stays.

The test commits the correction through the encrypted writer. History replayed
through the commit before the correction still shows the unresolved-equity
recognition as active. History replayed through the correction shows the same
postings on A, unresolved equity 3,300 lower and wages 3,300 higher. Reopening
the store and rebuilding it from history both give the same native head.

This covers one interpretation change on stable legs. It does not cover
corrections that change legs, claims or matches across reports, or the full
S5 reporting journey.
