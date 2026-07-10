//! TeamsStore 통합 동작 확인용. cargo run -p teams-core --example demo
use teams_core::TeamsStore;

fn main() {
    let store = TeamsStore::open(None).expect("open");
    let chats = store.list_chats().expect("list");
    println!("=== 대화 {}개 (최근순 상위 12) ===", chats.len());
    for chat in chats.iter().take(12) {
        println!(
            "[{}] {} · {}건 · 마지막 {}",
            chat.last_message_time, chat.topic, chat.message_count, chat.conversation_id
        );
    }

    // topic 부분일치로 읽기
    println!("\n=== 'TechTalk' 부분일치 최근 3개 ===");
    for m in store.read_messages("TechTalk", 3, None).unwrap() {
        let c: String = m.content.chars().take(70).collect();
        println!("{} | {}: {}", m.time, m.sender, c);
    }
    // 검색
    println!("\n=== 검색 'ArgoCD' 최근 3개 ===");
    for m in store.search("ArgoCD", 3).unwrap() {
        let c: String = m.content.chars().take(70).collect();
        println!("{} | {}: {}", m.time, m.sender, c);
    }

    if let Some(top) = chats.first() {
        println!("\n=== '{}' 최근 5개 ===", top.topic);
        let msgs = store.read_messages(&top.conversation_id, 5, None).unwrap();
        for m in &msgs {
            let c: String = m.content.chars().take(80).collect();
            println!("{} | {}: {}", m.time, m.sender, c);
        }
    }
}
