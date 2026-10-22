use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn dependency_record_matches_workspace() {
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root().join("Cargo.toml")).unwrap()).unwrap();
    let declared: BTreeMap<_, _> = manifest["workspace"]["dependencies"]
        .as_table()
        .unwrap()
        .iter()
        .filter(|(_, value)| value.get("path").is_none())
        .map(|(name, value)| {
            (
                name.clone(),
                value["version"]
                    .as_str()
                    .unwrap()
                    .strip_prefix('=')
                    .expect("exact direct pin")
                    .to_owned(),
            )
        })
        .collect();
    let document = fs::read_to_string(root().join("docs/provenance/dependencies.md")).unwrap();
    let section = document
        .split("## Crates\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let rows: Vec<_> = section
        .lines()
        .filter(|line| line.starts_with("| "))
        .skip(1)
        .map(|line| {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            (cells[1].to_owned(), cells[2].to_owned())
        })
        .collect();
    let recorded: BTreeMap<_, _> = rows.iter().cloned().collect();
    assert_eq!(
        rows.len(),
        recorded.len(),
        "duplicate dependency provenance row"
    );
    let normalized: BTreeMap<_, _> = recorded
        .iter()
        .map(|(name, version)| {
            // Cargo ignores build metadata in requirements; provenance retains the full resolved identity.
            (name.clone(), version.split('+').next().unwrap().to_owned())
        })
        .collect();
    assert_eq!(declared, normalized);
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(root())
        .other_options(vec!["--locked".into()])
        .exec()
        .unwrap();
    for (name, version) in recorded {
        assert!(
            metadata
                .packages
                .iter()
                .any(|package| package.name.as_str() == name
                    && package.version.to_string() == version
                    && package.source.is_some()),
            "recorded {name} {version} missing from resolved registry graph"
        );
    }
}

#[test]
fn crate_edges_match_plan() {
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(root())
        .exec()
        .unwrap();
    let members: BTreeSet<_> = metadata
        .workspace_packages()
        .iter()
        .map(|p| p.name.to_string())
        .collect();
    let mut actual = BTreeSet::new();
    for package in metadata.workspace_packages() {
        for dependency in &package.dependencies {
            if members.contains(&dependency.name) {
                actual.insert((package.name.to_string(), dependency.name.clone()));
            }
        }
    }
    let expected = [
        ("persistence", "core"),
        ("client", "core"),
        ("conformance", "client"),
        ("backend", "core"),
        ("mount", "core"),
        ("mount", "client"),
        ("mount", "service"),
        ("mount", "persistence"),
        ("mount", "backend"),
        ("service", "core"),
        ("service", "persistence"),
        ("service", "backend"),
        ("service", "client"),
        ("conformance", "backend"),
        ("conformance", "core"),
        ("conformance", "persistence"),
        ("conformance", "service"),
    ]
    .into_iter()
    .map(|(from, to)| (format!("anchorfs-{from}"), format!("anchorfs-{to}")))
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn test_image_record_matches_fixtures() {
    let mut fixtures = BTreeSet::new();
    for file in [
        "crates/anchorfs-conformance/tests/common/minio.rs",
        "crates/anchorfs-conformance/tests/common/mod.rs",
        "crates/anchorfs-persistence/tests/migrations.rs",
        "crates/anchorfs-service/tests/binary.rs",
        "crates/anchorfs-mount/tests/linux/fixture.rs",
    ] {
        fixtures.extend(image_digests(
            &fs::read_to_string(root().join(file)).unwrap(),
        ));
    }
    let record = fs::read_to_string(root().join("docs/provenance/dependencies.md")).unwrap();
    let images = record
        .split("## Test images\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert_eq!(
        fixtures,
        image_digests(images),
        "fixture/image approval digests must match both ways"
    );
}

fn image_digests(text: &str) -> BTreeSet<String> {
    text.match_indices("sha256:")
        .map(|(at, _)| {
            let digest: String = text[at..].chars().take(71).collect();
            assert_eq!(digest.len(), 71, "full image digest");
            assert!(
                digest[7..].bytes().all(|b| b.is_ascii_hexdigit()),
                "valid digest {digest}"
            );
            digest
        })
        .collect()
}
#[test]
fn base_image_record_matches_dockerfiles() {
    let dockerfile = fs::read_to_string(root().join("docker/Dockerfile")).unwrap();
    let record = fs::read_to_string(root().join("docs/provenance/dependencies.md")).unwrap();
    let section = record
        .split("## Container base images\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert_eq!(image_digests(&dockerfile), image_digests(section));
    assert_eq!(image_digests(&dockerfile).len(), 2);
}
#[test]
fn builder_image_matches_toolchain() {
    let toolchain: toml::Value =
        toml::from_str(&fs::read_to_string(root().join("rust-toolchain.toml")).unwrap()).unwrap();
    let expected = format!(
        "{}-bookworm",
        toolchain["toolchain"]["channel"].as_str().unwrap()
    );
    let dockerfile = fs::read_to_string(root().join("docker/Dockerfile")).unwrap();
    let builders: Vec<_> = dockerfile
        .lines()
        .filter_map(|line| line.strip_prefix("FROM rust:"))
        .collect();
    assert!(!builders.is_empty(), "Rust builder required");
    for builder in builders {
        assert_eq!(builder.split('@').next().unwrap(), expected);
        assert!(builder.contains("@sha256:"));
    }
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_sha256_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|h| is_hex(h, 64))
}

/// Stage 3b section of the dependency record (appended last).
fn stage3b_section() -> String {
    read("docs/provenance/dependencies.md")
        .split("## Stage 3b delivery tools, actions and images")
        .nth(1)
        .expect("Stage 3b dependency section")
        .to_owned()
}

#[test]
fn bases_record_is_well_formed() {
    let record: serde_json::Value = serde_json::from_str(&read("docker/bases.json")).unwrap();
    assert_eq!(record["schema_version"], 1);
    let dockerfile = read("docker/Dockerfile");
    let dependencies = read("docs/provenance/dependencies.md");
    let bases = record["bases"].as_object().unwrap();
    assert_eq!(
        bases.keys().map(String::as_str).collect::<Vec<_>>(),
        ["mount", "service"]
    );
    for (name, base) in bases {
        let target = base["dockerfile_target"].as_str().unwrap();
        assert!(
            dockerfile
                .lines()
                .any(|l| l.starts_with("FROM ") && l.ends_with(&format!(" AS {target}"))),
            "{name}: Dockerfile stage {target} missing"
        );
        assert!(
            base["repository"]
                .as_str()
                .unwrap()
                .starts_with("anchorfs/")
        );
        let mut digests = vec![base["index_digest"].clone()];
        digests.extend(base["platforms"].as_object().unwrap().values().cloned());
        assert_eq!(
            base["platforms"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["linux/amd64", "linux/arm64"]
        );
        for digest in digests {
            if let Some(digest) = digest.as_str() {
                assert!(
                    is_sha256_digest(digest),
                    "{name}: malformed digest {digest}"
                );
                assert!(
                    dependencies.contains(digest),
                    "{name}: published digest {digest} missing from dependencies.md"
                );
            } else {
                assert!(digest.is_null(), "{name}: digest must be null or a string");
            }
        }
    }
}

#[test]
fn dockerfile_runtime_stages_derive_from_bases() {
    let dockerfile = read("docker/Dockerfile");
    let lines: Vec<_> = dockerfile.lines().collect();
    for (derived, base) in [("service", "service-base"), ("mount", "mount-base")] {
        let from = format!("FROM {base} AS {derived}");
        assert!(lines.contains(&from.as_str()), "missing `{from}`");
    }
    // A base stage runs from its FROM line to the next FROM line.
    for base in ["service-base", "mount-base"] {
        let start = lines
            .iter()
            .position(|l| l.starts_with("FROM ") && l.ends_with(&format!(" AS {base}")))
            .unwrap_or_else(|| panic!("stage {base} missing"));
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.starts_with("FROM "))
            .map_or(lines.len(), |n| start + 1 + n);
        for line in &lines[start..end] {
            assert!(
                !line.contains("--from=builder"),
                "{base} must not copy from the builder stage: {line}"
            );
        }
    }
}

#[test]
fn chart_metadata_is_flat_and_recorded() {
    let chart = read("charts/anchorfs/Chart.yaml");
    let mut fields = BTreeMap::new();
    for line in chart.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        assert!(
            !line.starts_with(char::is_whitespace) && !line.starts_with("- "),
            "Chart.yaml must be flat key: scalar lines: {line}"
        );
        let (key, value) = line.split_once(": ").expect("key: scalar");
        assert!(!value.trim().is_empty(), "empty scalar for {key}");
        assert!(
            !value.trim().starts_with(['[', '{', '|', '>']),
            "non-scalar value for {key}"
        );
        fields.insert(key.to_owned(), value.trim().trim_matches('"').to_owned());
    }
    assert_eq!(fields["name"], "anchorfs");
    let delivery = read("docs/ops/forgejo-delivery.md");
    let named = delivery
        .lines()
        .find_map(|l| l.split("`charts/anchorfs` version `").nth(1))
        .and_then(|rest| rest.split('`').next())
        .expect("forgejo-delivery.md names the chart version");
    assert_eq!(fields["version"], named);
}

const HELPER_PLACEHOLDER: &str = "PLACEHOLDER";

/// Parsed `uses:` values, per-job container images and env pins of the workflow.
fn workflow_lines() -> Vec<String> {
    read(".forgejo/workflows/deploy.yml")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn scalar(line: &str, key: &str) -> Option<String> {
    let rest = line.trim().trim_start_matches("- ").strip_prefix(key)?;
    let value = rest.strip_prefix(':')?.split(" #").next()?.trim();
    Some(value.trim_matches('"').to_owned())
}

#[test]
fn forgejo_workflow_pins_are_immutable() {
    let lines = workflow_lines();
    let dependencies = read("docs/provenance/dependencies.md");
    let record = stage3b_section();

    let uses: Vec<_> = lines.iter().filter_map(|l| scalar(l, "uses")).collect();
    assert!(!uses.is_empty());
    for action in &uses {
        let (_, pin) = action.rsplit_once('@').expect("uses without @pin");
        assert!(is_hex(pin, 40), "mutable action reference {action}");
        let recorded = action
            .trim_start_matches("https://")
            .trim_start_matches("github.com/");
        assert!(
            dependencies.contains(recorded),
            "{action} missing from dependencies.md"
        );
    }

    let ci_image = lines
        .iter()
        .find_map(|l| scalar(l, "CI_IMAGE"))
        .expect("CI_IMAGE env");
    assert!(
        ci_image
            .rsplit_once("@sha256:")
            .is_some_and(|(_, h)| is_hex(h, 64)),
        "CI_IMAGE must be digest-pinned"
    );
    let images: Vec<_> = lines.iter().filter_map(|l| scalar(l, "image")).collect();
    assert_eq!(images.len(), 3, "one container image per job");
    for image in &images {
        assert!(!image.contains("${{"), "container.image must be a literal");
        assert_eq!(image, &ci_image, "job image must equal CI_IMAGE");
    }
    assert!(
        record.contains(&ci_image),
        "CI image missing from dependencies.md"
    );

    let helper = lines
        .iter()
        .find_map(|l| scalar(l, "BIRBHOMES_HELPER_COMMIT"))
        .expect("BIRBHOMES_HELPER_COMMIT env");
    if is_hex(&helper, 40) {
        assert!(
            record.contains(&helper),
            "helper commit missing from dependencies.md"
        );
    } else {
        assert_eq!(helper, HELPER_PLACEHOLDER);
        assert!(
            read("docs/ops/forgejo-delivery.md")
                .lines()
                .any(|l| l.trim() == "status: prepared"),
            "helper placeholder is allowed only while the delivery doc is `status: prepared`"
        );
    }

    let secret_uses: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("${{ secrets."))
        .collect();
    assert_eq!(secret_uses.len(), 1, "exactly one secret reference");
    assert!(
        secret_uses[0].trim_start().starts_with("token: "),
        "secret allowed only as a token: input"
    );
}

#[test]
fn exporter_exists_and_is_referenced_by_workflow() {
    assert!(root().join("scripts/oci_export.py").is_file());
    assert!(root().join("scripts/tests/test_oci_export.py").is_file());
    assert!(read(".forgejo/workflows/deploy.yml").contains("scripts/oci_export.py"));
}

#[test]
fn service_depends_on_client_only_for_probe() {
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(root())
        .exec()
        .unwrap();
    let service = metadata
        .workspace_packages()
        .into_iter()
        .find(|p| p.name.as_str() == "anchorfs-service")
        .unwrap();
    let edges: Vec<_> = service
        .dependencies
        .iter()
        .filter(|d| d.name == "anchorfs-client")
        .collect();
    assert_eq!(edges.len(), 1, "exactly one anchorfs-client edge");
    assert_eq!(edges[0].kind, cargo_metadata::DependencyKind::Normal);
    // The edge exists for the `probe` subcommand's readiness call; nothing else may use it.
    let sources = service.manifest_path.parent().unwrap().join("src");
    let mut users = Vec::new();
    for entry in fs::read_dir(sources.as_std_path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs")
            && fs::read_to_string(&path)
                .unwrap()
                .contains("anchorfs_client")
        {
            users.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert_eq!(
        users,
        ["main.rs"],
        "anchorfs_client is used only by the probe in main.rs"
    );
}

#[test]
fn cluster_scripts_never_print_secrets() {
    let mut scripts: Vec<_> = fs::read_dir(root().join("scripts/cluster"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sh"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty());
    for script in scripts {
        for (number, line) in fs::read_to_string(&script).unwrap().lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with('#') || !(code.contains("echo") || code.contains("printf")) {
                continue;
            }
            // Only inspect the printed text, not the shell tests around it.
            let printed = code
                .split_once("echo")
                .or_else(|| code.split_once("printf"))
                .unwrap()
                .1;
            let mut rest = printed;
            while let Some(at) = rest.find('$') {
                rest = &rest[at + 1..];
                let name: String = rest
                    .trim_start_matches('{')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let upper = name.to_ascii_uppercase();
                assert!(
                    !["PASSWORD", "SECRET", "TOKEN", "BEARER"]
                        .iter()
                        .any(|w| upper.contains(w)),
                    "{}:{} prints secret-named variable ${name}",
                    script.display(),
                    number + 1
                );
            }
        }
    }
    for line in workflow_lines() {
        if line.contains("${{ secrets.") {
            assert!(
                line.trim_start().starts_with("token: "),
                "secret outside token: {line}"
            );
        }
        let trimmed = line.trim_start();
        if (trimmed.contains("echo") || trimmed.contains("printf")) && !trimmed.starts_with('#') {
            assert!(
                !trimmed.to_ascii_uppercase().contains("TOKEN"),
                "workflow prints a token-named value: {line}"
            );
        }
    }
}
