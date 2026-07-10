//! 작은 유틸: epoch(ms) → UTC 문자열, HTML → 평문.

/// ms epoch → "YYYY-MM-DD HH:MM:SS" (UTC). 음수/0 은 빈 문자열.
pub fn format_epoch_ms(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// days since 1970-01-01 → (year, month, day). Howard Hinnant 알고리즘.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// HTML 조각을 사람이 읽을 평문으로. 태그 제거 + 기본 엔티티/숫자 엔티티 디코드 + 공백 정리.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut in_tag = false;
    while let Some(c) = chars.next() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            '&' => {
                // 엔티티 수집
                let mut ent = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc == ';' {
                        chars.next();
                        break;
                    }
                    if ent.len() > 10 || nc == '<' || nc == '&' {
                        break;
                    }
                    ent.push(nc);
                    chars.next();
                }
                out.push_str(&decode_entity(&ent));
            }
            _ => out.push(c),
        }
    }
    // 공백 정리: 연속 공백/개행을 단일 공백으로
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entity(ent: &str) -> String {
    match ent {
        "nbsp" => " ".to_string(),
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" | "#39" => "'".to_string(),
        _ => {
            if let Some(num) = ent.strip_prefix('#') {
                let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                if let Some(ch) = code.and_then(char::from_u32) {
                    return ch.to_string();
                }
            }
            format!("&{ent};") // 모르면 원문 유지
        }
    }
}
