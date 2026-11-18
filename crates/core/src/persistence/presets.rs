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

pub fn save_preset(paths: &AppPaths, preset: &Preset) -> Result<(), PersistenceError> {
    let dir = paths.presets_dir();
    create_dir_with_permissions(&dir)?;
    let filename = format!("{}.json", slugify(&preset.name));
    let json = serde_json::to_string_pretty(preset)?;
    atomic_write(&dir.join(filename), json.as_bytes()).map_err(PersistenceError::Io)
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
    let filename = format!("{}.json", slugify(name));
    let path = dir.join(filename);
    if path.exists() {
        fs::remove_file(&path)?;
        return Ok(true);
    }
    if !dir.exists() {
        return Ok(false);
    }
    // Fallback for legacy DefaultHasher filenames created before fixed FNV-1a.
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let candidate = entry.path();
        if candidate.extension().is_some_and(|ext| ext == "json")
            && let Ok(contents) = read_file(&candidate)
            && let Ok(preset) = serde_json::from_str::<Preset>(&contents)
            && preset.name == name
        {
            fs::remove_file(&candidate)?;
            return Ok(true);
        }
    }
    Ok(false)
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
}
