//! Teams v2 의 로컬 데이터 위치 (IndexedDB leveldb · Cookies · Local State).

use std::path::PathBuf;

/// EBWebView 프로파일 루트까지의 공통 꼬리. macOS·Windows 모두 동일 구조.
/// (`/` 는 Windows `Path` 에서도 구분자로 인식되므로 그대로 push 가능.)
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PROFILE_TAIL: &str = "Microsoft/MSTeams/EBWebView/WV2Profile_tfw";

/// 프로파일 루트 이하 IndexedDB(leveldb) 경로. (macOS·Windows 전용; Linux 는 오리진을 탐색한다.)
#[cfg(any(target_os = "macos", target_os = "windows"))]
const DB_TAIL: &str = "IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb";

/// 기본 DB 경로. `TEAMS_MCP_DB` 환경변수로 override 가능.
///
/// - macOS: `$HOME/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/` + 꼬리
/// - Windows: `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\` + 꼬리
/// - Linux: 공식 신 Teams 2 네이티브 클라이언트는 없지만, 비공식 `teams-for-linux`(Electron)
///   가 웹앱과 동일한 IndexedDB 를 캐싱한다. 설치 방식(AppImage/deb·Snap·Flatpak)과
///   세션 파티션·오리진을 훑어 자동 탐색한다. (없으면 `None` → `TEAMS_MCP_DB` 로만 지정 가능.)
pub fn default_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_DB") {
        return Some(PathBuf::from(p));
    }
    platform_default_db_path()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn platform_default_db_path() -> Option<PathBuf> {
    platform_profile_root().map(|mut p| {
        p.push(DB_TAIL);
        p
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_default_db_path() -> Option<PathBuf> {
    linux_default_db_path()
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
    // Linux 는 단일 프로파일 루트가 없다(설치 방식·파티션마다 다름).
    // DB 는 linux_default_db_path() 가 별도 탐색한다.
    None
}

/// Teams 웹앱 오리진별 leveldb 디렉토리명. 신규 오리진(teams.cloud.microsoft) 우선.
/// MS 가 오리진을 이전 중이라 구 `teams.microsoft.com` 이 stale 상태로 함께 남아 있을 수 있다.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const LINUX_ORIGIN_DIRS: [&str; 2] = [
    "https_teams.cloud.microsoft_0.indexeddb.leveldb",
    "https_teams.microsoft.com_0.indexeddb.leveldb",
];

/// Linux: `teams-for-linux` 의 IndexedDB 를 자동 탐색한다.
///
/// 설치 방식마다 config 루트가, 버전/마이그레이션마다 파티션명·오리진이 달라지므로
/// 경로를 고정하지 않는다. 후보를 전부 모아 **가장 최근에 수정된**(=실사용 중인) leveldb 를
/// 고른다. 이렇게 하면 stale/빈 오리진(예전 `teams.microsoft.com` 등)이 남아 있어도
/// 항상 최신 데이터를 안전하게 선택한다.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn linux_default_db_path() -> Option<PathBuf> {
    linux_leveldb_candidates()
        .into_iter()
        .filter_map(|dir| newest_mtime(&dir).map(|t| (t, dir)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, dir)| dir)
}

/// 설치 방식별 `teams-for-linux` config 루트 후보 (AppImage/deb·Snap·Flatpak).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn linux_config_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = Vec::new();
    // AppImage/deb/tar: $XDG_CONFIG_HOME 또는 ~/.config
    if let Some(cfg) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")))
    {
        roots.push(cfg.join("teams-for-linux"));
    }
    if let Some(h) = home {
        // Snap: 홈 컨테이너 안의 .config
        roots.push(h.join("snap/teams-for-linux/current/.config/teams-for-linux"));
        // Flatpak
        roots.push(h.join(".var/app/com.github.IsmaelMartinez.teams_for_linux/config/teams-for-linux"));
    }
    roots
}

/// 존재하는 (IndexedDB 디렉토리 × 오리진) leveldb 디렉토리 전부를 모은다.
/// IndexedDB 는 config 루트 바로 아래 또는 `Partitions/<name>/` 아래에 있을 수 있다.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn linux_leveldb_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in linux_config_roots() {
        let mut idb_dirs: Vec<PathBuf> = vec![root.join("IndexedDB")];
        if let Ok(entries) = std::fs::read_dir(root.join("Partitions")) {
            for e in entries.flatten() {
                idb_dirs.push(e.path().join("IndexedDB"));
            }
        }
        for idb in idb_dirs {
            for origin in LINUX_ORIGIN_DIRS {
                let p = idb.join(origin);
                if p.is_dir() {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// leveldb 디렉토리 안 파일들의 가장 최근 수정 시각(활성도 지표).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn newest_mtime(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .max()
}
