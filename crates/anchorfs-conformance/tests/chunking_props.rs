//! Independently authored dense-vector oracle for sparse immutable chunk plans.
use anchorfs_core::{
    ChunkGeometry, ExistingChunk, materialize, plan_read, plan_truncate, plan_write,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

const CHUNK: u32 = 16;
const MAX: usize = 192;
type Stored = BTreeMap<u32, (u32, Vec<u8>)>;

fn rows(stored: &Stored) -> Vec<ExistingChunk> {
    stored
        .iter()
        .map(|(&index, (logical_length, object))| ExistingChunk {
            index,
            logical_length: *logical_length,
            object_length: object.len() as u32,
        })
        .collect()
}

fn read(g: ChunkGeometry, size: usize, offset: usize, length: u32, stored: &Stored) -> Vec<u8> {
    let mut result = Vec::new();
    for piece in plan_read(g, size as u64, offset as u64, length, &rows(stored)) {
        if let Some(row) = piece.row {
            let object = &stored[&row.index].1;
            assert_eq!(row.object_length as usize, object.len());
            assert!(piece.chunk_range.end <= row.logical_length);
            result.extend_from_slice(
                &object[piece.chunk_range.start as usize..piece.chunk_range.end as usize],
            );
        } else {
            assert!(piece.chunk_range.is_empty());
        }
        result.resize(result.len() + piece.zero_fill as usize, 0);
    }
    result
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn sparse_chunk_plans_match_dense_bytes(
        size in 0usize..=MAX,
        seed in prop::collection::vec((any::<bool>(), 1u32..=CHUNK, prop::collection::vec(any::<u8>(), CHUNK as usize)), MAX / CHUNK as usize),
        ops in prop::collection::vec((0u8..3, 0usize..=MAX, prop::collection::vec(any::<u8>(), 0..=32), 0u32..=256), 1..100),
    ) {
        let g = ChunkGeometry { chunk_size: CHUNK, max_file_size: MAX as u64 };
        let mut stored = Stored::new();
        let mut dense = vec![0; size];
        for (index, (present, logical, object)) in seed.into_iter().enumerate() {
            let start = index * CHUNK as usize;
            if present && start < size {
                let logical = (logical as usize).min(size - start);
                dense[start..start+logical].copy_from_slice(&object[..logical]);
                stored.insert(index as u32, (logical as u32, object));
            }
        }
        prop_assert_eq!(read(g, dense.len(), 0, MAX as u32, &stored), dense.clone());
        for (kind, offset, mut data, length) in ops {
            if kind == 0 {
                data.truncate(MAX - offset);
                if !data.is_empty() {
                    let old_rows = rows(&stored);
                    let plan = plan_write(g, dense.len() as u64, offset as u64, data.len() as u32, &old_rows).unwrap();
                    prop_assert_eq!(&plan, &plan_write(g, dense.len() as u64, offset as u64, data.len() as u32, &old_rows).unwrap());
                    let mut reserved = 0;
                    for touched in plan.touched {
                        let old = stored.get(&touched.index).map(|(_, object)| object.as_slice());
                        let object = materialize(&touched, old, &data);
                        prop_assert_eq!(object.len(), touched.new_logical_length as usize);
                        reserved += object.len() as u64;
                        stored.insert(touched.index, (touched.new_logical_length, object));
                    }
                    prop_assert_eq!(reserved, plan.reserved_bytes);
                    dense.resize(dense.len().max(offset + data.len()), 0);
                    dense[offset..offset+data.len()].copy_from_slice(&data);
                    prop_assert_eq!(plan.new_size, dense.len() as u64);
                }
            } else if kind == 1 {
                let plan = plan_truncate(g, dense.len() as u64, offset as u64, &rows(&stored)).unwrap();
                prop_assert_eq!(plan.changed, dense.len() != offset);
                stored.retain(|index, _| plan.kept.iter().any(|row| row.index == *index));
                for row in plan.kept { stored.get_mut(&row.index).unwrap().0 = row.logical_length; }
                dense.resize(offset, 0);
            }
            let expected = if offset >= dense.len() { &[] } else { &dense[offset..dense.len().min(offset + length as usize)] };
            prop_assert_eq!(read(g, dense.len(), offset, length, &stored), expected);
            prop_assert_eq!(read(g, dense.len(), 0, MAX as u32, &stored), dense.clone());
        }
    }
}
