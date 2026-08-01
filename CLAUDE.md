# teams-mcp

macOS·Windows·Linux에서 실행 중인 **Microsoft Teams** 의 로컬 데이터를 읽는 **읽기 전용 MCP 서버** (Rust). macOS·Windows 는 네이티브 Teams 2(Edge WebView2), Linux 는 네이티브 클라이언트가 없어 Teams PWA(Edge·Chrome) 또는 teams-for-linux(Electron) — 어느 쪽이든 Chromium 계열이라 저장 포맷이 같아 같은 리더로 읽는다.

## 왜 이 방식인가 (요약)

- 테넌트 관리자 동의(admin consent)를 받을 수 없는 환경이라 **정식 Microsoft Graph API 경로가 막힘.** 개인계정도 Teams 채팅 API 불가.
- **macOS 접근성(a11y) 방식은 폐기됨** — Teams v2는 WebView2라 접근성 트리에 메시지 텍스트가 안 실리고, 강제로 켤 방법이 없음.
- **확정 방식: Teams가 로컬 IndexedDB(LevelDB)에 캐싱해둔 메시지를 직접 읽는다.** 토큰·봇·앱등록 불필요. 메시지 조회는 순수 로컬 파일 읽기라 네트워크 요청이 전혀 없음.
- **예외: `fetch_image` 만 네트워크 사용.** 이미지는 원격 AMS URL 참조라 로컬에 바이트가 없어, 로컬 쿠키를 복호화해 인증 GET 한다 (`docs/image-fetch.md`).
- **Teams 로의 쓰기(전송·수정)는 의도적으로 미지원.** Teams 데이터는 읽기만 한다 (이미지도 GET 만).
- **단, "클립보드에 복사"까지는 지원한다.** 전송을 못 하는 대신, 사용자가 Teams 입력창에 그대로 붙여넣을 수 있는 리치 텍스트(링크·불릿·강조 유지)를 클립보드에 올려준다(`copy_to_clipboard`). Teams 데이터에 대한 쓰기가 아니라 로컬 OS 클립보드 쓰기이므로 위 원칙과 충돌하지 않는다. 붙여넣기·전송이라는 최종 행위의 주체는 사용자다.

전체 의사결정 과정과 시행착오는 `docs/journey-and-troubleshooting.md` 참고.

## 현재 스코프

- ✅ **로컬 캐시에 이미 있는 메시지 읽기** (Teams가 자동으로 캐싱한 수천 건. 채팅별로 수개월치 히스토리).
- ✅ **붙여넣기용 리치 텍스트 클립보드 복사** (`copy_to_clipboard`). 전송 미지원의 보완 경로.
- ❌ **스크롤로 캐시에 없는 과거를 새로 수집하는 기능은 드롭됨.** (이유·계획·재개 방법은 `docs/scroll-collection/` 참고)

## docs/ 폴더 안내

`docs/`는 이 프로젝트의 모든 설계·조사 문서를 모아두는 곳이다.

- **`docs/teams-storage-structure.md`** — Teams가 메시지를 로컬에 어떻게 저장하는지 완전 분석 (IndexedDB/LevelDB/idb_cmp1/snappy/V8 ValueSerializer, object store 구조, 디코더 참조 구현). **Rust 리더 구현의 청사진.**
- **`docs/journey-and-troubleshooting.md`** — 여기까지 온 전체 과정과 막혔던 지점들 (Graph 차단 → a11y 시도/폐기 → 데이터 경로 확정). 같은 삽질 반복 방지용.
- **`docs/scroll-collection/`** — **드롭된 "스크롤 수집" 기능** 전용 폴더. 이 기능의 제작 계획, 실험으로 알아낸 것, 왜 막혀서 드롭했는지, **그리고 나중에 어떻게 검증·구현해야 하는지**를 상세히 적어둠.
  - 남겨둔 이유: 언젠가 **"캐시에 없어서 읽기 불가"** 상황이 실제로 발생하면, 그때 이 문서를 근거로 스크롤 수집을 검증·구현하기 위함.

## 기술 핵심 (빠른 참조)

- 대상 프로세스: macOS·Windows `MSTeams` (bundle id `com.microsoft.teams2`) / Linux 는 Edge·Chrome(PWA) 또는 `teams-for-linux`.
- 데이터 위치 (macOS): `~/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb/`
- 오리진 디렉터리는 고정이 아니다 — MS 가 `teams.microsoft.com` → `teams.cloud.microsoft` 로 이전 중이라 한 프로파일에 둘이 공존할 수 있다(한쪽은 stale). `location.rs` 의 `ORIGIN_DIRS` 후보 중 **최근에 쓰인 것**을 고른다(플랫폼 공통).
- 데이터 위치 (Linux): 고정 경로가 없어 **후보를 훑어 DB 가 실제로 있는 프로파일을 고른다**(`location.rs` 의 `linux_profile_candidates`). teams-for-linux 는 `~/.config/teams-for-linux/Partitions/teams-4-linux`, PWA 는 `~/.config/{microsoft-edge,google-chrome,chromium}/{Default,Profile N}`, snap·flatpak 설치도 같은 구조로 탐색. 플랫폼 무관하게 `TEAMS_MCP_DB` 로 override 가능.
- 값 인코딩: 표준 **V8 ValueSerializer** (Teams 커스텀 아님). blink 봉투만 스킵하면 파싱됨.
- **권한**: 이 서버(및 초기 osascript/CGEvent를 쓰는 프로세스)에 macOS "손쉬운 사용(Accessibility)" 권한 필요. TCC는 실행파일 경로/서명 단위로 권한을 구분하므로, **실제로 띄우는 그 바이너리/앱**에 부여해야 함. (Linux 는 별도 권한 없음 — 해당 파일 읽기 권한이면 충분.)

## Git 커밋 규칙 (Agent 필수 준수)

- **작업 하나 = 커밋 하나.** 뭔가 하나의 작업(기능/수정/문서 등)을 끝내면 곧바로 그것만 담은 커밋을 하나 만든다. 여러 작업을 한 커밋에 몰지 않는다.
- **큰 작업을 여러 단계로 나눴으면, 각 단계마다 커밋.** (예: M1 끝 → 커밋, M2 끝 → 커밋.)
- **push는 Agent가 하지 않는다.** 사용자가 명시적으로 "push 해"라고 지시했을 때만, 또는 사용자가 직접 push 한다. Agent는 로컬 커밋까지만.

## 구조 (Cargo 워크스페이스)

- `crates/teams-core` — LevelDB 리더 + V8 디코더 + Teams 스키마 + 조회 API (transport/MCP 무관). 조회는 순수 로컬 파일 읽기이고, 외부에 손을 뻗는 모듈은 둘뿐이다: `media`(네트워크 — 이미지 조회 = 쿠키 복호화 + AMS GET, `docs/image-fetch.md`), `clipboard`(OS 클립보드 — HTML+평문 flavor 동시 세팅, arboard). 플랫폼 분기도 이 둘과 `location` 뿐이다 — `media` 는 쿠키 복호화 방식(macOS 키체인 / Windows DPAPI / Linux `peanuts`·키링), `clipboard` 는 리눅스에서만 인스턴스를 프로세스 수명 동안 살려둔다(X11·Wayland 는 소유 프로세스가 내용을 내어주는 구조라 드롭하면 사라짐).
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
