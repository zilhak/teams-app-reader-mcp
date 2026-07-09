# teams-mcp

macOS 접근성(Accessibility) API로 **실행 중인 Microsoft Teams 데스크톱 앱**을 읽는 **읽기 전용** MCP 서버.

Microsoft Graph API·봇·토큰을 전혀 쓰지 않는다. 이미 로그인해서 쓰고 있는 진짜 Teams 앱의 화면을 그대로 읽을 뿐이라, 관리자 승인이나 앱 등록이 필요 없다.

## 동작 원리

- 신 Teams 2(프로세스명 `MSTeams`, Chromium 웹뷰)의 접근성 트리에서 텍스트를 읽는다.
- 내부적으로 `osascript`(System Events)를 짧게 호출한다.
- **읽기 전용**: 메시지 전송·수정·클릭·스크롤 등 어떤 조작도 하지 않는다.

## 제약 (반드시 이해할 것)

- **지금 열려 있는 채팅**만 읽힌다. 다른 방을 읽으려면 그 방을 열어야 한다.
- 화면에 렌더된 최근 메시지만 읽힌다. 과거 메시지는 직접 스크롤해서 화면에 올려야 한다(가상 스크롤).
- Teams 앱이 실행 중이어야 한다. 창을 최소화하면 렌더가 멈춰 안 읽힐 수 있다.
- Teams가 **백그라운드에 있어도** 읽힌다(앞으로 꺼낼 필요 없음).

## 필수 권한 (1회성)

이 서버를 실행하는 **프로세스**에 macOS "손쉬운 사용" 권한이 있어야 한다.

- **시스템 설정 → 개인정보 보호 및 보안 → 손쉬운 사용**
- 이 MCP를 띄우는 터미널/호스트 앱(예: Tasty, iTerm, Claude 등)을 목록에 추가하고 토글을 켠다.
- 권한이 없으면 osascript가 `-1719 (보조 접근 허용 안 됨)` 오류를 낸다.

> 주의: TCC는 실행 파일의 **경로/서명** 단위로 권한을 구분한다. `/Applications`의 앱과 개발 빌드 경로의 앱은 별개로 취급되므로, **실제로 서버를 띄우는 바로 그 앱**에 권한을 줘야 한다.

## 빌드

```bash
cargo build --release
# 산출물: target/release/teams-mcp
```

## 도구

| 도구 | 설명 |
|---|---|
| `active_view` | 현재 열린 Teams 창 제목(어느 채팅/화면인지) |
| `read_messages` | 현재 창에 보이는 모든 텍스트(메시지 본문·발신자·시간, 좌측 채팅 목록 포함)를 위→아래 순서로 반환 |

## Claude Code 등록

```bash
claude mcp add teams-mcp -s user -- /Users/ljh/workspace/etc/teams-mcp/target/release/teams-mcp
```

또는 설정 JSON에 직접:

```json
{
  "mcpServers": {
    "teams-mcp": {
      "command": "/Users/ljh/workspace/etc/teams-mcp/target/release/teams-mcp"
    }
  }
}
```
