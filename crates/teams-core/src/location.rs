//! Teams v2 의 IndexedDB(leveldb) 디렉토리 위치.

use std::path::PathBuf;

const REL_PATH: &str = "Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb";

/// 기본 macOS 경로. `$HOME` 기준. `TEAMS_MCP_DB` 환경변수로 override 가능.
pub fn default_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TEAMS_MCP_DB") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(REL_PATH);
    Some(p)
}
