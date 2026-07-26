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

/// 글로벌 메타데이터 영역에서 (origin, database name) → database id 를 담는 타입 바이트.
const DATABASE_NAME_TYPE_BYTE: u8 = 201;
/// database 내부 object store 메타데이터 타입 바이트.
const OBJECT_STORE_META_DATA_TYPE_BYTE: u8 = 50;
/// object store 메타데이터 중 "이름" 항목(meta_data_type).
const OBJECT_STORE_META_DATA_NAME: u8 = 0;

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

    /// 글로벌 메타데이터 영역(<0,0,0>) 인가.
    fn is_global_meta(&self) -> bool {
        self.database_id == 0 && self.object_store_id == 0 && self.index_id == 0
    }

    /// database 메타데이터 영역(<db,0,0>) 인가.
    fn is_database_meta(&self) -> bool {
        self.database_id != 0 && self.object_store_id == 0 && self.index_id == 0
    }
}

/// IndexedDB leveldb coding 의 `EncodeInt`(최소 바이트 little-endian) 값을 u64 로 디코드.
fn decode_int_le(buf: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, &b) in buf.iter().enumerate().take(8) {
        v |= u64::from(b) << (8 * i);
    }
    v
}

/// `EncodeString` (UTF-16 big-endian, 길이 접두어 없음) 를 String 으로.
fn decode_utf16_be(buf: &[u8]) -> Option<String> {
    if buf.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// `EncodeStringWithLength` (varint 코드유닛 수 + UTF-16BE) 를 읽어 `(문자열, 소비 바이트)` 반환.
fn read_string_with_length(buf: &[u8], pos: usize) -> Option<(String, usize)> {
    let (n_units, used) = crate::varint::read_varint(buf, pos)?;
    let start = pos + used;
    let byte_len = (n_units as usize).checked_mul(2)?;
    let bytes = buf.get(start..start + byte_len)?;
    let s = decode_utf16_be(bytes)?;
    Some((s, used + byte_len))
}

/// 글로벌 메타데이터의 `DatabaseNameKey` 레코드에서 database 이름과 id 를 추출.
/// key = <0,0,0> + [201] + StringWithLength(origin) + StringWithLength(name),
/// value = database id (EncodeInt, LE).
///
/// 다른 종류의 글로벌 메타데이터 키이거나 파싱 실패 시 `None`.
pub fn parse_database_name(key: &[u8], value: &[u8]) -> Option<(String, u64)> {
    let kp = KeyPrefix::parse(key)?;
    if !kp.is_global_meta() {
        return None;
    }
    let mut pos = kp.len;
    if *key.get(pos)? != DATABASE_NAME_TYPE_BYTE {
        return None;
    }
    pos += 1;
    // origin (건너뜀)
    let (_origin, used) = read_string_with_length(key, pos)?;
    pos += used;
    let (name, _used) = read_string_with_length(key, pos)?;
    Some((name, decode_int_le(value)))
}

/// database 메타데이터의 `ObjectStoreMetaDataKey` (이름 항목) 에서
/// (database id, object store id, object store 이름) 을 추출.
/// key = <db,0,0> + [50] + varint(object_store_id) + [0],
/// value = object store 이름 (UTF-16BE).
///
/// 이름 항목이 아니거나 파싱 실패 시 `None`.
pub fn parse_object_store_name(key: &[u8], value: &[u8]) -> Option<(u64, u64, String)> {
    let kp = KeyPrefix::parse(key)?;
    if !kp.is_database_meta() {
        return None;
    }
    let mut pos = kp.len;
    if *key.get(pos)? != OBJECT_STORE_META_DATA_TYPE_BYTE {
        return None;
    }
    pos += 1;
    let (store_id, used) = crate::varint::read_varint(key, pos)?;
    pos += used;
    if *key.get(pos)? != OBJECT_STORE_META_DATA_NAME {
        return None;
    }
    let name = decode_utf16_be(value)?;
    Some((kp.database_id, store_id, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16_be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn string_with_length(s: &str) -> Vec<u8> {
        // varint(코드유닛 수) + UTF-16BE. 여기 문자열은 모두 BMP(1 유닛/문자).
        let mut out = vec![s.chars().count() as u8]; // < 128 이라 varint 1바이트
        out.extend(utf16_be(s));
        out
    }

    #[test]
    fn parses_database_name_key() {
        // <0,0,0> + [201] + StringWithLength(origin) + StringWithLength(name)
        let mut key = vec![0x00, 0x00, 0x00, 0x00, DATABASE_NAME_TYPE_BYTE];
        key.extend(string_with_length("https_teams.microsoft.com_0"));
        key.extend(string_with_length("Teams:replychain-manager:react-web-client:x"));
        let value = [0x2C]; // EncodeInt LE 44

        let (name, id) = parse_database_name(&key, &value).unwrap();
        assert_eq!(id, 44);
        assert_eq!(name, "Teams:replychain-manager:react-web-client:x");
    }

    #[test]
    fn parses_object_store_name_key() {
        // <db=44,0,0> + [50] + varint(store=2) + [0], value = 이름(UTF-16BE)
        let key = [
            0x00,
            44,
            0x00,
            0x00,
            OBJECT_STORE_META_DATA_TYPE_BYTE,
            0x02,
            OBJECT_STORE_META_DATA_NAME,
        ];
        let value = utf16_be("replychains-2");

        let (db, store, name) = parse_object_store_name(&key, &value).unwrap();
        assert_eq!(db, 44);
        assert_eq!(store, 2);
        assert_eq!(name, "replychains-2");
    }

    #[test]
    fn ignores_non_metadata_keys() {
        // object-store-data 키(<db,store,index=1>)는 메타 파서가 무시해야 한다.
        let key = [0x00, 44, 0x01, 0x01];
        assert!(parse_database_name(&key, &[]).is_none());
        assert!(parse_object_store_name(&key, &[]).is_none());
    }
}
