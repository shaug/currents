//! C11 security and position records. CUR-021 owns their validation.
use crate::*;
// One identifier a source assigns to a security, scoped by that source.
dto!(SecuritySourceIdV1 {
    source: String,
    identifier: String
});
// An owner's decision that another source's identifier names this security.
dto!(SecurityMappingV1 {
    identifier: SecuritySourceIdV1,
    reason: String
});
// One immutable revision of a security. The first revision's `id` is the
// security's `root`; each later revision names the head it replaces. The
// display symbol is never identity.
dto!(SecurityV1 { id:RecordId, schema_version:u32, root:RecordId, prior_revision:Option<RecordId>, unit_id:UnitId, source_identifiers:Vec<SecuritySourceIdV1>, display_symbol:Option<String>, local_mappings:Vec<SecurityMappingV1> });
// A new security, its own security unit and the unit catalog record that
// declares that unit.
dto!(SecurityDeclarationV1 {
    security: SecurityV1,
    unit: UnitDefinition,
    unit_catalog: RecordId
});
// Links one security to the external account that holds its quantity and
// the internal account that holds its carrying amount.
dto!(PositionV1 {
    id: RecordId,
    schema_version: u32,
    security: RecordId,
    quantity_account: AccountId,
    carrying_account: AccountId
});
