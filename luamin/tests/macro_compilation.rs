use std::{fs, path::Path, process::Command};

#[test]
fn downstream_macros_compile_execute_and_report_errors() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project = manifest_dir.join("../target/macro-consumer");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::copy(
        manifest_dir.join("tests/fixtures/macro-consumer.rs"),
        project.join("src/lib.rs"),
    )
    .unwrap();
    // Use the versions fetched for the workspace, including in a clean CI cache.
    fs::copy(
        manifest_dir.join("../Cargo.lock"),
        project.join("Cargo.lock"),
    )
    .unwrap();
    let cases = [
        (
            "missing_option",
            "incorrect_arg_type_error_func property is required",
        ),
        ("unsafe_method", "Unsafe methods are not supported"),
        ("too_many_inputs", "At most 15 script inputs"),
        ("too_many_outputs", "At most 3 script outputs"),
        ("invalid_gas", "Gas cost must be nonnegative"),
        ("conditional_attribute", "cfg_attr is not supported"),
        ("value_receiver", "Receiver must be shorthand"),
        ("manual_gas", "Manual gas cannot be combined"),
        ("method_generics", "Method generics are not supported"),
        ("unsupported_output", "Unsupported output type"),
        ("wrong_error", "Return error must match error_type"),
        ("too_many_lifetimes", "At most 4 lifetime parameters"),
        ("invalid_derive_option", "Expected crate = path"),
    ];
    let runtime = manifest_dir.to_str().unwrap().replace('\\', "/");
    let features: String = cases
        .iter()
        .map(|(name, _)| format!("{name} = []\n"))
        .collect();
    fs::write(
        project.join("Cargo.toml"),
        format!(
            r#"
[package]
name = "luamin-macro-consumer"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
engine = {{ package = "luamin", path = "{runtime}" }}
thiserror = "2.0.18"
[features]
{features}
"#
        ),
    )
    .unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let run = |command: &str, feature: Option<&str>| {
        let mut cmd = Command::new(&cargo);
        cmd.args([command, "--offline", "--manifest-path"])
            .arg(project.join("Cargo.toml"));
        cmd.env("CARGO_TARGET_DIR", project.join("build"));
        if let Some(feature) = feature {
            cmd.args(["--features", feature]);
        }
        let output = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        (output.status.success(), stderr)
    };
    let (success, stderr) = run("test", None);
    assert!(success, "Downstream consumer failed:\n{stderr}");
    for (feature, expected) in cases {
        let (success, stderr) = run("check", Some(feature));
        assert!(!success, "{feature} unexpectedly compiled");
        assert!(
            stderr.contains(expected),
            "{feature}: expected {expected:?}, got:\n{stderr}"
        );
        assert!(!stderr.contains("procedural macro panicked"), "{stderr}");
        assert!(
            stderr.contains("src"),
            "Diagnostic should identify user source: {stderr}"
        );
    }
}
