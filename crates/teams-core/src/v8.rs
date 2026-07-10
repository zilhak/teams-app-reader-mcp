//! V8 ValueSerializer(structured clone) 디코더.
//!
//! Teams 의 IndexedDB 값은 커스텀 포맷이 아니라 **Chromium 표준 V8 ValueSerializer** 다.
//! 값 바이트는 `blink 봉투(0xFF 0x15 ...) + V8 payload(0xFF <ver> o...{)` 형태로,
//! blink 봉투를 건너뛴 payload 를 아래 태그 문법으로 파싱한다.
//! (참조 구현: docs/teams-storage-structure.md §4, Python PoC 로 검증됨.)

use serde_json::{Map, Value};

/// 값 바이트에서 V8 payload 시작 오프셋을 찾는다.
/// blink 헤더 `FF 15` 를 찾고, 그 뒤 패딩(FE/00)을 지나 나오는 V8 version 태그
/// `FF <ver>` 다음(=payload)을 반환. blink 헤더가 없으면 마지막 `FF` 뒤로 폴백.
pub fn find_v8_start(buf: &[u8]) -> Option<usize> {
    // blink 헤더 FF 15 탐색
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == 0xFF && buf[i + 1] == 0x15 {
            // FF15 뒤부터 다음 FF(=V8 version 태그)까지 스킵
            let mut j = i + 2;
            while j < buf.len() && buf[j] != 0xFF {
                j += 1;
            }
            if j + 1 < buf.len() {
                return Some(j + 2); // FF <ver> 다음
            }
            return None;
        }
        i += 1;
    }
    // 폴백: 마지막 FF 뒤
    buf.iter().rposition(|&b| b == 0xFF).map(|p| p + 2)
}

pub struct V8Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> V8Reader<'a> {
    pub fn new(buf: &'a [u8], start: usize) -> Self {
        V8Reader { b: buf, i: start }
    }

    /// buf 에서 V8 payload 를 찾아 최상위 값을 디코드하는 편의 함수.
    pub fn decode(buf: &'a [u8]) -> Option<Value> {
        let start = find_v8_start(buf)?;
        let mut r = V8Reader::new(buf, start);
        r.read_value().ok()
    }

    fn u8(&mut self) -> Result<u8, ()> {
        let x = *self.b.get(self.i).ok_or(())?;
        self.i += 1;
        Ok(x)
    }

    fn varint(&mut self) -> Result<u64, ()> {
        let (v, n) = crate::varint::read_varint(self.b, self.i).ok_or(())?;
        self.i += n;
        Ok(v)
    }

    fn zigzag(&mut self) -> Result<i64, ()> {
        let n = self.varint()?;
        Ok(((n >> 1) as i64) ^ -((n & 1) as i64))
    }

    /// 0x00(padding) / 0xFF<ver>(version) 를 건너뛰고 다음 유효 태그 위치로 이동.
    fn skip_padding(&mut self) {
        while let Some(&b) = self.b.get(self.i) {
            match b {
                0x00 => self.i += 1,
                0xFF => self.i += 2,
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> Result<u8, ()> {
        self.skip_padding();
        self.b.get(self.i).copied().ok_or(())
    }

    pub fn read_value(&mut self) -> Result<Value, ()> {
        self.skip_padding();
        let c = self.u8()?;
        match c {
            b'T' => Ok(Value::Bool(true)),
            b'F' => Ok(Value::Bool(false)),
            b'0' | b'_' => Ok(Value::Null), // null / undefined
            b'I' => Ok(Value::from(self.zigzag()?)),
            b'U' => Ok(Value::from(self.varint()?)),
            b'N' => {
                let bytes = self.b.get(self.i..self.i + 8).ok_or(())?;
                let mut a = [0u8; 8];
                a.copy_from_slice(bytes);
                self.i += 8;
                Ok(Value::from(f64::from_le_bytes(a)))
            }
            b'"' => self.read_one_byte_string(),
            b'c' => self.read_two_byte_string(),
            b'S' => self.read_utf8_string(),
            b'o' => self.read_object(),
            b'A' => self.read_dense_array(),
            b'a' => self.read_sparse_array(),
            b'^' => {
                let _id = self.varint()?;
                Ok(Value::Null) // object reference: 순환/공유 참조. 메시지 본문엔 없음.
            }
            b'D' => {
                let bytes = self.b.get(self.i..self.i + 8).ok_or(())?;
                let mut a = [0u8; 8];
                a.copy_from_slice(bytes);
                self.i += 8;
                Ok(Value::from(f64::from_le_bytes(a))) // date: ms epoch double
            }
            _ => Err(()),
        }
    }

    fn read_one_byte_string(&mut self) -> Result<Value, ()> {
        let n = self.varint()? as usize;
        let bytes = self.b.get(self.i..self.i + n).ok_or(())?;
        self.i += n;
        // Latin1 → String
        let s: String = bytes.iter().map(|&b| b as char).collect();
        Ok(Value::String(s))
    }

    fn read_two_byte_string(&mut self) -> Result<Value, ()> {
        let n = self.varint()? as usize; // 바이트 길이
        let bytes = self.b.get(self.i..self.i + n).ok_or(())?;
        self.i += n;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(Value::String(String::from_utf16_lossy(&units)))
    }

    fn read_utf8_string(&mut self) -> Result<Value, ()> {
        let n = self.varint()? as usize;
        let bytes = self.b.get(self.i..self.i + n).ok_or(())?;
        self.i += n;
        Ok(Value::String(String::from_utf8_lossy(bytes).into_owned()))
    }

    fn read_object(&mut self) -> Result<Value, ()> {
        let mut map = Map::new();
        while self.peek()? != b'{' {
            let k = self.read_value()?;
            let v = self.read_value()?;
            let key = match k {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            map.insert(key, v);
        }
        self.i += 1; // '{'
        let _props = self.varint()?;
        Ok(Value::Object(map))
    }

    fn read_dense_array(&mut self) -> Result<Value, ()> {
        let _len = self.varint()?;
        let mut arr = Vec::new();
        while self.peek()? != b'$' {
            arr.push(self.read_value()?);
        }
        self.i += 1; // '$'
        let _props = self.varint()?;
        let _length = self.varint()?;
        Ok(Value::Array(arr))
    }

    fn read_sparse_array(&mut self) -> Result<Value, ()> {
        let _len = self.varint()?;
        let mut map = Map::new();
        while self.peek()? != b'@' {
            let k = self.read_value()?;
            let v = self.read_value()?;
            let key = match k {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            map.insert(key, v);
        }
        self.i += 1; // '@'
        let _props = self.varint()?;
        let _length = self.varint()?;
        Ok(Value::Object(map))
    }
}
