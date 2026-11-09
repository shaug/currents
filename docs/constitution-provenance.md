# Constitution provenance

The base of [constitution.md](constitution.md) is the complete source file **Currents-Constitution(3).md**, the version supplied for this project. Its internal status is “Foundational draft” and its internal date is **2026-11-08**.

- Source byte length: **17,894**.
- Source SHA-256: `2155ec966bf4b45383bfbc7e43004880f0bcb9c6b9efb9a1a44b4ae48501ccac`.
- Source retrieved: **2026-11-09 UTC**.
- Preservation: the first 17,894 bytes of `constitution.md` are byte-for-byte identical to that source, including the final newline.
- Addition: section 8, **Clarification: read-only institutional access**, records the subsequent explicit direction that connections cannot execute payments, transfers, trades, or other institutional financial mutations, while validated changes to the local ledger are allowed. No source section was shortened or replaced.

The design may make provisional implementation recommendations. They do not amend the constitution or establish that those recommendations have been approved.

To verify preservation against the supplied source file:

```sh
python3 - /path/to/Currents-Constitution\(3\).md docs/constitution.md <<'PYCODE'
import hashlib, pathlib, sys
source, current = (pathlib.Path(p).read_bytes() for p in sys.argv[1:])
assert len(source) == 17894
assert hashlib.sha256(source).hexdigest() == "2155ec966bf4b45383bfbc7e43004880f0bcb9c6b9efb9a1a44b4ae48501ccac"
assert current[:len(source)] == source
print("Complete source preserved; later clarification follows it.")
PYCODE
```
