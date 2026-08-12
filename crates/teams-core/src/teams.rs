//! Teams 스키마 매핑 + 조회 API (`TeamsStore`).
//!
//! - 대화 목록: `conversation-manager` DB 의 `conversations` store 에 있는
//!   `OneGQL_Conversation` 레코드
//!   (`id`=conversationId, `threadProperties.topic` ?? `chatTitle.longTitle/shortTitle`)
//! - 메시지: `replychain-manager` DB 의 `replychains`/`replychains-2` store 에 있는
//!   replychain 레코드의 `messageMap{ msgId -> 메시지객체 }`
//!
//! IndexedDB 의 database/object-store id 는 프로파일마다 생성 순서대로 동적 할당되므로
//! 숫자 id 를 하드코딩하면 다른 환경에서 안 맞는다. 여기서는 leveldb 메타데이터를 읽어
//! **이름으로 id 를 런타임에 해석**한다.
//!
//! 값은 leveldb 전수 스캔 후 인메모리로 인덱싱하고, TTL 로 갱신한다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::idb::{self, KeyPrefix};
use crate::leveldb;
use crate::util::{format_epoch_ms, html_to_text};
use crate::v8::V8Reader;

/// 대화 DB 를 식별하는 database 이름 세그먼트(`Teams:conversation-manager:...`).
const CONV_MANAGER: &str = "conversation-manager";
/// 메시지 DB 를 식별하는 database 이름 세그먼트(`Teams:replychain-manager:...`).
const MSG_MANAGER: &str = "replychain-manager";
/// 사람 프로필 DB 를 식별하는 database 이름 세그먼트(`Teams:profiles:...`).
const PROFILE_MANAGER: &str = "profiles";
/// 대화 레코드가 담긴 object store 이름.
const CONV_STORE_NAME: &str = "conversations";
/// 메시지(replychain) 레코드가 담긴 object store 이름들.
const MSG_STORE_NAMES: [&str; 2] = ["replychains", "replychains-2"];
/// 사람 프로필 레코드가 담긴 object store 이름.
const PROFILE_STORE_NAME: &str = "profiles";

/// database 이름 목록에서 콜론 구분 세그먼트가 `manager` 와 정확히 일치하는 db id 를 찾는다.
/// (예: `conversation-manager` 는 `conversation-folder-manager` 와 구분됨)
fn find_db_by_manager(databases: &[(String, u64)], manager: &str) -> Option<u64> {
    databases
        .iter()
        .find(|(name, _)| name.split(':').any(|seg| seg == manager))
        .map(|(_, id)| *id)
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub conversation_id: String,
    pub sender: String,
    pub content: String,
    /// 손대지 않은 원본 content HTML. 평문화(`content`)로 사라지는 링크 URL·목록 구조·
    /// 줄바꿈·코드블록·표를 그대로 보려는 용도. 원본이 HTML 이 아니면(평문 `Text` 메시지)
    /// `content` 와 같은 값이라 담지 않는다.
    /// 캐시에는 항상 들고 있고, `read_messages(raw=true)` 로 요청했을 때만 응답에 실린다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_html: Option<String>,
    pub time: String, // "YYYY-MM-DD HH:MM:SS" UTC
    pub time_ms: i64,
    pub message_type: String,
    pub id: String,
    /// 답장 메시지인 경우 인용 대상 메시지의 id (인용 본문은 content 에서 제거됨). 없으면 생략.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// 본문에 포함된 이미지(AMS) 참조. content 평문화 시 사라지므로 별도로 보존한다.
    /// 실제 바이트는 원격이라 여기엔 URL·크기만 담고, 조회는 `fetch_image` 도구로 한다. 없으면 생략.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    /// 이 메시지에 달린 리액션. 없으면 생략.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
}

impl Message {
    /// 원본 HTML 을 뺀 사본. raw 를 요청하지 않은 모든 응답 경로가 이걸 쓴다.
    fn without_html(&self) -> Message {
        let mut m = self.clone();
        m.content_html = None;
        m
    }
}

/// 메시지에 달린 리액션 한 종류와 그걸 누른 사람들.
#[derive(Debug, Clone, Serialize)]
pub struct Reaction {
    /// 리액션 종류. 유니코드 이모지는 실제 문자(🎉), 커스텀 이모지는 이름(`nepp`).
    pub key: String,
    /// 누른 사람. 표시이름으로 해석되며, 캐시에 프로필도 발언 기록도 없는 사람은
    /// 원본 mri(`8:orgid:<UUID>`)가 그대로 남는다.
    pub users: Vec<String>,
}

/// 메시지 본문의 AMS(Async Media Service) 이미지 참조. `url` 로 `fetch_image` 호출.
#[derive(Debug, Clone, Serialize)]
pub struct ImageRef {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
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

        // 1) 메타데이터에서 이름 → id 해석 (환경마다 id 가 다르므로 하드코딩 불가).
        let mut databases: Vec<(String, u64)> = Vec::new();
        let mut store_names: HashMap<(u64, u64), String> = HashMap::new();
        for (key, value) in &records {
            if let Some((name, id)) = idb::parse_database_name(key, value) {
                databases.push((name, id));
            } else if let Some((db, store, name)) = idb::parse_object_store_name(key, value) {
                store_names.insert((db, store), name);
            }
        }

        let conv_db = find_db_by_manager(&databases, CONV_MANAGER);
        let msg_db = find_db_by_manager(&databases, MSG_MANAGER);
        let profile_db = find_db_by_manager(&databases, PROFILE_MANAGER);

        // 이름으로 대상 (db, store) 셀 확정.
        let conv_cell: Option<(u64, u64)> = conv_db.and_then(|db| {
            store_names
                .iter()
                .find(|((d, _), name)| *d == db && name.as_str() == CONV_STORE_NAME)
                .map(|((d, s), _)| (*d, *s))
        });
        let msg_cells: Vec<(u64, u64)> = match msg_db {
            Some(db) => store_names
                .iter()
                .filter(|((d, _), name)| *d == db && MSG_STORE_NAMES.contains(&name.as_str()))
                .map(|((d, s), _)| (*d, *s))
                .collect(),
            None => Vec::new(),
        };
        let profile_cell: Option<(u64, u64)> = profile_db.and_then(|db| {
            store_names
                .iter()
                .find(|((d, _), name)| *d == db && name.as_str() == PROFILE_STORE_NAME)
                .map(|((d, s), _)| (*d, *s))
        });

        let mut messages_by_conv: HashMap<String, Vec<Message>> = HashMap::new();
        let mut topic_by_conv: HashMap<String, String> = HashMap::new();
        // 리액션은 사람을 mri 로만 가리키므로 표시이름 대응표가 필요하다.
        let mut name_by_mri: HashMap<String, String> = HashMap::new();

        // 2) object-store-data 레코드를 대상 셀에 한해 디코드.
        for (key, value) in &records {
            let Some(kp) = KeyPrefix::parse(key) else {
                continue;
            };
            if !kp.is_object_store_data() {
                continue;
            }
            let cell = (kp.database_id, kp.object_store_id);
            if msg_cells.contains(&cell) {
                if let Some(v) = V8Reader::decode(value) {
                    collect_messages(&v, &mut messages_by_conv, &mut name_by_mri);
                }
            } else if conv_cell == Some(cell) {
                if let Some(v) = V8Reader::decode(value) {
                    if let Some((cid, topic)) = extract_topic(&v) {
                        topic_by_conv.insert(cid, topic);
                    }
                }
            } else if profile_cell == Some(cell) {
                if let Some(v) = V8Reader::decode(value) {
                    if let (Some(mri), Some(name)) = (
                        v.get("mri").and_then(Value::as_str),
                        v.get("displayName").and_then(Value::as_str),
                    ) {
                        // 프로필이 정본이므로 발신자 역매핑보다 우선한다.
                        name_by_mri.insert(mri.to_string(), name.to_string());
                    }
                }
            }
        }

        // 3) 리액션의 mri 를 표시이름으로 치환 (대응표가 다 모인 뒤에 한 번에).
        for msgs in messages_by_conv.values_mut() {
            for m in msgs {
                for r in &mut m.reactions {
                    for u in &mut r.users {
                        if let Some(name) = name_by_mri.get(u.as_str()) {
                            *u = name.clone();
                        }
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
                    let last = msgs.last().map(Message::without_html);
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
    /// `raw` 면 각 메시지에 원본 HTML(`content_html`)도 함께 실린다.
    pub fn read_messages(
        &self,
        chat: &str,
        limit: usize,
        before_ms: Option<i64>,
        raw: bool,
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
            filtered[start..]
                .iter()
                .map(|m| if raw { (*m).clone() } else { m.without_html() })
                .collect()
        })
    }

    /// 캐시 전역 키워드 검색(대소문자 무시, content/sender 대상). 최근 `limit` 개.
    /// 여러 대화에 흩어진 결과라 원본 HTML 은 싣지 않는다(필요하면 read_messages 로).
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
            hits[start..].iter().map(|m| m.without_html()).collect()
        })
    }
}

/// replychain 레코드에서 messageMap 을 순회해 메시지를 수집.
/// 겸사겸사 발신자(`creator` mri → `imDisplayName`)를 모아 리액션 이름 해석에 쓴다 —
/// 프로필 store 에 없는 사람도 이쪽에 있으면 이름이 붙는다.
fn collect_messages(
    record: &Value,
    out: &mut HashMap<String, Vec<Message>>,
    name_by_mri: &mut HashMap<String, String>,
) {
    let Some(map) = record.get("messageMap").and_then(Value::as_object) else {
        return;
    };
    let conv_fallback = record
        .get("conversationId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    for msg in map.values() {
        if let (Some(mri), Some(name)) = (
            msg.get("creator").and_then(Value::as_str),
            msg.get("imDisplayName")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty()),
        ) {
            name_by_mri
                .entry(mri.to_string())
                .or_insert_with(|| name.to_string());
        }
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
    // raw 조회용 원본은 아무 가공도 하기 전(답장 인용 포함) 상태로 보관한다.
    let content_html = raw_content.contains('<').then(|| raw_content.clone());
    // 답장 인용(blockquote) 제거 + 대상 id 추출
    let (raw_content, reply_to) = strip_reply_quote(&raw_content);
    // 평문화 전에 이미지 참조를 뽑아 둔다 (html_to_text 가 <img> 를 통째로 버리기 때문).
    let images = extract_images(&raw_content);
    let mut content = if raw_content.contains('<') {
        html_to_text(&raw_content)
    } else {
        raw_content
    };
    // 이미지만 있던 메시지는 평문이 비므로 마커를 남긴다 (검색·문맥 보존).
    if content.is_empty() && !images.is_empty() {
        content = "[이미지]".to_string();
    }
    let time_ms = num_field(obj, "originalArrivalTime")
        .or_else(|| str_field(obj, "id").and_then(|s| s.parse::<i64>().ok()))
        .or_else(|| num_field(obj, "clientArrivalTime"))
        .unwrap_or(0);
    let id = str_field(obj, "id").unwrap_or_default();
    let reactions = extract_reactions(obj.get("properties"));

    Some(Message {
        conversation_id,
        sender,
        content,
        content_html,
        time: format_epoch_ms(time_ms),
        time_ms,
        message_type,
        id,
        reply_to,
        images,
        reactions,
    })
}

/// `properties.emotions` 에서 리액션을 뽑는다. 원본 형태는
/// `[{ key, users: [{ mri, time, value }] }]` 이며, 여기서는 key 를 사람이 읽을 수 있게
/// 정규화하고 users 는 mri 만 남긴다(표시이름 치환은 대응표가 모인 뒤 build_cache 에서).
/// `deltaEmotions` 는 같은 내용의 증분이라 쓰지 않는다.
fn extract_reactions(properties: Option<&Value>) -> Vec<Reaction> {
    let Some(list) = properties
        .and_then(|p| p.get("emotions"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let key = e.get("key").and_then(Value::as_str)?;
            let users: Vec<String> = e
                .get("users")
                .and_then(Value::as_array)
                .map(|us| {
                    us.iter()
                        .filter_map(|u| u.get("mri").and_then(Value::as_str))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            if users.is_empty() {
                return None; // 아무도 안 누른 종류는 버린다(취소된 리액션의 잔재)
            }
            Some(Reaction {
                key: normalize_reaction_key(key),
                users,
            })
        })
        .collect()
}

/// 리액션 key 정규화.
/// - 커스텀(테넌트) 이모지는 `nepp;0-ckr-d1-<해시>` 형태라 `;` 앞의 이름만 남긴다.
/// - 유니코드 이모지는 `1f389_partypopper` 처럼 코드포인트가 앞에 붙어 있어 실제 문자(🎉)로 바꾼다.
/// - `like`, `heart` 처럼 이름뿐인 것은 그대로 둔다.
fn normalize_reaction_key(key: &str) -> String {
    let base = key.split(';').next().unwrap_or(key);
    if let Some((hex, _)) = base.split_once('_') {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                return ch.to_string();
            }
        }
    }
    base.to_string()
}

/// content HTML 에서 AMS 이미지(`<img itemtype=".../AMSImage" src=... style="width:..;height:..">`)를
/// 추출한다. 이모지 등 다른 인라인 이미지는 제외하고, 공유된 이미지/스크린샷(AMS)만 대상으로 한다.
fn extract_images(html: &str) -> Vec<ImageRef> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("<img") {
        rest = &rest[pos..];
        let tag_end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        rest = &rest[tag_end..];
        // AMS 이미지만 (이모지/스티커 배제). itemtype 또는 src 로 판별.
        let is_ams = tag.contains("schema.skype.com/AMSImage")
            || extract_attr(tag, "src").is_some_and(|s| s.contains("asyncgw"));
        if !is_ams {
            continue;
        }
        let Some(url) = extract_attr(tag, "src").filter(|s| s.starts_with("http")) else {
            continue;
        };
        let (width, height) = extract_img_dims(tag);
        out.push(ImageRef { url, width, height });
    }
    out
}

/// `<img>` 여는 태그에서 크기를 추출. `style="width:NNNpx;height:NNNpx"` 우선, 없으면 width/height 속성.
fn extract_img_dims(tag: &str) -> (Option<u32>, Option<u32>) {
    let parse_px = |s: &str| -> Option<u32> {
        s.trim()
            .trim_end_matches("px")
            .trim()
            .parse::<f64>()
            .ok()
            .map(|f| f as u32)
    };
    if let Some(style) = extract_attr(tag, "style") {
        let mut w = None;
        let mut h = None;
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                match k.trim() {
                    "width" => w = parse_px(v),
                    "height" => h = parse_px(v),
                    _ => {}
                }
            }
        }
        if w.is_some() || h.is_some() {
            return (w, h);
        }
    }
    (
        extract_attr(tag, "width").and_then(|s| parse_px(&s)),
        extract_attr(tag, "height").and_then(|s| parse_px(&s)),
    )
}

/// content HTML 에서 답장 인용(`<blockquote itemtype=".../Reply" itemid=ID>...`)을 통째로
/// 제거하고 인용 대상 메시지 id 를 반환한다. 인용은 앞선 메시지의 중복 복제이므로 버린다.
/// Reply 가 아닌 일반 blockquote 는 보존한다.
fn strip_reply_quote(html: &str) -> (String, Option<String>) {
    if !html.contains("<blockquote") {
        return (html.to_string(), None);
    }
    let mut result = String::with_capacity(html.len());
    let mut reply_to: Option<String> = None;
    let mut rest = html;
    loop {
        let Some(pos) = rest.find("<blockquote") else {
            result.push_str(rest);
            break;
        };
        let Some(end_rel) = rest[pos..].find("</blockquote>") else {
            result.push_str(rest);
            break;
        };
        let end = pos + end_rel + "</blockquote>".len();
        let block = &rest[pos..end];
        result.push_str(&rest[..pos]); // blockquote 앞부분
        if block.contains("schema.skype.com/Reply") {
            if reply_to.is_none() {
                reply_to = extract_attr(block, "itemid");
            }
            // 인용 블록은 버림
        } else {
            result.push_str(block); // 일반 blockquote 는 유지
        }
        rest = &rest[end..];
    }
    (result, reply_to)
}

/// 여는 태그(첫 `>` 이전)에서 `attr="value"` 의 value 를 추출.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let open_end = tag.find('>').unwrap_or(tag.len());
    let opening = &tag[..open_end];
    let needle = format!("{attr}=\"");
    let start = opening.find(&needle)? + needle.len();
    let val_end = opening[start..].find('"')?;
    Some(opening[start..start + val_end].to_string())
}

/// `conversations` store 레코드에서 (conversationId, topic) 추출. OneGQL_Conversation 만.
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
