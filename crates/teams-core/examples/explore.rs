//! 실 DB 탐색용 임시 도구 (M1~M3 조사). 정식 API 아님.
//! - (db, store, index) 별 레코드 수 집계
//! - object-store-data(index_id=1) 레코드의 V8 값을 디코드해 메시지/대화 판별
//!
//! 실행: cargo run -p teams-core --example explore
//!       (선택) cargo run -p teams-core --example explore -- <db경로> [dump=<db>/<store>]

use std::collections::BTreeMap;
use teams_core::idb::KeyPrefix;
use teams_core::leveldb;
use teams_core::location;
use teams_core::v8::V8Reader;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .get(1)
        .map(std::path::PathBuf::from)
        .or_else(location::default_db_path)
        .expect("db path");
    eprintln!("DB: {}", db_path.display());

    let records = leveldb::read_dir(&db_path).expect("read_dir");
    eprintln!("총 레코드: {}", records.len());

    // (db, store, index) → count
    let mut groups: BTreeMap<(u64, u64, u64), usize> = BTreeMap::new();
    // object-store-data 값의 최상위 필드 이름 빈도 (store 별)
    let mut field_freq: BTreeMap<(u64, u64), BTreeMap<String, usize>> = BTreeMap::new();
    let mut decoded_ok = 0usize;
    let mut decoded_fail = 0usize;

    // 샘플 저장: store 별로 첫 디코드 성공 값 하나
    let mut samples: BTreeMap<(u64, u64), serde_json::Value> = BTreeMap::new();

    for (key, value) in &records {
        let Some(kp) = KeyPrefix::parse(key) else {
            continue;
        };
        *groups
            .entry((kp.database_id, kp.object_store_id, kp.index_id))
            .or_default() += 1;

        if kp.is_object_store_data() {
            match V8Reader::decode(value) {
                Some(v) => {
                    decoded_ok += 1;
                    if let Some(obj) = v.as_object() {
                        let e = field_freq
                            .entry((kp.database_id, kp.object_store_id))
                            .or_default();
                        for k in obj.keys() {
                            *e.entry(k.clone()).or_default() += 1;
                        }
                    }
                    samples
                        .entry((kp.database_id, kp.object_store_id))
                        .or_insert(v);
                }
                None => decoded_fail += 1,
            }
        }
    }

    println!("\n=== (db, store, index) 별 레코드 수 ===");
    for ((db, store, index), n) in &groups {
        println!("db{db} store{store} index{index}: {n}");
    }

    println!("\n=== object-store-data V8 디코드: 성공 {decoded_ok} / 실패 {decoded_fail} ===");

    println!("\n=== store 별 최상위 필드 빈도 (index_id=1) ===");
    for ((db, store), fields) in &field_freq {
        let mut items: Vec<_> = fields.iter().collect();
        items.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = items
            .iter()
            .take(15)
            .map(|(k, n)| format!("{k}({n})"))
            .collect();
        println!("db{db} store{store}: {}", top.join(", "));
    }

    println!("\n=== store 별 샘플 값 (앞 600자) ===");
    for ((db, store), v) in &samples {
        let s = serde_json::to_string(v).unwrap_or_default();
        let cut: String = s.chars().take(600).collect();
        println!("--- db{db} store{store} ---\n{cut}\n");
    }
}
