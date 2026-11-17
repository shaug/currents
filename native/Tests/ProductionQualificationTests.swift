import Testing

// An explicit failing gate, not skipped tests or a synthetic production pass.
// Replace only when signed installed targets and their actual checks exist.
@Test func cur_004_production_requires_signed_installed_targets() {
    Issue.record("BLOCKED: signed installed CurrentsApp/CurrentsOwner, authorization helper, entitlements, and minimum/current OS qualification are unavailable; synthetic fixtures cannot qualify production")
}
