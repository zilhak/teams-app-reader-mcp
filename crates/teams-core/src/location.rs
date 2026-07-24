//! Teams v2 의 IndexedDB(leveldb) 디렉토리 위치.

use std::path::PathBuf;

/// OS별 컨테이너 루트 이하 공통 꼬리. macOS·Windows 모두
/// `Microsoft/MSTeams/EBWebView/WV2Profile_tfw/...` 구조가 동일하다.
/// (`/` 는 Windows `Path` 에서도 구분자로 인식되므로 그대로 push 가능.)
#[cfg(any(target_os = "macos", target_os = "windows"))]
const REL_TAIL: &str =
    "Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb";

/// 기본 DB 경로. `TEAMS_MCP_DB` 환경변수로 override 가능.
///
/// - macOS: `$HOME/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/` + 공통 꼬리
/// - Windows: `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\` + 공통 꼬리
/// - 그 외(Linux 등): 신 Teams 2 네이티브 클라이언트가 없어 로컬 캐시 자체가 없음 → `None`
///   (`TEAMS_MCP_DB` 로만 지정 가능).
pub fn default_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_DB") {
        return Some(PathBuf::from(p));
    }
    platform_default_db_path()
}

#[cfg(target_os = "macos")]
fn platform_default_db_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push("Library/Containers/com.microsoft.teams2/Data/Library/Application Support");
    p.push(REL_TAIL);
    Some(p)
}

#[cfg(target_os = "windows")]
fn platform_default_db_path() -> Option<PathBuf> {
    // `MSTeams_8wekyb3d8bbwe` 의 `8wekyb3d8bbwe` 는 MS Store publisher ID(고정값).
    let local = std::env::var_os("LOCALAPPDATA")?;
    let mut p = PathBuf::from(local);
    p.push("Packages/MSTeams_8wekyb3d8bbwe/LocalCache");
    p.push(REL_TAIL);
    Some(p)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_default_db_path() -> Option<PathBuf> {
    None
}
