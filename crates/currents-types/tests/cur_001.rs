//! Public command-contract tests. Fixtures contain no user data or credentials.
use std::{env, fs, path::PathBuf, process::Command};

fn root() -> PathBuf {
    env::var_os("CURRENTS_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            option_env!("CARGO_MANIFEST_DIR")
                .map(|path| {
                    PathBuf::from(path)
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_path_buf()
                })
                .unwrap_or_else(|| PathBuf::from(env!("PWD")))
        })
}
fn xtask(args: &[&str]) -> std::process::Output {
    Command::new("cargo")
        .current_dir(root())
        .args(["xtask"])
        .args(args)
        .output()
        .expect("Cargo must be installed")
}
#[test]
fn cur_001_1() {
    let output = xtask(&["qualify", "workspace"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"status\": \"pass\""));
}
#[test]
fn cur_001_2() {
    let dir = env::temp_dir().join(format!("currents-direction-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for dependency in ["reqwest", "objc2-foundation"] {
        for name in ["currents-domain", dependency] {
            fs::create_dir_all(dir.join(name).join("src")).unwrap();
            fs::write(dir.join(name).join("src/lib.rs"), "").unwrap();
            fs::write(dir.join(name).join("Cargo.toml"), format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n")).unwrap();
        }
        fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[workspace]\nmembers = [\"currents-domain\", \"{dependency}\"]\nresolver = \"3\"\n"
            ),
        )
        .unwrap();
        let manifest = dir.join("currents-domain/Cargo.toml");
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str(&format!(
            "[dependencies]\n{dependency} = {{ path = \"../{dependency}\" }}\n"
        ));
        fs::write(manifest, text).unwrap();
        let output = xtask(&[
            "qualify",
            "dependency-direction",
            "--manifest-path",
            dir.join("Cargo.toml").to_str().unwrap(),
        ]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("forbidden dependency"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cur_001_3() {
    let output = xtask(&["qualify", "msrv", "--probe"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("rustc 1.94.0"));
}
#[test]
fn cur_001_4() {
    let dir = env::temp_dir().join(format!("currents-pins-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let inventory = dir.join("inventory.json");
    let artifact = dir.join("tool");
    fs::write(&artifact, "abc").unwrap();
    let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let write = |checksum: &str| {
        fs::write(&inventory, format!(
        "{{\"schema_version\":1,\"artifacts\":[{{\"name\":\"synthetic-tool\",\"version\":\"1.0.0\",\"source\":\"https://example.invalid/synthetic\",\"sha256\":\"{checksum}\",\"path\":\"tool\"}}]}}"
    )).unwrap()
    };
    write(good);
    let check = || {
        xtask(&[
            "qualify",
            "pin-inventory",
            "--inventory",
            inventory.to_str().unwrap(),
        ])
    };
    let output = check();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    write(&"0".repeat(64));
    let output = check();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
    write(good);
    fs::remove_file(&artifact).unwrap();
    let output = check();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing artifact"));
    let missing_tool = Command::new("cargo")
        .current_dir(root())
        .args(["xtask", "qualify", "msrv", "--probe"])
        .env("CURRENTS_MSRV_BIN", &dir)
        .output()
        .unwrap();
    assert!(!missing_tool.status.success());
    assert!(String::from_utf8_lossy(&missing_tool.stderr).contains("BLOCKED"));
    fs::remove_dir_all(dir).unwrap();
}
