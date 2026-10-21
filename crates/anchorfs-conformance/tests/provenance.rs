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
    fn digests(text: &str) -> BTreeSet<String> {
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
    let mut fixtures = BTreeSet::new();
    for file in [
        "crates/anchorfs-conformance/tests/common/minio.rs",
        "crates/anchorfs-conformance/tests/common/mod.rs",
        "crates/anchorfs-persistence/tests/migrations.rs",
        "crates/anchorfs-service/tests/binary.rs",
        "crates/anchorfs-mount/tests/linux/fixture.rs",
    ] {
        fixtures.extend(digests(&fs::read_to_string(root().join(file)).unwrap()));
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
        digests(images),
        "fixture/image approval digests must match both ways"
    );
}
