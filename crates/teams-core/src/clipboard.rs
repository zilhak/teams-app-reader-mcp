//! 클립보드 리치 텍스트 복사.
//!
//! 이 서버는 Teams 에 메시지를 **보내지 않는다**. 대신 사용자가 Teams 입력창에
//! 그대로 붙여넣을 수 있는 형태까지 만들어 클립보드에 올려준다 — 이 모듈이 그
//! 경로다. Teams 데이터에 대한 쓰기가 아니라 로컬 OS 클립보드 쓰기다.
//!
//! 클립보드에는 같은 내용이 두 가지 표현(flavor)으로 올라간다:
//! - **HTML** (macOS `public.html`, Windows `CF_HTML`, Linux `text/html`) — 링크(`<a href>`)·불릿
//!   (`<ul>/<li>`)·강조가 살아있는 표현. 서식 붙여넣기(Cmd+V) 시 Teams 가 이걸 쓴다.
//! - **평문** (macOS `public.utf8-plain-text`, Windows `CF_UNICODETEXT`, Linux `text/plain`) — 여기에도
//!   **HTML 원문(태그 포함)을 그대로** 넣는다.
//!
//! 평문에 태그째로 넣는 이유: 이 도구의 붙여넣기 대상은 서식을 받는 Teams 입력창이라
//! 평문 표현이 실제로 쓰이는 경로가 아니다. 그렇다고 평문을 비워두면(arboard 는
//! `alt_text: None` 시 평문 flavor 를 아예 안 만든다) `pbpaste` 가 빈 출력이 되어
//! **클립보드에 뭐가 올라갔는지 확인할 방법이 사라진다.** HTML 원문을 넣어두면
//! `pbpaste` 로 검증이 되고, 서식 없이 붙여넣기를 하면 태그가 그대로 보이므로
//! 잘못 붙여넣었다는 것도 즉시 드러난다.
//!
//! 클립보드는 사용자의 전역 상태다 — 기존 복사 내용을 덮어쓰므로, 사용자가 명시적으로
//! 요청했을 때만 호출한다.

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("클립보드를 열지 못함: {0}")]
    Open(String),
    #[error("클립보드 쓰기 실패: {0}")]
    Write(String),
}

/// HTML 을 클립보드에 올린다 (HTML flavor + 같은 문자열의 평문 flavor).
pub fn copy_html(html: &str) -> Result<(), ClipboardError> {
    with_clipboard(|cb| {
        cb.set()
            .html(html, Some(html))
            .map_err(|e| ClipboardError::Write(e.to_string()))
    })?
}

/// macOS·Windows 는 복사한 내용을 OS 가 들고 있으므로 매번 새 인스턴스로 충분하다.
#[cfg(not(target_os = "linux"))]
fn with_clipboard<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Result<R, ClipboardError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ClipboardError::Open(e.to_string()))?;
    Ok(f(&mut clipboard))
}

/// 리눅스(X11·Wayland)는 클립보드 내용을 OS 가 아니라 **소유 프로세스**가 요청받을 때마다
/// 내어준다. 인스턴스를 드롭하면 붙여넣을 내용도 같이 사라지므로, 하나를 만들어 프로세스
/// 수명 동안 살려둔다. (그래도 서버가 종료되면, 데스크톱의 클립보드 매니저가 내용을
/// 받아가지 않는 한 클립보드는 비워진다 — 붙여넣기는 서버가 떠 있는 동안 하면 된다.)
#[cfg(target_os = "linux")]
fn with_clipboard<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Result<R, ClipboardError> {
    use std::sync::{Mutex, OnceLock};
    static CLIPBOARD: OnceLock<Mutex<arboard::Clipboard>> = OnceLock::new();

    if CLIPBOARD.get().is_none() {
        let clipboard =
            arboard::Clipboard::new().map_err(|e| ClipboardError::Open(e.to_string()))?;
        // 경합에서 진 쪽의 인스턴스는 그냥 버려진다 (이긴 쪽 하나만 살아있으면 된다).
        let _ = CLIPBOARD.set(Mutex::new(clipboard));
    }
    let mut clipboard = CLIPBOARD
        .get()
        .expect("바로 위에서 초기화됨")
        .lock()
        .map_err(|e| ClipboardError::Open(e.to_string()))?;
    Ok(f(&mut clipboard))
}
