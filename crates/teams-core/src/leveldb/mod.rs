//! LevelDB 디렉토리 전체(.ldb + .log)를 전수 스캔해 (key, value) 원시 쌍을 모은다.
//! 정렬/comparator 무시. `.log`(최신)가 같은 key 를 덮어쓰면 그쪽을 우선한다.

mod log;
mod sstable;

use std::collections::HashMap;
use std::path::Path;

/// leveldb 디렉토리에서 모든 레코드를 읽어 `key -> value` 맵으로 반환.
/// `.ldb` 를 먼저 넣고 `.log` 로 덮어써 최신값이 이기게 한다.
pub fn read_dir(dir: &Path) -> std::io::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let mut ldb_files: Vec<_> = Vec::new();
    let mut log_files: Vec<_> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("ldb") => ldb_files.push(path),
            Some("log") => log_files.push(path),
            _ => {}
        }
    }
    // 파일명(=번호) 순 → 대체로 오래된→최신. .log 를 마지막에 적용.
    ldb_files.sort();
    log_files.sort();

    let mut map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

    for path in ldb_files {
        if let Ok(bytes) = std::fs::read(&path) {
            let _ = sstable::scan_table(&bytes, |k, v| {
                map.insert(k.to_vec(), v.to_vec());
            });
        }
    }
    for path in log_files {
        if let Ok(bytes) = std::fs::read(&path) {
            log::scan_log(&bytes, |k, v| {
                map.insert(k.to_vec(), v.to_vec());
            });
        }
    }
    Ok(map)
}
