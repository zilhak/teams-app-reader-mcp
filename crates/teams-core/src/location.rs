//! Teams v2 의 로컬 데이터 위치 (IndexedDB leveldb · Cookies · Local State).

use std::path::PathBuf;

/// EBWebView 프로파일 루트까지의 공통 꼬리. macOS·Windows 모두 동일 구조.
/// (`/` 는 Windows `Path` 에서도 구분자로 인식되므로 그대로 push 가능.)
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PROFILE_TAIL: &str = "Microsoft/MSTeams/EBWebView/WV2Profile_tfw";

/// 프로파일 루트 이하 IndexedDB(leveldb) 경로.
const DB_TAIL: &str = "IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb";

/// 기본 DB 경로. `TEAMS_MCP_DB` 환경변수로 override 가능.
///
/// - macOS: `$HOME/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/` + 꼬리
/// - Windows: `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\` + 꼬리
/// - 그 외(Linux 등): 신 Teams 2 네이티브 클라이언트가 없어 로컬 캐시 자체가 없음 → `None`
///   (`TEAMS_MCP_DB` 로만 지정 가능).
pub fn default_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_DB") {
        return Some(PathBuf::from(p));
    }
    platform_profile_root().map(|mut p| {
        p.push(DB_TAIL);
        p
    })
}

/// 기본 Cookies(SQLite) 경로. `TEAMS_MCP_COOKIES` 로 override 가능.
/// 신 Chromium 은 `Network/Cookies`, 구버전은 프로파일 루트 바로 아래 `Cookies` 에 둔다 — 둘 다 시도.
pub fn default_cookies_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_COOKIES") {
        return Some(PathBuf::from(p));
    }
    let root = platform_profile_root()?;
    let network = root.join("Network/Cookies");
    if network.exists() {
        return Some(network);
    }
    Some(root.join("Cookies"))
}

/// (Windows 전용) 쿠키 암호화 키가 담긴 `Local State` 경로. EBWebView 루트 바로 아래.
#[cfg(target_os = "windows")]
pub fn default_local_state_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_LOCAL_STATE") {
        return Some(PathBuf::from(p));
    }
    // PROFILE_TAIL 의 마지막 세그먼트(WV2Profile_tfw)를 떼고 그 부모(EBWebView)에 Local State.
    let mut root = platform_profile_root()?;
    root.pop(); // WV2Profile_tfw 제거 → EBWebView
    Some(root.join("Local State"))
}

/// 플랫폼별 EBWebView 프로파일 루트(`.../WV2Profile_tfw`).
#[cfg(target_os = "macos")]
fn platform_profile_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push("Library/Containers/com.microsoft.teams2/Data/Library/Application Support");
    p.push(PROFILE_TAIL);
    Some(p)
}

#[cfg(target_os = "windows")]
fn platform_profile_root() -> Option<PathBuf> {
    // `MSTeams_8wekyb3d8bbwe` 의 `8wekyb3d8bbwe` 는 MS Store publisher ID(고정값).
    let local = std::env::var_os("LOCALAPPDATA")?;
    let mut p = PathBuf::from(local);
    p.push("Packages/MSTeams_8wekyb3d8bbwe/LocalCache");
    p.push(PROFILE_TAIL);
    Some(p)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_profile_root() -> Option<PathBuf> {
    None
}
