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
/// - Linux: 네이티브 Teams 2 가 없어 Chromium 계열 껍데기(PWA·teams-for-linux)를 쓴다 →
///   후보 프로파일들을 훑어 DB 가 실제로 있는 것을 고른다 (`linux_profile_candidates`).
/// - 그 외: `None` (`TEAMS_MCP_DB` 로만 지정 가능).
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

/// 리눅스에는 신 Teams 2 네이티브 클라이언트가 없다. 대신 같은 웹 Teams 를 감싼 Chromium
/// 계열 껍데기 두 종류로 쓴다 — 공식 PWA(Edge·Chrome·Chromium)와 비공식 teams-for-linux
/// (Electron). 껍데기가 무엇이든 저장 포맷은 동일한 IndexedDB(LevelDB)라 리더는 그대로
/// 통하고, 다른 것은 프로파일 경로뿐이다. 그래서 후보를 훑어 **DB 가 실제로 존재하는**
/// 프로파일을 고른다.
#[cfg(target_os = "linux")]
fn platform_profile_root() -> Option<PathBuf> {
    linux_profile_candidates()
        .into_iter()
        .find(|p| p.join(DB_TAIL).exists())
}

/// 리눅스 프로파일 후보. `<config 루트>/<앱>` 아래에서
/// 앱 디렉터리 자신(Electron 기본 세션) · 직속 하위(브라우저 `Default`·`Profile N`) ·
/// `Partitions/*`(Electron 파티션) 를 모은다.
#[cfg(target_os = "linux")]
fn linux_profile_candidates() -> Vec<PathBuf> {
    // (config 하위 앱 디렉터리 이름, snap 이름, flatpak app id)
    const APPS: [(&str, &str, &str); 4] = [
        (
            "teams-for-linux",
            "teams-for-linux",
            "com.github.IsmaelMartinez.teams_for_linux",
        ),
        ("microsoft-edge", "microsoft-edge", "com.microsoft.Edge"),
        ("google-chrome", "google-chrome", "com.google.Chrome"),
        ("chromium", "chromium", "org.chromium.Chromium"),
    ];

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    let mut out = Vec::new();
    for (app, snap, flatpak) in APPS {
        // deb/tarball · snap · flatpak 은 config 루트만 다르고 그 아래 구조는 같다.
        for app_dir in [
            xdg_config.join(app),
            home.join("snap").join(snap).join("current/.config").join(app),
            home.join(".var/app").join(flatpak).join("config").join(app),
        ] {
            push_subdirs(&app_dir.join("Partitions"), &mut out);
            push_subdirs(&app_dir, &mut out);
            out.push(app_dir);
        }
    }
    out
}

/// `dir` 의 직속 하위 디렉터리를 이름순(결정적)으로 `out` 에 넣는다.
#[cfg(target_os = "linux")]
fn push_subdirs(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    subdirs.sort();
    out.extend(subdirs);
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_profile_root() -> Option<PathBuf> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// 후보 중 **DB 가 실제로 있는** 프로파일만 골라내는지. (여러 Chromium 앱·프로파일이
    /// 깔려 있어도 Teams DB 를 가진 하나를 찾아야 한다.)
    #[test]
    fn linux_picks_profile_that_actually_has_the_db() {
        let home = std::env::temp_dir().join("teams-mcp-loc-test");
        let _ = std::fs::remove_dir_all(&home);

        // Teams DB 가 없는 미끼: Edge 의 두 프로파일.
        for p in ["Default", "Profile 1"] {
            std::fs::create_dir_all(home.join(".config/microsoft-edge").join(p)).unwrap();
        }
        // 진짜: teams-for-linux 의 Electron 파티션.
        let real = home.join(".config/teams-for-linux/Partitions/teams-4-linux");
        std::fs::create_dir_all(real.join(DB_TAIL)).unwrap();

        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("TEAMS_MCP_DB");

        assert_eq!(default_db_path(), Some(real.join(DB_TAIL)));

        std::fs::remove_dir_all(&home).unwrap();
    }
}
