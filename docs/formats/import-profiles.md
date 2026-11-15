# Bounded synthetic import profiles

CUR012 implements C07 evidence capture and pure publication planning. It does not
connect accounts, fetch resources, recognize purchases, select provider revisions,
write SQL, or enable a production client. All examples are synthetic.

## Explicit formats and versions

`ParserProfileV1` selects schema version 1, parser version `1` or `2`, an exact
unit definition, and one format. Unknown versions fail visibly. Version 2 is a
separately retained parser product using the same currently supported grammar.
`ImportContextV1` supplies the accepted receipt, source, capture instant, coverage
and row/product/facet IDs. Capture and receipt times are separate explicit inputs. No parser or replay code generates IDs or samples time.
Only the synthetic source contract is enabled; live file formats require inventory
and their own reviewed profile.

- `SyntheticCsv`: UTF-8 with the exact header
  `original_id,amount,unit,effective,description`. RFC-style quoted fields are
  supported, but malformed/unclosed quotes, inconsistent column counts and text
  after a closing quote are rejected. Every cell is an explicit known string.
- `SyntheticManual`: a bounded JSON array of objects with exactly `original_id`,
  `amount`, `unit`, `effective`, and `description`. All except `unit` use the
  existing explicit SourceValue states. Absent, Missing, Unknown, Null, Deleted,
  and Known are preserved. Unknown fields and excessive nesting are rejected.
- `InertText`: UTF-8 text retained as an attachment without observations or parsed
  money. It exposes bytes, not an executable viewer, URL loader or local path.
  Markup, remote resource URLs, archives, executable headers and binary payloads
  are rejected. PDFs/OCR and live statement grammars remain unsupported.

Numbers use an optional minus sign, decimal digits and optional dot fraction.
Thousands separators, exponents, mixed decimal locales and precision beyond the
selected unit are rejected, never rounded. Dates are strict Gregorian YYYY-MM-DD;
unknown date states stay unknown. No date becomes a fabricated midnight.

## Limits and completion

Files are at most 100 MiB and exposed in chunks no larger than 1 MiB. Manual JSON,
individual normalized rows and serialized evidence records are at most 1 MiB.
The storage contract still caps observations at 10,000 and total commit items at
20,000. This profile rejects more than 9,998 rows, reserving room for a receipt,
frozen no-effect manifest and command decision. Oversize input returns an error
without a success receipt, partial commit or advanced cursor. It does not silently
truncate. Larger sources require a future explicitly linked incomplete-batch
contract, not an increased limit.

A complete receipt means the selected file was fully processed. It does not make
unknown account/time coverage complete. Explicit supplied coverage is retained.
The synthetic profile has no provider cursor; its cursor remains Absent.

## Immutable evidence and adoption

Every receipt names exactly one capture metadata content blob using media type
`application/vnd.currents.capture-metadata.v1+json`. Its schema version is 1;
required fields are receipt_id, source_identity, original_capture_time,
file_digest, sanitizer_version, profile_id, parser_version and unit. Unknown
fields or versions, missing metadata, mismatched identity/profile/time, invalid
lengths and digests fail before publication. The metadata is encrypted and retained
through the existing content store, for both structured sources and zero-product
attachments. Capture time is supplied provenance, never file modification time or
reparse time. Receipt time remains the separate ImportReceipt.received_time.

The exact file and lossless normalized row representations are retained by digest.
Row order and original identifiers are preserved, including file/row identities
when no stable provider ID exists. A later file does not establish provider
revision order. `prior_version` and `tombstone_of` are not inferred from receipt
time. Similar observations from different files are returned as review candidates,
never silently merged or recognized.

`AddressedParseProductV1` adds an explicit accepted RecordId and schema_version 1
around the unchanged `ParseProductV1`. Its distinct authoritative record kind is
`AddressedParseProduct`. Its ID cannot alias its source or facet identity. It is an
explicit reference role in decisions and interpretation evidence. Storage validates
same/prior-prefix closure, including source observation and facet relationships.
Old unaddressed ParseProduct records remain readable and are not given fabricated
addresses. Reparse appends a new addressed product, retaining the original source,
facet lineage and all previous adoption references. Unsupported wrapper versions
fail serialization/deserialization and publication validation.

## Publication and privacy

`FileParser::parse` returns a capture batch. `ValidatedEvidenceBatchV1::new`
checks bounds, content digests, source/receipt bindings and product references.
The host stages its exposed contents through the existing encrypted store, then
appends the `EvidenceEngine::publish` plan atomically through the existing command
lifecycle. The plan has `AccountingIntent::NoEffect`; acquisition and domain have
no storage or native accounting writer dependencies.

Unexpected secret markers fail before any capture batch is returned. No raw error
or input body appears in errors. Rejected input is quarantined from authoritative
history by rejection; no plaintext quarantine file is created. This conservative
synthetic allowlist is not a claim that a general secret scanner can prove arbitrary
live files safe. Production transport envelopes, credentials, PDF viewing and
provider precedence remain disabled and separately owned.

## Verification

Run `cargo test --locked -p currents-acquisition --test cur_012` and
`cargo xtask ticket CUR-012`. The suite uses only synthetic inputs and the existing
encrypted store, and awaits shutdown explicitly. Acceptance requires actual
current/MSRV platform runs, the affected predecessor regressions and repository
quality/privacy/docs gates; this document is not a passed-test claim.
