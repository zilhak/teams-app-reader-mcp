# 구현 계획 (Rust)

teams-mcp를 **로직 core 크레이트 + stdio 빌드 + HTTP 빌드**로 나눠 구현하는 설계. stdio/HTTP는 전송(transport)만 다르고 실제 동작은 core를 공유한다.

---

## 1. 워크스페이스 구조 (Cargo workspace)

```
teams-mcp/                      # 워크스페이스 루트
├── Cargo.toml                  # [workspace] members
├── crates/
│   ├── teams-core/             # ★ 실제 동작 (transport 무관, MCP 무관)
│   │   └── src/                #   LevelDB 읽기 + V8 디코드 + Teams 스키마 + 조회 API
│   ├── teams-mcp/              # rmcp ServerHandler + 도구 정의 (transport 무관). teams-core 호출
│   │   └── src/lib.rs          #   TeamsServer(도구: list_chats/read_messages/search) 노출
│   ├── teams-mcp-stdio/        # 바이너리: TeamsServer를 stdio 전송에 연결
│   │   └── src/main.rs
│   └── teams-mcp-http/         # 바이너리: TeamsServer를 HTTP(streamable) 전송에 연결
│       └── src/main.rs
```

**핵심 원칙**: MCP 도구 구현(`teams-mcp`)과 데이터 로직(`teams-core`)은 전송을 전혀 모른다. 바이너리 두 개는 **같은 `TeamsServer`를 서로 다른 rmcp 전송에 꽂기만** 한다.

### 분리 빌드
```bash
cargo build -p teams-mcp-stdio --release   # → target/release/teams-mcp-stdio
cargo build -p teams-mcp-http  --release   # → target/release/teams-mcp-http
```
- 각 바이너리는 필요한 rmcp feature만 켠다 (stdio: `transport-io`, http: `transport-streamable-http-server`) → HTTP용 axum 등이 stdio 바이너리에 안 딸려감.
- (대안) 단일 바이너리 + cargo feature `stdio`/`http` + 런타임 `--transport` 플래그도 가능하나, **"따로 빌드" 요구에는 바이너리 2개 분리가 더 명확**해서 이걸 택함.

---

## 2. teams-core (실제 동작) 모듈 설계

전부 **동기(sync)** 로 구현 (LevelDB 파싱은 CPU/IO 블로킹). MCP 쪽에서 `tokio::task::spawn_blocking`으로 감싸 호출.

```
teams-core/src/
├── lib.rs          # 공개 API (아래 §4)
├── location.rs     # IndexedDB leveldb 경로 탐색 (기본 macOS 경로, env override)
├── snapshot.rs     # 락 회피: leveldb 디렉토리를 temp로 복사 후 LOCK 삭제 (또는 .ldb 직접 읽기)
├── leveldb/
│   ├── mod.rs      # 디렉토리 전체(.ldb + .log) 순회 → (key, value) 원시 쌍 스트림
│   ├── sstable.rs  # .ldb SSTable 파서 (footer→index→data block, restart prefix, 블록 snappy 해제)
│   └── log.rs      # .log write-ahead log 파서 (32KB 블록, crc/len/type 레코드, 조각 재조립→batch)
├── snappy.rs       # snappy 블록 해제 (crate `snap`)
├── varint.rs       # LEB128 varint / zigzag
├── v8.rs           # ★ V8 ValueSerializer 디코더 (PoC 60줄 이식, docs/teams-storage-structure.md §4)
├── idb.rs          # IndexedDB 레이어: 키 파싱(database_id/object_store_id/키타입), 값에서 blink 봉투 스킵→v8
└── teams.rs        # Teams 스키마 매핑: 디코드 객체 → Chat / Message 도메인 타입
```

### 왜 이렇게?
- **LevelDB는 SSTable 직접 파싱**으로 간다 (idb_cmp1 comparator 우회). 우리는 전수 스캔만 하므로 정렬이 불필요 → comparator 구현 회피. `rusty-leveldb`는 idb_cmp1에서 패닉하므로 안 씀. (자세한 근거: teams-storage-structure.md §2)
- 값 디코드는 **직접 짠 V8 파서**. dfindexeddb(파이썬)는 blink 봉투에서 실패하므로 라이브러리 의존 대신 이식. (§4 참조구현 존재, 검증됨)

### 의존 크레이트 후보
`snap`(snappy), `crc`(log 레코드 검증), `serde`/`serde_json`(값 표현), `thiserror`(에러), `tempfile`(snapshot). SSTable/log 파서·varint·v8은 직접 구현.

---

## 3. 캐시 전략 (성능)

leveldb 전체는 ~17MB·수만 레코드. **매 도구 호출마다 전체 파싱은 비쌈.**

- `teams-core`에 **인메모리 인덱스** 유지: 첫 조회 시 snapshot→파싱→`{threadid → Vec<Message>, topic맵}` 구성.
- **TTL 기반 갱신** (예: 30초) 또는 명시적 `refresh()`. Teams가 계속 쓰므로 최신성 필요 시 재-snapshot.
- MCP 서버가 이 인덱스를 `Arc<RwLock<Cache>>`로 들고, 도구는 여기서 읽음.

---

## 4. teams-core 공개 API (도메인)

```rust
pub struct Chat { pub thread_id: String, pub topic: Option<String>,
                  pub last_message: Option<Message>, pub cached_from: Option<i64>, pub cached_count: usize }
pub struct Message { pub thread_id: String, pub sender: Option<String>,  // imdisplayname
                     pub content: String, pub time_ms: i64, pub message_type: String, pub id: String }

pub struct TeamsStore { /* Arc<RwLock<Cache>>, db path */ }
impl TeamsStore {
    pub fn open(db_path: Option<PathBuf>) -> Result<Self>;
    pub fn list_chats(&self) -> Result<Vec<Chat>>;
    pub fn read_messages(&self, chat: &str, limit: usize, before_ms: Option<i64>) -> Result<Vec<Message>>;
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>>;
    pub fn refresh(&self) -> Result<()>;
}
```
`chat`은 topic 이름 또는 thread_id 둘 다 받게.

---

## 5. teams-mcp (도구 정의, 전송 무관)

rmcp `#[tool_router]`/`#[tool]`로 `TeamsStore`를 감싼다. (읽기 전용, 전송 무관.)

- `list_chats()` → 채팅 목록(topic, 마지막 메시지, 캐시 범위)
- `read_messages(chat, limit?, before?)` → 특정 채팅 메시지
- `search_messages(query, limit?)` → 캐시 전역 키워드 검색
- (전송 안 함 — 읽기 전용)

```rust
// teams-mcp/src/lib.rs
#[derive(Clone)]
pub struct TeamsServer { store: Arc<TeamsStore>, tool_router: ToolRouter<Self> }
#[tool_router] impl TeamsServer { /* 도구들: spawn_blocking(store.read_messages …) */ }
#[tool_handler] impl ServerHandler for TeamsServer { /* get_info: 읽기전용 명시 */ }
impl TeamsServer { pub fn new() -> Result<Self> { … } }
```

---

## 6. 전송 바이너리

### stdio (`teams-mcp-stdio/src/main.rs`)
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = TeamsServer::new()?.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```
rmcp feature: `server`, `transport-io`, `macros`.

### HTTP (`teams-mcp-http/src/main.rs`)
rmcp의 **Streamable HTTP 서버 전송**(axum 기반, `StreamableHttpService`)에 같은 `TeamsServer`를 꽂는다. 대략:
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = StreamableHttpService::new(
        || Ok(TeamsServer::new()?),
        Default::default(), // 세션 매니저 등
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8787").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```
rmcp feature: `server`, `transport-streamable-http-server`, `macros` (+ axum).
> 정확한 rmcp HTTP API(타입/피처명)는 구현 시점 rmcp 버전에서 확인 필요. Streamable HTTP가 현행 MCP 표준. SSE는 레거시.

### ⚠️ HTTP 보안 (중요)
HTTP 전송은 **남의 Teams 메시지(민감 데이터)를 네트워크로 노출**한다. 반드시:
- **`127.0.0.1`(localhost) 바인드 기본** (외부 노출 금지).
- 가능하면 **Bearer 토큰 인증** (환경변수 시크릿) 추가.
- CORS 차단, 로깅 최소화.

---

## 7. 남은 조사/리스크 (구현 중 해결)

- **개별 메시지 본문 store 확정** — db44의 `ObjectStoreDataKey` 구조/키. 현재 인덱스(IndexDataKey)만 확인됨. `teams.rs` 매핑의 최대 미지수. (dfindexeddb 크래시로 미추출 → SSTable 직접 파싱으로 전수 확보하면 드러날 것)
- **여러 IndexedDB db** 중 무엇이 무엇인지 (db0=대화, db44=메시지인덱스 확인. 본문 db 확정 필요).
- **.log 파싱** 정확도 (최신 메시지가 여기 있음).
- **락/최신성**: .ldb는 immutable이라 직접 읽기 가능, .log는 복사 후.
- macOS 전용 경로 (다른 OS는 미지원 명시).

---

## 8. 구현 순서 (마일스톤)

1. **M1 — teams-core 읽기 골격**: snapshot + SSTable 파서 + snappy + varint → 원시 (key,value) 전수 스캔. `.log`는 후순위.
2. **M2 — V8 디코더**: v8.rs 이식, db0/store0 대화 레코드 완전 디코드(테스트: topic·lastMessage 복원).
3. **M3 — 메시지 본문 store 확정 + teams.rs 매핑**: Chat/Message 도메인 완성. (M1의 전수 스캔 데이터로 본문 store 규명)
4. **M4 — TeamsStore API + 캐시**: list_chats/read_messages/search + 인메모리 인덱스 + TTL.
5. **M5 — teams-mcp 도구**: rmcp ServerHandler + 3개 도구, spawn_blocking 연결.
6. **M6 — stdio 바이너리** + 실제 MCP 클라이언트로 검증.
7. **M7 — HTTP 바이너리** + localhost 바인드 + (선택)토큰 인증.
8. **M8 — 정리**: 초기 a11y 스캐폴드(`src/ax.rs` 등) 제거, README 갱신.

각 마일스톤은 이전 것 위에서 독립 검증 가능. M1~M3이 진짜 어려운 부분(포맷 파싱), M4 이후는 조립.
```
