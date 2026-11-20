//! Explicit verification registry. An unregistered check can never become green.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
const MEMBERS: &[&str] = &[
    "types",
    "protocol",
    "cipher",
    "transfer",
    "domain",
    "accounting",
    "storage",
    "service",
    "planning",
    "acquisition",
    "ffi",
    "cli",
    "mcp",
    "verify",
];
const ASSERTIONS: &[&str] = &["cur_001_1", "cur_001_2", "cur_001_3", "cur_001_4"];

pub(crate) fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}
fn output(program: &Path, args: &[&str], cwd: &Path, rustc: Option<&Path>) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if let Some(rustc) = rustc {
        command
            .env("RUSTC", rustc)
            .env("RUSTDOC", rustc.with_file_name("rustdoc"));
    }
    command
        .output()
        .map_err(|e| format!("BLOCKED: {}: {e}", program.display()))
}
pub(crate) fn checked(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    rustc: Option<&Path>,
) -> Result<String> {
    let out = output(program, args, cwd, rustc)?;
    crate::report::record(program, args, cwd, rustc, &out);
    let evidence_dir = root().join("target/qualification");
    fs::create_dir_all(&evidence_dir).map_err(|e| e.to_string())?;
    let record = json!({"program":program,"args":args,"cwd":cwd,"rustc":rustc,"exit":out.status.code(),"stdout":String::from_utf8_lossy(&out.stdout),"stderr":String::from_utf8_lossy(&out.stderr)});
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(evidence_dir.join("commands.jsonl"))
        .map_err(|e| e.to_string())?;
    writeln!(log, "{record}").map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "{} {} failed ({}):\n{}\n{}",
            program.display(),
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
pub(crate) fn cargo(args: &[&str]) -> Result<String> {
    checked(Path::new("cargo"), args, &root(), None)
}
fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub(crate) fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn metadata(manifest: &Path) -> Result<Value> {
    let text = cargo(&[
        "metadata",
        "--offline",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        manifest.to_str().ok_or("invalid path")?,
    ])?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
fn packages(meta: &Value) -> Result<&Vec<Value>> {
    meta["packages"]
        .as_array()
        .ok_or_else(|| "missing Cargo packages".into())
}
fn workspace() -> Result<Value> {
    let meta = metadata(&root().join("Cargo.toml"))?;
    let found: BTreeSet<_> = packages(&meta)?
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    let mut expected: BTreeSet<String> = MEMBERS.iter().map(|n| format!("currents-{n}")).collect();
    expected.insert("xtask".into());
    if found != expected.iter().map(String::as_str).collect() {
        return Err("workspace membership differs from C01".into());
    }
    for p in packages(&meta)? {
        if p["publish"] != json!([]) {
            return Err(format!("{} must set publish=false", p["name"]));
        }
        if p["edition"] != "2024" {
            return Err(format!("{} must use edition 2024", p["name"]));
        }
    }
    Ok(json!({"members":found,"publish":false,"edition":"2024"}))
}
fn allowed_internal(name: &str) -> &'static [&'static str] {
    match name {
        "currents-types" => &[],
        "currents-protocol" => &["currents-types"],
        "currents-domain"
        | "currents-planning"
        | "currents-accounting"
        | "currents-acquisition" => &["currents-types"],
        "currents-cipher" => &[],
        "currents-transfer" => &["currents-types", "currents-protocol", "currents-cipher"],
        "currents-storage" => &["currents-types", "currents-accounting", "currents-cipher"],
        "currents-service" => &[
            "currents-types",
            "currents-protocol",
            "currents-domain",
            "currents-planning",
            "currents-storage",
        ],
        "currents-ffi" | "currents-cli" | "currents-mcp" => {
            &["currents-service", "currents-protocol"]
        }
        "currents-verify" => &[
            "currents-cipher",
            "currents-service",
            "currents-storage",
            "currents-protocol",
            "currents-types",
        ],
        _ => &[],
    }
}
// C01 gives cipher exclusive key/profile/sys ownership, not exclusive SQLx use.
// Trusted storage/accounting need SQLx for the C04 native-pool/schema bridge.
fn sqlite_dependency_allowed(name: &str, dependency: &str) -> bool {
    if dependency == "libsqlite3-sys" {
        return name == "currents-cipher";
    }
    if dependency.starts_with("sqlx") {
        return name == "currents-cipher"
            || (dependency == "sqlx"
                && matches!(name, "currents-storage" | "currents-accounting"));
    }
    true
}

fn direction(manifest: &Path) -> Result<Value> {
    let meta = metadata(manifest)?;
    for p in packages(&meta)? {
        let name = p["name"].as_str().ok_or("missing package name")?;
        if !name.starts_with("currents-") {
            continue;
        }
        for dep in p["dependencies"].as_array().ok_or("missing dependencies")? {
            // Dev-only dependencies do not ship; build dependencies still matter.
            if dep["kind"] == "dev" {
                continue;
            }
            let d = dep["name"].as_str().ok_or("missing dependency name")?;
            let pure = matches!(
                name,
                "currents-types" | "currents-domain" | "currents-planning"
            );
            let safe = matches!(
                d,
                "serde"
                    | "serde_json"
                    | "time"
                    | "thiserror"
                    | "uuid"
                    | "num-bigint"
                    | "num-integer"
                    | "num-traits"
                    | "sha2"
                    | "jiff"
                    | "jiff-tzdb"
            );
            let forbidden = (d.starts_with("currents-") && !allowed_internal(name).contains(&d))
                || (pure && !d.starts_with("currents-") && !safe)
                || (d == "doubleentry" && name != "currents-accounting")
                || !sqlite_dependency_allowed(name, d)
                || (name == "currents-cipher"
                    && !matches!(
                        d,
                        "sqlx"
                            | "sqlx-core"
                            | "sqlx-sqlite"
                            | "libsqlite3-sys"
                            | "zeroize"
                            | "secrecy"
                    ))
                || (name == "currents-verify"
                    && (d == "reqwest"
                        || d.starts_with("objc")
                        || d.starts_with("core-foundation")));
            if forbidden {
                return Err(format!("forbidden dependency: {name} -> {d}"));
            }
        }
    }
    Ok(json!({"manifest":manifest,"dependency_direction":"accepted"}))
}
fn inventory(path: &Path, artifact_root: Option<&Path>) -> Result<Value> {
    let data = read_json(path)?;
    if data["schema_version"] != 1 {
        return Err("unsupported pin inventory schema".into());
    }
    let entries = data["artifacts"].as_array().ok_or("missing artifacts")?;
    if entries.is_empty() {
        return Err("BLOCKED: zero pinned local artifacts; supply the qualified tool/source cache inventory".into());
    }
    let mut names = BTreeSet::new();
    let mut evidence = vec![];
    for entry in entries {
        let field = |key| {
            entry[key]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("missing artifact {key}"))
        };
        let name = field("name")?;
        if !names.insert(name) {
            return Err(format!("duplicate artifact: {name}"));
        }
        let version = field("version")?;
        if version == "latest" || version.contains(['*', '^', '~']) {
            return Err(format!("unqualified version: {name}"));
        }
        let source = field("source")?;
        if !source.starts_with("https://") {
            return Err(format!("source must be HTTPS: {name}"));
        }
        let digest = field("sha256")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(format!("invalid checksum: {name}"));
        }
        let artifact = artifact_root
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")))
            .join(field("path")?);
        let bytes = fs::read(&artifact).map_err(|e| format!("missing artifact {name}: {e}"))?;
        if sha(&bytes) != digest {
            return Err(format!("checksum mismatch: {name}"));
        }
        evidence.push(json!({"name":name,"version":version,"source":source,"sha256":digest}));
    }
    Ok(json!({"artifacts":evidence}))
}
fn msrv_tools() -> Result<(PathBuf, PathBuf, String)> {
    let bin = env::var_os("CURRENTS_MSRV_BIN")
        .ok_or("BLOCKED: set CURRENTS_MSRV_BIN to the verified Rust 1.94.0 bin directory")?;
    let rustc = PathBuf::from(&bin).join("rustc");
    let cargo = PathBuf::from(bin).join("cargo");
    let version = checked(&rustc, &["--version"], &root(), None)?;
    if !version.starts_with("rustc 1.94.0 ") {
        return Err(format!("wrong MSRV compiler: {version}"));
    }
    checked(&cargo, &["--version"], &root(), None)?;
    Ok((cargo, rustc, version))
}
pub(crate) fn check_native_environment() -> Result<()> {
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy();
        if key.ends_with("OPENSSL_DIR")
            || key.ends_with("OPENSSL_NO_VENDOR")
            || key.ends_with("OPENSSL_LIB_DIR")
            || key.ends_with("OPENSSL_INCLUDE_DIR")
            || key == "LIBSQLITE3_SYS_USE_PKG_CONFIG"
            || key == "SQLITE3_LIB_DIR"
            || key == "SQLITE3_INCLUDE_DIR"
        {
            return Err(format!(
                "BLOCKED: clear native provider override {key} before qualification"
            ));
        }
    }
    Ok(())
}
enum CorePhase {
    Build,
    Test,
    NativeDriver,
}

impl CorePhase {
    fn run<T>(self, enabled: bool, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        if !enabled {
            return operation();
        }
        let label = match self {
            Self::Build => "workspace-build",
            Self::Test => "workspace-test",
            Self::NativeDriver => "native-driver",
        };
        // Best-effort diagnostics must not change command results or captured evidence.
        let _ = writeln!(std::io::stderr().lock(), "core phase={label} start");
        let start = Instant::now();
        let result = operation();
        let status = if result.is_ok() { "ok" } else { "error" };
        let _ = writeln!(
            std::io::stderr().lock(),
            "core phase={label} finish status={status} duration_ms={}",
            start.elapsed().as_millis()
        );
        result
    }
}

fn ci_core_phases(has_manifest: bool) -> bool {
    !has_manifest
        && env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
        && env::var("CURRENTS_CI_CORE_PHASES").as_deref() == Ok("1")
}

fn core(msrv: bool, manifest: Option<&Path>) -> Result<Value> {
    let phases = ci_core_phases(manifest.is_some());
    check_native_environment()?;
    let (program, rustc, version) = if msrv {
        let (c, r, v) = msrv_tools()?;
        (c, Some(r), v)
    } else {
        (
            PathBuf::from("cargo"),
            None,
            checked(Path::new("rustc"), &["--version"], &root(), None)?,
        )
    };
    if !msrv && !version.starts_with("rustc 1.99.0 ") {
        return Err(format!("wrong release compiler: {version}"));
    }
    let manifest = manifest
        .map(|path| path.to_str().ok_or("invalid manifest path"))
        .transpose()?;
    let mut build = vec!["build", "--locked", "--workspace", "--all-targets"];
    let mut test = vec!["test", "--locked", "--workspace"];
    for args in [&mut build, &mut test] {
        if let Some(path) = manifest {
            args.extend(["--manifest-path", path]);
        }
    }
    test.extend(["--", "--test-threads=1"]);
    let mut evidence = vec![];
    for (phase, args) in [(CorePhase::Build, build), (CorePhase::Test, test)] {
        let text = phase.run(phases, || {
            checked(&program, &args, &root(), rustc.as_deref())
        })?;
        evidence.push(json!({"command":args,"stdout_sha256":sha(text.as_bytes())}));
    }
    let driver = if manifest.is_none() {
        Some(
            CorePhase::NativeDriver
                .run(phases, || crate::cipher::driver(&program, rustc.as_deref()))?,
        )
    } else {
        None
    };
    Ok(
        json!({"compiler":version.trim(),"commands":evidence,"driver":driver,"host":env::consts::OS,"arch":env::consts::ARCH}),
    )
}
fn ticket_cur_016(inventory_only: bool) -> Result<Value> {
    let mut evidence = Vec::new();
    let suites: [(&str, &str, &[&str]); 3] = [
        (
            "currents-domain",
            "cur_016",
            &[
                "cur_016_original_segment_is_backed_by_frozen_c05_authority",
                "cur_016_1_cross_account_backfill",
                "cur_016_standalone_segment_family_replays_without_overwriting_old_opening",
                "cur_016_1_cross_account_backfill_replays_the_entire_transfer_family",
                "cur_016_4_actual_planner_emits_a_closed_1200_operation_replacement",
                "cur_016_4_actual_planner_rejects_20001_native_operations_without_a_plan",
            ],
        ),
        (
            "currents-service",
            "cur_016",
            &[
                "cur_016_2_replayed_retirement_blocks_correction",
                "cur_016_durable_correction_rebuilds_immutable_lineage",
                "cur_016_standalone_segment_commit_reopens_and_rebuilds_native_authority",
                "cur_016_1_encrypted_cross_account_segment_commits_full_transfer_family",
                "cur_016_3_2024_date_floor_preserves_old_basis_and_repairs_current_as_of",
                "cur_016_4_real_1200_operation_replacement_is_atomic_at_c04_midpoint",
                "cur_016_forecast_anchor_rejects_other_live_counterposting_through_cutoff",
            ],
        ),
        (
            "currents-service",
            "cur_016_correction_chain",
            &[
                "cur_016_2_live_tip_can_be_corrected_again_without_a_supersession_cycle",
                "cur_016_2_correction_preserves_note_and_invalidates_only_matching_predicate",
                "cur_016_2_refund_is_a_new_economic_event_not_a_native_reversal",
            ],
        ),
    ];
    for (package, target, required) in suites {
        let list = cargo(&[
            "test", "--locked", "-p", package, "--test", target, "--", "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != required.iter().copied().collect() {
            return Err(format!(
                "CUR016 assertion inventory mismatch in {package}/{target}: {list}"
            ));
        }
        let mut suite = json!({"package":package,"target":target,"assertions":required});
        if !inventory_only {
            let output = cargo(&["test", "--locked", "-p", package, "--test", target])?;
            if !output.contains(&format!("{} passed; 0 failed; 0 ignored;", required.len()))
                || required
                    .iter()
                    .any(|test| !output.contains(&format!("test {test} ... ok")))
            {
                return Err(format!(
                    "CUR016 incomplete qualification in {package}/{target}: {output}"
                ));
            }
            suite["output_sha256"] = json!(sha(output.as_bytes()));
        }
        evidence.push(suite);
    }
    let mut fixture_hashes = serde_json::Map::new();
    for path in [
        "fixtures/journeys/s4/date-floor.json",
        "fixtures/journeys/s5/refund.csv",
    ] {
        let bytes = std::fs::read(root().join(path))
            .map_err(|error| format!("CUR016 fixture {path}: {error}"))?;
        fixture_hashes.insert(path.into(), json!(sha(&bytes)));
    }
    Ok(json!({
        "ticket":"CUR-016",
        "suites":evidence,
        "fixture_sha256": fixture_hashes,
        "mode": if inventory_only {"inventory only; no tests executed"} else {"synthetic execution"},
        "scope":"Bounded synthetic correction and segment evidence only; not production dispatch, general stable remapping or predicate invalidation, or full S4/S5 release qualification."
    }))
}

fn ticket(name: &str) -> Result<Value> {
    if name == "CUR-044" {
        return crate::financekit_gate::ticket();
    }
    if name == "CUR-004" {
        return crate::native::qualify(&root());
    }
    if name == "CUR-006" {
        return crate::cipher::qualify();
    }
    if name == "CUR-003" {
        return crate::quality::ticket();
    }
    if name == "CUR-047" {
        return crate::readiness::qualify();
    }
    if name == "CUR-007" {
        // Later tickets share the target; retain this ticket's exact original inventory.
        let assertions = ["cur_007_1", "cur_007_2", "cur_007_3", "cur_007_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_007",
            "cur_007_",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR007 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_007",
            "cur_007_",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR007 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"synthetic encrypted multi-operation authority; not service admission or physical power-loss qualification"}),
        );
    }
    if name == "CUR-011" {
        let assertions = ["cur_011_1", "cur_011_2", "cur_011_3", "cur_011_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_011",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR011 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_011",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR011 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"synthetic complete frozen replay through encrypted C04/C05/C06; no production host or generation activation","checkpoint_limit":"A coherent rewrite requires an independent saved checkpoint to detect."}),
        );
    }
    if name == "CUR-015" {
        let mut evidence = Vec::new();
        for (package, target, filter, required) in [
            ("currents-types", "relationship_parse", "", ["cur_015_3_parser_binding_preserves_v1_and_addresses_correction"].as_slice()),
            ("currents-acquisition", "cur_015_source", "", ["relationship_source_keeps_payroll_facets_and_net_only_unknowns", "relationship_source_rejects_ambiguous_identity_and_secret_bodies", "relationship_profile_is_explicit_bounded_and_never_orders_by_arrival", "relationship_capture_retains_exact_bytes_and_separate_times_without_money"].as_slice()),
            ("currents-domain", "cur_015", "", ["explicit_clearing_and_prepayment_are_declared_assets_not_external_openings", "cur_015_4_explicit_owner_expiry_is_nonmonetary_and_absence_is_not_expiry", "quantified_source_capacity_is_not_multiplied_by_many_to_many_edges", "corroborating_evidence_does_not_double_the_economic_leg", "association_revision_prefixes_are_immutable_and_forks_are_rejected", "captured_facets_have_exact_typed_identity_and_explicit_revision_order", "cur_015_1_complete_transfer_preserves_dates_and_separate_fee", "transfer_source_arrival_permutations_have_identical_financial_plan", "cur_015_2_payroll_requires_distinct_facets_and_explicit_withholding_choice", "complete_association_replay_rejects_balanced_wrong_role_atomically", "cur_015_3_source_revision_replaces_all_old_claims_without_mutating_history", "cur_015_4_source_tombstone_retains_history_and_never_reverses_money", "explicit_n_way_duplicates_keep_chosen_survivor_and_reverse_every_redundant_original", "cur_015_4_reconnect_is_an_explicit_stable_account_alias_not_new_money", "cur_015_opening_boundary_initial_and_remap", "cur_015_remap_preserves_existing_recognition_lineage", "cur_015_3_explicit_parser_correction_orders_without_recapture_capacity"].as_slice()),
            ("currents-protocol", "cur_015_contracts", "", ["complete_association_is_additive_and_rejects_incomplete_bindings"].as_slice()),
            ("currents-service", "cur_015", "", ["relationship_import_identity_binds_complete_retained_metadata", "relationship_capture_uses_real_service_staging_and_reopens_exactly", "cur_015_1_real_service_transfer_keeps_dates_and_only_separate_fee_is_spending", "cur_015_1_separate_arrival_permutations_rebuild_same_economic_postings", "cur_015_2_real_service_payroll_retains_five_facets_and_owner_tax_choice", "cur_015_3_real_service_remap_cancellation_reopen_and_tombstone", "cur_015_4_real_service_duplicates_expiry_reparse_and_reconnect", "cur_015_opening_boundary_encrypted_initial_and_remap", "cur_015_remap_lineage_rejection_preserves_encrypted_replay", "cur_015_3_real_parser_remap_preserves_raw_cancellation_retry_and_reopen", "cur_015_3_parser_profile_rejects_forgery_and_preserves_capture_wire"].as_slice()),
            ("currents-accounting", "cur_009", "cur_015", ["cur_015_atomic_multiple_corrections_use_one_prior_prefix", "cur_015_initial_recognition_set_preserves_each_effective_date", "cur_015_remap_requires_exact_native_retirement_set"].as_slice()),
            ("currents-storage", "cur_007", "cur_015", ["cur_015_multiple_corrections_never_publish_a_partial_native_prefix", "cur_015_initial_multidate_batch_is_atomic_and_replayable", "cur_015_revision_family_preserves_prefix_and_rejects_forks", "cur_015_replay_checks_retained_capture_bytes_before_activation", "cur_015_replay_checks_retained_parser_profile_bytes_before_activation"].as_slice()),
        ] {
            let list = cargo(&["test", "--locked", "-p", package, "--test", target, filter, "--", "--list"])?;
            let actual: BTreeSet<_> = list.lines().filter_map(|line| line.strip_suffix(": test")).collect();
            if actual != required.iter().copied().collect() {
                return Err(format!("CUR015 assertion inventory mismatch in {package}/{target}: {list}"));
            }
            let text = cargo(&["test", "--locked", "-p", package, "--test", target, filter])?;
            if !text.contains(&format!("{} passed; 0 failed; 0 ignored;", required.len()))
                || required.iter().any(|test| !text.contains(&format!("test {test} ... ok"))) {
                return Err(format!("CUR015 incomplete qualification in {package}/{target}: {text}"));
            }
            evidence.push(json!({"package":package,"target":target,"assertions":required,"output_sha256":sha(text.as_bytes())}));
        }
        return Ok(
            json!({"ticket":name,"suites":evidence,"scope":"synthetic explicit transfer/payroll/duplicate and immutable source revision associations through encrypted Service/native replay; no production activation or arbitrary segment correction"}),
        );
    }
    if name == "CUR-014" {
        let assertions = [
            "cur_014_1_import_retains_real_parsed_evidence",
            "cur_014_1_slice_end_to_end",
            "cur_014_2_slice_fault",
            "cur_014_2_import_rejection_and_cancel",
            "cur_014_3_synthetic_enforcement",
        ];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-service",
            "--test",
            "cur_014",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR014 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-service",
            "--test",
            "cur_014",
        ])?;
        if !text.contains("5 passed; 0 failed; 0 ignored;")
            || assertions
                .iter()
                .any(|name| !text.contains(&format!("test {name} ... ok")))
        {
            return Err(format!("CUR014 incomplete qualification: {text}"));
        }
        let paths = [
            "crates/currents-service/tests/cur_014.rs",
            "crates/currents-service/src/import.rs",
            "crates/currents-service/src/commands.rs",
            "crates/currents-service/src/service.rs",
            "crates/currents-storage/src/writer.rs",
            "crates/currents-storage/src/staging.rs",
            "fixtures/journeys/s1/authorization.csv",
            "fixtures/journeys/s1/posted.csv",
            "fixtures/journeys/s1/enrichment.csv",
            "docs/qualification/synthetic-slice.md",
        ];
        let sources: Vec<_> = paths
            .iter()
            .map(|path| {
                fs::read(root().join(path)).map(|bytes| json!({"path":path,"sha256":sha(&bytes)}))
            })
            .collect::<std::io::Result<_>>()
            .map_err(|error| error.to_string())?;
        return Ok(
            json!({"ticket":name,"assertions":assertions,"sources":sources,"output_sha256":sha(text.as_bytes()),"scope":"synthetic-only encrypted import/recognition/current history/replay/crash and retry; not household MVP or production activation"}),
        );
    }
    if name == "CUR-013" {
        let domain = [
            "cur_013_1",
            "cur_013_2",
            "cur_013_3",
            "cur_013_4",
            "cur_013_catalog_commands",
        ];
        let durable = [
            "cur_013_1_service_pending_posted_reopen_and_no_double_money",
            "cur_013_2_service_corroboration_is_one_entry",
            "cur_013_3_service_unknown_and_supported_openings_are_distinct",
            "cur_013_4_service_invalid_money_never_changes_domain_or_native_head",
            "cur_013_catalog_commands_service",
            "catalog_authority_and_ownership_negatives_leave_exact_prefix_unchanged",
            "historical_adoption_keeps_exact_policy_versions_after_later_policy_records",
            "coherent_catalog_corruption_cannot_activate_replay_projection",
        ];
        let mut evidence = Vec::new();
        for (package, target, required) in [
            ("currents-domain", "cur_013", domain.as_slice()),
            ("currents-service", "cur_013", durable.as_slice()),
            (
                "currents-protocol",
                "cur_013_contracts",
                ["cur_013_complete_command_goldens"].as_slice(),
            ),
        ] {
            let text = cargo(&["test", "--locked", "-p", package, "--test", target])?;
            if !text.contains("test result: ok.")
                || !text.contains("; 0 failed; 0 ignored;")
                || required
                    .iter()
                    .any(|name| !text.contains(&format!("test {name} ... ok")))
            {
                return Err(format!(
                    "CUR013 incomplete qualification in {package}/{target}: {text}"
                ));
            }
            evidence.push(json!({"package":package,"target":target,"assertions":required,"output_sha256":sha(text.as_bytes())}));
        }
        let paths = [
            "crates/currents-domain/src/catalog.rs",
            "crates/currents-domain/src/recognition.rs",
            "crates/currents-domain/src/segments.rs",
            "crates/currents-domain/src/policy_history.rs",
            "crates/currents-domain/src/recognition_replay.rs",
            "crates/currents-domain/src/replay.rs",
            "crates/currents-service/src/replay.rs",
            "fixtures/journeys/s1/authorization.json",
            "fixtures/journeys/s1/posted.json",
            "fixtures/journeys/s1/enrichment.json",
            "crates/currents-domain/tests/cur_013.rs",
            "crates/currents-domain/tests/support/recognition.rs",
            "crates/currents-service/tests/cur_013.rs",
            "crates/currents-types/src/recognition_contracts.rs",
            "crates/currents-storage/src/references.rs",
            "fixtures/contracts/v1/recognition/catalog.json",
            "fixtures/contracts/v1/recognition/pending.json",
            "fixtures/contracts/v1/recognition/digests.json",
        ];
        let sources: Vec<_> = paths
            .iter()
            .map(|path| {
                fs::read(root().join(path)).map(|bytes| json!({"path":path,"sha256":sha(&bytes)}))
            })
            .collect::<std::io::Result<_>>()
            .map_err(|error| error.to_string())?;
        return Ok(
            json!({"ticket":name,"suites":evidence,"sources":sources,"scope":"explicit synthetic evidence adoption, immutable catalog, encrypted Service/native recognition and initial supported opening; no production host, inferred associations or replacement correction"}),
        );
    }
    if name == "CUR-012" {
        let assertions = ["cur_012_1", "cur_012_2", "cur_012_3", "cur_012_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-acquisition",
            "--test",
            "cur_012",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR012 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-acquisition",
            "--test",
            "cur_012",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR012 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"bounded synthetic C07 evidence and inert text; no live formats, credential access, recognition or provider precedence","limits":{"file_bytes":104857600,"chunk_message_bytes":1048576,"normalized_observations":10000,"profile_rows":9998,"total_commit_items":20000}}),
        );
    }
    if name == "CUR-025" {
        let assertions = ["cur_025_1", "cur_025_2", "cur_025_3", "cur_025_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-planning",
            "--test",
            "cur_025",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR025 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-planning",
            "--test",
            "cur_025",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR025 incomplete qualification".into());
        }
        let durable = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-service",
            "--test",
            "cur_025",
        ])?;
        if !durable.contains("1 passed; 0 failed; 0 ignored") {
            return Err("CUR025 incomplete durable qualification".into());
        }
        let source_paths = [
            "crates/currents-planning/src/calendar.rs",
            "crates/currents-planning/src/commitments.rs",
            "crates/currents-planning/tests/cur_025.rs",
            "crates/currents-service/tests/cur_025.rs",
            "fixtures/planning/calendar/edges.json",
            "fixtures/contracts/v1/records.json",
        ];
        let source_hashes: Vec<_> = source_paths
            .iter()
            .map(|path| {
                fs::read(root().join(path)).map(|bytes| json!({"path":path,"sha256":sha(&bytes)}))
            })
            .collect::<std::io::Result<_>>()
            .map_err(|error| error.to_string())?;
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"durable_output_sha256":sha(durable.as_bytes()),"source_hashes":source_hashes,"scope":"portable planning and durable synthetic C03 history, no production activation or financial recognition"}),
        );
    }
    if name == "CUR-010" {
        let assertions = ["cur_010_1", "cur_010_2", "cur_010_3", "cur_010_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-service",
            "--test",
            "cur_010",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR010 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-service",
            "--test",
            "cur_010",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR010 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"synthetic serialized C03 lifecycle through encrypted C04/C05; not signed-host production or sibling domain semantics","checkpoint_limit":"A coherent rewrite requires an independent saved checkpoint to detect."}),
        );
    }
    if name == "CUR-009" {
        // Later tickets share the target; retain this ticket's exact original inventory.
        let assertions = ["cur_009_1", "cur_009_2", "cur_009_3", "cur_009_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-accounting",
            "--test",
            "cur_009",
            "cur_009_",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR009 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-accounting",
            "--test",
            "cur_009",
            "cur_009_",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR009 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"synthetic exact C05 freezing; not production activation or adoption","checkpoint_limit":"A coherent rewrite requires an independent saved checkpoint to detect."}),
        );
    }
    if name == "CUR-008" {
        let assertions = [
            "chain_tamper",
            "duplicate_bytes",
            "prefix_visibility",
            "staging_failure",
        ];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_008",
            "--",
            "--list",
        ])?;
        let actual: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if actual != assertions.into_iter().collect() {
            return Err(format!("CUR008 missing assertions: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_008",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err("CUR008 incomplete qualification".into());
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes()),"scope":"synthetic encrypted authority; not C05 financial freeze or C10 lifecycle","checkpoint_limit":"A coherent rewrite requires an independent saved checkpoint to detect."}),
        );
    }
    if name == "CUR-005" {
        let assertions = ["cur_005_1", "cur_005_2", "cur_005_3", "cur_005_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_005",
            "--",
            "--list",
        ])?;
        let tests: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if tests != assertions.iter().copied().collect() {
            return Err(format!("test registry mismatch or zero tests: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-storage",
            "--test",
            "cur_005",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err(format!("incomplete test execution: {text}"));
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes())}),
        );
    }
    if name == "CUR-002" {
        let assertions = ["cur_002_1", "cur_002_2", "cur_002_3", "cur_002_4"];
        let list = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-types",
            "--test",
            "cur_002",
            "--",
            "--list",
        ])?;
        let tests: BTreeSet<_> = list
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .collect();
        if tests != assertions.iter().copied().collect() {
            return Err(format!("test registry mismatch or zero tests: {list}"));
        }
        let text = cargo(&[
            "test",
            "--locked",
            "-p",
            "currents-types",
            "--test",
            "cur_002",
        ])?;
        if !text.contains("4 passed; 0 failed; 0 ignored") {
            return Err(format!("incomplete test execution: {text}"));
        }
        return Ok(
            json!({"ticket":name,"assertions":assertions,"output_sha256":sha(text.as_bytes())}),
        );
    }
    if name != "CUR-001" {
        return Err(format!("BLOCKED: no registered test suite for {name}"));
    }
    let args = [
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_001",
        "--",
        "--list",
    ];
    let list = cargo(&args)?;
    let tests: BTreeSet<_> = list
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    if tests != ASSERTIONS.iter().copied().collect() {
        return Err(format!("test registry mismatch or zero tests: {list}"));
    }
    let text = cargo(&[
        "test",
        "--locked",
        "-p",
        "currents-types",
        "--test",
        "cur_001",
        "--",
        "--test-threads=1",
    ])?;
    if !text.contains("4 passed; 0 failed; 0 ignored") {
        return Err(format!("incomplete test execution: {text}"));
    }
    Ok(json!({"ticket":name,"assertions":ASSERTIONS,"output_sha256":sha(text.as_bytes())}))
}
pub(crate) fn native_source() -> Result<Value> {
    let text = cargo(&["metadata", "--locked", "--format-version", "1"])?;
    let meta: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let packages = packages(&meta)?;
    let sys: Vec<_> = packages
        .iter()
        .filter(|p| p["name"] == "libsqlite3-sys")
        .collect();
    if sys.len() != 1 || sys[0]["version"] != "0.37.0" || !sys[0]["source"].is_null() {
        return Err("native source must use exactly the private sys0.37 adaptation".into());
    }
    let expected = root().join("vendor/libsqlite3-sys/Cargo.toml");
    if sys[0]["manifest_path"].as_str() != expected.to_str() {
        return Err("unexpected sys manifest path".into());
    }
    let inventory = read_json(&root().join("tools/versions.lock.json"))?;
    let native = &inventory["native_source"];
    let source = packages
        .iter()
        .find(|p| p["name"] == "sqlcipher-src")
        .ok_or("missing fixed native source package")?;
    if source["version"] != native["version"] {
        return Err("native source version mismatch".into());
    }
    let manifest = PathBuf::from(
        source["manifest_path"]
            .as_str()
            .ok_or("missing source manifest")?,
    );
    for (name, digest) in native["files"]
        .as_object()
        .ok_or("missing source file inventory")?
    {
        let bytes = fs::read(manifest.parent().unwrap().join(name)).map_err(|e| e.to_string())?;
        if digest.as_str() != Some(sha(&bytes).as_str()) {
            return Err(format!("native source checksum mismatch: {name}"));
        }
    }
    Ok(
        json!({"sys_manifest":expected,"source":native,"scope":"fixed upstream source bytes and one native stack; runtime behavior separately tested"}),
    )
}
fn quality_checks() -> Result<Value> {
    native_source()?;
    workspace()?;
    direction(&root().join("Cargo.toml"))?;
    cargo(&["fmt", "--all", "--", "--check"])?;
    cargo(&[
        "clippy",
        "--locked",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    crate::quality::all()?;
    ticket("CUR-001")
}
fn quality() -> Result<Value> {
    let tests = quality_checks()?;
    Ok(json!({"tests":tests,"core":core(false, None)?}))
}
fn quality_ci() -> Result<Value> {
    let tests = quality_checks()?;
    Ok(json!({"tests":tests,"scope":"PR policy checks only; core required separately"}))
}
fn option(args: &[String], name: &str) -> Result<Option<PathBuf>> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    args.get(i + 1)
        .map(PathBuf::from)
        .map(Some)
        .ok_or_else(|| format!("missing {name} value"))
}
pub fn run(args: Vec<String>) -> Result<()> {
    if args == ["household-readiness"] {
        return crate::readiness::inspect_stdin();
    }
    let command = args.first().map(String::as_str).unwrap_or("");
    let gate = args.get(1).map(String::as_str).unwrap_or("");
    for (program, version_args) in [
        ("rustc", &["-vV"][..]),
        ("cargo", &["--version"][..]),
        ("cc", &["--version"][..]),
    ] {
        let _ = checked(Path::new(program), version_args, &root(), None);
    }
    let result = (|| {
        match (command,gate) {
        ("ticket","CUR-016") if args.len() == 2 => ticket_cur_016(false),
        ("ticket","CUR-016") if args.get(2).is_some_and(|arg| arg == "--inventory-only") && args.len() == 3 => ticket_cur_016(true),
        ("ticket","CUR-016") => Err("CUR-016 accepts only optional --inventory-only".into()),
        ("ticket",name)=>ticket(name),
        ("check","quality")=>quality(), ("check","quality-ci")=>quality_ci(), ("check","core")=>core(false, None), ("check","msrv")=>core(true, None),
        ("check","docs")=>crate::quality::docs(None),
        ("check","privacy")=>crate::quality::privacy(&root()),
        ("qualify", "format"|"compile"|"lints"|"allowlist"|"schema-drift"|"workflow-security"|"privacy"|"docs"|"generated-drift"|"dependencies")=>crate::quality::qualify(gate, &args),
        ("qualify","workspace")=>workspace(),
        ("qualify","native-platform")=>crate::native::command(&root(), &args[2..]),
        ("qualify","financekit")=>crate::financekit_gate::qualify(&args[2..]),
        ("qualify","household-inputs")=>crate::readiness::qualify(),
        ("qualify","native-source")=>native_source(),
        ("qualify","cipher")=>crate::cipher::qualify(),
        ("qualify","dependency-direction")=>direction(&option(&args,"--manifest-path")?.unwrap_or(root().join("Cargo.toml"))),
        ("qualify","pin-inventory")=>inventory(&option(&args,"--inventory")?.unwrap_or(root().join("tools/versions.lock.json")), option(&args,"--artifact-root")?.as_deref()),
        ("qualify","msrv") if args.iter().any(|s|s=="--probe")=>msrv_tools().map(|(_,_,v)|json!({"compiler":v.trim(),"scope":"compiler identity only; not a production feature qualification"})),
        ("qualify","msrv")=>core(true, option(&args,"--manifest-path")?.as_deref()),
        ("check","native"|"security"|"deep"|"release"|"all") | ("gates",_)=>Err(format!("BLOCKED: {command} {gate} requires suites registered by later owning tickets")),
        _=>Err(format!("BLOCKED: unregistered command: {}",args.join(" "))),
    }
    })();
    let head = checked(Path::new("git"), &["rev-parse", "HEAD"], &root(), None)
        .unwrap_or_else(|_| "unknown".into());
    let dirty = checked(Path::new("git"), &["status", "--porcelain"], &root(), None)
        .map(|s| !s.is_empty())
        .unwrap_or(true);
    // FinanceKit accepts no private input; do not persist rejected argument values.
    let report_args = if command == "qualify" && gate == "financekit" && args.len() > 2 {
        vec![
            "qualify".into(),
            "financekit".into(),
            "[unsupported arguments redacted]".into(),
        ]
    } else {
        args.clone()
    };
    let report = crate::report::build(&root(), &report_args, &head, dirty, &result);
    let report_dir = root().join("target/qualification");
    fs::create_dir_all(&report_dir).map_err(|e| e.to_string())?;
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    let filename = format!(
        "report-{}-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    fs::write(report_dir.join(filename), report_bytes).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    result.map(|_| ())
}

#[cfg(test)]
mod cipher_direction_tests {
    use super::sqlite_dependency_allowed;
    #[test]
    fn service_consumes_c03_without_native_access() {
        assert!(super::allowed_internal("currents-service").contains(&"currents-protocol"));
        assert_eq!(
            super::allowed_internal("currents-protocol"),
            ["currents-types"]
        );
        assert!(!super::allowed_internal("currents-service").contains(&"currents-accounting"));
        assert!(!sqlite_dependency_allowed("currents-service", "sqlx"));
    }
    #[test]
    fn verifier_key_plumbing_does_not_grant_sql_or_native_access() {
        assert!(super::allowed_internal("currents-verify").contains(&"currents-cipher"));
        assert!(!super::allowed_internal("currents-verify").contains(&"currents-accounting"));
        for dependency in ["sqlx", "sqlx-core", "sqlx-sqlite", "libsqlite3-sys"] {
            assert!(!sqlite_dependency_allowed("currents-verify", dependency));
        }
    }
    #[test]
    fn private_adapter_sqlx_edges_only() {
        for name in ["currents-storage", "currents-accounting"] {
            assert!(sqlite_dependency_allowed(name, "sqlx"));
            assert!(!sqlite_dependency_allowed(name, "libsqlite3-sys"));
            assert!(!sqlite_dependency_allowed(name, "sqlx-sqlite"));
        }
        for name in [
            "currents-types",
            "currents-domain",
            "currents-service",
            "currents-transfer",
        ] {
            assert!(!sqlite_dependency_allowed(name, "sqlx"));
            assert!(!sqlite_dependency_allowed(name, "libsqlite3-sys"));
        }
        for dependency in ["sqlx", "sqlx-core", "sqlx-sqlite", "libsqlite3-sys"] {
            assert!(sqlite_dependency_allowed("currents-cipher", dependency));
        }
    }
}
