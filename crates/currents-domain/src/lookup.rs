//! Record lookups that one plan repeats against a single prior prefix.
use crate::catalog::{CatalogEngine, unit};
use crate::correction::{ReplacedAdoptions, replaced_adoptions};
use currents_types::*;
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;

type Memo<K, V> = RefCell<BTreeMap<K, Result<V, EngineError>>>;

/// Each lookup here would otherwise scan every record of `state`. A plan that
/// checks many drafts against the same prefix shares one `Lookup`, so each
/// table is built once instead of once per draft. Tables are built on first
/// use and keep the record map's order, so every answer, error included, is
/// the one the scan would give.
pub(crate) struct Lookup<'a> {
    pub(crate) state: &'a DomainState,
    components: OnceCell<BTreeMap<AccountId, Vec<&'a AccountComponentV1>>>,
    parsed: OnceCell<BTreeMap<RecordId, Vec<&'a AuthoritativeRecordV1>>>,
    recognitions: OnceCell<Vec<&'a RecognitionV1>>,
    replaced: OnceCell<ReplacedAdoptions>,
    catalogs: Memo<RecordId, AccountCatalogRevisionV1>,
    units: Memo<UnitId, UnitDefinition>,
    roots: Memo<RecordId, (RecordId, ObservationKind)>,
}

fn memo<K: Ord + Copy, V: Clone>(
    table: &Memo<K, V>,
    key: K,
    compute: impl FnOnce() -> Result<V, EngineError>,
) -> Result<V, EngineError> {
    if let Some(found) = table.borrow().get(&key) {
        return found.clone();
    }
    let found = compute();
    table.borrow_mut().insert(key, found.clone());
    found
}

impl<'a> Lookup<'a> {
    pub(crate) fn new(state: &'a DomainState) -> Self {
        Self {
            state,
            components: OnceCell::new(),
            parsed: OnceCell::new(),
            recognitions: OnceCell::new(),
            replaced: OnceCell::new(),
            catalogs: RefCell::default(),
            units: RefCell::default(),
            roots: RefCell::default(),
        }
    }
    /// The account's components, in record order.
    pub(crate) fn components(&self, account: AccountId) -> &[&'a AccountComponentV1] {
        let state = self.state;
        self.components
            .get_or_init(|| {
                let mut all = BTreeMap::<_, Vec<_>>::new();
                for record in state.records.values() {
                    if let AuthoritativeRecordV1::AccountComponent(c) = record {
                        all.entry(c.account_id).or_default().push(c);
                    }
                }
                all
            })
            .get(&account)
            .map_or(&[], Vec::as_slice)
    }
    /// The parse products, addressed or not, of one observation version, in
    /// record order.
    pub(crate) fn parsed(&self, version: RecordId) -> &[&'a AuthoritativeRecordV1] {
        let state = self.state;
        self.parsed
            .get_or_init(|| {
                let mut all = BTreeMap::<_, Vec<_>>::new();
                for record in state.records.values() {
                    let version = match record {
                        AuthoritativeRecordV1::AddressedParseProduct(p) => {
                            p.product.source_version_id
                        }
                        AuthoritativeRecordV1::ParseProduct(p) => p.source_version_id,
                        _ => continue,
                    };
                    all.entry(version).or_default().push(record);
                }
                all
            })
            .get(&version)
            .map_or(&[], Vec::as_slice)
    }
    /// Every recognition record, in record order.
    pub(crate) fn recognitions(&self) -> &[&'a RecognitionV1] {
        let state = self.state;
        self.recognitions.get_or_init(|| {
            state
                .records
                .values()
                .filter_map(|r| match r {
                    AuthoritativeRecordV1::Recognition(r) => Some(r),
                    _ => None,
                })
                .collect()
        })
    }
    pub(crate) fn replaced(&self) -> &ReplacedAdoptions {
        self.replaced.get_or_init(|| replaced_adoptions(self.state))
    }
    pub(crate) fn current(
        &self,
        component: RecordId,
    ) -> Result<AccountCatalogRevisionV1, EngineError> {
        memo(&self.catalogs, component, || {
            CatalogEngine::current(self.state, component)
        })
    }
    pub(crate) fn unit(&self, id: UnitId) -> Result<UnitDefinition, EngineError> {
        memo(&self.units, id, || unit(self.state, id))
    }
    pub(crate) fn root(&self, id: RecordId) -> Result<(RecordId, ObservationKind), EngineError> {
        memo(&self.roots, id, || {
            crate::recognition::source_root(self.state, id)
        })
    }
}
