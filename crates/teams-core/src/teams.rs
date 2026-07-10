//! Teams 스키마 매핑 + 조회 API (`TeamsStore`).
//!
//! - 대화 목록: db31/store1 의 `OneGQL_Conversation` 레코드
//!   (`id`=conversationId, `threadProperties.topic` ?? `chatTitle.longTitle/shortTitle`)
//! - 메시지: db44/store1 replychain 레코드의 `messageMap{ msgId -> 메시지객체 }`
//!
//! 값은 leveldb 전수 스캔 후 인메모리로 인덱싱하고, TTL 로 갱신한다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::idb::KeyPrefix;
use crate::leveldb;
use crate::util::{format_epoch_ms, html_to_text};
use crate::v8::V8Reader;

const CONV_DB: u64 = 31;
const CONV_STORE: u64 = 1;
const MSG_DB: u64 = 44;
const MSG_STORE: u64 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub conversation_id: String,
    pub sender: String,
    pub content: String,
    pub time: String, // "YYYY-MM-DD HH:MM:SS" UTC
    pub time_ms: i64,
    pub message_type: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub conversation_id: String,
    pub topic: String,
    pub message_count: usize,
    pub last_message_time: String,
    pub last_message: Option<Message>,
}

pub struct TeamsStore {
    db_path: PathBuf,
    ttl: Duration,
    cache: Mutex<Option<Cache>>,
}

struct Cache {
    built_at: Instant,
    messages_by_conv: HashMap<String, Vec<Message>>,
    topic_by_conv: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Teams IndexedDB 경로를 찾을 수 없음 (TEAMS_MCP_DB 로 지정 가능)")]
    NoDbPath,
    #[error("DB 디렉토리 없음: {0}")]
    DbMissing(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl TeamsStore {
    /// db_path 미지정 시 기본 macOS 경로.
    pub fn open(db_path: Option<PathBuf>) -> Result<Self, StoreError> {
        let db_path = db_path
            .or_else(crate::location::default_db_path)
            .ok_or(StoreError::NoDbPath)?;
        if !db_path.exists() {
            return Err(StoreError::DbMissing(db_path));
        }
        Ok(TeamsStore {
            db_path,
            ttl: Duration::from_secs(15),
            cache: Mutex::new(None),
        })
    }

    /// 캐시를 강제로 다시 빌드.
    pub fn refresh(&self) -> Result<(), StoreError> {
        let fresh = self.build_cache()?;
        *self.cache.lock().unwrap() = Some(fresh);
        Ok(())
    }

    fn with_cache<T>(&self, f: impl FnOnce(&Cache) -> T) -> Result<T, StoreError> {
        let mut guard = self.cache.lock().unwrap();
        let stale = match guard.as_ref() {
            Some(c) => c.built_at.elapsed() > self.ttl,
            None => true,
        };
        if stale {
            *guard = Some(self.build_cache()?);
        }
        Ok(f(guard.as_ref().unwrap()))
    }

    fn build_cache(&self) -> Result<Cache, StoreError> {
        let records = leveldb::read_dir(&self.db_path)?;

        let mut messages_by_conv: HashMap<String, Vec<Message>> = HashMap::new();
        let mut topic_by_conv: HashMap<String, String> = HashMap::new();

        for (key, value) in &records {
            let Some(kp) = KeyPrefix::parse(key) else {
                continue;
            };
            if !kp.is_object_store_data() {
                continue;
            }
            if kp.database_id == MSG_DB && kp.object_store_id == MSG_STORE {
                if let Some(v) = V8Reader::decode(value) {
                    collect_messages(&v, &mut messages_by_conv);
                }
            } else if kp.database_id == CONV_DB && kp.object_store_id == CONV_STORE {
                if let Some(v) = V8Reader::decode(value) {
                    if let Some((cid, topic)) = extract_topic(&v) {
                        topic_by_conv.insert(cid, topic);
                    }
                }
            }
        }

        // 대화별 메시지 시간 오름차순 정렬
        for msgs in messages_by_conv.values_mut() {
            msgs.sort_by_key(|m| m.time_ms);
        }

        Ok(Cache {
            built_at: Instant::now(),
            messages_by_conv,
            topic_by_conv,
        })
    }

    /// 대화 목록. 메시지가 있는 대화만. 최근 메시지 순 내림차순.
    pub fn list_chats(&self) -> Result<Vec<Chat>, StoreError> {
        self.with_cache(|c| {
            let mut chats: Vec<Chat> = c
                .messages_by_conv
                .iter()
                .map(|(cid, msgs)| {
                    let last = msgs.last().cloned();
                    let topic = c
                        .topic_by_conv
                        .get(cid)
                        .cloned()
                        .unwrap_or_else(|| cid.clone());
                    Chat {
                        conversation_id: cid.clone(),
                        topic,
                        message_count: msgs.len(),
                        last_message_time: last
                            .as_ref()
                            .map(|m| m.time.clone())
                            .unwrap_or_default(),
                        last_message: last,
                    }
                })
                .collect();
            chats.sort_by(|a, b| {
                b.last_message
                    .as_ref()
                    .map(|m| m.time_ms)
                    .unwrap_or(0)
                    .cmp(&a.last_message.as_ref().map(|m| m.time_ms).unwrap_or(0))
            });
            chats
        })
    }

    /// 특정 대화의 메시지. `chat` 은 conversationId 정확일치 또는 topic 부분일치(대소문자 무시).
    /// 최근 `limit` 개, `before_ms` 지정 시 그 이전만. 시간 오름차순 반환.
    pub fn read_messages(
        &self,
        chat: &str,
        limit: usize,
        before_ms: Option<i64>,
    ) -> Result<Vec<Message>, StoreError> {
        self.with_cache(|c| {
            let Some(cid) = resolve_conversation(c, chat) else {
                return Vec::new();
            };
            let Some(msgs) = c.messages_by_conv.get(&cid) else {
                return Vec::new();
            };
            let filtered: Vec<&Message> = msgs
                .iter()
                .filter(|m| before_ms.map_or(true, |b| m.time_ms < b))
                .collect();
            let start = filtered.len().saturating_sub(limit);
            filtered[start..].iter().map(|m| (*m).clone()).collect()
        })
    }

    /// 캐시 전역 키워드 검색(대소문자 무시, content/sender 대상). 최근 `limit` 개.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>, StoreError> {
        let q = query.to_lowercase();
        self.with_cache(|c| {
            let mut hits: Vec<&Message> = c
                .messages_by_conv
                .values()
                .flatten()
                .filter(|m| {
                    m.content.to_lowercase().contains(&q) || m.sender.to_lowercase().contains(&q)
                })
                .collect();
            hits.sort_by_key(|m| m.time_ms);
            let start = hits.len().saturating_sub(limit);
            hits[start..].iter().map(|m| (*m).clone()).collect()
        })
    }
}

/// replychain 레코드에서 messageMap 을 순회해 메시지를 수집.
fn collect_messages(record: &Value, out: &mut HashMap<String, Vec<Message>>) {
    let Some(map) = record.get("messageMap").and_then(Value::as_object) else {
        return;
    };
    let conv_fallback = record
        .get("conversationId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    for msg in map.values() {
        if let Some(m) = parse_message(msg, &conv_fallback) {
            out.entry(m.conversation_id.clone()).or_default().push(m);
        }
    }
}

fn parse_message(v: &Value, conv_fallback: &str) -> Option<Message> {
    let obj = v.as_object()?;
    let conversation_id = str_field(obj, "conversationId")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| conv_fallback.to_string());
    if conversation_id.is_empty() {
        return None;
    }
    let sender = str_field(obj, "imDisplayName")
        .filter(|s| !s.is_empty())
        .or_else(|| str_field(obj, "fromDisplayNameInToken"))
        .unwrap_or_default();
    let message_type = str_field(obj, "messageType").unwrap_or_default();
    let raw_content = str_field(obj, "content").unwrap_or_default();
    let content = if raw_content.contains('<') {
        html_to_text(&raw_content)
    } else {
        raw_content
    };
    let time_ms = num_field(obj, "originalArrivalTime")
        .or_else(|| str_field(obj, "id").and_then(|s| s.parse::<i64>().ok()))
        .or_else(|| num_field(obj, "clientArrivalTime"))
        .unwrap_or(0);
    let id = str_field(obj, "id").unwrap_or_default();

    Some(Message {
        conversation_id,
        sender,
        content,
        time: format_epoch_ms(time_ms),
        time_ms,
        message_type,
        id,
    })
}

/// db31 대화 레코드에서 (conversationId, topic) 추출. OneGQL_Conversation 만.
fn extract_topic(v: &Value) -> Option<(String, String)> {
    let obj = v.as_object()?;
    let cid = str_field(obj, "id")?;
    if !cid.starts_with("19:") {
        return None; // 실제 대화(thread)만
    }
    // 1) threadProperties.topic (명명된 그룹챗)
    let topic = obj
        .get("threadProperties")
        .and_then(|t| t.get("topic"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        // 2) chatTitle.longTitle / shortTitle
        .or_else(|| {
            obj.get("chatTitle").and_then(|ct| {
                ct.get("longTitle")
                    .and_then(Value::as_str)
                    .or_else(|| ct.get("shortTitle").and_then(Value::as_str))
                    .map(String::from)
            })
        })
        .unwrap_or_else(|| cid.clone());
    Some((cid, topic))
}

/// chat 인자를 conversationId 로 해석.
fn resolve_conversation(c: &Cache, chat: &str) -> Option<String> {
    // 1) conversationId 정확 일치
    if c.messages_by_conv.contains_key(chat) {
        return Some(chat.to_string());
    }
    // 2) topic 부분 일치(대소문자 무시) 중 메시지 있는 것 → 가장 최근 대화
    let needle = chat.to_lowercase();
    let mut candidates: Vec<(&String, i64)> = c
        .topic_by_conv
        .iter()
        .filter(|(cid, topic)| {
            topic.to_lowercase().contains(&needle) && c.messages_by_conv.contains_key(*cid)
        })
        .map(|(cid, _)| {
            let last = c
                .messages_by_conv
                .get(cid)
                .and_then(|m| m.last())
                .map(|m| m.time_ms)
                .unwrap_or(0);
            (cid, last)
        })
        .collect();
    candidates.sort_by_key(|(_, t)| -*t);
    candidates.first().map(|(cid, _)| (*cid).clone())
}

fn str_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(String::from)
}

fn num_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<i64> {
    obj.get(key).and_then(Value::as_f64).map(|f| f as i64)
}
