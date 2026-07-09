# 스크롤 수집 기능 (드롭됨 — 미래 재개용 문서)

> **상태: 드롭.** 전제가 확인되지 않았고, 실험 대상 채팅이 권한 경계에 막혀 검증 불가였다.
> **이 문서를 남기는 이유**: 언젠가 **"캐시에 없어서 읽기 불가"** 상황이 실제로 발생하면, 그때 이 문서를 근거로 **검증 → 구현**하기 위함. 아래에 무엇을 어떻게 테스트/구현해야 하는지 상세히 적어둔다.

---

## 1. 기능의 목적

기본 MCP는 **Teams가 이미 로컬 IndexedDB에 캐싱해둔 메시지**만 읽는다. 그런데 사용자가 이 기기에서 한 번도 로드한 적 없는 **아주 오래된 과거**는 로컬에 없다.

이 기능은 그 **"캐시 miss" 과거를 MCP가 시간을 들여 재수집**하는 것이 목표다.

## 2. 핵심 전제 (아이디어)

MCP가 직접 서버 API를 때리는 게 아니라(네트워크 요청 발생), **띄워둔 Teams 앱에 스크롤을 보내서 Teams가 스스로 서버에서 과거를 fetch → IndexedDB에 캐시**하게 하고, MCP는 **그 IndexedDB를 다시 읽는다.**

```
[Teams 메시지창에 스크롤 주입]
   → Teams가 서버에서 과거 fetch (정상 클라이언트 동작)
   → Teams가 IndexedDB에 캐시
[MCP가 IndexedDB 재읽기]
   → 새로 생긴 과거 메시지 확인
   → 더 안 늘 때까지(맨 위 도달) 반복
```

읽기는 항상 IndexedDB에서 하므로(a11y 안 씀), WebView2 접근성 문제와 무관하다.

## 3. 스크롤 주입 메커니즘 (실험으로 확정)

여러 방식을 실측한 결과:

| 방식 | 작동? | 부작용 |
|---|---|---|
| `CGEventPostToPid` + **Page Up 키** | ✅ 스크롤됨 | ❌ **포커스 따라감.** 메시지창이 아닌 채팅목록에 포커스가 있으면 **채팅방을 바꿔버림**(실제 사고 발생). 위험. |
| `CGEventPostToPid` + **스크롤휠** | ❌ WebView2가 무시 | — |
| **`cghidEventTap` + 스크롤휠 + `ev.location`** | ✅ 스크롤됨 | ⚠️ 커서가 그 좌표로 이동됨 → **warp+restore로 ~0.1초 왕복**으로 완화 |

**→ 채택 메커니즘: `cghidEventTap` 스크롤휠 + 좌표 타겟 + 커서 저장/복원.**
좌표(위치)로 대상 창을 정하므로 **채팅방이 절대 안 바뀌고** 메시지 목록만 정확히 스크롤한다. 단점은 커서가 잠깐(~0.1s) 그 지점에 다녀오는 것 → **사용자 마우스 사용과 충돌 방지 위해 idle일 때만 실행** 필요.

### 작동 확인된 스크롤 코드 (swift)

창 좌표는 osascript로 얻는다:
```bash
osascript -e 'tell application "System Events" to tell process "MSTeams"
  set p to position of window 1
  set s to size of window 1
  return (item 1 of p as string) & "," & (item 2 of p as string) & "," & (item 1 of s as string) & "," & (item 2 of s as string)
end tell'
# 예: 3272,-68,1920,1050  (x,y,w,h)  ← 멀티모니터라 x가 클 수 있음
```

메시지 영역 좌표 ≈ `(x + w*0.6, y + h*0.45)` (좌측 사이드바 ~폭의 22%, 상단 툴바/하단 입력창 피함).

```swift
import Cocoa
let pt = CGPoint(x: 4424, y: 420)   // 위 공식으로 계산
for batch in 0..<6 {
    let before = CGEvent(source: nil)?.location ?? .zero
    CGWarpMouseCursorPosition(pt)                     // 커서 순간 이동
    for _ in 0..<8 {
        if let ev = CGEvent(scrollWheelEvent2Source: nil, units: .line,
                            wheelCount: 1, wheel1: 15, wheel2: 0, wheel3: 0) {
            ev.location = pt
            ev.post(tap: .cghidEventTap)              // 위로 스크롤(wheel1 양수)
        }
        usleep(20000)
    }
    CGWarpMouseCursorPosition(before)                 // 커서 즉시 복원 (~0.1s 왕복)
    usleep(2500000)                                   // 배치 사이 fetch 대기
}
usleep(12000000)                                      // 최종 persist 대기
```

## 4. 실험 결과 (드롭 근거)

전제 = "스크롤 → 서버 fetch → IndexedDB에 새 레코드 기록"이 성립하는가?

**검증 방법**: `db44/store1`의 메시지 인덱스 엔트리(`IndexDataKey`+`ExistsEntryKey`, 메시지 1건당 1개)를 스크롤 **전/후로 카운트**. 스레드별로 timestamp까지 봄. (인덱스 엔트리는 blink 문제 없이 깔끔히 디코드됨 → 완벽한 카운트 지표.)

| 테스트 | 방식 | 결과 |
|---|---|---|
| #1 | Page Up ×40 (@네이티브 플랫폼팀) | 총 +3(노이즈). 게다가 **엉뚱한 스레드**가 늘었고 채팅방이 바뀜 → Page Up이 채팅목록을 훑음. **무효.** |
| #2 | 스크롤휠 ×48 (@R&D본부 출석체크방) | **모든 스레드 Δ0.** 채팅방 안 바뀜. 화면엔 예전 메시지가 로드됐으나 **IndexedDB는 0 증가.** |

**결정적 교란 요인 발견**: 테스트한 [출석체크방]은 사용자가 **"채팅방 메시지 공유" 방식으로 최근 초대**된 방이었다. Teams는 초대 시 히스토리 공유 범위(공유 안 함 / N일 / 전체)를 정하는데, **제한 공유면 그 이전 메시지는 애초에 이 사용자 계정에 공유되지 않는다.**

즉 사용자가 본 "벽"(어느 시점 이전 안 올라감)은 **로컬 캐시의 끝이 아니라 권한 경계**일 가능성이 크다. 그러면:
- 스크롤로 로드된 과거 = 로컬 캐시에 있던 것(재생)일 뿐.
- 그 이전은 **서버가 이 계정에 안 준다** → 어떤 방법으로도 불가.
- 그래서 IndexedDB가 0 증가한 것도 당연.

**이 채팅으로는 전제를 검증할 수 없다.** (권한상 애초에 fetch할 게 없으므로.)

## 5. 왜 드롭했나

1. **전제 미확인** — "스크롤이 서버 fetch를 유발하는가"가 깨끗한 조건에서 검증 안 됨.
2. **권한 경계** — 제한 공유로 초대된 채팅은 벽 너머가 영구 불가. 수집기를 만들어도 못 가져옴.
3. **당장 불필요** — 이미 캐시된 데이터가 방대해서, "사용자가 스크롤로 볼 수 있는 것 ≈ 로컬 캐시 ≈ MCP가 읽는 것". 기본 읽기 MCP로 충분.

## 6. 나중에 이 기능이 필요해지는 조건

MCP 사용 중 **"요청한 범위의 메시지가 로컬 캐시에 없어서 못 읽는"** 상황이 실제로 반복되고, 그 채팅이 **사용자에게 전체 히스토리 접근권이 있는데** 로컬에만 없는 경우. 이때 아래 절차로 재개한다.

## 7. 재개 시 검증 절차 (반드시 이 순서로)

**전제부터 깨끗하게 재검증**해야 한다. 실패했던 테스트의 교란요인(권한 경계)을 배제할 것.

1. **테스트 채팅 선정**: 사용자가 **처음부터/오래 있었던**, **전체 히스토리 접근권**이 있고, **예전 메시지가 아직 로컬 캐시에 없는** 채팅. (제한 공유로 초대된 방 절대 금지.)
2. **베이스라인**: 그 채팅의 threadid 확인 → `db44/store1`에서 그 threadid의 인덱스 **개수 + 가장 오래된 timestamp** 기록. (threadid↔topic 매핑은 db0/store0 대화 레코드의 값 앞부분 UTF-16 threadid + 디코드된 topic으로.)
3. **스크롤 주입**: §3 스크롤휠 방식으로 공격적으로(수십~수백 회, 배치 사이 fetch 대기). **화면에 "이전에 없던 더 오래된 메시지"가 실제로 로드되는지 눈으로 확인.**
4. **persist 대기**: 12초로 부족할 수 있음 → **수십 초~수 분** 기다리거나, Teams가 idle될 때 flush되는지 관찰. (async 지연 가능성)
5. **애프터 측정**: 같은 threadid 인덱스 재카운트. **가장 오래된 timestamp가 더 과거로 내려갔거나 개수가 늘면 → 전제 성립.** 그대로면 → Teams가 스크롤 히스토리를 로컬에 안 쓰는 것(전제 실패, 이 경로 폐기하고 토큰+내부 API 검토).

> 카운트/매핑 스크립트 참고: `db44/store1` 레코드 값 = `[seq, {value:[threadid, "ts_ms"]}]`. `datetime.fromtimestamp(int(ts)/1000)`로 날짜화. 전/후 스냅샷은 leveldb 디렉토리를 복사(LOCK 삭제) 후 비교.

## 8. 구현 설계 (전제 성립 시)

별도 MCP 도구로, 예: `collect_history(chat, until_date?)`.

수집 루프:
```
loop:
  스크롤휠 버스트 주입 (§3, 커서 warp+restore)
  persist 대기 (충분히)
  IndexedDB 재읽기 → 대상 threadid의 최소 timestamp 확인
  if 최소 timestamp가 안 내려감(연속 N회): break   # 맨 위/벽 도달
  if until_date 도달: break
```

구현 주의:
- **충돌 방지(idle 가드)**: 커서가 ~0.1s 왕복하므로, **최근 사용자 입력이 있으면 스킵**. macOS `CGEventSourceSecondsSinceLastEventType`로 idle 시간 확인 → 일정 시간 이상 idle일 때만 버스트.
- **좌표 재계산**: 매 버스트 전 창 위치/크기 재조회(창 이동/리사이즈 대비).
- **Teams 상태 의존**: 대상 채팅이 열려 있어야 하고, 최소화되면 렌더/fetch 안 될 수 있음.
- **종료 판정**: 인덱스 최소 timestamp 정체를 "벽"으로 간주. 단 벽이 권한 경계일 수 있음(그 이상 불가).
- **Rust에서 스크롤**: `core-graphics` 크레이트의 `CGEvent`(scroll wheel) + `CGWarpMouseCursorPosition`, 또는 위 swift 스니펫을 헬퍼 바이너리로.

## 9. 대안 (스크롤 수집이 최종 실패할 경우)

전제가 최종적으로 실패(=Teams가 스크롤 히스토리를 로컬에 안 씀)로 확인되면, 캐시에 없는 과거를 얻는 유일한 길은 **토큰 + 내부 API replay**다:
- 로컬(Local Storage/IndexedDB)에서 세션 토큰(AAD bearer + skypetoken) 추출 → Teams 메시징 백엔드에 과거 페이지 직접 요청.
- 장점: 렌더/스크롤 무관, 임의 범위 fetch, 서버가 정제된 JSON 제공(파싱 쉬움).
- 단점: **위조 트래픽 발생**, 토큰 수명 관리, 내부 API 역공학.
- 이건 별도 결정 사항. 스크롤 수집이 "정상 클라이언트 동작"이라는 유일한 이점이었으므로, 그게 안 되면 토큰 방식의 부담을 감수할지부터 사용자와 재논의.
