//! IndexedDB 메타데이터(database 이름→id, object store 이름→id) 덤프 (검증용, 정식 API 아님).
//! 실행: cargo run -p teams-core --example meta

use std::collections::BTreeMap;
use teams_core::idb;
use teams_core::leveldb;
use teams_core::location;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .get(1)
        .map(std::path::PathBuf::from)
        .or_else(location::default_db_path)
        .expect("db path");
    eprintln!("DB: {}", db_path.display());

    let records = leveldb::read_dir(&db_path).expect("read_dir");

    let mut dbs: BTreeMap<u64, String> = BTreeMap::new();
    let mut stores: BTreeMap<(u64, u64), String> = BTreeMap::new();

    for (key, value) in &records {
        if let Some((name, id)) = idb::parse_database_name(key, value) {
            dbs.insert(id, name);
        }
        if let Some((db, sid, name)) = idb::parse_object_store_name(key, value) {
            stores.insert((db, sid), name);
        }
    }

    println!("\n=== database 이름 → id ===");
    for (id, name) in &dbs {
        println!("db{id}: {name}");
    }

    println!("\n=== (db, store id) → object store 이름 ===");
    for ((db, sid), name) in &stores {
        println!("db{db} store{sid}: {name}");
    }
}
