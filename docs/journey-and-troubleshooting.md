# 여정과 시행착오

teams-mcp가 지금의 "로컬 IndexedDB 읽기" 방식에 도달하기까지의 과정과, 막혔던 지점들. **같은 삽질을 반복하지 않기 위한 기록.**

---

## 목표

Claude(MCP)가 Microsoft Teams 채팅을 **읽는다**. (쓰기는 처음부터 배제.)

제약: 테넌트 관리자 동의(admin consent)를 받을 수 없는 환경이라, 정식 Graph API 경로를 쓸 수 없음.

---

## 시도 1: Microsoft Graph API (정식) — ❌ 차단

- Teams 읽기/쓰기는 `Chat.Read`, `ChannelMessage.Read.All` 등 **위임 권한**이 필요하고, 대부분 **테넌트 관리자 동의(admin consent)** 가 강제.
- 방법 자체는 표준(Azure 앱 등록 → device code flow → Graph). 다만 **admin consent** 단계가 필수인데, 이 환경에선 테넌트 관리자 동의를 확보할 수 없어 진행 불가.
- 개인 계정은 Teams 채팅 API 자체가 막힘.
- **결론**: admin consent 없이는 불가. 폐기.

## 시도 2: macOS 접근성(Accessibility, a11y)으로 앱 화면 읽기 — ❌ 폐기

아이디어: 이미 로그인된 실제 Teams 앱의 화면 텍스트를 macOS 접근성 트리로 읽는다. (토큰·서버·네트워크 무관.)

진행/발견:
- 프로세스는 `MSTeams`, bundle `com.microsoft.teams2`.
- **권한 함정**: 다른 앱 UI를 읽으려면 제어 프로세스에 **"손쉬운 사용(Accessibility)" 권한**이 필요. `-1719`("보조 접근 허용 안 됨") 에러.
  - **TCC는 실행파일 경로/서명 단위로 권한을 구분.** 터미널이 `/Applications`가 아니라 개발 빌드 경로(`.../dist/Tasty.app`)에서 실행 중이면, `/Applications`의 동명 앱에 권한을 줘도 안 통함. 실제 실행 중인 그 바이너리에 줘야 함. (프로세스 조상 추적으로 확인: `ps -o ppid,comm`)
- 권한 부여 후: 처음엔 **AXStaticText 228개**가 읽혔다(메시지 텍스트 포함). 성공하는 듯했으나...
- **치명적 결함**: 다시 시도하면 AXStaticText **0개**(전체 2325개 노드는 껍데기). Teams v2는 **Edge WebView2**인데, **Chromium은 성능상 접근성 트리를 기본적으로 안 만든다.** 스크린리더(VoiceOver)가 붙거나 앱이 켤 때만 생성.
  - `AXManualAccessibility`(Electron 전용 강제 스위치) 설정 시도 → osascript로는 설정 불가, **네이티브(swift)로도 `-25205`(kAXErrorAttributeUnsupported)**. WebView2는 이 속성 미지원.
  - `AXEnhancedUserInterface` 설정 → 무효(여전히 0개).
- **결론**: WebView2가 접근성 트리를 안 채우면 강제할 수단이 없음. 최초 228개는 우연히 켜진 순간이었을 뿐. **신뢰 불가 → 폐기.**

## 시도 3: 로컬 데이터 직접 읽기 — ✅ 채택

a11y가 화면(렌더)에 의존해 실패하니, **Teams가 로컬에 저장한 데이터 자체**를 읽기로 전환.

- Teams v2 데이터 위치 발견: `EBWebView/WV2Profile_tfw/IndexedDB/.../https_teams.microsoft.com_0.indexeddb.leveldb/`.
- raw grep으로 화면의 메시지 문자열("SECloudit" 등)이 `.ldb`에 실제 존재함을 확인.
- 저장 포맷 해독 (상세는 `teams-storage-structure.md`):
  - LevelDB + **커스텀 comparator `idb_cmp1`** → 범용 Rust 리더(`rusty-leveldb`) **패닉**. (Python `dfindexeddb`는 처리 가능.)
  - 값은 **snappy 압축** + **표준 V8 ValueSerializer**(처음엔 Teams 커스텀 포맷으로 오해했으나 표준이었음).
  - `dfindexeddb`의 blink 디코더는 `Unsupported header`로 실패 → **V8 payload를 직접 파싱**(마지막 `0xFF version` 뒤부터)해 우회. **직접 짠 ~60줄 V8 파서로 대화·메시지 완전 복원 성공** (발신자·본문·시간, 한글 정확).
- **장점**: 순수 로컬 파일 읽기 → 네트워크·UI 0. 이미 캐시된 히스토리가 방대(수개월·수천 건).
- **결론**: 이 방식으로 확정.

## 파생 시도: 스크롤 수집 (캐시에 없는 과거 가져오기) — ⏸ 드롭(문서화)

로컬 캐시에 없는 과거를 Teams에 스크롤을 보내 서버에서 fetch·캐시하게 하는 아이디어. 실험 결과 전제가 확인되지 않아 드롭. **상세 및 재개 방법은 `docs/scroll-collection/`** 참고.

---

## 재사용 가능한 교훈

- **Teams v2 = Edge WebView2**. Electron 자동화 트릭(`AXManualAccessibility`, remote debug port 등)이 대부분 안 통함. 원격 디버깅 포트도 안 열려 있었음.
- **macOS 권한(TCC)은 경로/서명 단위.** 개발 빌드에서 돌 땐 그 경로의 앱에 직접 권한 부여.
- **입력 주입 방식별 차이** (스크롤 실험에서):
  - `CGEventPostToPid` + **키 이벤트** = 작동 (커서·포커스 안 건드림). 단 **포커스 따라감** → Page Up이 채팅목록에 가면 채팅방이 바뀜(사고).
  - `CGEventPostToPid` + **스크롤휠** = WebView2가 무시(안 됨).
  - `cghidEventTap` + **스크롤휠 + location** = 작동하나 **커서가 그 좌표로 끌려감**(warp+restore로 완화).
- **Chromium IndexedDB 값 = 표준 V8 ValueSerializer.** 리버스 엔지니어링 불필요, 명세 존재.
- 로컬 LevelDB는 실행 중 락 → immutable `.ldb` 직접 읽기 또는 복사 후 읽기.
