# teams-mcp

macOS·Windows·Linux에서 **Microsoft Teams** 가 로컬에 캐싱해둔 대화/메시지를 읽는 MCP 서버 (Rust).

Microsoft Graph API·봇·토큰·앱 등록을 쓰지 않는다. Teams 앱이 자기 IndexedDB(로컬 LevelDB)에 이미 캐싱해둔 데이터를 파일에서 직접 읽을 뿐이라, 관리자 승인이 필요 없다. 메시지 조회는 네트워크도 쓰지 않는다 — 단, 이미지 실물을 받아오는 `fetch_image` 만 예외로 로컬 쿠키를 복호화해 AMS CDN 에 인증 요청한다(아래 참조).

**Teams 로 메시지를 전송·수정하지 않는다.** 대신 그 자리를 메우는 경로로, 사용자가 Teams 입력창에 그대로 붙여넣을 수 있는 리치 텍스트(링크·불릿·강조 유지)를 클립보드에 올려주는 `copy_to_clipboard` 까지를 지원한다. 붙여넣고 보내는 최종 행위는 사용자가 한다.

## 동작 원리

- 신 Teams 2(프로세스명 `MSTeams`, Edge WebView2)는 메시지를 로컬 IndexedDB에 캐싱한다.
- 그 LevelDB(`.ldb`/`.log`)를 직접 파싱한다: **SSTable 직접 스캔**(Chromium `idb_cmp1` comparator 우회) + snappy 해제 + **V8 ValueSerializer** 디코드.
- 대화 목록 = `conversation-manager` DB 의 `conversations` store(OneGQL_Conversation), 메시지 = `replychain-manager` DB 의 `replychains`/`replychains-2` store(replychain `messageMap`). IndexedDB 의 db/store id 는 프로파일마다 동적 할당되므로 **이름으로 런타임 해석**한다.
- **Teams 데이터는 읽기만**: 메시지 전송·수정 기능은 제공하지 않고, Teams 앱을 조작하지도 않는다. 클립보드 복사(`copy_to_clipboard`)는 Teams 가 아닌 로컬 OS 클립보드에 쓰는 것이라 여기 해당하지 않는다.

### 플랫폼

| OS | Teams 클라이언트 | 비고 |
|---|---|---|
| macOS · Windows | 네이티브 Teams 2 (Edge WebView2) | 기본 경로 자동 |
| Linux | **Teams PWA**(Edge·Chrome·Chromium) 또는 **teams-for-linux**(Electron) | 기본 경로 자동탐지. 아래 참고 |

리눅스에는 신 Teams 2 네이티브 클라이언트가 없다(2022년 12월 단종). 대신 같은 웹 Teams를 감싼 Chromium 계열 껍데기 두 종류로 쓴다 — MS 공식 경로인 **PWA**(Edge/Chrome에서 앱으로 설치)와 비공식 Electron 래퍼 **teams-for-linux**. 껍데기가 무엇이든 저장 포맷은 동일한 IndexedDB(LevelDB)라 리더는 그대로 통하고, 다른 것은 프로파일 경로뿐이다. 그래서 후보 경로들(deb/tarball·snap·flatpak × 브라우저 프로파일·Electron 파티션)을 훑어 **Teams DB가 실제로 있는** 프로파일을 자동으로 고른다. 못 찾으면 `TEAMS_MCP_DB`로 직접 지정하면 된다.

자세한 저장 구조·의사결정 과정은 [`docs/`](docs/) 참고 (특히 `docs/teams-storage-structure.md`, `docs/implementation-plan.md`).

## 스코프

- ✅ **로컬 캐시에 이미 있는 메시지 읽기** (Teams가 자동 캐싱한 수개월치, 채팅별 수백~1600건).
- ✅ **메시지 이미지 참조 노출 + 실물 조회** (`read_messages` 의 `images[]` → `fetch_image`). 상세: `docs/image-fetch.md`.
- ✅ **붙여넣기용 리치 텍스트 클립보드 복사** (`copy_to_clipboard`) — 전송 미지원의 보완 경로.
- ❌ 캐시에 없는 과거를 새로 수집하는 기능은 드롭됨 (`docs/scroll-collection/`).

## 구조 (Cargo 워크스페이스)

| 크레이트 | 역할 |
|---|---|
| `teams-core` | LevelDB 리더 + V8 디코더 + Teams 스키마 + 조회 API (transport/MCP 무관) |
| `teams-mcp-server` | rmcp `TeamsServer`(도구 정의). 전송 무관 |
| `teams-mcp-stdio` | stdio 전송 바이너리 |
| `teams-mcp-http` | Streamable HTTP 전송 바이너리 (localhost 바인드) |

## 빌드

```bash
cargo build --release -p teams-mcp-stdio   # → target/release/teams-mcp-stdio
cargo build --release -p teams-mcp-http    # → target/release/teams-mcp-http
```

## 도구

| 도구 | 설명 |
|---|---|
| `list_chats` | 캐시된 대화 목록(대화명·conversationId·메시지 수·마지막 메시지)을 최근순으로 |
| `read_messages` | 특정 대화의 메시지(`chat`=conversationId 정확일치 또는 대화명 부분일치, `limit`, `before_ms`). 이미지가 있으면 `images[]{url,width,height}` 포함 |
| `search_messages` | 캐시 전역 키워드 검색(`query`, `limit`) |
| `fetch_image` | `images[].url`(AMS 이미지)을 인증 쿠키로 GET 해 이미지로 반환. **유일하게 네트워크 사용** |
| `copy_to_clipboard` | `html` 을 클립보드에 올린다. Teams 입력창에 붙여넣으면 링크·불릿·강조가 살아난다. **유일하게 로컬 상태를 변경**(기존 클립보드 내용을 덮어씀) |

### `copy_to_clipboard` 상세

Teams 로 메시지를 보내지 못하는 대신, 붙여넣으면 되는 형태까지 만들어주는 도구다. 여러 줄 업무보고나 티켓 링크가 섞인 메시지를 만들 때 쓴다.

클립보드에는 같은 내용이 두 표현으로 올라간다:

| flavor | 내용 | 쓰이는 곳 |
|---|---|---|
| HTML (macOS `public.html` / Windows `CF_HTML` / Linux `text/html`) | 서식이 살아있는 HTML | Teams 입력창에 Cmd+V / Ctrl+V |
| 평문 (`public.utf8-plain-text` / `CF_UNICODETEXT` / `text/plain`) | **HTML 원문(태그 포함)** | `pbpaste` 검증용 |

평문 쪽에 태그째로 넣는 것은 의도된 선택이다. 붙여넣기 대상이 서식을 받는 Teams 입력창이라 평문 표현은 실제 사용 경로가 아닌데, 비워두면(arboard 는 `alt_text: None` 이면 평문 flavor 를 만들지 않는다) `pbpaste` 가 빈 출력이 되어 클립보드 내용을 확인할 수 없게 된다. 그 대가로 **서식 없이 붙여넣기(Cmd+Shift+V)를 하면 태그가 그대로 보인다** — 잘못 붙여넣었음이 즉시 드러나므로 조용히 실패하지는 않는다.

Teams 입력창에서 **동작 확인된** 태그: `<p>`, `<strong>`, `<ul>`/`<li>`(중첩), `<a href>`. 그 외(`<ol>`, `<em>`, `<br>`, `<table>`, 코드블록 등)는 Teams 가 붙여넣기에서 받아주는지 확인되지 않았다. `<html>`/`<body>` 로 감쌀 필요는 없다(자동으로 감싼다).

### 리눅스에서 다른 점

- **`copy_to_clipboard`**: X11·Wayland 는 클립보드 내용을 OS 가 보관하지 않고 **소유 프로세스**가 요청받을 때마다 내어준다. 서버가 떠 있는 동안 붙여넣으면 되지만, 서버가 종료되면 데스크톱의 클립보드 매니저가 받아가지 않는 한 클립보드는 비워진다.
- **`fetch_image`**: 쿠키 키가 키링에 있는 경우(`v11`) `secret-tool`(Ubuntu: `libsecret-tools`)이 필요하다. 없으면 키링 없는 환경용 고정 비밀번호(`v10`) 경로로만 시도한다.

## 실행 · 등록

### stdio (Claude Code 등)

```bash
claude mcp add teams-mcp -s user -- /Users/ljh/workspace/etc/teams-mcp/target/release/teams-mcp-stdio
```

설정 JSON 직접:

```json
{
  "mcpServers": {
    "teams-mcp": {
      "command": "/Users/ljh/workspace/etc/teams-mcp/target/release/teams-mcp-stdio"
    }
  }
}
```

### HTTP

```bash
# 기본 127.0.0.1:8787/mcp, 무인증(로컬 전용)
target/release/teams-mcp-http

# 주소 변경 + Bearer 토큰 인증
TEAMS_MCP_HTTP_ADDR=127.0.0.1:9000 TEAMS_MCP_TOKEN=secret target/release/teams-mcp-http
```

> ⚠️ HTTP는 민감한 Teams 메시지를 네트워크로 노출한다. **기본 localhost 바인드**이며, 외부 노출이 필요하면 반드시 `TEAMS_MCP_TOKEN` 을 설정할 것.

## 환경변수

- `TEAMS_MCP_DB` — IndexedDB leveldb 경로 override. 미설정 시 OS별 표준 Teams v2 경로를 사용한다:
  - macOS: `$HOME/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb`
  - Windows: `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\Microsoft\MSTeams\EBWebView\WV2Profile_tfw\IndexedDB\https_teams.microsoft.com_0.indexeddb.leveldb`
  - Linux: 아래 후보 중 Teams DB가 실제로 있는 프로파일을 자동 선택 (`<프로파일>/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb`)
    - teams-for-linux: `~/.config/teams-for-linux/Partitions/teams-4-linux`
    - PWA: `~/.config/{microsoft-edge,google-chrome,chromium}/{Default,Profile N}`
    - snap·flatpak 설치도 같은 구조로 탐색 (`~/snap/<앱>/current/.config/...`, `~/.var/app/<app-id>/config/...`)
  - 그 외 OS: 기본 경로 없음. `TEAMS_MCP_DB` 로만 지정 가능.
- `TEAMS_MCP_HTTP_ADDR` — HTTP 바인드 주소 (기본 `127.0.0.1:8787`).
- `TEAMS_MCP_TOKEN` — 설정 시 HTTP `Authorization: Bearer <token>` 필수.
