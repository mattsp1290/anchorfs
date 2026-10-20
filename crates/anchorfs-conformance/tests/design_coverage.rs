mod common;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn values(section: &str) -> BTreeMap<String, u64> {
    let text = fs::read_to_string(root().join("docs/design/contracts.md")).unwrap();
    let block = text
        .split(&format!("## {section}\n"))
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let rows: Vec<_> = block
        .lines()
        .filter(|line| line.starts_with("| "))
        .skip(1)
        .map(|line| {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            (cells[1].to_owned(), cells[2].parse().unwrap())
        })
        .collect();
    let map: BTreeMap<_, _> = rows.iter().cloned().collect();
    assert_eq!(map.len(), rows.len(), "duplicate contract row");
    map
}
#[test]
fn contracts_limits_match_defaults() {
    let expected = anchorfs_core::Limits::default()
        .values()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    assert_eq!(values("Limits"), expected);
}
#[test]
fn contracts_timers_match_defaults() {
    let expected = anchorfs_core::Timers::default()
        .values()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    assert_eq!(values("Timers"), expected);
}
#[tokio::test]
async fn persistent_state_covers_every_column() {
    let (c, _pool, _) = common::database(&[]).await;
    let mut result=c.exec(testcontainers::core::ExecCommand::new(["psql","-U","postgres","-At","-v","ON_ERROR_STOP=1","-c","SELECT table_name || '.' || column_name FROM information_schema.columns WHERE table_schema='public' AND table_name<>'_sqlx_migrations' ORDER BY 1"])).await.unwrap();
    let output = result.stdout_to_vec().await.unwrap();
    assert_eq!(result.exit_code().await.unwrap(), Some(0));
    let actual: BTreeSet<_> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(actual.len(), 155);
    let doc = fs::read_to_string(root().join("docs/design/persistent-state.md")).unwrap();
    let mut table = "";
    let mut documented = BTreeSet::new();
    for line in doc.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            table = heading;
        }
        if line.starts_with("| ") {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            if let Some((prefix, _)) = cells[1].split_once('.') {
                assert_eq!(prefix, table);
                assert!(documented.insert(cells[1].to_owned()), "duplicate column");
                assert_eq!(cells.len(), 7);
                for cell in &cells[2..6] {
                    assert!(!cell.is_empty());
                }
            }
        }
    }
    assert_eq!(documented, actual);
    for boundary in 1..=13 {
        assert!(doc.contains(&format!("### B{boundary} —")));
    }
}
