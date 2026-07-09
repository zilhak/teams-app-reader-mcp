//! macOS 접근성(AXUIElement) 조회를 osascript로 감싼 읽기 전용 헬퍼.
//! Microsoft Teams(신 Teams 2, 프로세스명 "MSTeams") 창을 대상으로 한다.

use std::process::Command;

/// osascript를 실행하고 표준출력을 문자열로 반환한다.
fn run_osascript(script: &str) -> anyhow::Result<String> {
    let output = Command::new("osascript").arg("-e").arg(script).output()?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("osascript 실패: {}", err.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

/// 현재 Teams 창의 제목(어느 채팅/화면이 열려 있는지).
pub fn window_title() -> anyhow::Result<String> {
    run_osascript(
        r#"tell application "System Events" to tell process "MSTeams" to get name of window 1"#,
    )
}

/// 현재 Teams 창에 보이는 모든 정적 텍스트를 위→아래 순서로 개행 구분해 반환한다.
/// (메시지 본문·발신자·시간, 좌측 채팅 목록 등이 렌더 순서대로 섞여 나온다)
pub fn read_static_texts() -> anyhow::Result<String> {
    let script = r#"tell application "System Events" to tell process "MSTeams"
set allElems to entire contents of window 1
set out to ""
repeat with e in allElems
if role of e is "AXStaticText" then
set v to ""
try
set v to value of e as string
end try
if v is not "" then set out to out & v & linefeed
end if
end repeat
return out
end tell"#;
    run_osascript(script)
}
