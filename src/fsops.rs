//! File-system side of the notes tree. Notes are plain `.txt`/`.md` files, folders are folders.

use std::fs;
use std::path::{Path, PathBuf};

const NOTE_EXT: &str = "md";

fn is_note(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("md" | "txt"))
}

/// Folders first, then notes, both alphabetically; hidden entries skipped.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .filter(|p| p.is_dir() || is_note(p))
        .collect();
    v.sort_by_key(|p| (!p.is_dir(), display_name(p).to_lowercase()));
    v
}

/// Does this entry (or, for a folder, anything inside it) match the lowercase filter?
pub fn matches(p: &Path, filter: &str) -> bool {
    if display_name(p).to_lowercase().contains(filter) {
        return true;
    }
    p.is_dir() && list(p).iter().any(|c| matches(c, filter))
}

pub fn display_name(p: &Path) -> String {
    let os = if p.is_dir() { p.file_name() } else { p.file_stem() };
    os.map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn unique(dir: &Path, base: &str, ext: Option<&str>) -> PathBuf {
    let make = |n: &str| match ext {
        Some(e) => dir.join(format!("{n}.{e}")),
        None => dir.join(n),
    };
    let mut p = make(base);
    let mut i = 2;
    while p.exists() {
        p = make(&format!("{base} {i}"));
        i += 1;
    }
    p
}

pub fn new_note(dir: &Path) -> Result<PathBuf, String> {
    let p = unique(dir, "Без названия", Some(NOTE_EXT));
    fs::write(&p, "").map_err(|e| format!("Не удалось создать заметку: {e}"))?;
    Ok(p)
}

pub fn new_folder(dir: &Path) -> Result<PathBuf, String> {
    let p = unique(dir, "Новая папка", None);
    fs::create_dir(&p).map_err(|e| format!("Не удалось создать папку: {e}"))?;
    Ok(p)
}

fn valid_name(name: &str) -> Result<&str, String> {
    let n = name.trim();
    if n.is_empty() || n.contains('/') || n.starts_with('.') {
        return Err("Недопустимое название".into());
    }
    Ok(n)
}

pub fn rename(p: &Path, name: &str) -> Result<PathBuf, String> {
    if name.trim().is_empty() {
        return Ok(p.to_path_buf()); // empty input = keep the old name
    }
    let n = valid_name(name)?;
    let parent = p.parent().ok_or("Нет родительской папки")?;
    let target = if p.is_dir() {
        parent.join(n)
    } else {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or(NOTE_EXT);
        parent.join(format!("{n}.{ext}"))
    };
    if target == p {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("«{n}» уже существует"));
    }
    fs::rename(p, &target).map_err(|e| format!("Не удалось переименовать: {e}"))?;
    Ok(target)
}

pub fn move_into(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    if dir.starts_with(src) {
        return Err("Нельзя переместить папку внутрь самой себя".into());
    }
    let name = src.file_name().ok_or("Некорректный путь")?;
    let target = dir.join(name);
    if target == src {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("В этой папке уже есть «{}»", display_name(src)));
    }
    fs::rename(src, &target).map_err(|e| format!("Не удалось переместить: {e}"))?;
    Ok(target)
}

pub fn delete(p: &Path) -> Result<(), String> {
    trash::delete(p).map_err(|e| format!("Не удалось удалить: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quiet-note-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn create_rename_move() {
        let root = tmp("crm");
        let a = new_note(&root).unwrap();
        let b = new_note(&root).unwrap();
        assert_ne!(a, b);
        let a = rename(&a, "План").unwrap();
        assert_eq!(a, root.join("План.md"));
        assert!(rename(&b, "План").is_err());
        let f = new_folder(&root).unwrap();
        let moved = move_into(&a, &f).unwrap();
        assert!(moved.exists() && !a.exists());
        assert!(move_into(&f, &f).is_err());
        assert_eq!(list(&root).first(), Some(&f)); // folders first
    }
}

/// Opens a file, folder or URL with the system handler. With `reveal`, shows the item in the file manager.
pub fn open_external(target: &str, reveal: bool) {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let r = if reveal { Command::new("open").arg("-R").arg(target).spawn() } else { Command::new("open").arg(target).spawn() };
    #[cfg(target_os = "windows")]
    let r = if reveal { Command::new("explorer").arg(format!("/select,{target}")).spawn() } else { Command::new("cmd").args(["/C", "start", "", target]).spawn() };
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = {
        let p = std::path::Path::new(target);
        let t = if reveal && p.is_file() { p.parent().map(|d| d.display().to_string()).unwrap_or_default() } else { target.to_string() };
        Command::new("xdg-open").arg(t).spawn()
    };
    let _ = r;
}

/// Name of the system file manager for menu labels.
pub const FILE_MANAGER: &str = if cfg!(target_os = "macos") { "Показать в Finder" } else if cfg!(target_os = "windows") { "Показать в Проводнике" } else { "Показать в папке" };
