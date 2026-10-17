use std::{collections::BTreeSet, fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn dependency_record_matches_workspace() {
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root().join("Cargo.toml")).unwrap()).unwrap();
    let declared: BTreeSet<_> = manifest["workspace"]["dependencies"]
        .as_table()
        .unwrap()
        .iter()
        .filter(|(_, value)| value.get("path").is_none())
        .map(|(name, _)| name.clone())
        .collect();
    let document = fs::read_to_string(root().join("docs/provenance/dependencies.md")).unwrap();
    let section = document
        .split("## Crates\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let recorded: BTreeSet<_> = section
        .lines()
        .filter(|line| line.starts_with("| "))
        .skip(1)
        .map(|line| line.split('|').nth(1).unwrap().trim().to_owned())
        .collect();
    assert_eq!(declared, recorded);
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
        ("backend", "core"),
        ("mount", "core"),
        ("service", "core"),
        ("service", "persistence"),
        ("service", "backend"),
        ("conformance", "core"),
        ("conformance", "persistence"),
        ("conformance", "service"),
    ]
    .into_iter()
    .map(|(from, to)| (format!("anchorfs-{from}"), format!("anchorfs-{to}")))
    .collect();
    assert_eq!(actual, expected);
}
