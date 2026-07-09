# Teams 로컬 저장 구조 완전 분석

Teams v2(신 Teams, macOS)가 메시지를 로컬에 어떻게 저장하는지, 그리고 그것을 어떻게 디코드하는지에 대한 조사 결과. **Rust IndexedDB 리더 구현의 청사진.**

---

## 1. 파일 위치

Teams v2는 내부적으로 **Microsoft Edge WebView2** (Chromium 기반) 위에서 돈다. 데이터는 WebView2의 프로파일 디렉토리에 있다.

```
~/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/
  EBWebView/
    WV2Profile_tfw/
      IndexedDB/
        https_teams.microsoft.com_0.indexeddb.leveldb/   ← 메시지/대화 데이터 (핵심)
        https_teams.microsoft.com_0.indexeddb.blob/       ← 첨부 blob
        https_innogridoffice-my.sharepoint.com_0.indexeddb.leveldb/  (SharePoint 관련, 별개)
      Local Storage/
```

`https_teams.microsoft.com_0.indexeddb.leveldb/` 안에는 표준 LevelDB 파일들:
- `NNNNNN.ldb` — SSTable (immutable, 컴팩션으로 번호 바뀜)
- `NNNNNN.log` — 활성 write-ahead log (최신 데이터, memtable)
- `CURRENT`, `MANIFEST-*`, `LOCK`, `LOG`

**주의**: Teams 실행 중엔 `LOCK` 파일로 DB가 잠긴다. 읽으려면:
- 파일들을 다른 곳으로 **복사 후** 읽거나 (복사본의 LOCK 삭제),
- immutable `.ldb` SSTable을 직접 파싱 (락 무관). `.log`의 최신분은 별도 처리.

---

## 2. LevelDB 계층의 함정: 커스텀 comparator

Chromium IndexedDB는 **커스텀 키 comparator `idb_cmp1`** 로 SSTable을 정렬한다. 표준 bytewise comparator가 아니다.

- 결과: 범용 Rust LevelDB 리더(`rusty-leveldb` 등)로 열면 **정렬 검증 assertion에서 패닉**한다.
  ```
  thread 'main' panicked at rusty-leveldb/src/version_set.rs:798: assertion `left == right` failed: left: Greater, right: Less
  ```
- 해결 방법 (택1):
  1. **`idb_cmp1` comparator를 구현**해서 리더에 주입 (Chromium `content/browser/indexed_db/indexed_db_leveldb_coding.cc` 참조).
  2. **SSTable(.ldb)을 직접 파싱**하고 정렬을 무시 — 우리는 전수 스캔만 하므로 정렬이 필요 없다. LevelDB table 포맷(footer→index block→data blocks, 블록별 snappy)만 파싱하면 됨. 이게 가장 견고. `.log`(write-ahead log)는 별도 record 포맷.
- **snappy 압축**: 각 데이터 블록은 snappy 압축될 수 있다. 리더가 블록 단위로 해제해야 한다. (raw grep으로 문자열이 잡히기도/안 잡히기도 하는 이유 = 블록별 압축 여부 차이.)

> PoC에서는 Rust 대신 Python 도구 **`dfindexeddb`**(Google 포렌식 툴)를 LevelDB 리더로 썼다. idb_cmp1 + snappy를 처리해준다. 설치: `pip install dfindexeddb` (libsnappy 필요: `brew install snappy` 후 `CPPFLAGS=-I/opt/homebrew/include LDFLAGS=-L/opt/homebrew/lib`).
> **단, dfindexeddb의 값 디코더(blink 레이어)는 Teams 레코드에서 `Unsupported header`로 실패**한다. LevelDB 읽기 용도로만 쓰고, 값 디코드는 아래 우리 V8 파서로 직접 한다.

---

## 3. IndexedDB 논리 구조

하나의 leveldb 안에 **여러 개의 IndexedDB 데이터베이스**가 들어있다 (`database_id` 0, 13, 32, 44, ...). 각 db는 여러 object store를 가진다.

조사로 파악한 주요 store:

| database_id / store | 내용 | 값 형태 |
|---|---|---|
| **db0 / store0** | **대화(thread) 레코드.** topic(채팅명), threadProperties, memberProperties, **lastMessage**(발신자·본문·시간) 포함 | V8 (blink 봉투) |
| **db44 / store1** | **메시지 인덱스.** 메시지 1건당 `IndexDataKey` 1개 + `ExistsEntryKey`. 값 = `[seq, IDBKey([threadid, "timestamp_ms"])]` | dfindexeddb가 list로 디코드 |
| db44 / store0 | db44 메타데이터 (ObjectStoreMetaDataKey 등) | 메타 |
| db13, db32 등 | 사람/검색 등 보조 인덱스 | 다양 |

**메시지 본문 객체(`ObjectStoreDataKey`)의 정확한 위치는 미완 조사**다. db44/store1에는 인덱스(IndexDataKey)와 존재표시(ExistsEntryKey)만 있고, 실제 본문 객체 레코드는 dfindexeddb 크래시로 전수 추출이 안 됐다. 인덱스가 수천 개 존재하는 것으로 보아 본문도 로컬에 있으며, db0/store0의 `lastMessage`처럼 V8로 인코딩돼 있을 것이다. **Rust 리더 구현 시 첫 과제 = 개별 메시지 본문 레코드의 store와 키 구조 확정.**

### 캐시 규모 (실측)

`db44/store1` 인덱스의 timestamp를 스레드별로 집계한 결과 (한 시점 스냅샷):

| threadid (일부) | 캐시된 메시지 수 | 가장 오래된 ~ 최신 |
|---|---|---|
| 19:2141...bce8 | 1665 | 2026-04-10 ~ 2026-07-09 |
| 19:edcb...786f | 1448 | 2026-06-10 ~ 2026-07-09 |
| 19:51e5...5754 | 872 | **2025-11-05** ~ 2026-07-09 |

→ Teams는 **채팅별로 수백~1600건, 수개월치 히스토리를 이미 로컬에 캐싱**하고 있다. "읽기 전용 MCP"는 이걸 읽으면 된다.

---

## 4. 값 인코딩: 표준 V8 ValueSerializer

**핵심 발견: Teams 값은 커스텀 포맷이 아니라 Chromium 표준 V8 ValueSerializer(structured clone) 이다.** (한때 Teams 자체 포맷으로 오해했으나, 표준이었음.)

leveldb의 값 바이트 레이아웃 (db0/store0 대화 레코드 기준):

```
0x0A <varint len> <object-store 키 (UTF-16LE, 예 "-19:...@thread.v2")>   ← protobuf field 1
0x12 <varint len>                                                       ← protobuf field 2 (값)
  0xA5 0x06                                                             ← (wrapper/길이류)
  0xFF 0x15  0xFE 0x00...0x00                                           ← blink 봉투 (version 0x15=21) + 패딩
  0xFF 0x10                                                             ← V8 version 16 태그
  o ... {                                                               ← V8 ValueSerializer payload (여기부터 파싱)
```

**파싱 전략**: 바이트에서 **마지막 `0xFF <version>` 뒤**를 V8 payload 시작으로 본다 (blink 봉투를 건너뜀). 그 지점부터 아래 태그 문법으로 파싱.

### V8 ValueSerializer 태그 (필요한 것만)

| 태그(바이트) | 의미 | 페이로드 |
|---|---|---|
| `0xFF` | version | 다음 1바이트 version (스킵) |
| `0x00` | padding | 없음 (스킵) |
| `T` / `F` | true / false | 없음 |
| `0` (0x30) | null | 없음 |
| `_` (0x5f) | undefined | 없음 |
| `I` (0x49) | int32 | zigzag varint |
| `U` (0x55) | uint32 | varint |
| `N` (0x4e) | double | 8바이트 LE |
| `"` (0x22) | one-byte string | varint 길이 + Latin1 바이트 |
| `c` (0x63) | two-byte string | varint 길이(바이트) + UTF-16LE |
| `S` (0x53) | utf8 string | varint 길이 + UTF-8 |
| `o` (0x6f) | begin object | `{` 나올 때까지 (key,value) 쌍 반복 |
| `{` (0x7b) | end object | varint 프로퍼티 수 |
| `A` (0x41) | begin dense array | varint 길이 ... `$` 종료 |
| `$` (0x24) | end dense array | varint props, varint length |
| `a` (0x61) | begin sparse array | ... `@` 종료 |
| `^` (0x5e) | object reference | varint id |
| `D` (0x44) | date | 8바이트 double |

### 참조 구현 (Python PoC, 검증 완료)

이 파서로 실제 대화 레코드를 완전 복원했다 (topic "[TechTalk] PaaS, DevOps 기술 논의", lastMessage.content "넵 SRE팀에서 패치 파일 공유주시면...", imdisplayname "황윤재" 등, 한글 포함 정확).

```python
import struct

class V8Reader:
    def __init__(self, buf): self.b, self.i = buf, 0
    def varint(self):
        r=s=0
        while True:
            x=self.b[self.i]; self.i+=1; r|=(x&0x7f)<<s
            if not (x&0x80): return r
            s+=7
    def zigzag(self):
        n=self.varint(); return (n>>1)^-(n&1)
    def read_value(self):
        c=chr(self.b[self.i]); self.i+=1
        if c=='\x00': return self.read_value()          # padding
        if c=='T': return True
        if c=='F': return False
        if c in ('0','_'): return None                  # null / undefined
        if c=='I': return self.zigzag()
        if c=='U': return self.varint()
        if c=='N':
            v=struct.unpack_from('<d',self.b,self.i)[0]; self.i+=8; return v
        if c=='"':
            n=self.varint(); s=self.b[self.i:self.i+n].decode('latin1'); self.i+=n; return s
        if c=='c':
            n=self.varint(); s=self.b[self.i:self.i+n].decode('utf-16-le','replace'); self.i+=n; return s
        if c=='S':
            n=self.varint(); s=self.b[self.i:self.i+n].decode('utf-8','replace'); self.i+=n; return s
        if c=='o':
            obj={}
            while chr(self.b[self.i])!='{':
                k=self.read_value(); v=self.read_value(); obj[k]=v
            self.i+=1; self.varint(); return obj
        if c=='A':
            self.varint(); arr=[]
            while chr(self.b[self.i])!='$':
                arr.append(self.read_value())
            self.i+=1; self.varint(); self.varint(); return arr
        if c=='a':
            self.varint(); obj={}
            while chr(self.b[self.i])!='@':
                k=self.read_value(); v=self.read_value(); obj[k]=v
            self.i+=1; self.varint(); self.varint(); return obj
        if c=='^': return {'__ref__': self.varint()}
        if c=='D':
            v=struct.unpack_from('<d',self.b,self.i)[0]; self.i+=8; return {'__date__':v}
        raise ValueError(f"unknown tag 0x{ord(c):02x} at {self.i-1}")

def find_v8_start(buf):
    # 마지막 0xFF <version> 뒤부터 V8 payload
    ffs=[i for i,b in enumerate(buf) if b==0xFF]
    return (ffs[-1]+2) if ffs else 0
```

### 디코드된 대화 레코드 예시 (구조)

```json
{
  "type": "Chat",
  "threadProperties": { "topic": "[TechTalk] PaaS, DevOps 기술 논의", "threadType": "chat", "createdat": "1742365449530", ... },
  "memberProperties": { "role": "Admin" },
  "lastMessage": {
    "content": "넵 SRE팀에서 패치 파일 공유주시면 같이 반입하도록 하겠습니다",
    "messagetype": "RichText/Html",
    "imdisplayname": "황윤재",           // 발신자 표시명
    "clientmessageid": "3308258671224430311",
    "sequenceId": 2502
  }
}
```

- **threadid**는 값의 맨 앞(protobuf field1, UTF-16LE)에 있음: `19:...@thread.v2` 또는 `-19:...` 형태.
- 메시지 인덱스(db44/store1)의 timestamp는 ms epoch 문자열. `datetime.fromtimestamp(int(ts)/1000)`.

---

## 5. Rust 구현 시 할 일 (요약 체크리스트)

1. LevelDB 접근: **SSTable 직접 파싱**(정렬 무시 전수 스캔) 또는 idb_cmp1 comparator 구현. `.ldb` + `.log` 둘 다. 블록 snappy 해제.
2. 각 레코드 값에서 `find_v8_start` → **V8 ValueSerializer 디코더**(위 참조 구현을 Rust로 포팅. ~60줄).
3. **개별 메시지 본문 store 확정** (db44의 ObjectStoreDataKey 등) — 미완 조사 항목.
4. 스키마 매핑: 대화(topic/threadid) + 메시지(content/imdisplayname/originalarrivaltime 등) → MCP가 반환할 구조.
5. 락 회피: 실행 중 Teams의 `.ldb`는 immutable이라 직접 읽기 가능. `.log`는 복사 후 읽거나 별도 처리.
6. Rust 크레이트 후보: snappy(`snap`), varint 직접 구현, SSTable 파서(직접 or 포크).

## 참고: PoC 스크립트 위치

PoC는 세션 scratchpad(휘발성)에서 진행됨. 핵심 코드는 위에 박제해둠. 재현이 필요하면:
- `dfindexeddb db -s <leveldb복사본> -f chromium -o jsonl` 로 레코드 덤프 (값은 디코드 실패 시 `\xHH` 이스케이프 문자열로 나옴 → un-escape 후 위 V8Reader로 파싱).
