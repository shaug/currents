# Currents contracts v1

CUR-002 publishes the foundational C02/C03 DTOs, not a service, replay engine,
financial validator, native accounting adapter or storage implementation.
`contracts.json` is generated from the same Rust field/variant declarations.
Select a root using `#/$defs/CommandBodyV1`, `#/$defs/RequestV1`,
`#/$defs/AuthoritativeRecordV1`, etc. Definitions are intentionally closed:
unknown financial variants and fields are errors, not silently discarded data.

Regenerate from the repository root:

```sh
cargo run --locked -p currents-protocol --example contracts > schemas/v1/contracts.json
python3 fixtures/contracts/v1/generate.py
cargo test --locked -p currents-types --test cur_002
cargo xtask ticket CUR-002
cargo xtask check quality
```

## Version and ownership

Protocol major `1` selects these exact DTOs and canonical rules. Another major
is refused. Compatible optional additions must be deliberately published with
updated schemas/readers/fixtures before consumers use them; arbitrary unknown
fields are not an extension mechanism. Authoritative variants remain explicit.
The owning subsystem may extend its variant through the documented coordinated
contract-change workflow before use. Fixtures are synthetic shape examples,
not valid adopted financial transactions, grants or complete datasets. Domain
owners must validate reference kinds/closure, conservation, dataset ownership,
policy versions, supersession cycles and financial semantics before adoption.
The typed native checkpoint envelope is not a C05 frozen-operation implementation.

## Numbers, bounds and temporal meaning

Untrusted messages enter through `currents_protocol::decode`, which rejects input
above 1,048,576 bytes, structural nesting above 64 and collections above 16,384
members before typed use. Duplicate object keys (including escaped equivalents)
are rejected. Errors discard parser diagnostics so private input is not echoed.
The transport bounds are not archive/large-closure limits. Those owners must
provide their separately bounded streaming/staging paths, not enlarge this codec.

Amount coefficients are decimal integer strings in `[-i128::MAX, i128::MAX]`,
scales are 0..18, and unit IDs are UUID strings. Serialization removes trailing
fractional zeros and normalizes zero; `12300/10^4` becomes `123/10^2` (1.23).
`decimal_string` never emits exponent notation. No implicit rounding occurs.
Rescaling and exact rational arithmetic use bounded BigInt intermediates and
checked conversion back to i128. Unit exponents are 0..18; native codes are
1–16 uppercase ASCII letters/digits. Immutable/collision-checked catalog changes
remain domain/accounting validation, not merely JSON shape checks.

Commit sequences are decimal strings through u64::MAX. Digests are exactly 64
lowercase hexadecimal characters. JSON Schema captures the syntactic decimal
limits; the Rust decoder additionally checks exact integer ranges and calendar
validity. Validate through the decoder even after JSON Schema validation.

Civil dates remain `time::Date`, encoded as YYYY-MM-DD, never midnight instants.
Instants are explicit `time::OffsetDateTime`, encoded as RFC3339 UTC; unknown
source timezone/offset/precision stays unknown. Original source text is retained.
`ReadBasis.evaluation_instant` is supplied by the caller, not sampled implicitly.
`SourceValue` explicitly tags Missing, Null, Absent, Deleted or Known. None of
these states is silently converted into another. Required nullable DTO fields
must be present with null; they cannot disappear by serde's Option default.

## Canonical preimages and authorization

Canonical bytes are RFC 8785/JCS UTF-8, using the pinned canonicalizer. Property
order uses UTF-16 code units, not Unicode scalar order. Ordered arrays stay
ordered; evidence bytes keep their independent content digests.

* Command digest = SHA-256(canonical bytes of the complete CommandBodyV1).
  Expected head is included. No session, capability or approval bytes are input.
* Domain commit digest = SHA-256(canonical bytes of DomainCommitV1 with only its
  own `digest` field omitted). Previous digest, ordered records and decisions,
  mapped native checkpoint and command result are included. The command result's
  digest is the command digest, not the outer commit digest.
* Native prebinding/Document digests belong to C05 and are not computed here.

AuthorizationEnvelope and RequestV1 deliberately have no Serialize or Debug
implementation. Trusted RequestContext cannot be deserialized. Accepted decision
envelopes contain actor/grant identity, recorded time, policy, references,
command digest and supersession, never bearer fields. Identity/time generation
and authorization decisions belong to host/service acceptance; these DTOs neither
create IDs/timestamps nor confer authority. `TrustedClock` is a host interface;
ReadBasis timestamps cannot establish current authorization expiry.
