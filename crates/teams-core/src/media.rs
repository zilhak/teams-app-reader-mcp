//! AMS(Async Media Service) 이미지 fetch.
//!
//! Teams 메시지의 이미지는 본문에 원격 URL(`*.asyncgw.teams.microsoft.com/v1/objects/...`)
//! 참조로만 있고, 실제 바이트는 인증(쿠키)이 걸린 CDN 에 있다. 이 모듈은
//! **로컬 Teams 쿠키를 복호화**해 그 URL 을 인증된 상태로 GET 한다.
//!
//! - 쿠키 저장소: EBWebView 프로파일의 `Cookies`(SQLite). 값은 Chromium `os_crypt` 로 암호화됨.
//! - macOS: 키체인 "Microsoft Teams Safe Storage" → PBKDF2(saltysalt,1003)-AES128-CBC.
//! - Windows: `Local State` 의 DPAPI 마스터키 → AES256-GCM.
//!
//! 이 모듈만 네트워크를 쓴다 (나머지 서버는 순수 로컬 읽기). `fetch_image` 도구에서만 호출된다.

use std::io::Read;

use crate::location;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("허용되지 않은 호스트 (asyncgw AMS URL 만 가능): {0}")]
    ForbiddenHost(String),
    #[error("Teams 쿠키 DB 를 찾을 수 없음")]
    NoCookiesDb,
    #[error("쿠키 DB 읽기 실패: {0}")]
    Cookies(String),
    #[error("복호화 키 획득 실패: {0}")]
    Key(String),
    #[error("asyncgw 인증 쿠키를 복호화하지 못함 (Teams 로그인 상태 확인)")]
    Decrypt,
    #[error("이미지 요청 실패: {0}")]
    Http(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// AMS 이미지 URL 을 인증 쿠키를 붙여 GET 한다. `(바이트, MIME)` 반환.
/// URL 은 `*.asyncgw.teams.microsoft.com` 호스트만 허용(SSRF 방지).
pub fn fetch_ams_image(url: &str) -> Result<(Vec<u8>, String), MediaError> {
    let host = host_of(url).ok_or_else(|| MediaError::ForbiddenHost(url.to_string()))?;
    if !host.ends_with(".asyncgw.teams.microsoft.com") {
        return Err(MediaError::ForbiddenHost(host.to_string()));
    }

    let cookies = load_asyncgw_cookies()?;
    if cookies.is_empty() {
        return Err(MediaError::Decrypt);
    }
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let resp = ureq::get(url)
        .set("Cookie", &cookie_header)
        .set("User-Agent", "Mozilla/5.0")
        .call()
        .map_err(|e| MediaError::Http(e.to_string()))?;

    let mime = resp
        .header("Content-Type")
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();

    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf)?;
    Ok((buf, mime))
}

/// `Cookies` SQLite 에서 asyncgw 도메인 쿠키를 읽어 복호화한다.
/// DB 잠금을 피하려고 임시 파일로 복사 후 읽는다.
fn load_asyncgw_cookies() -> Result<Vec<(String, String)>, MediaError> {
    let path = location::default_cookies_path().ok_or(MediaError::NoCookiesDb)?;
    if !path.exists() {
        return Err(MediaError::NoCookiesDb);
    }
    let tmp = std::env::temp_dir().join("teams-mcp-cookies.tmp");
    std::fs::copy(&path, &tmp)?;

    let conn = rusqlite::Connection::open_with_flags(
        &tmp,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| MediaError::Cookies(e.to_string()))?;

    let mut stmt = conn
        .prepare("SELECT name, encrypted_value FROM cookies WHERE host_key LIKE '%asyncgw%'")
        .map_err(|e| MediaError::Cookies(e.to_string()))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| MediaError::Cookies(e.to_string()))?;

    let cipher = CookieCipher::load()?;
    let mut out = Vec::new();
    for row in rows {
        let (name, enc) = row.map_err(|e| MediaError::Cookies(e.to_string()))?;
        if let Some(val) = cipher.decrypt(&enc) {
            out.push((name, val));
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(out)
}

/// 복호화된 쿠키 평문에서 실제 값 문자열을 얻는다.
/// 신 Chromium 은 평문 앞 32바이트에 `sha256(도메인)` 을 붙인다 — 붙어있으면 떼어낸다.
fn plaintext_to_value(pt: &[u8]) -> Option<String> {
    // 접두어가 없으면 전체가 유효 UTF-8. 접두어(이진 해시)가 있으면 전체 UTF-8 디코드가 실패한다.
    if let Ok(s) = std::str::from_utf8(pt) {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if pt.len() > 32 {
        if let Ok(s) = std::str::from_utf8(&pt[32..]) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// URL 의 호스트명 추출 (URL 크레이트 없이).
fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next()?;
    let hostport = authority.rsplit('@').next()?; // userinfo 제거
    hostport.split(':').next() // 포트 제거
}

// ---- 플랫폼별 쿠키 복호화 -----------------------------------------------------

#[cfg(target_os = "macos")]
struct CookieCipher {
    aes_key: [u8; 16],
}

#[cfg(target_os = "macos")]
impl CookieCipher {
    fn load() -> Result<Self, MediaError> {
        let pw = macos_safe_storage_key()?;
        let mut aes_key = [0u8; 16];
        pbkdf2_sha1(&pw, b"saltysalt", 1003, &mut aes_key);
        Ok(Self { aes_key })
    }

    fn decrypt(&self, enc: &[u8]) -> Option<String> {
        use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
        let ct = enc.strip_prefix(b"v10")?; // macOS(Edge WebView2) = v10
        let iv = [0x20u8; 16];
        let pt = cbc::Decryptor::<aes::Aes128>::new(self.aes_key.as_slice().into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(ct)
            .ok()?;
        plaintext_to_value(&pt)
    }
}

/// 키체인에서 Chromium `os_crypt` 마스터 비밀번호를 읽는다 (`security` CLI).
#[cfg(target_os = "macos")]
fn macos_safe_storage_key() -> Result<Vec<u8>, MediaError> {
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            "Microsoft Teams Safe Storage",
        ])
        .output()
        .map_err(|e| MediaError::Key(e.to_string()))?;
    if !out.status.success() {
        return Err(MediaError::Key(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let mut pw = out.stdout;
    while matches!(pw.last(), Some(b'\n') | Some(b'\r')) {
        pw.pop();
    }
    Ok(pw)
}

/// PBKDF2-HMAC-SHA1. dkLen(16) ≤ 20 이라 1블록이지만 일반 루프로 둔다.
#[cfg(target_os = "macos")]
fn pbkdf2_sha1(pw: &[u8], salt: &[u8], iters: u32, out: &mut [u8]) {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    type HmacSha1 = Hmac<Sha1>;

    let mut block_index: u32 = 1;
    let mut pos = 0;
    while pos < out.len() {
        let mut u = {
            let mut mac = HmacSha1::new_from_slice(pw).expect("hmac key");
            mac.update(salt);
            mac.update(&block_index.to_be_bytes());
            mac.finalize().into_bytes()
        };
        let mut t = u;
        for _ in 1..iters {
            let mut mac = HmacSha1::new_from_slice(pw).expect("hmac key");
            mac.update(&u);
            u = mac.finalize().into_bytes();
            for (a, b) in t.iter_mut().zip(u.iter()) {
                *a ^= *b;
            }
        }
        let n = (out.len() - pos).min(t.len());
        out[pos..pos + n].copy_from_slice(&t[..n]);
        pos += n;
        block_index += 1;
    }
}

#[cfg(target_os = "windows")]
struct CookieCipher {
    master_key: Vec<u8>,
}

#[cfg(target_os = "windows")]
impl CookieCipher {
    fn load() -> Result<Self, MediaError> {
        let master_key = windows_master_key()?;
        Ok(Self { master_key })
    }

    fn decrypt(&self, enc: &[u8]) -> Option<String> {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        // 신 형식: v10/v11 = AES-256-GCM ([3 prefix][12 nonce][ct||16 tag]).
        if enc.starts_with(b"v10") || enc.starts_with(b"v11") {
            if enc.len() < 3 + 12 + 16 {
                return None;
            }
            let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.master_key));
            let nonce = Nonce::from_slice(&enc[3..15]);
            let pt = cipher.decrypt(nonce, &enc[15..]).ok()?;
            return plaintext_to_value(&pt);
        }
        // 구 형식: DPAPI 로 직접 암호화된 값.
        let pt = dpapi_unprotect(enc).ok()?;
        plaintext_to_value(&pt)
    }
}

/// `Local State` 의 `os_crypt.encrypted_key` 를 DPAPI 로 풀어 AES-GCM 마스터키를 얻는다.
#[cfg(target_os = "windows")]
fn windows_master_key() -> Result<Vec<u8>, MediaError> {
    use base64::Engine;
    let path = location::default_local_state_path().ok_or(MediaError::NoCookiesDb)?;
    let text = std::fs::read_to_string(&path)?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| MediaError::Key(e.to_string()))?;
    let b64 = json
        .get("os_crypt")
        .and_then(|v| v.get("encrypted_key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| MediaError::Key("os_crypt.encrypted_key 없음".into()))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| MediaError::Key(e.to_string()))?;
    // 앞 5바이트 "DPAPI" 접두어 제거 후 DPAPI 복호화.
    let enc_key = raw
        .strip_prefix(b"DPAPI")
        .ok_or_else(|| MediaError::Key("DPAPI 접두어 없음".into()))?;
    dpapi_unprotect(enc_key).map_err(|e| MediaError::Key(e.to_string()))
}

/// Windows DPAPI `CryptUnprotectData` 래퍼.
#[cfg(target_os = "windows")]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    use windows_sys::Win32::Foundation::LocalFree;

    let mut in_blob = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: 유효한 입력 blob 을 넘기고, 성공 시 out_blob.pbData 를 LocalFree 로 해제한다.
    let ok = unsafe {
        CryptUnprotectData(
            &mut in_blob,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut out_blob,
        )
    };
    if ok == 0 {
        return Err("CryptUnprotectData 실패".into());
    }
    let out = unsafe {
        std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec()
    };
    unsafe { LocalFree(out_blob.pbData as *mut _) };
    Ok(out)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct CookieCipher;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl CookieCipher {
    fn load() -> Result<Self, MediaError> {
        Err(MediaError::Key("지원하지 않는 플랫폼".into()))
    }
    fn decrypt(&self, _enc: &[u8]) -> Option<String> {
        None
    }
}
