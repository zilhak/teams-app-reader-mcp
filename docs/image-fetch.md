# 이미지 조회 (fetch_image)

Teams 메시지의 이미지(붙여넣은 스크린샷 등)를 실제 바이트로 가져오는 경로. 두 단계로 나뉜다.

## 왜 두 단계인가

메시지 본문(replychain `messageMap` 의 `content` HTML)에 이미지는 **원격 URL 참조**로만 들어있다:

```html
<p><img src="https://kr-prod.asyncgw.teams.microsoft.com/v1/objects/0-skr-d4-<id>/views/imgo"
   itemscope itemtype="http://schema.skype.com/AMSImage" style="width:2573px;height:1448px" /></p>
```

- 실제 픽셀은 로컬 IndexedDB 에 **없다.** AMS(Async Media Service) CDN 에 있고 **인증이 걸려 있다.**
- 그래서 `read_messages` 는 URL·크기만 노출하고(**네트워크 0**), 실물은 `fetch_image` 가 받아온다(**네트워크 사용**).

## 1단계 — 참조 노출 (`read_messages`, 네트워크 없음)

`teams-core/src/teams.rs` 의 `extract_images` 가 평문화(`html_to_text`) **전에** `<img>` 를 훑어
`itemtype=".../AMSImage"` 또는 `src` 에 `asyncgw` 가 든 것만 골라 `images[]{url,width,height}` 로 담는다.
이모지·스티커는 제외. 이미지만 있던 메시지는 평문이 비므로 `content` 에 `[이미지]` 마커를 남긴다.

## 2단계 — 실물 조회 (`fetch_image`, 네트워크 사용)

`teams-core/src/media.rs`. 흐름: **쿠키 복호화 → 인증 GET**.

1. **SSRF 방지** — URL 호스트가 `*.asyncgw.teams.microsoft.com` 이 아니면 거부.
2. **쿠키 로드** — EBWebView 프로파일의 `Cookies`(SQLite)를 임시 파일로 복사해 열고(잠금 회피),
   `host_key LIKE '%asyncgw%'` 인 쿠키(`skypetoken_asm`·`authtoken_asm` 등)의 `encrypted_value` 를 읽는다.
3. **복호화** (Chromium `os_crypt` 방식, 플랫폼별):
   - **macOS**: 키체인 `security find-generic-password -s "Microsoft Teams Safe Storage"` 로 비밀번호를 얻어
     `PBKDF2-HMAC-SHA1(salt="saltysalt", iter=1003, len=16)` → **AES-128-CBC**(IV=공백 16바이트), `v10` 접두어 제거.
   - **Windows**: `Local State` 의 `os_crypt.encrypted_key`(base64, `DPAPI` 접두어) → `CryptUnprotectData` 로 마스터키 →
     `v10`/`v11` 쿠키를 **AES-256-GCM**(`[3 prefix][12 nonce][ct||16 tag]`)로 복호화.
   - 신 Chromium 은 평문 앞 32바이트에 `sha256(도메인)` 을 붙이므로, 전체 UTF-8 디코드가 실패하면 앞 32바이트를 떼고 재시도.
4. **GET** — `Cookie: skypetoken_asm=…; authtoken_asm=…` 헤더로 URL 을 받아 `(바이트, MIME)` 반환.
   서버(`teams-mcp-server`)가 이를 base64 로 MCP 이미지 블록으로 감싼다.

## 경로·환경변수

| 대상 | macOS | Windows | override |
|---|---|---|---|
| IndexedDB | `~/Library/Containers/com.microsoft.teams2/…/WV2Profile_tfw/IndexedDB/https_teams…leveldb` | `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\…` | `TEAMS_MCP_DB` |
| Cookies | 위 프로파일의 `Network/Cookies` 또는 `Cookies` | 동일 | `TEAMS_MCP_COOKIES` |
| Local State | (불필요) | EBWebView 루트의 `Local State` | `TEAMS_MCP_LOCAL_STATE` |

## 한계 / 주의

- **토큰 만료** — asyncgw 쿠키는 수명이 있다. Teams 앱이 켜져 있으면 디스크 쿠키가 갱신되지만, 오래 꺼져 있던 경우 만료로 401 가능.
- **macOS 키체인 접근** — 서버 바이너리가 "Microsoft Teams Safe Storage" 항목 접근 권한을 물을 수 있다(최초 1회 허용).
- **읽기 전용 유지** — GET 만 하며 Teams 에 아무것도 쓰지 않는다.
- Windows 경로는 표준 Chromium 방식으로 구현했으나 실측 검증은 macOS 기준이다.
