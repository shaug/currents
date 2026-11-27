//! One commit's definition lookups, deduplicated and answered a chunk at a time.
//!
//! Every reference and command effect still needs exactly one of its permitted
//! kinds to resolve. Only the number of SQL statements changes: each distinct
//! `(kind, id, version)` probe is answered once, and a chunk of probes shares
//! one statement instead of one statement per kind.
use crate::references::Reference;
use std::collections::BTreeMap;

/// Probes per statement. The commit limit bounds the total, so this only keeps
/// each bound JSON value small.
pub(super) const PROBES_PER_QUERY: usize = 4_096;

/// The lowest version of each probe's definition visible at the commit's own
/// prefix, in probe order. `?1` is a JSON array of `[kind, id, version|null]`.
pub(crate) const DEFINITION_VERSIONS: &str = r#"
SELECT (
    SELECT d.version FROM currents_definitions AS d
    WHERE d.kind=json_extract(p.value,'$[0]') AND d.id=json_extract(p.value,'$[1]')
      AND d.seq<=?2
      AND (json_extract(p.value,'$[2]') IS NULL OR d.version=json_extract(p.value,'$[2]'))
    ORDER BY d.version LIMIT 1
) FROM json_each(?1) AS p ORDER BY p.key
"#;

type Probe = (String, String, Option<i64>);

/// The distinct probes one commit needs, each with a fixed answer slot.
#[derive(Default)]
pub(super) struct Probes {
    probes: Vec<Probe>,
    slots: BTreeMap<Probe, usize>,
}

/// A reference with the answer slot of each of its kinds, in kind order.
pub(super) struct Probed {
    pub reference: Reference,
    slots: Vec<usize>,
}

impl Probes {
    pub(super) fn add(&mut self, reference: Reference) -> Probed {
        let slots = reference
            .kinds
            .iter()
            .map(|kind| {
                let probe = (kind.clone(), reference.id.clone(), reference.version);
                let next = self.probes.len();
                let slot = *self.slots.entry(probe.clone()).or_insert(next);
                if slot == next {
                    self.probes.push(probe);
                }
                slot
            })
            .collect();
        Probed { reference, slots }
    }
    pub(super) fn chunks(&self) -> impl Iterator<Item = &[Probe]> {
        self.probes.chunks(PROBES_PER_QUERY)
    }
    pub(super) fn len(&self) -> usize {
        self.probes.len()
    }
}

impl Probed {
    /// The one permitted kind that resolves, with its lowest matching version.
    /// `None` when no kind or more than one kind resolves.
    pub(super) fn single(&self, answers: &[Option<i64>]) -> Option<(&str, i64)> {
        let mut found = None;
        for (kind, slot) in self.reference.kinds.iter().zip(&self.slots) {
            if let Some(version) = answers.get(*slot).copied().flatten() {
                if found.is_some() {
                    return None;
                }
                found = Some((kind.as_str(), version));
            }
        }
        found
    }
}
