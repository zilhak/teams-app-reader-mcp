//! 특정 store 의 레코드를 통째로 예쁘게 찍어 스키마 확인용. 임시 도구.
//! 실행: cargo run -p teams-core --example dump -- <db> <store>
//! 예:   cargo run -p teams-core --example dump -- 44 1

use teams_core::idb::KeyPrefix;
use teams_core::leveldb;
use teams_core::location;
use teams_core::v8::V8Reader;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want_db: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(44);
    let want_store: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let filter = args.get(3).cloned(); // 값에 이 문자열이 포함된 레코드만

    let db_path = location::default_db_path().unwrap();
    let records = leveldb::read_dir(&db_path).unwrap();

    let mut shown = 0;
    for (key, value) in &records {
        let Some(kp) = KeyPrefix::parse(key) else {
            continue;
        };
        if kp.database_id == want_db && kp.object_store_id == want_store && kp.is_object_store_data()
        {
            if let Some(v) = V8Reader::decode(value) {
                let s = serde_json::to_string(&v).unwrap();
                if let Some(f) = &filter {
                    if !s.contains(f.as_str()) {
                        continue;
                    }
                }
                println!("=== record {} (key {} bytes) ===", shown, key.len());
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                shown += 1;
                if shown >= 1 {
                    break;
                }
            }
        }
    }
}
