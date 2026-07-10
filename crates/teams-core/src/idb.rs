//! Chromium IndexedDB 의 leveldb key 앞부분(KeyPrefix)만 해석한다.
//! 어느 database / object store / index 에 속한 레코드인지 구분하는 용도.
//!
//! KeyPrefix 인코딩 (indexed_db_leveldb_coding.cc):
//!   first_byte = (dbIdBytes-1)<<5 | (storeIdBytes-1)<<2 | (indexIdBytes-1)
//!   그 뒤 dbId(LE), storeId(LE), indexId(LE) 를 각 바이트수만큼.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPrefix {
    pub database_id: u64,
    pub object_store_id: u64,
    pub index_id: u64,
    /// KeyPrefix 가 차지한 바이트 수(그 뒤부터 실제 key payload).
    pub len: usize,
}

/// object store 의 실제 레코드(값 객체)를 담는 index_id.
pub const OBJECT_STORE_DATA_INDEX_ID: u64 = 1;

fn read_le(buf: &[u8], pos: usize, n: usize) -> Option<u64> {
    let bytes = buf.get(pos..pos + n)?;
    let mut v = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        v |= u64::from(b) << (8 * i);
    }
    Some(v)
}

impl KeyPrefix {
    pub fn parse(key: &[u8]) -> Option<KeyPrefix> {
        let first = *key.first()?;
        let db_bytes = ((first >> 5) & 0x07) as usize + 1;
        let store_bytes = ((first >> 2) & 0x07) as usize + 1;
        let index_bytes = (first & 0x03) as usize + 1;
        let mut pos = 1;
        let database_id = read_le(key, pos, db_bytes)?;
        pos += db_bytes;
        let object_store_id = read_le(key, pos, store_bytes)?;
        pos += store_bytes;
        let index_id = read_le(key, pos, index_bytes)?;
        pos += index_bytes;
        Some(KeyPrefix {
            database_id,
            object_store_id,
            index_id,
            len: pos,
        })
    }

    /// object store 의 값 객체(ObjectStoreDataKey) 레코드인가.
    pub fn is_object_store_data(&self) -> bool {
        self.index_id == OBJECT_STORE_DATA_INDEX_ID
    }
}
