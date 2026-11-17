//! Automatic checks for an entity against open sources. Every check runs only when
//! the person presses the button; results come back one by one through a channel.

use crate::investigation::Kind;
use serde_json::Value;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) QuietNote/0.5";

#[derive(Clone, Debug)]
pub enum Add {
    /// A profile or page that confirms something: becomes a source.
    Source { url: String, note: String },
    /// A fact about the entity: becomes a field.
    Field { key: String, value: String },
    /// Something new: becomes a linked entity.
    Entity { kind: Kind, name: String, relation: String },
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub check: &'static str,
    pub text: String,
    pub add: Option<Add>,
}

fn get(url: &str) -> Result<ureq::Response, ureq::Error> {
    ureq::get(url).set("User-Agent", UA).timeout(Duration::from_secs(12)).call()
}

fn json(url: &str) -> Option<Value> {
    get(url).ok()?.into_json().ok()
}

/// HTTP status, or None if unreachable.
fn status(url: &str) -> Option<u16> {
    match get(url) {
        Ok(r) => Some(r.status()),
        Err(ureq::Error::Status(c, _)) => Some(c),
        Err(_) => None,
    }
}

fn body(url: &str) -> Option<String> {
    get(url).ok()?.into_string().ok()
}

/// Which checks exist for an entity; empty means the button is hidden.
pub fn supported(kind: Kind, value: &str) -> bool {
    match kind {
        Kind::Nick | Kind::Email | Kind::Domain | Kind::Phone => true,
        Kind::Other => is_ip(value),
        _ => false,
    }
}

fn is_ip(v: &str) -> bool {
    v.parse::<std::net::IpAddr>().is_ok()
}

/// Runs all checks for an entity; `emit` is called for each finding as it arrives.
/// Checks run in parallel threads; the call returns when all are done.
pub fn run(kind: Kind, value: &str, emit: impl Fn(Finding) + Send + Sync + Clone + 'static) {
    let v = value.trim().to_string();
    let mut tasks: Vec<Box<dyn FnOnce() -> Vec<Finding> + Send>> = vec![];
    match kind {
        Kind::Nick => {
            let nick = v.trim_start_matches('@').to_string();
            for site in SITES {
                let n = nick.clone();
                tasks.push(Box::new(move || check_site(site, &n).into_iter().collect()));
            }
        }
        Kind::Email => {
            let e = v.to_lowercase();
            let e2 = e.clone();
            tasks.push(Box::new(move || gravatar(&e)));
            if let Some(dom) = e2.split('@').nth(1).map(str::to_string) {
                tasks.push(Box::new(move || mail_domain(&dom)));
            }
        }
        Kind::Domain => {
            let d = v.to_lowercase().trim_start_matches("www.").to_string();
            let (d1, d2, d3, d4) = (d.clone(), d.clone(), d.clone(), d.clone());
            tasks.push(Box::new(move || dns(&d1)));
            tasks.push(Box::new(move || rdap(&d2)));
            tasks.push(Box::new(move || subdomains(&d3)));
            tasks.push(Box::new(move || wayback(&d4)));
        }
        Kind::Phone => tasks.push(Box::new(move || phone(&v))),
        Kind::Other if is_ip(&v) => tasks.push(Box::new(move || ip_info(&v))),
        _ => {}
    }
    let handles: Vec<_> = tasks
        .into_iter()
        .map(|t| {
            let emit = emit.clone();
            std::thread::spawn(move || {
                for f in t() {
                    emit(f);
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
}

// ------------------------------------------------------------------ nicknames

enum Probe {
    /// 200 means found, 404 means not.
    Status,
    /// JSON body: found unless it is null/empty or has an error.
    Json,
    /// Page text contains this marker when the profile exists.
    Contains(&'static str),
    /// Page text contains this marker when the profile does not exist.
    Missing(&'static str),
}

struct Site {
    name: &'static str,
    /// What to request; `{}` is the nickname.
    check: &'static str,
    /// What to show and save; `{}` is the nickname.
    profile: &'static str,
    probe: Probe,
}

const SITES: &[Site] = &[
    Site { name: "GitHub", check: "https://api.github.com/users/{}", profile: "https://github.com/{}", probe: Probe::Status },
    Site { name: "GitLab", check: "https://gitlab.com/api/v4/users?username={}", profile: "https://gitlab.com/{}", probe: Probe::Json },
    Site { name: "Reddit", check: "https://www.reddit.com/user/{}/about.json", profile: "https://www.reddit.com/user/{}", probe: Probe::Status },
    Site { name: "Telegram", check: "https://t.me/{}", profile: "https://t.me/{}", probe: Probe::Contains("tgme_page_title") },
    Site { name: "Keybase", check: "https://keybase.io/_/api/1.0/user/lookup.json?usernames={}", profile: "https://keybase.io/{}", probe: Probe::Contains("\"them\":[{") },
    Site { name: "Hacker News", check: "https://hacker-news.firebaseio.com/v0/user/{}.json", profile: "https://news.ycombinator.com/user?id={}", probe: Probe::Json },
    Site { name: "Dev.to", check: "https://dev.to/api/users/by_username?url={}", profile: "https://dev.to/{}", probe: Probe::Status },
    Site { name: "Docker Hub", check: "https://hub.docker.com/v2/users/{}/", profile: "https://hub.docker.com/u/{}", probe: Probe::Status },
    Site { name: "npm", check: "https://www.npmjs.com/~{}", profile: "https://www.npmjs.com/~{}", probe: Probe::Status },
    Site { name: "Codeberg", check: "https://codeberg.org/api/v1/users/{}", profile: "https://codeberg.org/{}", probe: Probe::Status },
    Site { name: "Chess.com", check: "https://api.chess.com/pub/player/{}", profile: "https://www.chess.com/member/{}", probe: Probe::Status },
    Site { name: "Lichess", check: "https://lichess.org/api/user/{}", profile: "https://lichess.org/@/{}", probe: Probe::Status },
    Site { name: "Steam", check: "https://steamcommunity.com/id/{}", profile: "https://steamcommunity.com/id/{}", probe: Probe::Missing("The specified profile could not be found") },
    Site { name: "Хабр", check: "https://habr.com/ru/users/{}/", profile: "https://habr.com/ru/users/{}/", probe: Probe::Status },
    Site { name: "Pikabu", check: "https://pikabu.ru/@{}", profile: "https://pikabu.ru/@{}", probe: Probe::Status },
    Site { name: "Linktree", check: "https://linktr.ee/{}", profile: "https://linktr.ee/{}", probe: Probe::Status },
];

fn check_site(s: &Site, nick: &str) -> Option<Finding> {
    let n = crate::extract::enc(nick);
    let url = s.check.replace("{}", &n);
    let found = match s.probe {
        Probe::Status => status(&url)? == 200,
        Probe::Json => json(&url).is_some_and(|v| !(v.is_null() || v.as_array().is_some_and(|a| a.is_empty()) || v.get("error").is_some())),
        Probe::Contains(m) => body(&url)?.contains(m),
        Probe::Missing(m) => !body(&url)?.contains(m),
    };
    let profile = s.profile.replace("{}", &n);
    found.then(|| Finding {
        check: s.name,
        text: format!("Найден профиль: {profile}"),
        add: Some(Add::Source { url: profile, note: format!("профиль на {} (автопроверка)", s.name) }),
    })
}

// ------------------------------------------------------------------ email

fn gravatar(email: &str) -> Vec<Finding> {
    let hash = format!("{:x}", md5::compute(email.as_bytes()));
    let Some(v) = json(&format!("https://en.gravatar.com/{hash}.json")) else {
        return vec![];
    };
    let Some(e) = v["entry"].get(0) else { return vec![] };
    let mut out = vec![Finding {
        check: "Gravatar",
        text: format!("Есть профиль Gravatar: {}", e["profileUrl"].as_str().unwrap_or("")),
        add: e["profileUrl"].as_str().map(|u| Add::Source { url: u.into(), note: "профиль Gravatar (автопроверка)".into() }),
    }];
    if let Some(n) = e["displayName"].as_str().filter(|s| !s.is_empty()) {
        out.push(Finding { check: "Gravatar", text: format!("Имя в профиле: {n}"), add: Some(Add::Entity { kind: Kind::Person, name: n.into(), relation: "имя в Gravatar".into() }) });
    }
    if let Some(u) = e["preferredUsername"].as_str().filter(|s| !s.is_empty()) {
        out.push(Finding { check: "Gravatar", text: format!("Никнейм: {u}"), add: Some(Add::Entity { kind: Kind::Nick, name: format!("@{u}"), relation: "никнейм в Gravatar".into() }) });
    }
    if let Some(l) = e["currentLocation"].as_str().filter(|s| !s.is_empty()) {
        out.push(Finding { check: "Gravatar", text: format!("Место: {l}"), add: Some(Add::Field { key: "Место (Gravatar)".into(), value: l.into() }) });
    }
    for a in e["accounts"].as_array().into_iter().flatten() {
        if let Some(u) = a["url"].as_str() {
            out.push(Finding { check: "Gravatar", text: format!("Привязан аккаунт: {u}"), add: Some(Add::Source { url: u.into(), note: "аккаунт из Gravatar (автопроверка)".into() }) });
        }
    }
    out
}

fn mail_domain(dom: &str) -> Vec<Finding> {
    let mx = doh(dom, "MX");
    if mx.is_empty() {
        return vec![Finding { check: "Почтовый домен", text: format!("У домена {dom} нет почтовых серверов (MX) — адрес может быть недействительным"), add: None }];
    }
    let free = ["gmail.com", "mail.ru", "yandex.ru", "ya.ru", "outlook.com", "hotmail.com", "proton.me", "protonmail.com", "icloud.com", "bk.ru", "list.ru", "inbox.ru", "rambler.ru", "yahoo.com", "gmx.com", "tutanota.com"];
    let mut out = vec![Finding { check: "Почтовый домен", text: format!("Почта принимается: {}", mx.join(", ")), add: None }];
    if !free.contains(&dom) {
        out.push(Finding { check: "Почтовый домен", text: format!("Личный или корпоративный домен: {dom}"), add: Some(Add::Entity { kind: Kind::Domain, name: dom.into(), relation: "домен почты".into() }) });
    }
    out
}

// ------------------------------------------------------------------ domains

/// DNS over HTTPS (Cloudflare): record values of a type.
fn doh(name: &str, kind: &str) -> Vec<String> {
    let url = format!("https://cloudflare-dns.com/dns-query?name={}&type={kind}", crate::extract::enc(name));
    let Ok(r) = ureq::get(&url).set("accept", "application/dns-json").timeout(Duration::from_secs(10)).call() else { return vec![] };
    let Ok(v) = r.into_json::<Value>() else { return vec![] };
    v["Answer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["data"].as_str())
        .map(|d| {
            let d = d.trim_matches('"').trim_end_matches('.');
            if kind == "MX" { d.split_whitespace().last().unwrap_or(d).trim_end_matches('.').to_string() } else { d.to_string() }
        })
        .filter(|d| !d.is_empty() && d != "0")
        .collect()
}

fn dns(d: &str) -> Vec<Finding> {
    let mut out = vec![];
    let a = doh(d, "A");
    for ip in a.iter().filter(|x| is_ip(x)).take(4) {
        out.push(Finding { check: "DNS", text: format!("IP-адрес: {ip}"), add: Some(Add::Entity { kind: Kind::Other, name: ip.clone(), relation: "IP домена".into() }) });
    }
    let ns = doh(d, "NS");
    if !ns.is_empty() {
        out.push(Finding { check: "DNS", text: format!("DNS-серверы: {}", ns.join(", ")), add: Some(Add::Field { key: "NS".into(), value: ns.join(", ") }) });
    }
    let mx = doh(d, "MX");
    if !mx.is_empty() {
        out.push(Finding { check: "DNS", text: format!("Почтовые серверы: {}", mx.join(", ")), add: Some(Add::Field { key: "MX".into(), value: mx.join(", ") }) });
    }
    for t in doh(d, "TXT") {
        if let Some(rest) = t.strip_prefix("google-site-verification=").or_else(|| t.strip_prefix("yandex-verification: ")) {
            let _ = rest;
            out.push(Finding { check: "DNS", text: format!("Подтверждение владения сайтом: {t}"), add: Some(Add::Field { key: "TXT".into(), value: t.clone() }) });
        }
    }
    if a.is_empty() && ns.is_empty() {
        out.push(Finding { check: "DNS", text: "Домен не отвечает в DNS".into(), add: None });
    }
    out
}

fn rdap(d: &str) -> Vec<Finding> {
    let Some(v) = json(&format!("https://rdap.org/domain/{}", crate::extract::enc(d))) else {
        return vec![];
    };
    let mut out = vec![];
    for e in v["events"].as_array().into_iter().flatten() {
        let date = e["eventDate"].as_str().unwrap_or("").chars().take(10).collect::<String>();
        let what = match e["eventAction"].as_str() {
            Some("registration") => "Зарегистрирован",
            Some("expiration") => "Истекает",
            Some("last changed") => "Изменён",
            _ => continue,
        };
        out.push(Finding { check: "WHOIS (RDAP)", text: format!("{what}: {date}"), add: Some(Add::Field { key: what.into(), value: date }) });
    }
    for ent in v["entities"].as_array().into_iter().flatten() {
        if ent["roles"].as_array().is_some_and(|r| r.iter().any(|x| x == "registrar")) {
            if let Some(name) = ent["vcardArray"][1].as_array().and_then(|a| a.iter().find(|x| x[0] == "fn")).and_then(|x| x[3].as_str()) {
                out.push(Finding { check: "WHOIS (RDAP)", text: format!("Регистратор: {name}"), add: Some(Add::Field { key: "Регистратор".into(), value: name.into() }) });
            }
        }
    }
    out
}

fn subdomains(d: &str) -> Vec<Finding> {
    let Some(v) = json(&format!("https://crt.sh/?q=%25.{}&output=json", crate::extract::enc(d))) else {
        return vec![];
    };
    let mut names: Vec<String> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name_value"].as_str())
        .flat_map(|s| s.split('\n').map(|x| x.trim().trim_start_matches("*.").to_lowercase()).collect::<Vec<_>>())
        .filter(|n| n != d && n.ends_with(&format!(".{d}")))
        .collect();
    names.sort();
    names.dedup();
    let total = names.len();
    let mut out: Vec<Finding> = names
        .into_iter()
        .take(15)
        .map(|n| Finding { check: "Сертификаты", text: format!("Поддомен: {n}"), add: Some(Add::Entity { kind: Kind::Domain, name: n, relation: "поддомен".into() }) })
        .collect();
    if total > 15 {
        out.push(Finding { check: "Сертификаты", text: format!("…и ещё {} поддоменов на crt.sh", total - 15), add: Some(Add::Source { url: format!("https://crt.sh/?q=%25.{d}"), note: "все сертификаты домена".into() }) });
    }
    out
}

fn wayback(d: &str) -> Vec<Finding> {
    let Some(v) = json(&format!("https://web.archive.org/cdx/search/cdx?url={}&output=json&limit=1&fl=timestamp", crate::extract::enc(d))) else {
        return vec![];
    };
    let Some(ts) = v[1][0].as_str() else {
        return vec![Finding { check: "Веб-архив", text: "В веб-архиве копий нет".into(), add: None }];
    };
    let date = format!("{}-{}-{}", &ts[0..4], &ts[4..6], &ts[6..8]);
    vec![Finding {
        check: "Веб-архив",
        text: format!("Первая копия сайта в архиве: {date}"),
        add: Some(Add::Source { url: format!("https://web.archive.org/web/{ts}/{d}"), note: format!("первая копия в веб-архиве, {date}") }),
    }]
}

// ------------------------------------------------------------------ phone, IP

const PREFIXES: &[(&str, &str)] = &[
    ("7", "Россия или Казахстан"), ("375", "Беларусь"), ("380", "Украина"), ("374", "Армения"), ("994", "Азербайджан"), ("995", "Грузия"),
    ("996", "Киргизия"), ("998", "Узбекистан"), ("992", "Таджикистан"), ("993", "Туркмения"), ("373", "Молдова"), ("371", "Латвия"),
    ("370", "Литва"), ("372", "Эстония"), ("1", "США или Канада"), ("44", "Великобритания"), ("49", "Германия"), ("33", "Франция"),
    ("34", "Испания"), ("39", "Италия"), ("48", "Польша"), ("90", "Турция"), ("972", "Израиль"), ("971", "ОАЭ"), ("86", "Китай"),
    ("91", "Индия"), ("81", "Япония"), ("82", "Южная Корея"), ("66", "Таиланд"), ("84", "Вьетнам"), ("62", "Индонезия"), ("31", "Нидерланды"),
    ("41", "Швейцария"), ("43", "Австрия"), ("420", "Чехия"), ("421", "Словакия"), ("36", "Венгрия"), ("40", "Румыния"), ("359", "Болгария"),
    ("381", "Сербия"), ("30", "Греция"), ("357", "Кипр"), ("351", "Португалия"), ("46", "Швеция"), ("47", "Норвегия"), ("45", "Дания"), ("358", "Финляндия"),
];

fn phone(v: &str) -> Vec<Finding> {
    let mut digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 11 && digits.starts_with('8') {
        digits.replace_range(0..1, "7");
    }
    let mut out = vec![Finding { check: "Номер", text: format!("В международном виде: +{digits}"), add: Some(Add::Field { key: "Номер".into(), value: format!("+{digits}") }) }];
    if let Some((_, country)) = PREFIXES.iter().filter(|(p, _)| digits.starts_with(p)).max_by_key(|(p, _)| p.len()) {
        let c = if digits.starts_with("77") { "Казахстан" } else if digits.starts_with('7') { "Россия" } else { country };
        out.push(Finding { check: "Номер", text: format!("Страна по коду: {c}"), add: Some(Add::Field { key: "Страна".into(), value: c.into() }) });
    }
    out.push(Finding { check: "Номер", text: "Проверить в WhatsApp".into(), add: Some(Add::Source { url: format!("https://wa.me/{digits}"), note: "ссылка WhatsApp".into() }) });
    out
}

fn ip_info(ip: &str) -> Vec<Finding> {
    let Some(v) = json(&format!("https://ipinfo.io/{ip}/json")) else { return vec![] };
    let mut out = vec![];
    let s = |k: &str| v[k].as_str().filter(|x| !x.is_empty()).map(str::to_string);
    let place = [s("city"), s("region"), s("country")].into_iter().flatten().collect::<Vec<_>>().join(", ");
    if !place.is_empty() {
        out.push(Finding { check: "IP", text: format!("Расположение: {place}"), add: Some(Add::Field { key: "Расположение".into(), value: place }) });
    }
    if let Some(org) = s("org") {
        out.push(Finding { check: "IP", text: format!("Сеть: {org}"), add: Some(Add::Field { key: "Сеть".into(), value: org }) });
    }
    if let Some(h) = s("hostname") {
        out.push(Finding { check: "IP", text: format!("Имя хоста: {h}"), add: Some(Add::Field { key: "Хост".into(), value: h }) });
    }
    if let Some(loc) = s("loc") {
        out.push(Finding { check: "IP", text: format!("Координаты (примерно): {loc}"), add: Some(Add::Field { key: "Координаты".into(), value: loc.replace(',', ", ") }) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_parsing() {
        let f = phone("8 (900) 123-45-67");
        assert!(f.iter().any(|x| x.text.contains("+79001234567")));
        assert!(f.iter().any(|x| x.text.contains("Россия")));
    }

    /// Hits real services; run by hand: cargo test live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live() {
        for (k, v) in [(Kind::Nick, "qnzz_nobody_84721x"), (Kind::Nick, "torvalds"), (Kind::Domain, "example.com"), (Kind::Other, "8.8.8.8")] {
            let out = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
            let o = out.clone();
            run(k, v, move |f| o.lock().unwrap().push(f));
            println!("== {v}");
            for f in out.lock().unwrap().iter() {
                println!("  [{}] {}", f.check, f.text);
            }
        }
    }
}
