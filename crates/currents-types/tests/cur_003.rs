//! Real CLI counterexamples, exclusively synthetic and isolated from the candidate.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn run(args: &[&str]) -> Output {
    Command::new("cargo")
        .current_dir(root())
        .arg("xtask")
        .args(args)
        .output()
        .unwrap()
}
fn pass(out: Output) {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn fail(out: Output, marker: &str) {
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(marker),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
fn project(dir: &Path, source: &str) -> PathBuf {
    fs::create_dir_all(dir.join("src")).unwrap();
    let policy = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let policy = policy
        .split("[workspace.lints.rust]")
        .nth(1)
        .unwrap()
        .split("[workspace.lints.clippy]")
        .next()
        .unwrap();
    fs::write(dir.join("Cargo.toml"), format!("[package]\nname=\"currents-domain\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[lints.rust]\n{policy}")).unwrap();
    fs::write(dir.join("src/lib.rs"), source).unwrap();
    dir.join("Cargo.toml")
}
#[test]
fn cur_003_1() {
    let d = tempfile::tempdir().unwrap();
    let manifest = project(d.path(), "pub fn good() {}\n");
    for gate in ["format", "compile", "lints"] {
        pass(run(&[
            "qualify",
            gate,
            "--manifest-path",
            manifest.to_str().unwrap(),
        ]));
    }
    for (gate, source, marker) in [
        ("format", "pub fn bad( ){}\n", "format"),
        (
            "compile",
            "pub fn bad(p: *const u8) -> u8 { unsafe { *p } }\n",
            "unsafe",
        ),
        ("compile", "fn unused() {}\n", "dead_code"),
        (
            "lints",
            "pub fn bad() { Some(1).unwrap(); }\n",
            "unwrap_used",
        ),
        (
            "lints",
            "pub fn bad(a: u8,b:u8)->u8 { a+b }\n",
            "arithmetic_side_effects",
        ),
        (
            "allowlist",
            "#[allow(clippy::unwrap_used)]\npub fn bad() { Some(1).unwrap(); }\n",
            "allowlist",
        ),
    ] {
        fs::write(d.path().join("src/lib.rs"), source).unwrap();
        fail(
            run(&[
                "qualify",
                gate,
                "--manifest-path",
                manifest.to_str().unwrap(),
            ]),
            marker,
        );
    }
    for source in [
        "#[allow(clippy::unwrap_used, reason = \"tracked in #12\")]\npub fn bad(v: Option<u8>) -> u8 { v.unwrap() }\n",
        "#![allow(clippy::unwrap_used, reason = \"tracked in #12\")]\npub fn bad(v: Option<u8>) -> u8 { v.unwrap() }\n",
        "#[expect(clippy::unwrap_used, reason = r\"review #12 ]\")]\npub fn bad(v: Option<u8>) -> u8 { v.unwrap() }\n",
        "#[cfg_attr(not(test), allow(clippy::unwrap_used, reason = \"tracked in #12\"))]\npub fn bad(v: Option<u8>) -> u8 { v.unwrap() }\n",
    ] {
        fs::write(d.path().join("src/lib.rs"), source).unwrap();
        fail(
            run(&[
                "qualify",
                "allowlist",
                "--manifest-path",
                manifest.to_str().unwrap(),
            ]),
            "allowlist",
        );
    }
    pass(run(&["qualify", "allowlist"]));
    pass(run(&["qualify", "schema-drift"]));
    let schema = d.path().join("stale.json");
    fs::write(&schema, "{}").unwrap();
    fail(
        run(&[
            "qualify",
            "schema-drift",
            "--schema",
            schema.to_str().unwrap(),
        ]),
        "schema drift",
    );
}
#[test]
fn cur_003_2() {
    pass(run(&["qualify", "workflow-security"]));
    let value: Value =
        serde_json::from_slice(&fs::read(root().join(".github/workflows/ci.yml")).unwrap())
            .unwrap();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ci.yml");
    let mut variants = vec![];
    let mut v = value.clone();
    v["on"]["pull_request_target"] = serde_json::json!({});
    variants.push(v);
    let mut v = value.clone();
    v["permissions"]["contents"] = "write".into();
    variants.push(v);
    let mut v = value.clone();
    v["jobs"]["quality"]["steps"][0]["with"]["persist-credentials"] = true.into();
    variants.push(v);
    let mut v = value.clone();
    v["jobs"]["quality"]["env"] = serde_json::json!({"KEY":"${{ secrets.SIGNING_KEY }}"});
    variants.push(v);
    let mut v = value;
    v["jobs"]["quality"]["steps"].as_array_mut().unwrap().push(serde_json::json!({"uses":"actions/upload-artifact@0000000000000000000000000000000000000000"}));
    variants.push(v);
    for v in variants {
        fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        fail(
            run(&[
                "qualify",
                "workflow-security",
                "--workflow",
                p.to_str().unwrap(),
            ]),
            "workflow security",
        );
    }
}
#[test]
fn cur_003_3() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("safe.txt"), "public synthetic data\n").unwrap();
    pass(run(&[
        "qualify",
        "privacy",
        "--source",
        d.path().to_str().unwrap(),
    ]));
    let key = ["AKIA", "ABCDEFGHIJKLMNOP"].concat();
    let account = ["CURRENTS_PRIVATE_ACCOUNT=", "123456789012"].concat();
    for canary in [&key, &account] {
        fs::write(d.path().join("canary.txt"), format!("{canary}\n")).unwrap();
        let out = run(&["qualify", "privacy", "--source", d.path().to_str().unwrap()]);
        assert!(!out.status.success());
        for bytes in [&out.stdout, &out.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(&key));
            assert!(!text.contains(&account));
        }
        let log = fs::read_to_string(root().join("target/qualification/commands.jsonl")).unwrap();
        assert!(!log.contains(&key));
        assert!(!log.contains(&account));
        fail(out, "privacy scan failed; details redacted");
    }
}
#[test]
fn cur_003_4() {
    let d = tempfile::tempdir().unwrap();
    let manifest = project(
        d.path(),
        "/// ```\n/// assert_eq!(2+2,4);\n/// ```\npub fn documented() {}\n",
    );
    pass(run(&[
        "qualify",
        "docs",
        "--manifest-path",
        manifest.to_str().unwrap(),
    ]));
    fs::write(
        d.path().join("src/lib.rs"),
        "/// ```\n/// assert_eq!(2+2,5);\n/// ```\npub fn documented() {}\n",
    )
    .unwrap();
    fail(
        run(&[
            "qualify",
            "docs",
            "--manifest-path",
            manifest.to_str().unwrap(),
        ]),
        "doc",
    );
    fs::write(
        d.path().join("src/lib.rs"),
        "/// [MissingType]\npub fn documented() {}\n",
    )
    .unwrap();
    fail(
        run(&[
            "qualify",
            "docs",
            "--manifest-path",
            manifest.to_str().unwrap(),
        ]),
        "rustdoc::broken_intra_doc_links",
    );
}
