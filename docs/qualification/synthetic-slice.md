# CUR014 synthetic developer-preview gate

This is a **synthetic-only developer preview**, not a usable household MVP.
No private financial input, live provider, signed host or production profile is enabled.
Full-picture queries, native minimum-OS qualification, recovery/key custody,
providers and Apple capture remain their own open gates.

`cargo xtask ticket CUR-014` runs the named service assertions and emits exact
source/tool/platform/fixture-bound evidence. This document is not a candidate pass
claim; consult generated qualification reports and PR evidence.

The S1 CSV fixtures deliberately match the earlier JSON examples but use the real
CUR012 bounded synthetic parser. Trusted owner interpretation explicitly adopts
80 as pending and 96 as posted and links the prior pending adoption. Import alone
never moves money. Enrichment is retained without adoption. Original content,
metadata and parse bytes survive encrypted reopen and independent canonical replay.

`ImportEvidence` is an opaque trusted in-process descriptor, not a wire DTO. It
validates the entire capture using C07; its identity commits to unit, receipt,
observations, addressed parse products and content digest/length descriptors. Raw
file SHA alone is not batch identity. Raw buffers are excluded from canonical
preview serialization but counted against existing retained-admission bounds.
Only matching PublishImport may carry evidence. Service checks its exact
receipt/checkpoint/batch identity and compares the evidence-engine plan at the
verified prefix. The existing closure digest commits to descriptor and plan.
Writer verifies supplied content against exact required references, stages in
the same encrypted store and publishes records/content/result atomically. Scoped
temporary-token cleanup never deletes immutable authority or other intakes.
Unclean process termination may leave encrypted staging from the established C04
path; this does not constitute an import or change its audited command outcome.

The trusted test QueryHistory adapter projects accepted recognized legs at the
verified current prefix, after explicit account scope authorization. It is not
PositionEngine, balance calculation, classification, source selection or household
aggregation. It reports unknown opening/incomplete history rather than a fabricated
balance. Unsupported filter/version shapes reject. Shared lifecycle host scope is
not weakened.

Fault proof uses actual subprocess termination before COMMIT and after a confirmed
commit result; a separate dropped caller/acknowledgment exercises uncertain outcome
and identical retry. These tests demonstrate process-crash recovery, not physical
power-loss guarantees. Only the selected recognition command must have exactly one
result; setup/import commands retain independent results.
