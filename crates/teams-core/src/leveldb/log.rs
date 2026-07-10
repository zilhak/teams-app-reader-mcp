//! LevelDB write-ahead log(`.log`) 파서. 최신(아직 SSTable 로 컴팩션 안 된) 데이터가 여기 있다.
//!
//! 포맷: 32KB 블록 단위. 각 블록 = record 들의 나열.
//!   record = crc(4) + length(2 LE) + type(1) + data(length)
//!   type: 1=FULL, 2=FIRST, 3=MIDDLE, 4=LAST (조각을 이어붙임)
//! 이어붙인 record = WriteBatch:
//!   seq(8) + count(4) + [entry...]
//!   entry: tag(1) + varstring key (+ varstring value if tag==1(kTypeValue))
//!          tag==0 이면 kTypeDeletion (value 없음)

use crate::varint::read_varint;

const BLOCK_SIZE: usize = 32 * 1024;
const HEADER_SIZE: usize = 7; // crc(4) + len(2) + type(1)

/// varstring: varint 길이 + 바이트.
fn read_varstring(buf: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    let (len, n) = read_varint(buf, pos)?;
    let start = pos + n;
    let end = start + len as usize;
    let s = buf.get(start..end)?;
    Some((s, n + len as usize))
}

/// 이어붙인 WriteBatch 하나에서 (key, value) 쌍을 뽑아 콜백에 넘긴다. deletion 은 스킵.
fn scan_batch<F: FnMut(&[u8], &[u8])>(batch: &[u8], f: &mut F) -> Option<()> {
    // seq(8) + count(4) 헤더
    if batch.len() < 12 {
        return None;
    }
    let count = u32::from_le_bytes([batch[8], batch[9], batch[10], batch[11]]);
    let mut p = 12usize;
    for _ in 0..count {
        let tag = *batch.get(p)?;
        p += 1;
        match tag {
            1 => {
                // kTypeValue: key, value
                let (key, kn) = read_varstring(batch, p)?;
                p += kn;
                let (val, vn) = read_varstring(batch, p)?;
                p += vn;
                f(key, val);
            }
            0 => {
                // kTypeDeletion: key 만
                let (_key, kn) = read_varstring(batch, p)?;
                p += kn;
            }
            _ => return None, // 알 수 없는 tag → 이 batch 중단
        }
    }
    Some(())
}

/// `.log` 파일 전체 바이트에서 모든 (key, value) 를 뽑아 콜백에 넘긴다.
/// 꼬리가 잘려 있어도(쓰기 도중) 가능한 만큼만 읽고 멈춘다.
pub fn scan_log<F: FnMut(&[u8], &[u8])>(file: &[u8], mut f: F) {
    let mut fragment: Vec<u8> = Vec::new();
    let mut block_start = 0usize;
    while block_start < file.len() {
        let block_end = (block_start + BLOCK_SIZE).min(file.len());
        let block = &file[block_start..block_end];
        let mut p = 0usize;
        while p + HEADER_SIZE <= block.len() {
            let length = u16::from_le_bytes([block[p + 4], block[p + 5]]) as usize;
            let rtype = block[p + 6];
            let data_start = p + HEADER_SIZE;
            let data_end = data_start + length;
            if rtype == 0 && length == 0 {
                break; // zero padding → 블록 끝
            }
            let data = match block.get(data_start..data_end) {
                Some(d) => d,
                None => break, // 잘림
            };
            match rtype {
                1 => {
                    // FULL
                    let _ = scan_batch(data, &mut f);
                }
                2 => {
                    // FIRST
                    fragment.clear();
                    fragment.extend_from_slice(data);
                }
                3 => {
                    // MIDDLE
                    fragment.extend_from_slice(data);
                }
                4 => {
                    // LAST
                    fragment.extend_from_slice(data);
                    let _ = scan_batch(&fragment, &mut f);
                    fragment.clear();
                }
                _ => {}
            }
            p = data_end;
        }
        block_start += BLOCK_SIZE;
    }
}
