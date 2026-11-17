//! Finds people's traces in pasted text: emails, phones, domains, IPs, @nicks, crypto wallets.

use crate::investigation::Kind;
use regex::Regex;

pub struct Found {
    pub kind: Kind,
    pub value: String,
    pub what: &'static str,
}

pub fn find(text: &str) -> Vec<Found> {
    let mut out: Vec<Found> = vec![];
    let mut push = |kind, value: String, what| {
        let v = value.trim_matches(|c: char| ".,;:!?)(\"'«»".contains(c)).to_string();
        if !v.is_empty() && !out.iter().any(|f| f.value.eq_ignore_ascii_case(&v)) {
            out.push(Found { kind, value: v, what });
        }
    };
    let email = Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap();
    let mut emails = vec![];
    for m in email.find_iter(text) {
        emails.push(m.range());
        push(Kind::Email, m.as_str().to_string(), "почта");
    }
    let in_email = |r: std::ops::Range<usize>| emails.iter().any(|e| e.start <= r.start && r.end <= e.end);

    let url = Regex::new(r"(?i)\bhttps?://([A-Za-z0-9.\-]+\.[A-Za-z]{2,})[^\s]*").unwrap();
    let mut urls = vec![];
    for c in url.captures_iter(text) {
        urls.push(c.get(0).unwrap().range());
        push(Kind::Domain, c[1].to_lowercase().trim_start_matches("www.").to_string(), "домен из ссылки");
    }
    let in_url = |r: &std::ops::Range<usize>| urls.iter().any(|u| u.start <= r.start && r.end <= u.end);

    let ip = Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap();
    let mut ips = vec![];
    for m in ip.find_iter(text) {
        ips.push(m.range());
        push(Kind::Other, m.as_str().to_string(), "IP-адрес");
    }
    let domain = Regex::new(r"(?i)\b(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+(?:com|net|org|ru|рф|io|me|info|biz|su|ua|by|kz|de|uk|co|app|dev|xyz|online|site|tech|pro|cc|tv|to|onion)\b").unwrap();
    for m in domain.find_iter(text) {
        let r = m.range();
        if !in_email(r.clone()) && !in_url(&r) && !ips.iter().any(|i| i.start <= r.start && r.end <= i.end) {
            push(Kind::Domain, m.as_str().to_lowercase().trim_start_matches("www.").to_string(), "домен");
        }
    }
    let phone = Regex::new(r"(?:\+\d{1,3}[\s\-]?)?\(?\d{3}\)?[\s\-]?\d{3}[\s\-]?\d{2}[\s\-]?\d{2}\b").unwrap();
    for m in phone.find_iter(text) {
        let r = m.range();
        if in_url(&r) || ips.iter().any(|i| i.start <= r.end && r.start <= i.end) {
            continue;
        }
        let digits: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
        if (10..=13).contains(&digits.len()) {
            push(Kind::Phone, m.as_str().trim().to_string(), "телефон");
        }
    }
    let nick = Regex::new(r"(?:^|[\s(])@([A-Za-z0-9_.]{3,32})").unwrap();
    for c in nick.captures_iter(text) {
        let r = c.get(0).unwrap().range();
        if !in_email(r) {
            push(Kind::Nick, format!("@{}", c[1].trim_end_matches('.')), "никнейм");
        }
    }
    let btc = Regex::new(r"\b(?:bc1[a-z0-9]{25,59}|[13][a-km-zA-HJ-NP-Z1-9]{25,34})\b").unwrap();
    for m in btc.find_iter(text) {
        push(Kind::Other, m.as_str().to_string(), "кошелёк BTC");
    }
    let eth = Regex::new(r"\b0x[a-fA-F0-9]{40}\b").unwrap();
    for m in eth.find_iter(text) {
        push(Kind::Other, m.as_str().to_string(), "кошелёк ETH");
    }
    out
}

/// Ready-made search links for an entity: (label, url). They open in the browser.
pub fn search_links(kind: Kind, value: &str) -> Vec<(&'static str, String)> {
    let v = value.trim();
    if v.is_empty() {
        return vec![];
    }
    let q = enc(v);
    let quoted = enc(&format!("\"{v}\""));
    let bare = v.trim_start_matches('@');
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut l = vec![("Google", format!("https://www.google.com/search?q={quoted}")), ("Яндекс", format!("https://yandex.ru/search/?text={quoted}"))];
    match kind {
        Kind::Nick => l.extend([
            ("WhatsMyName", format!("https://whatsmyname.app/?q={}", enc(bare))),
            ("GitHub", format!("https://github.com/{}", enc(bare))),
            ("Telegram", format!("https://t.me/{}", enc(bare))),
            ("X", format!("https://x.com/{}", enc(bare))),
            ("Instagram", format!("https://www.instagram.com/{}/", enc(bare))),
            ("VK", format!("https://vk.com/{}", enc(bare))),
        ]),
        Kind::Email => l.extend([
            ("Have I Been Pwned", format!("https://haveibeenpwned.com/account/{q}")),
            ("Epieos", format!("https://epieos.com/?q={q}")),
            ("Домен почты", format!("https://who.is/whois/{}", enc(v.split('@').nth(1).unwrap_or("")))),
        ]),
        Kind::Phone => l.extend([
            ("WhatsApp", format!("https://wa.me/{digits}")),
            ("Telegram", format!("https://t.me/+{digits}")),
        ]),
        Kind::Domain => l.extend([
            ("WHOIS", format!("https://who.is/whois/{q}")),
            ("Сертификаты", format!("https://crt.sh/?q={q}")),
            ("Архив", format!("https://web.archive.org/web/*/{q}")),
            ("DNS", format!("https://securitytrails.com/domain/{q}/dns")),
            ("VirusTotal", format!("https://www.virustotal.com/gui/domain/{q}")),
        ]),
        Kind::Place => l.extend([
            ("OpenStreetMap", format!("https://www.openstreetmap.org/search?query={q}")),
            ("Google Карты", format!("https://www.google.com/maps/search/{q}")),
        ]),
        Kind::Person | Kind::Org | Kind::Other => l.push(("DuckDuckGo", format!("https://duckduckgo.com/?q={quoted}"))),
    }
    l
}

pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// "55.75, 37.62" or "55.75 37.62" → (lat, lon).
pub fn coords(s: &str) -> Option<(f64, f64)> {
    let re = Regex::new(r"(-?\d{1,2}(?:\.\d+)?)\s*[,;\s]\s*(-?\d{1,3}(?:\.\d+)?)").unwrap();
    let c = re.captures(s)?;
    let (lat, lon): (f64, f64) = (c[1].parse().ok()?, c[2].parse().ok()?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_traces() {
        let f = find("Пишите на fox.mail@example.org или +7 (900) 123-45-67, сайт https://www.secure-login.example.com/x, @dark_fox, сервер 185.22.10.7, кошелёк 0x52908400098527886E0F7030069857D2E4169EE7");
        let has = |k: Kind, v: &str| f.iter().any(|x| x.kind == k && x.value == v);
        assert!(has(Kind::Email, "fox.mail@example.org"));
        assert!(has(Kind::Phone, "+7 (900) 123-45-67"));
        assert!(has(Kind::Domain, "secure-login.example.com"));
        assert!(has(Kind::Nick, "@dark_fox"));
        assert!(has(Kind::Other, "185.22.10.7"));
        assert!(f.iter().any(|x| x.what == "кошелёк ETH"));
        assert_eq!(coords("55.7558, 37.6173"), Some((55.7558, 37.6173)));
    }
}
