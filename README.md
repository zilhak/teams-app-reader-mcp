# teams-mcp

macOS에서 **Microsoft Teams(신 Teams 2)** 가 로컬에 캐싱해둔 대화/메시지를 읽는 **읽기 전용** MCP 서버 (Rust).

Microsoft Graph API·봇·토큰·네트워크를 전혀 쓰지 않는다. Teams 앱이 자기 IndexedDB(로컬 LevelDB)에 이미 캐싱해둔 데이터를 파일에서 직접 읽을 뿐이라, 관리자 승인이나 앱 등록이 필요 없다.

## 동작 원리

- 신 Teams 2(프로세스명 `MSTeams`, Edge WebView2)는 메시지를 로컬 IndexedDB에 캐싱한다.
- 그 LevelDB(`.ldb`/`.log`)를 직접 파싱한다: **SSTable 직접 스캔**(Chromium `idb_cmp1` comparator 우회) + snappy 해제 + **V8 ValueSerializer** 디코드.
- 대화 목록 = `db31/store1`(OneGQL_Conversation), 메시지 = `db44/store1`(replychain `messageMap`).
- **읽기 전용**: 메시지 전송·수정 등 쓰기 기능은 제공하지 않는다. Teams 앱을 조작하지도 않는다.

자세한 저장 구조·의사결정 과정은 [`docs/`](docs/) 참고 (특히 `docs/teams-storage-structure.md`, `docs/implementation-plan.md`).

## 스코프

- ✅ **로컬 캐시에 이미 있는 메시지 읽기** (Teams가 자동 캐싱한 수개월치, 채팅별 수백~1600건).
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
| `read_messages` | 특정 대화의 메시지(`chat`=conversationId 정확일치 또는 대화명 부분일치, `limit`, `before_ms`) |
| `search_messages` | 캐시 전역 키워드 검색(`query`, `limit`) |

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

- `TEAMS_MCP_DB` — IndexedDB leveldb 경로 override (기본: 표준 macOS Teams v2 경로).
- `TEAMS_MCP_HTTP_ADDR` — HTTP 바인드 주소 (기본 `127.0.0.1:8787`).
- `TEAMS_MCP_TOKEN` — 설정 시 HTTP `Authorization: Bearer <token>` 필수.
