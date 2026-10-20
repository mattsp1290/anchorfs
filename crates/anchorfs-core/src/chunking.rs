//! Pure plans for immutable chunks. A row's logical prefix can be shorter than
//! its stored object after truncation; the remainder must never become visible.
use crate::Limits;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkGeometry {
    pub chunk_size: u32,
    pub max_file_size: u64,
}

impl ChunkGeometry {
    pub fn from_limits(limits: &Limits) -> Self {
        Self {
            chunk_size: limits.chunk_size,
            max_file_size: limits
                .max_file_size
                .min(u64::from(limits.chunk_size) * u64::from(limits.max_chunks_per_file)),
        }
    }

    fn bound(self, size: u64) -> Result<(), ChunkError> {
        if self.chunk_size == 0 {
            return Err(ChunkError::Overflow);
        }
        if size > self.max_file_size {
            return Err(ChunkError::FileTooLarge);
        }
        if size.div_ceil(u64::from(self.chunk_size)) > u64::from(u32::MAX) + 1 {
            return Err(ChunkError::TooManyChunks);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChunkError {
    #[error("empty write")]
    ZeroLength,
    #[error("file exceeds configured size")]
    FileTooLarge,
    #[error("file exceeds chunk index capacity")]
    TooManyChunks,
    #[error("chunk arithmetic overflow or invalid geometry")]
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExistingChunk {
    pub index: u32,
    pub logical_length: u32,
    pub object_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchedChunk {
    pub index: u32,
    pub old: Option<ExistingChunk>,
    pub needs_fetch: bool,
    pub splice: Range<u32>,
    pub data: Range<usize>,
    pub new_logical_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    pub new_size: u64,
    pub touched: Vec<TouchedChunk>,
    pub reserved_bytes: u64,
}

/// Existing rows must be unique by index and have valid logical/object lengths.
pub fn plan_write(
    g: ChunkGeometry,
    size: u64,
    offset: u64,
    len: u32,
    existing: &[ExistingChunk],
) -> Result<WritePlan, ChunkError> {
    if len == 0 {
        return Err(ChunkError::ZeroLength);
    }
    let end = offset
        .checked_add(u64::from(len))
        .ok_or(ChunkError::Overflow)?;
    let new_size = size.max(end);
    g.bound(new_size)?;
    let chunk_size = u64::from(g.chunk_size);
    let mut touched = Vec::new();
    let mut reserved_bytes = 0u64;
    for index in offset / chunk_size..=(end - 1) / chunk_size {
        let start = index.checked_mul(chunk_size).ok_or(ChunkError::Overflow)?;
        let a = offset.max(start);
        let b = end.min(start.checked_add(chunk_size).ok_or(ChunkError::Overflow)?);
        let index = u32::try_from(index).map_err(|_| ChunkError::TooManyChunks)?;
        let old = existing.iter().find(|row| row.index == index).copied();
        let splice = (a - start) as u32..(b - start) as u32;
        let new_logical_length = old.map_or(0, |row| row.logical_length).max(splice.end);
        reserved_bytes = reserved_bytes
            .checked_add(u64::from(new_logical_length))
            .ok_or(ChunkError::Overflow)?;
        touched.push(TouchedChunk {
            index,
            old,
            needs_fetch: old
                .is_some_and(|row| !(splice.start == 0 && splice.end >= row.logical_length)),
            splice,
            data: (a - offset) as usize..(b - offset) as usize,
            new_logical_length,
        });
    }
    Ok(WritePlan {
        new_size,
        touched,
        reserved_bytes,
    })
}

/// Materialize a valid plan using a whole, verified old object when requested.
/// Never copy the discarded tail of a shortened logical prefix.
pub fn materialize(t: &TouchedChunk, old_object: Option<&[u8]>, data: &[u8]) -> Vec<u8> {
    let mut result = vec![0; t.new_logical_length as usize];
    if t.needs_fetch {
        let old = t.old.expect("fetch requires an existing row");
        let object = old_object.expect("required old object");
        assert_eq!(object.len(), old.object_length as usize);
        let length = old.logical_length as usize;
        result[..length].copy_from_slice(&object[..length]);
    }
    result[t.splice.start as usize..t.splice.end as usize].copy_from_slice(&data[t.data.clone()]);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncatePlan {
    pub kept: Vec<ExistingChunk>,
    pub changed: bool,
}

pub fn plan_truncate(
    g: ChunkGeometry,
    old_size: u64,
    new_size: u64,
    existing: &[ExistingChunk],
) -> Result<TruncatePlan, ChunkError> {
    g.bound(old_size.max(new_size))?;
    let mut kept = Vec::new();
    for row in existing {
        let start = u64::from(row.index) * u64::from(g.chunk_size);
        if start < new_size {
            kept.push(ExistingChunk {
                logical_length: u64::from(row.logical_length).min(new_size - start) as u32,
                ..*row
            });
        }
    }
    kept.sort_by_key(|row| row.index);
    Ok(TruncatePlan {
        kept,
        changed: old_size != new_size,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPiece {
    pub index: u32,
    pub row: Option<ExistingChunk>,
    pub chunk_range: Range<u32>,
    pub zero_fill: u32,
}

/// Clip a read to EOF. Callers supply valid geometry and manifest rows.
pub fn plan_read(
    g: ChunkGeometry,
    size: u64,
    offset: u64,
    length: u32,
    existing: &[ExistingChunk],
) -> Vec<ReadPiece> {
    if offset >= size || length == 0 {
        return Vec::new();
    }
    let end = offset.saturating_add(u64::from(length)).min(size);
    let chunk_size = u64::from(g.chunk_size);
    let mut pieces = Vec::new();
    for index in offset / chunk_size..=(end - 1) / chunk_size {
        let start = index * chunk_size;
        let a = (offset.max(start) - start) as u32;
        let b = (end.min(start + chunk_size) - start) as u32;
        let index = u32::try_from(index).expect("validated geometry");
        let row = existing.iter().find(|row| row.index == index).copied();
        let prefix = row.map_or(0, |row| row.logical_length);
        let chunk_range = a.min(prefix)..b.min(prefix);
        let zero_fill = (b - a) - (chunk_range.end - chunk_range.start);
        pieces.push(ReadPiece {
            index,
            row,
            chunk_range,
            zero_fill,
        });
    }
    pieces
}

pub fn plan_replace(g: ChunkGeometry, length: u64) -> Result<Vec<(u32, u32)>, ChunkError> {
    g.bound(length)?;
    let chunk_size = u64::from(g.chunk_size);
    (0..length.div_ceil(chunk_size))
        .map(|index| {
            Ok((
                u32::try_from(index).map_err(|_| ChunkError::TooManyChunks)?,
                (length - index * chunk_size).min(chunk_size) as u32,
            ))
        })
        .collect()
}
