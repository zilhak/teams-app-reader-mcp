//! LevelDB SSTable(`.ldb`) 직접 파서.
//!
//! Chromium IndexedDB 는 커스텀 comparator `idb_cmp1` 로 정렬돼 있어 범용 리더가
//! 정렬 검증에서 패닉한다. 우리는 **전수 스캔**만 하므로 정렬을 전혀 신경쓰지 않고
//! 테이블 포맷(footer → index block → data blocks, 블록별 snappy)만 파싱한다.

use crate::varint::{read_u32_le, read_varint};

const FOOTER_SIZE: usize = 48;

struct BlockHandle {
    offset: usize,
    size: usize,
}

/// footer / index 의 BlockHandle(varint offset, varint size) 를 읽는다.
fn read_block_handle(buf: &[u8], pos: usize) -> Option<(BlockHandle, usize)> {
    let (offset, n1) = read_varint(buf, pos)?;
    let (size, n2) = read_varint(buf, pos + n1)?;
    Some((
        BlockHandle {
            offset: offset as usize,
            size: size as usize,
        },
        n1 + n2,
    ))
}

/// BlockHandle 이 가리키는 블록을 읽어 압축을 풀어서 반환.
fn read_block(file: &[u8], handle: &BlockHandle) -> Option<Vec<u8>> {
    let start = handle.offset;
    let end = handle.offset.checked_add(handle.size)?;
    let raw = file.get(start..end)?;
    let compression = *file.get(end)?; // trailer 첫 바이트
    match compression {
        0 => Some(raw.to_vec()),                       // 무압축
        1 => snap::raw::Decoder::new().decompress_vec(raw).ok(), // snappy
        _ => None,                                     // zstd(2/4) 등 미지원 → 스킵
    }
}

/// 압축 해제된 블록 하나를 순회하며 (key, value) 를 콜백에 넘긴다.
/// prefix compression(restart 기반)을 복원한다.
fn iter_block<F: FnMut(&[u8], &[u8])>(block: &[u8], mut f: F) -> Option<()> {
    if block.len() < 4 {
        return None;
    }
    let num_restarts = read_u32_le(block, block.len() - 4)? as usize;
    let restart_array_size = num_restarts * 4 + 4;
    let entries_end = block.len().checked_sub(restart_array_size)?;

    let mut key: Vec<u8> = Vec::new();
    let mut p = 0usize;
    while p < entries_end {
        let (shared, n1) = read_varint(block, p)?;
        let (non_shared, n2) = read_varint(block, p + n1)?;
        let (value_len, n3) = read_varint(block, p + n1 + n2)?;
        p += n1 + n2 + n3;
        let shared = shared as usize;
        let non_shared = non_shared as usize;
        let value_len = value_len as usize;

        if shared > key.len() {
            return None;
        }
        key.truncate(shared);
        let kd = block.get(p..p + non_shared)?;
        key.extend_from_slice(kd);
        p += non_shared;

        let value = block.get(p..p + value_len)?;
        p += value_len;

        f(&key, value);
    }
    Some(())
}

/// `.ldb` 파일 전체 바이트에서 모든 (key, value) 를 뽑아 콜백에 넘긴다.
/// 손상/미지원 블록은 조용히 건너뛴다(전수 스캔이라 부분 실패 허용).
pub fn scan_table<F: FnMut(&[u8], &[u8])>(file: &[u8], mut f: F) -> Option<()> {
    if file.len() < FOOTER_SIZE {
        return None;
    }
    let footer_start = file.len() - FOOTER_SIZE;
    // footer: metaindex_handle, index_handle, padding, 8-byte magic
    let (_metaindex, n1) = read_block_handle(file, footer_start)?;
    let (index_handle, _n2) = read_block_handle(file, footer_start + n1)?;

    let index_block = read_block(file, &index_handle)?;
    // index block 의 각 value = data block 의 BlockHandle
    let mut data_handles: Vec<BlockHandle> = Vec::new();
    iter_block(&index_block, |_k, v| {
        if let Some((h, _)) = read_block_handle(v, 0) {
            data_handles.push(h);
        }
    });

    for h in data_handles {
        if let Some(block) = read_block(file, &h) {
            let _ = iter_block(&block, &mut f);
        }
    }
    Some(())
}
