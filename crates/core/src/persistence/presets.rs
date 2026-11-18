use std::fs;

use crate::fs::atomic_write;
use crate::models::Preset;

use super::{AppPaths, PersistenceError, create_dir_with_permissions, read_file};

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn slugify(name: &str) -> String {
    let slug = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if !slug.is_empty() {
        return slug;
    }

    format!("preset-{:016x}", fnv1a_64(name.as_bytes()))
}

/// Returns the stored preset's name, or `None` when the file is missing,
/// unreadable, or corrupt.
fn stored_preset_name(path: &std::path::Path) -> Option<String> {
    let contents = read_file(path).ok()?;
    serde_json::from_str::<Preset>(&contents)
        .ok()
        .map(|p| p.name)
}

/// Upper bound for `-{hash}-N` advancement; 32 same-slug candidates cannot
/// legitimately coexist, and the bound keeps a pathological directory finite.
const MAX_DESTINATION_PROBES: u32 = 32;

/// First `*.json` file whose stored preset name equals `name`. Corrupt files
/// never match, so their content stays untouched.
fn find_by_name(
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<std::path::PathBuf>, PersistenceError> {
    for entry in fs::read_dir(dir)? {
        let candidate = entry?.path();
        if candidate.extension().is_some_and(|ext| ext == "json")
            && stored_preset_name(&candidate).is_some_and(|existing| existing == name)
        {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

pub fn save_preset(paths: &AppPaths, preset: &Preset) -> Result<(), PersistenceError> {
    let dir = paths.presets_dir();
    create_dir_with_permissions(&dir)?;

    let destination = match find_by_name(&dir, &preset.name)? {
        Some(existing) => existing,
        None => reserve_destination(&dir, &preset.name)?,
    };
    // Older builds could leave several files holding the same name; keep save
    // symmetric with delete by removing every shadow copy before writing the
    // canonical destination. Corrupt files and other names stay untouched.
    for entry in fs::read_dir(&dir)? {
        let candidate = entry?.path();
        if candidate == destination {
            continue;
        }
        if candidate.extension().is_some_and(|ext| ext == "json")
            && stored_preset_name(&candidate).is_some_and(|existing| existing == preset.name)
        {
            fs::remove_file(&candidate)?;
        }
    }
    write_preset(&destination, preset)
}

fn write_preset(path: &std::path::Path, preset: &Preset) -> Result<(), PersistenceError> {
    let json = serde_json::to_string_pretty(preset)?;
    atomic_write(path, json.as_bytes()).map_err(PersistenceError::Io)
}

/// Picks an unoccupied file for a name no file holds yet: the primary slug
/// path when free, otherwise the hashed name advancing deterministically.
/// Every candidate is verified unowned before the write, so a preset is never
/// overwritten by another one's fallback.
fn reserve_destination(
    dir: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, PersistenceError> {
    let slug = slugify(name);
    let primary = dir.join(format!("{slug}.json"));
    if !primary.exists() {
        return Ok(primary);
    }

    let hash = fnv1a_64(name.as_bytes());
    for suffix in 1..=MAX_DESTINATION_PROBES {
        let candidate = if suffix == 1 {
            dir.join(format!("{slug}-{hash:016x}.json"))
        } else {
            dir.join(format!("{slug}-{hash:016x}-{suffix}.json"))
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(PersistenceError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free preset file for {name:?} after {MAX_DESTINATION_PROBES} candidates"),
    )))
}

pub fn load_custom_presets(paths: &AppPaths) -> Result<Vec<Preset>, PersistenceError> {
    let dir = paths.presets_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut presets = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let contents = read_file(&path)?;
            match serde_json::from_str::<Preset>(&contents) {
                Ok(preset) => presets.push(preset),
                Err(err) => log::warn!("skip corrupt preset {path:?}: {err}"),
            }
        }
    }
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(presets)
}

pub fn delete_preset(paths: &AppPaths, name: &str) -> Result<bool, PersistenceError> {
    let dir = paths.presets_dir();
    if !dir.exists() {
        return Ok(false);
    }
    // Older saves could leave several files holding the same name; remove
    // every match so one delete leaves no shadow copy behind.
    let mut removed = false;
    for entry in fs::read_dir(&dir)? {
        let candidate = entry?.path();
        if candidate.extension().is_some_and(|ext| ext == "json")
            && stored_preset_name(&candidate).is_some_and(|existing| existing == name)
        {
            fs::remove_file(&candidate)?;
            removed = true;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_non_ascii_name_uses_hash_fallback() {
        assert_eq!(slugify("Работа"), "preset-4e6851489e20ed0b");
        assert_eq!(slugify("Дом"), "preset-9e6bfa5cb92c37cb");
        assert_eq!(slugify("Блокировка рекламы"), "preset-2a06eb44e8ba1509");
        assert_eq!(slugify("Другой пресет"), "preset-51bc0ac2bc4ce149");
    }

    #[test]
    fn test_delete_preset_falls_back_to_legacy_filename_by_name() {
        let (_tmp, paths) = super::super::test_paths();
        let dir = paths.presets_dir();
        create_dir_with_permissions(&dir).unwrap();
        let mut preset = crate::models::builtin_presets().remove(0);
        preset.name = "Работа".to_string();
        let json = serde_json::to_string_pretty(&preset).unwrap();
        atomic_write(&dir.join("preset-0000000000000000.json"), json.as_bytes()).unwrap();

        assert!(delete_preset(&paths, "Работа").unwrap());
        assert!(load_custom_presets(&paths).unwrap().is_empty());
    }

    #[test]
    fn test_save_and_load_cyrillic_preset() {
        let (_tmp, paths) = super::super::test_paths();
        let mut preset = crate::models::builtin_presets().remove(0);
        preset.name = "Русский пресет".to_string();
        preset.description = "Описание".to_string();

        save_preset(&paths, &preset).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, preset.name);
        assert!(delete_preset(&paths, &preset.name).unwrap());
    }

    #[test]
    fn test_save_and_load_custom_preset() {
        let (_tmp, paths) = super::super::test_paths();
        let presets = crate::models::builtin_presets();
        let preset = &presets[0];

        save_preset(&paths, preset).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, preset.name);
        assert_eq!(loaded[0].description, preset.description);
    }

    #[test]
    fn test_delete_preset() {
        let (_tmp, paths) = super::super::test_paths();
        let presets = crate::models::builtin_presets();
        save_preset(&paths, &presets[0]).unwrap();
        save_preset(&paths, &presets[1]).unwrap();

        let deleted = delete_preset(&paths, &presets[0].name).unwrap();
        assert!(deleted);

        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, presets[1].name);
    }

    #[test]
    fn test_delete_nonexistent_preset() {
        let (_tmp, paths) = super::super::test_paths();
        let deleted = delete_preset(&paths, "nope").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_load_custom_presets_empty() {
        let (_tmp, paths) = super::super::test_paths();
        let loaded = load_custom_presets(&paths).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_slug_collision_preserves_distinct_presets() {
        let (_tmp, paths) = super::super::test_paths();
        let dir = paths.presets_dir();
        let mut first = crate::models::builtin_presets().remove(0);
        first.name = "My Preset".to_string();
        let mut second = crate::models::builtin_presets().remove(0);
        second.name = "my-preset".to_string();

        save_preset(&paths, &first).unwrap();
        save_preset(&paths, &second).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 2);

        // Re-saving updates only its own file.
        first.description = "updated".to_string();
        save_preset(&paths, &first).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 2);
        let saved = loaded.iter().find(|p| p.name == "My Preset").unwrap();
        assert_eq!(saved.description, "updated");

        // Deleting one colliding preset leaves the sibling intact.
        assert!(delete_preset(&paths, "my-preset").unwrap());
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "My Preset");
        assert!(dir.join("my-preset.json").exists());
    }

    #[test]
    fn test_save_preset_with_corrupt_slug_file() {
        let (_tmp, paths) = super::super::test_paths();
        let dir = paths.presets_dir();
        create_dir_with_permissions(&dir).unwrap();
        atomic_write(&dir.join("my-preset.json"), b"{ not json").unwrap();

        let mut preset = crate::models::builtin_presets().remove(0);
        preset.name = "My Preset".to_string();
        save_preset(&paths, &preset).unwrap();

        assert!(dir.join("my-preset.json").exists());
        let fallback = dir.join(format!(
            "my-preset-{:016x}.json",
            fnv1a_64(preset.name.as_bytes())
        ));
        assert!(fallback.exists());
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "My Preset");

        // The same name keeps updating its own file, never the corrupt one.
        preset.description = "second".to_string();
        save_preset(&paths, &preset).unwrap();
        assert!(dir.join("my-preset.json").exists());
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].description, "second");
    }

    #[test]
    fn test_delete_then_resave_consolidates_shadow_duplicates() {
        let (_tmp, paths) = super::super::test_paths();
        let mut base = crate::models::builtin_presets().remove(0);

        base.name = "My Preset".to_string();
        save_preset(&paths, &base).unwrap();
        base.name = "my-preset".to_string();
        save_preset(&paths, &base).unwrap();
        assert!(delete_preset(&paths, "My Preset").unwrap());

        base.name = "my-preset".to_string();
        save_preset(&paths, &base).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1, "shadow copy must be consolidated on save");
        assert_eq!(loaded[0].name, "my-preset");

        assert!(delete_preset(&paths, "my-preset").unwrap());
        assert!(load_custom_presets(&paths).unwrap().is_empty());
    }

    #[test]
    fn test_save_never_overwrites_hashed_path_occupant() {
        let (_tmp, paths) = super::super::test_paths();
        let dir = paths.presets_dir();
        create_dir_with_permissions(&dir).unwrap();

        let mut holder = crate::models::builtin_presets().remove(0);
        holder.name = "My Preset".to_string();
        save_preset(&paths, &holder).unwrap();

        // Occupy the hashed fallback of "my-preset" with an unrelated preset.
        let mut occupant = crate::models::builtin_presets().remove(0);
        occupant.name = "Occupied Name".to_string();
        let hashed = dir.join(format!(
            "my-preset-{:016x}.json",
            fnv1a_64("my-preset".as_bytes())
        ));
        let occupant_json = serde_json::to_string_pretty(&occupant).unwrap();
        atomic_write(&hashed, occupant_json.as_bytes()).unwrap();

        let mut target = crate::models::builtin_presets().remove(0);
        target.name = "my-preset".to_string();
        save_preset(&paths, &target).unwrap();

        assert_eq!(
            stored_preset_name(&hashed).as_deref(),
            Some("Occupied Name")
        );
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 3);
        assert!(loaded.iter().any(|p| p.name == "my-preset"));
    }

    #[test]
    fn test_save_preset_consolidates_shadow_duplicates_on_save() {
        let (_tmp, paths) = super::super::test_paths();
        let dir = paths.presets_dir();
        create_dir_with_permissions(&dir).unwrap();

        let mut preset = crate::models::builtin_presets().remove(0);
        preset.name = "My Preset".to_string();
        let json = serde_json::to_string_pretty(&preset).unwrap();
        atomic_write(&dir.join("my-preset.json"), json.as_bytes()).unwrap();
        let shadow = dir.join(format!(
            "my-preset-{:016x}.json",
            fnv1a_64(preset.name.as_bytes())
        ));
        atomic_write(&shadow, json.as_bytes()).unwrap();

        preset.description = "resaved".to_string();
        save_preset(&paths, &preset).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        assert_eq!(
            remaining.len(),
            1,
            "exactly one preset file must remain: {remaining:?}"
        );
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "My Preset");
        assert_eq!(loaded[0].description, "resaved");
    }
}
