# teams-mcp

macOS에서 실행 중인 **Microsoft Teams(신 Teams 2, Edge WebView2 기반)** 의 로컬 데이터를 읽는 **읽기 전용 MCP 서버** (Rust).

## 왜 이 방식인가 (요약)

- 테넌트 관리자 동의(admin consent)를 받을 수 없는 환경이라 **정식 Microsoft Graph API 경로가 막힘.** 개인계정도 Teams 채팅 API 불가.
- **macOS 접근성(a11y) 방식은 폐기됨** — Teams v2는 WebView2라 접근성 트리에 메시지 텍스트가 안 실리고, 강제로 켤 방법이 없음.
- **확정 방식: Teams가 로컬 IndexedDB(LevelDB)에 캐싱해둔 메시지를 직접 읽는다.** 토큰·봇·앱등록·네트워크 전부 불필요. 순수 로컬 파일 읽기라 네트워크 요청이 전혀 없음.
- **쓰기(전송)는 의도적으로 미지원.** 읽기 전용.

전체 의사결정 과정과 시행착오는 `docs/journey-and-troubleshooting.md` 참고.

## 현재 스코프

- ✅ **로컬 캐시에 이미 있는 메시지 읽기** (Teams가 자동으로 캐싱한 수천 건. 채팅별로 수개월치 히스토리).
- ❌ **스크롤로 캐시에 없는 과거를 새로 수집하는 기능은 드롭됨.** (이유·계획·재개 방법은 `docs/scroll-collection/` 참고)

## docs/ 폴더 안내

`docs/`는 이 프로젝트의 모든 설계·조사 문서를 모아두는 곳이다.

- **`docs/teams-storage-structure.md`** — Teams가 메시지를 로컬에 어떻게 저장하는지 완전 분석 (IndexedDB/LevelDB/idb_cmp1/snappy/V8 ValueSerializer, object store 구조, 디코더 참조 구현). **Rust 리더 구현의 청사진.**
- **`docs/journey-and-troubleshooting.md`** — 여기까지 온 전체 과정과 막혔던 지점들 (Graph 차단 → a11y 시도/폐기 → 데이터 경로 확정). 같은 삽질 반복 방지용.
- **`docs/scroll-collection/`** — **드롭된 "스크롤 수집" 기능** 전용 폴더. 이 기능의 제작 계획, 실험으로 알아낸 것, 왜 막혀서 드롭했는지, **그리고 나중에 어떻게 검증·구현해야 하는지**를 상세히 적어둠.
  - 남겨둔 이유: 언젠가 **"캐시에 없어서 읽기 불가"** 상황이 실제로 발생하면, 그때 이 문서를 근거로 스크롤 수집을 검증·구현하기 위함.

## 기술 핵심 (빠른 참조)

- 대상 프로세스: `MSTeams` (bundle id `com.microsoft.teams2`).
- 데이터 위치: `~/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb/`
- 값 인코딩: 표준 **V8 ValueSerializer** (Teams 커스텀 아님). blink 봉투만 스킵하면 파싱됨.
- **권한**: 이 서버(및 초기 osascript/CGEvent를 쓰는 프로세스)에 macOS "손쉬운 사용(Accessibility)" 권한 필요. TCC는 실행파일 경로/서명 단위로 권한을 구분하므로, **실제로 띄우는 그 바이너리/앱**에 부여해야 함.

## Git 커밋 규칙 (Agent 필수 준수)

- **작업 하나 = 커밋 하나.** 뭔가 하나의 작업(기능/수정/문서 등)을 끝내면 곧바로 그것만 담은 커밋을 하나 만든다. 여러 작업을 한 커밋에 몰지 않는다.
- **큰 작업을 여러 단계로 나눴으면, 각 단계마다 커밋.** (예: M1 끝 → 커밋, M2 끝 → 커밋.)
- **push는 Agent가 하지 않는다.** 사용자가 명시적으로 "push 해"라고 지시했을 때만, 또는 사용자가 직접 push 한다. Agent는 로컬 커밋까지만.

## 구조 (Cargo 워크스페이스)

- `crates/teams-core` — LevelDB 리더 + V8 디코더 + Teams 스키마 + 조회 API (transport/MCP 무관).
- `crates/teams-mcp-server` — rmcp `TeamsServer`(도구 정의). 전송 무관.
- `crates/teams-mcp-stdio` / `teams-mcp-http` — stdio / Streamable HTTP 전송 바이너리.

메시지 store 매핑(**이름 기반 런타임 해석** — IndexedDB db/store id 는 프로파일마다 동적 할당이라 하드코딩 불가):
대화=`conversation-manager` DB 의 `conversations` store(OneGQL_Conversation),
메시지=`replychain-manager` DB 의 `replychains`/`replychains-2` store(replychain `messageMap`).
DB 구조를 다시 조사해야 하면 `cargo run -p teams-core --example explore`(집계) / `--example dump -- <db> <store>`(레코드 덤프) / `--example meta`(db·store 이름↔id 매핑) 사용.

## 빌드

```bash
cargo build --release -p teams-mcp-stdio   # 산출물: target/release/teams-mcp-stdio
cargo build --release -p teams-mcp-http    # 산출물: target/release/teams-mcp-http
```
