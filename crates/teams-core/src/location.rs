//! Teams v2 의 로컬 데이터 위치 (IndexedDB leveldb · Cookies · Local State).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// EBWebView 프로파일 루트까지의 공통 꼬리. macOS·Windows 모두 동일 구조.
/// (`/` 는 Windows `Path` 에서도 구분자로 인식되므로 그대로 push 가능.)
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PROFILE_TAIL: &str = "Microsoft/MSTeams/EBWebView/WV2Profile_tfw";

/// 프로파일 루트 이하 IndexedDB 디렉터리.
const IDB_DIR: &str = "IndexedDB";

/// IndexedDB 아래 Teams 오리진별 leveldb 디렉터리 이름. MS 가 `teams.microsoft.com` →
/// `teams.cloud.microsoft` 로 오리진을 옮기는 중이라 **한 프로파일에 둘이 공존**할 수 있고,
/// 그중 하나는 마이그레이션 후 버려진 stale 디렉터리다. 그래서 이름을 고정하지 않고
/// 후보를 훑어 **가장 최근에 쓰인**(=실사용 중인) 쪽을 고른다.
const ORIGIN_DIRS: [&str; 2] = [
    "https_teams.cloud.microsoft_0.indexeddb.leveldb",
    "https_teams.microsoft.com_0.indexeddb.leveldb",
];

/// 기본 DB 경로. `TEAMS_MCP_DB` 환경변수로 override 가능.
///
/// - macOS: `$HOME/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/` + 꼬리
/// - Windows: `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\` + 꼬리
/// - Linux: 네이티브 Teams 2 가 없어 Chromium 계열 껍데기(PWA·teams-for-linux)를 쓴다 →
///   후보 프로파일들을 훑어 DB 가 실제로 있는 것을 고른다 (`linux_profile_candidates`).
/// - 그 외: `None` (`TEAMS_MCP_DB` 로만 지정 가능).
///
/// 프로파일이 정해지면 그 안에서 오리진(`ORIGIN_DIRS`)까지 골라야 최종 경로가 된다.
pub fn default_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_DB") {
        return Some(PathBuf::from(p));
    }
    let root = platform_profile_root()?;
    // 오리진이 하나도 없으면(미설치·미로그인) 표준 경로를 그대로 돌려준다 —
    // 호출측이 "DB 디렉토리 없음: <경로>" 로 무엇을 찾았는지 보여줄 수 있게.
    Some(
        newest_db_in(&root)
            .map(|(_, db)| db)
            .unwrap_or_else(|| root.join(IDB_DIR).join(ORIGIN_DIRS[1])),
    )
}

/// 프로파일 루트 아래 실제로 존재하는 Teams leveldb 중 가장 최근에 쓰인 것과 그 시각.
fn newest_db_in(root: &Path) -> Option<(SystemTime, PathBuf)> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for origin in ORIGIN_DIRS {
        let dir = root.join(IDB_DIR).join(origin);
        if !dir.is_dir() {
            continue;
        }
        let Some(mtime) = newest_mtime(&dir) else {
            continue;
        };
        // 동률이면 앞선 후보(신규 오리진)를 유지한다.
        if best.as_ref().is_none_or(|(best_t, _)| mtime > *best_t) {
            best = Some((mtime, dir));
        }
    }
    best
}

/// leveldb 디렉터리가 마지막으로 쓰인 시각(= 안쪽 파일들의 최신 mtime). 활성도 지표.
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let own = dir.metadata().ok()?.modified().ok();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .chain(own)
        .max()
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
/// 프로파일을 고른다. 여러 껍데기에 Teams 를 띄워봤으면 DB 도 여러 개 남으므로,
/// 오리진과 같은 기준으로 **가장 최근에 쓰인** 프로파일을 고른다.
#[cfg(target_os = "linux")]
fn platform_profile_root() -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for cand in linux_profile_candidates() {
        let Some((mtime, _)) = newest_db_in(&cand) else {
            continue;
        };
        if best.as_ref().is_none_or(|(best_t, _)| mtime > *best_t) {
            best = Some((mtime, cand));
        }
    }
    best.map(|(_, root)| root)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 오리진이 둘 다 남아 있으면 **최근에 쓰인 쪽**을 고르는지. (마이그레이션으로 버려진
    /// stale 오리진이 후보 목록에서 앞에 있어도 실사용 중인 쪽이 이겨야 한다.)
    #[test]
    fn picks_the_origin_that_was_written_most_recently() {
        let root = std::env::temp_dir().join("teams-mcp-origin-test");
        let _ = std::fs::remove_dir_all(&root);
        let idb = root.join(IDB_DIR);

        // 후보 목록상 앞이지만 버려진 오리진.
        let stale = idb.join(ORIGIN_DIRS[0]);
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("CURRENT"), b"stale").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));

        // 실사용 중인 오리진.
        let active = idb.join(ORIGIN_DIRS[1]);
        std::fs::create_dir_all(&active).unwrap();
        std::fs::write(active.join("CURRENT"), b"active").unwrap();

        assert_eq!(newest_db_in(&root).map(|(_, db)| db), Some(active));

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 후보 중 **DB 가 실제로 있는** 프로파일만 골라내는지. (여러 Chromium 앱·프로파일이
    /// 깔려 있어도 Teams DB 를 가진 하나를 찾아야 한다.)
    #[cfg(target_os = "linux")]
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
        let db = real.join(IDB_DIR).join(ORIGIN_DIRS[1]);
        std::fs::create_dir_all(&db).unwrap();

        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("TEAMS_MCP_DB");

        assert_eq!(default_db_path(), Some(db));

        std::fs::remove_dir_all(&home).unwrap();
    }
}
