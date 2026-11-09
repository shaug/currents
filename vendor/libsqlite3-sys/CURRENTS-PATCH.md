# Private SQLCipher source adaptation

Base: official crates.io libsqlite3-sys 0.37.0, archive checksum recorded in
../../tools/versions.lock.json. CURRENTS-PROVENANCE.json records every original
file and its current SHA-256 or deliberate omission. Except Cargo.toml/build.rs,
all retained upstream files are byte-identical. The upstream MIT license remains.

Changes:
- Set publish=false; add exact build dependency sqlcipher-src420.0.0.
- Compile its unmodified official SQLCipher4.20.0/SQLite3.54.1 amalgamation and
  expose its matching public header directory; retain upstream Rust bindings/API.
- Keep the existing CommonCrypto build on Apple and vendored OpenSSL on Linux.
- Reject stock/system SQLite, Gecko/loadable-extension/buildtime-bindgen modes
  and the pkg-config override. This private fork serves only Currents' declared
  bundled encrypted-source targets; it is not a general replacement crate.
- Omit old stock/encrypted C amalgamations and C headers, avoiding duplicate or
  vulnerable fallback sources. Preserved generated Rust bindings expose the
  compatible subset used by SQLx0.9.0; new SQLite functions are not introduced.
- Remove Cargo vendor checksum metadata because this is an explicit path patch,
  not a claim to be an unmodified cargo-vendor registry copy.

No cryptographic, SQLite or SQLCipher function was written or modified here.
The fixed official source removes RUSTSEC-2026-0459's missing WAL tail clearing.
`cargo xtask qualify native-source` verifies exact selected native source bytes
and single sys composition; the in-memory cipher test verifies linked runtime
versions. Neither is a claim of full key initialization/encryption/restore
qualification, which belongs to later tickets.

The unchanged package version still falls within the RustSec package range, but
the observed cargo-deny advisory scan does not flag this local path package. Its
green result alone is not evidence that the fixed native bytes are safe. No
advisory ignore rule is installed. Preserve raw scan output. Applicability
must be reviewed against the pinned fixed source, not waived based on a new
version string. Requalification must include both Mac and Linux, candidate MSRV,
source hashes, API/link smoke tests, and preserved runtime provider choices.
