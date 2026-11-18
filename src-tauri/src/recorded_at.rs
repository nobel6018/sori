//! 가져온 파일의 실제 녹음 시각을 찾는다. 가져온 시각(createdAt)과 다르다
//! @see docs/recorded-at.md

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 파일 이름을 먼저 보고, 없으면 mp4 헤더를 본다.
/// 파일 이름은 녹음기가 시작할 때 붙인 시각이라 가장 정확하다
pub fn detect(file_name: &str, audio: &Path) -> Option<DateTime<Utc>> {
    from_file_name(file_name, &Local).or_else(|| from_mp4(audio)).filter(|t| plausible(*t))
}

fn plausible(t: DateTime<Utc>) -> bool {
    t.timestamp() >= 946_684_800 && t <= Utc::now() + chrono::Duration::days(1)
}

#[derive(Debug)]
struct Group<'a> {
    digits: &'a str,
    /// 앞 숫자 묶음과의 사이 글자
    before: &'a str,
    /// 이름 안에서 숫자가 끝나는 위치
    end: usize,
}

fn groups(name: &str) -> Vec<Group<'_>> {
    let mut out = Vec::new();
    let mut last_end = 0;
    let mut start = None;
    for (i, c) in name.char_indices().chain(std::iter::once((name.len(), ' '))) {
        match (c.is_ascii_digit(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(Group { digits: &name[s..i], before: &name[last_end..s], end: i });
                last_end = i;
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn num(s: &str) -> u32 {
    s.parse().unwrap_or(u32::MAX)
}

/// 날짜와 시각 사이, 시각 칸 사이에 올 수 있는 글자. 길면 다른 숫자로 본다
fn short_sep(sep: &str) -> bool {
    let t = sep.trim();
    sep.chars().count() <= 4 && (t.is_empty() || matches!(t, "-" | "_" | "." | ":" | "T" | "at" | "h" | "m" | "오전" | "오후"))
}

/// 날짜 묶음을 i에서 읽고 (날짜, 다음 묶음 위치)를 돌려준다
fn read_date(g: &[Group], i: usize) -> Option<(NaiveDate, usize)> {
    let d = g[i].digits;
    let date = |y: i32, m: &str, dd: &str| NaiveDate::from_ymd_opt(y, num(m), num(dd));
    match d.len() {
        8 | 12 | 14 => date(num(&d[..4]) as i32, &d[4..6], &d[6..8]).map(|x| (x, i + 1)),
        // 갤럭시 녹음기: 음성 261106_142115. 시각 묶음이 바로 붙어 있을 때만 날짜로 본다
        6 if g.get(i + 1).is_some_and(|n| n.digits.len() == 6 && n.before == "_") => {
            date(2000 + num(&d[..2]) as i32, &d[2..4], &d[4..6]).map(|x| (x, i + 1))
        }
        4 if g.len() > i + 2
            && g[i + 1].digits.len() == 2
            && g[i + 2].digits.len() == 2
            && [&g[i + 1], &g[i + 2]].iter().all(|n| matches!(n.before, "-" | "." | "_")) =>
        {
            date(num(d) as i32, g[i + 1].digits, g[i + 2].digits).map(|x| (x, i + 3))
        }
        _ => None,
    }
}

/// 시, 분, 초. 초가 없으면 0
fn read_time(g: &[Group], i: usize, inline: &str) -> Option<(u32, u32, u32, usize)> {
    // 14자리, 12자리 한 덩어리(20261106142115)는 날짜 뒤에 시각이 붙어 있다
    if inline.len() == 6 {
        return Some((num(&inline[..2]), num(&inline[2..4]), num(&inline[4..6]), i));
    }
    if inline.len() == 4 {
        return Some((num(&inline[..2]), num(&inline[2..4]), 0, i));
    }
    let first = g.get(i).filter(|n| short_sep(n.before))?;
    let d = first.digits;
    match d.len() {
        6 => Some((num(&d[..2]), num(&d[2..4]), num(&d[4..6]), i + 1)),
        4 => Some((num(&d[..2]), num(&d[2..4]), 0, i + 1)),
        1 | 2 => {
            let min = g.get(i + 1).filter(|n| n.digits.len() == 2 && short_sep(n.before))?;
            let sec = g.get(i + 2).filter(|n| n.digits.len() == 2 && matches!(n.before, "-" | "." | ":" | "m"));
            Some((num(d), num(min.digits), sec.map_or(0, |s| num(s.digits)), if sec.is_some() { i + 3 } else { i + 2 }))
        }
        _ => None,
    }
}

/// 파일 이름 속 날짜와 시각. 시각까지 있어야 쓴다(날짜만 있으면 몇 시인지 모른다).
/// 이름의 시각은 그 기기의 현지 시각으로 보고, GMT가 앞에 붙으면(Zoom) UTC로 본다
pub fn from_file_name<Tz: TimeZone>(name: &str, tz: &Tz) -> Option<DateTime<Utc>> {
    let g = groups(name);
    for i in 0..g.len() {
        let Some((date, next)) = read_date(&g, i) else { continue };
        let inline = g[i].digits.get(8..).unwrap_or("");
        let Some((mut h, m, s, end)) = read_time(&g, next, inline) else { continue };

        // 12시간제: "오후 2.30.15", "2.30.15 PM"
        let time_end = if end > next { g[end - 1].end } else { g[i].end };
        let before = if end > next { g[next].before } else { "" };
        let after = name[time_end..].trim_start().to_uppercase();
        if (before.contains("오후") || after.starts_with("PM")) && h < 12 {
            h += 12;
        } else if (before.contains("오전") || after.starts_with("AM")) && h == 12 {
            h = 0;
        }
        let Some(naive) = date.and_hms_opt(h, m, s) else { continue };
        if g[i].before.ends_with("GMT") || g[i].before.ends_with("UTC") {
            return Some(Utc.from_utc_datetime(&naive));
        }
        return local(naive, tz);
    }
    None
}

fn local<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc))
}

/// mp4(m4a, mov) 헤더 mvhd의 만든 시각에서 길이를 뺀다.
/// 녹음기들은 파일을 닫을 때 이 시각을 적어서 녹음이 끝난 시각에 가깝다(갤럭시 녹음기로 실측)
pub fn from_mp4(path: &Path) -> Option<DateTime<Utc>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let moov = find_box(&mut f, 0, len, b"moov")?;
    let mvhd = find_box(&mut f, moov.0, moov.1, b"mvhd")?;
    f.seek(SeekFrom::Start(mvhd.0)).ok()?;
    let mut head = [0u8; 4];
    f.read_exact(&mut head).ok()?;
    let (created, timescale, duration) = if head[0] == 1 {
        let mut b = [0u8; 28];
        f.read_exact(&mut b).ok()?;
        (u64::from_be_bytes(b[..8].try_into().ok()?), u32::from_be_bytes(b[16..20].try_into().ok()?), u64::from_be_bytes(b[20..28].try_into().ok()?))
    } else {
        let mut b = [0u8; 16];
        f.read_exact(&mut b).ok()?;
        (
            u32::from_be_bytes(b[..4].try_into().ok()?) as u64,
            u32::from_be_bytes(b[8..12].try_into().ok()?),
            u32::from_be_bytes(b[12..16].try_into().ok()?) as u64,
        )
    };
    // 1904-01-01부터 센 초
    let secs = created.checked_sub(2_082_844_800)? as i64;
    let length = if timescale > 0 { (duration / timescale as u64) as i64 } else { 0 };
    DateTime::from_timestamp(secs - length, 0)
}

/// [start, end) 안에서 이름이 kind인 상자를 찾아 (내용 시작, 내용 끝)을 돌려준다
fn find_box(f: &mut File, start: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    for _ in 0..64 {
        if pos + 8 > end {
            return None;
        }
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 8];
        f.read_exact(&mut h).ok()?;
        let mut size = u32::from_be_bytes(h[..4].try_into().ok()?) as u64;
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            f.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < header {
            return None;
        }
        if &h[4..8] == kind {
            return Some((pos + header, (pos + size).min(end)));
        }
        pos += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn kst() -> FixedOffset {
        FixedOffset::east_opt(9 * 3600).unwrap()
    }

    fn at(name: &str) -> Option<String> {
        from_file_name(name, &kst()).map(|t| t.with_timezone(&kst()).format("%Y-%m-%d %H:%M:%S").to_string())
    }

    #[test]
    fn 갤럭시_녹음기() {
        assert_eq!(at("음성 261106_142115.m4a").as_deref(), Some("2026-11-06 14:21:15"));
        assert_eq!(at("통화 녹음 홍길동_261106_142115.m4a").as_deref(), Some("2026-11-06 14:21:15"));
    }

    #[test]
    fn 붙여_쓴_날짜와_시각() {
        assert_eq!(at("Recording_20261030_143015.m4a").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("20261030 143015-ABCD1234.m4a").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("REC20261030143015.wav").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("202610301430.mp3").as_deref(), Some("2026-10-30 14:30:00"));
    }

    #[test]
    fn 나눠_쓴_날짜와_시각() {
        assert_eq!(at("2026-10-30 14-30-15.mkv").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("회의 2026.10.30 14.30.m4a").as_deref(), Some("2026-10-30 14:30:00"));
        assert_eq!(at("2026-10-30_143015.wav").as_deref(), Some("2026-10-30 14:30:15"));
    }

    #[test]
    fn 열두시간제() {
        assert_eq!(at("Screen Recording 2026-10-30 at 2.30.15 PM.mov").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("화면 기록 2026-10-30 오후 2.30.15.mov").as_deref(), Some("2026-10-30 14:30:15"));
        assert_eq!(at("화면 기록 2026-10-30 오전 12.05.00.mov").as_deref(), Some("2026-10-30 00:05:00"));
    }

    #[test]
    fn zoom은_utc() {
        assert_eq!(at("GMT20261030-053015_Recording.m4a").as_deref(), Some("2026-10-30 14:30:15"));
    }

    #[test]
    fn 시각이_없으면_모른다() {
        assert_eq!(at("Yuhua East Road.m4a"), None);
        assert_eq!(at("새로운 녹음 12.m4a"), None);
        assert_eq!(at("2026-10-30 회의.m4a"), None);
        assert_eq!(at("track 01 123456.mp3"), None);
        assert_eq!(at("20261399_999999.m4a"), None);
    }

    #[test]
    fn mp4_헤더는_끝난_시각에서_길이를_뺀다() {
        // ftyp + moov(mvhd v0): 만든 시각 2026-11-16T08:15:07Z, 1000 단위로 1839.616초
        let created = (DateTime::parse_from_rfc3339("2026-11-16T08:15:07Z").unwrap().timestamp() + 2_082_844_800) as u32;
        let mut mvhd = vec![0u8; 4];
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(1000u32.to_be_bytes());
        mvhd.extend(1_839_616u32.to_be_bytes());
        let boxed = |kind: &[u8], body: &[u8]| [((body.len() + 8) as u32).to_be_bytes().as_slice(), kind, body].concat();
        let file = [boxed(b"ftyp", b"M4A \0\0\0\0"), boxed(b"moov", &boxed(b"mvhd", &mvhd))].concat();

        let path = std::env::temp_dir().join(format!("sori-mvhd-{}.m4a", std::process::id()));
        std::fs::write(&path, file).unwrap();
        let t = from_mp4(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(t.unwrap().to_rfc3339(), "2026-11-16T07:44:28+00:00");
    }
}
