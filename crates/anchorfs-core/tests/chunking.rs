use anchorfs_core::*;

fn geometry() -> ChunkGeometry {
    ChunkGeometry {
        chunk_size: 8,
        max_file_size: 64,
    }
}
fn row(index: u32, logical_length: u32, object_length: u32) -> ExistingChunk {
    ExistingChunk {
        index,
        logical_length,
        object_length,
    }
}

#[test]
fn plan_write_aligned_single_chunk_needs_no_fetch() {
    let p = plan_write(geometry(), 8, 0, 8, &[row(0, 8, 8)]).unwrap();
    assert_eq!(p.new_size, 8);
    assert_eq!(p.reserved_bytes, 8);
    assert!(!p.touched[0].needs_fetch);
    assert_eq!(materialize(&p.touched[0], None, b"abcdefgh"), b"abcdefgh");
}

#[test]
fn plan_write_partial_prefix_fetches_old() {
    let p = plan_write(geometry(), 8, 0, 2, &[row(0, 8, 8)]).unwrap();
    assert!(p.touched[0].needs_fetch);
    assert_eq!(
        materialize(&p.touched[0], Some(b"abcdefgh"), b"XY"),
        b"XYcdefgh"
    );
}

#[test]
fn plan_write_spans_two_chunks_with_hole() {
    let p = plan_write(geometry(), 0, 7, 3, &[]).unwrap();
    assert_eq!(p.new_size, 10);
    assert_eq!(p.reserved_bytes, 10);
    assert_eq!(
        p.touched.iter().map(|t| t.index).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(materialize(&p.touched[0], None, b"ABC"), b"\0\0\0\0\0\0\0A");
    assert_eq!(materialize(&p.touched[1], None, b"ABC"), b"BC");
}

#[test]
fn plan_write_rejects_zero_length_and_overflow() {
    assert_eq!(
        plan_write(geometry(), 0, 0, 0, &[]),
        Err(ChunkError::ZeroLength)
    );
    assert_eq!(
        plan_write(geometry(), 0, u64::MAX, 1, &[]),
        Err(ChunkError::Overflow)
    );
    assert_eq!(
        plan_write(geometry(), 0, 63, 2, &[]),
        Err(ChunkError::FileTooLarge)
    );
}

#[test]
fn materialize_zero_fills_beyond_trimmed_prefix_of_longer_object() {
    let p = plan_write(geometry(), 8, 6, 1, &[row(0, 2, 8)]).unwrap();
    assert_eq!(p.reserved_bytes, 7);
    assert_eq!(
        materialize(&p.touched[0], Some(b"abSECRET"), b"X"),
        b"ab\0\0\0\0X"
    );
}

#[test]
fn plan_truncate_clips_boundary_and_drops_tail() {
    let p = plan_truncate(
        geometry(),
        24,
        10,
        &[row(0, 8, 8), row(1, 8, 8), row(2, 8, 8)],
    )
    .unwrap();
    assert!(p.changed);
    assert_eq!(p.kept, [row(0, 8, 8), row(1, 2, 8)]);
}

#[test]
fn plan_truncate_grow_changes_nothing_but_size() {
    let rows = [row(0, 2, 8)];
    let p = plan_truncate(geometry(), 2, 24, &rows).unwrap();
    assert_eq!(p.kept, rows);
    assert!(p.changed);
    assert!(!plan_truncate(geometry(), 2, 2, &rows).unwrap().changed);
}

#[test]
fn plan_read_clips_at_eof_and_zero_fills_holes() {
    let p = plan_read(geometry(), 20, 6, 32, &[row(0, 7, 8)]);
    assert_eq!(p.len(), 3);
    assert_eq!((p[0].chunk_range.clone(), p[0].zero_fill), (6..7, 1));
    assert_eq!((p[1].chunk_range.clone(), p[1].zero_fill), (0..0, 8));
    assert_eq!(p[2].zero_fill, 4);
    assert!(plan_read(geometry(), 20, 20, 1, &[]).is_empty());
    assert!(plan_read(geometry(), 20, 0, 0, &[]).is_empty());
    let tail = plan_read(geometry(), 8, 6, 2, &[row(0, 2, 8)]);
    assert_eq!(tail[0].chunk_range, 2..2);
    assert_eq!(tail[0].zero_fill, 2);
}

#[test]
fn plan_read_uses_object_length_for_fetch() {
    let p = plan_read(geometry(), 4, 0, 4, &[row(0, 4, 8)]);
    assert_eq!(p[0].row.unwrap().object_length, 8);
    assert_eq!(p[0].chunk_range, 0..4);
}

#[test]
fn plan_replace_full_and_tail() {
    assert_eq!(
        plan_replace(geometry(), 18).unwrap(),
        [(0, 8), (1, 8), (2, 2)]
    );
    assert!(plan_replace(geometry(), 0).unwrap().is_empty());
    assert_eq!(plan_replace(geometry(), 65), Err(ChunkError::FileTooLarge));
}

#[test]
fn geometry_bounds_file_size_by_chunk_capacity() {
    let limits = Limits {
        chunk_size: 4096,
        ..Limits::default()
    };
    let g = ChunkGeometry::from_limits(&limits);
    assert_eq!(
        g.max_file_size,
        4096 * u64::from(limits.max_chunks_per_file)
    );
    assert_eq!(
        plan_write(g, 0, g.max_file_size, 1, &[]),
        Err(ChunkError::FileTooLarge)
    );
}

#[test]
fn arithmetic_above_two_gib_is_not_truncated() {
    let g = ChunkGeometry::from_limits(&Limits::default());
    let offset = (1u64 << 32) + 7;
    let p = plan_write(g, 0, offset, 1, &[]).unwrap();
    assert_eq!(p.touched[0].index, 1024);
    assert_eq!(p.touched[0].splice, 7..8);
    assert_eq!(plan_read(g, p.new_size, offset, 1, &[])[0].zero_fill, 1);
}
