//! LevelDB / V8 에서 공통으로 쓰는 LEB128 varint 디코더.

/// `buf[pos..]` 에서 LEB128 varint(u64) 를 읽어 `(값, 소비한 바이트 수)` 반환.
/// 잘린 경우 `None`.
pub fn read_varint(buf: &[u8], pos: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    let mut i = pos;
    loop {
        let byte = *buf.get(i)?;
        i += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i - pos));
        }
        shift += 7;
        if shift >= 64 {
            return None; // malformed
        }
    }
}

/// 고정 4바이트 little-endian u32.
pub fn read_u32_le(buf: &[u8], pos: usize) -> Option<u32> {
    let b = buf.get(pos..pos + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
