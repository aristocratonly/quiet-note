//! Renders the main screens to PNG (target/shots) with sample data, to check the look.
//! Run: `cargo test shots -- --ignored --nocapture`

use super::*;
use crate::investigation::{Case, Event, Kind, Link, Reliability, Source};
use egui_kittest::Harness;
use std::fs;

fn sample(root: &Path) -> Settings {
    let _ = fs::remove_dir_all(root);
    let notes = root.join("notes");
    let invs = root.join("cases");
    fs::create_dir_all(notes.join("Статьи/Черновики")).unwrap();
    fs::create_dir_all(notes.join("Личное")).unwrap();
    let article = "Тихие инструменты\n\nХороший инструмент **не спорит с вами**. Он исчезает, когда вы начинаете работать, и появляется, только когда нужен.\n\n> Лучший интерфейс — тот, которого не замечаешь.\n\n## Что важно\n\n- обычные файлы, а не база данных\n- текст колонкой, *без лишнего шума*\n- связи через [[Идеи]]\n- [x] сохранять само\n- [ ] ничего не просить взамен\n\nЭтот текст — пример длинной заметки: здесь видно, как выглядит колонка, межстрочный интервал и поля. Пишите статьи, держите личные записи и ведите расследования в одном месте.\n\nВсё хранится в обычных файлах, поэтому ничего не потеряется.";
    fs::write(notes.join("Статьи/Тихие инструменты.md"), article).unwrap();
    fs::write(notes.join("Статьи/Черновики/Идеи.md"), "- первое\n- второе").unwrap();
    fs::write(notes.join("Личное/Список дел.md"), "Купить хлеб").unwrap();
    fs::write(notes.join("Входящие.md"), "").unwrap();

    let dir = investigation::create_case(&invs).unwrap();
    let dir2 = dir.with_file_name("Фишинговая рассылка");
    fs::rename(&dir, &dir2).unwrap();
    investigation::create_case(&invs).unwrap();
    let mut c = Case::load(&dir2);
    let mk = |c: &mut Case, k: Kind, name: &str| {
        let i = c.add_entity(k);
        c.entities[i].name = name.into();
        i
    };
    let p = mk(&mut c, Kind::Person, "Алексей Смирнов");
    let n = mk(&mut c, Kind::Nick, "@dark_fox");
    let m = mk(&mut c, Kind::Email, "fox.mail@example.org");
    let d = mk(&mut c, Kind::Domain, "secure-login.example");
    let o = mk(&mut c, Kind::Org, "ООО «Вектор»");
    let ph = mk(&mut c, Kind::Phone, "+7 900 000-00-00");
    let pl = mk(&mut c, Kind::Place, "Офис на Баумана");
    c.entities[pl].fields = vec![("Адрес".into(), "Казань, ул. Баумана, 1".into()), ("Координаты".into(), "55.7887, 49.1221".into())];
    let pl2 = mk(&mut c, Kind::Place, "Склад");
    c.entities[pl2].fields = vec![("Координаты".into(), "55.8304, 49.0661".into())];
    let link = |c: &mut Case, a: usize, b: usize, r: &str| {
        let t = c.entities[b].name.clone();
        c.entities[a].links.push(Link { target: t, relation: r.into() });
    };
    link(&mut c, p, n, "использует");
    link(&mut c, n, m, "указан в профиле");
    link(&mut c, m, d, "регистрант");
    link(&mut c, p, o, "директор");
    link(&mut c, p, ph, "владелец");
    link(&mut c, o, d, "оплачивает хостинг");
    let e = &mut c.entities[p];
    e.fields = vec![("ФИО".into(), "Смирнов Алексей Петрович".into()), ("Дата рождения".into(), "1991-04-12".into()), ("Город".into(), "Казань".into())];
    e.sources = vec![
        Source { url: "https://example.org/profile/dark_fox".into(), date: "2026-11-02".into(), reliability: Reliability::Confirmed, note: "фото совпадает".into() },
        Source { url: "files/whois.png".into(), date: "2026-11-03".into(), reliability: Reliability::Likely, note: "WHOIS до скрытия".into() },
        Source { url: "forum.example/thread/41".into(), date: "2026-11-05".into(), reliability: Reliability::Doubtful, note: "упоминание без подтверждения".into() },
    ];
    e.events = vec![Event { date: "2024-03-01".into(), text: "Регистрирует ООО «Вектор»".into() }];
    e.notes = "Проверить связь с объявлениями о продаже баз.".into();
    c.entities[d].events = vec![Event { date: "2026-10-28".into(), text: "Домен зарегистрирован".into() }];
    c.entities[m].events = vec![Event { date: "2025-06-14".into(), text: "Первое упоминание почты на форуме".into() }];
    c.overview.events = vec![Event { date: "2026-11-01".into(), text: "Первые жалобы на рассылку".into() }];
    c.overview.notes = "Цель: установить, кто стоит за рассылкой писем от имени банка.\n\nГипотеза: домен и никнейм принадлежат одному человеку.".into();
    c.touch(None);
    for k in 0..c.entities.len() {
        c.touch(Some(k));
    }
    c.save().unwrap();
    Settings {
        root: notes.clone(),
        last_open: Some(notes.join("Статьи/Тихие инструменты.md")),
        expanded: [notes.join("Статьи"), notes.join("Статьи/Черновики")].into_iter().collect(),
        inv_root: invs,
        last_case: Some(dir2),
        smooth: false,
        ..Default::default()
    }
}

struct State {
    settings: Option<Settings>,
    app: Option<App>,
    setup: Option<Box<dyn FnOnce(&mut App)>>,
}

fn shoot(name: &str, settings: Settings, setup: impl FnOnce(&mut App) + 'static) {
    let state = State { settings: Some(settings), app: None, setup: Some(Box::new(setup)) };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1180.0, 760.0))
        .with_pixels_per_point(2.0)
        .build_state(
            |ctx, st: &mut State| {
                if st.app.is_none() {
                    let mut app = App::with_settings(ctx, st.settings.take().unwrap());
                    if let Some(f) = st.setup.take() {
                        f(&mut app);
                    }
                    st.app = Some(app);
                    return; // fonts set in this frame apply from the next one
                }
                st.app.as_mut().unwrap().ui(ctx);
            },
            state,
        );
    h.run_steps(30);
    let img = h.render().expect("render");
    fs::create_dir_all("target/shots").unwrap();
    img.save(format!("target/shots/{name}.png")).unwrap();
}

#[test]
#[ignore]
fn shots() {
    let base = std::env::temp_dir().join("quiet-note-shots");
    let s = || {
        let s = sample(&base);
        s
    };
    // The sample writes entity files; build it once and reuse the folders.
    let settings = s();
    let ser = |s: &Settings| (s.root.clone(), s.last_open.clone(), s.expanded.clone(), s.inv_root.clone(), s.last_case.clone());
    let parts = ser(&settings);
    let mk = move |mode: Mode| Settings {
        root: parts.0.clone(),
        last_open: parts.1.clone(),
        expanded: parts.2.clone(),
        inv_root: parts.3.clone(),
        last_case: parts.4.clone(),
        mode,
        smooth: false,
        ..Default::default()
    };
    shoot("1-notes", mk(Mode::Notes), |_| {});
    shoot("2-overview", mk(Mode::Investigations), |a| a.inv.view = inv_ui::View::Overview);
    shoot("3-entity", mk(Mode::Investigations), |a| {
        let i = a.inv.case.as_ref().unwrap().find("Алексей Смирнов").unwrap();
        a.inv.view = inv_ui::View::Entity(i);
    });
    shoot("4-graph", mk(Mode::Investigations), |a| a.inv.view = inv_ui::View::Graph);
    shoot("5-timeline", mk(Mode::Investigations), |a| a.inv.view = inv_ui::View::Timeline);
    shoot("6-empty-notes", Settings { mode: Mode::Notes, last_open: None, ..mk(Mode::Notes) }, |_| {});
    shoot("7-settings", mk(Mode::Notes), |a| a.show_settings = true);
    shoot("8-reading", mk(Mode::Notes), |a| a.reading = true);
    shoot("9-quick", mk(Mode::Notes), |a| a.quick = Some(crate::Quick { query: "и".into(), sel: 0, focus: false }));
    shoot("10-focus", mk(Mode::Notes), |a| a.focus_mode = true);
    shoot("11-map", mk(Mode::Investigations), |a| a.inv.view = inv_ui::View::Map);
    shoot("15-auto-graph", mk(Mode::Investigations), |a| a.inv.fake_auto_added("@dark_fox"));
    shoot("14-auto", mk(Mode::Investigations), |a| a.inv.fake_auto("@dark_fox"));
    shoot("13-password", mk(Mode::Investigations), |a| a.inv.open_encrypt());
    shoot("12-extract", mk(Mode::Investigations), |a| a.inv.open_extract("Пишите на fox.mail@example.org или +7 (900) 123-45-67, сайт https://secure-login.example/x, @dark_fox, сервер 185.22.10.7"));
}
