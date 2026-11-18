use crate::core::logging::log_event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentConversion {
    pub source_name: String,
    pub target_format: String,
    pub output_path: String,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppPreferences {
    pub version: u32,
    #[serde(alias = "SoundEnabled")]
    pub sound_enabled: bool,
    pub haptics_enabled: bool,
    pub notifications_enabled: bool,
    #[serde(alias = "Trigger")]
    pub drag_trigger: String,
    pub idle_shortcut: String,
    pub mouse_trigger_button: Option<u8>,
    pub auto_rename_on_create: bool,
    pub open_after_creating: bool,
    pub launch_at_login: bool,
    pub output_subfolder: Option<String>,
    pub fallback_folder: Option<String>,
    pub image_quality: u8,
    pub preserve_exif: bool,
    pub strip_gps_exif: bool,
    pub pdf_dpi: u16,
    pub pdf_page_size: String,
    pub ffmpeg_path: Option<String>,
    pub pandoc_path: Option<String>,
    pub preferred_formats: HashMap<String, Vec<String>>,
    pub new_file_visible_kinds: Vec<String>,
    pub recent_conversions: Vec<RecentConversion>,
    #[serde(default)]
    pub onboarding_completed: bool,
    pub language: String,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            sound_enabled: true,
            haptics_enabled: true,
            notifications_enabled: true,
            drag_trigger: "Down".to_string(),
            idle_shortcut: "Option+Shift+Space".to_string(),
            mouse_trigger_button: None,
            auto_rename_on_create: true,
            open_after_creating: false,
            launch_at_login: false,
            output_subfolder: None,
            fallback_folder: None,
            image_quality: 92,
            preserve_exif: true,
            strip_gps_exif: false,
            pdf_dpi: 144,
            pdf_page_size: "a4".to_string(),
            ffmpeg_path: None,
            pandoc_path: None,
            preferred_formats: default_preferred_formats(),
            new_file_visible_kinds: default_new_file_kinds(),
            recent_conversions: Vec::new(),
            onboarding_completed: false,
            language: "en".to_string(),
        }
    }
}

pub fn default_preferred_formats() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    map.insert(
        "image".to_string(),
        vec![
            "PNG", "JPG", "WEBP", "HEIC", "PDF", "REMOVE_BG", "TIFF", "GIF", "BMP", "ICO",
            "ICNS", "TXT",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    map.insert(
        "pdf".to_string(),
        vec![
            "PNG", "JPG", "TXT", "DOCX", "OPTIMIZE_PDF", "MD", "HTML", "RTF", "WEBP", "HEIC",
            "TIFF", "ODT", "EPUB", "FB2", "RST",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    map.insert(
        "document".to_string(),
        vec![
            "PDF", "DOCX", "MD", "HTML", "TXT", "RTF", "ODT", "PNG", "JPG", "WEBP", "HEIC",
            "EPUB", "FB2", "RST",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    map.insert(
        "data".to_string(),
        vec!["JSON", "YAML", "TOML", "CSV", "XLSX", "XML", "MD", "HTML", "PDF", "TXT"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    map.insert(
        "audio".to_string(),
        vec!["MP3", "M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF", "OGG", "OPUS", "AAC"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    map.insert(
        "video".to_string(),
        vec!["MP4", "MOV", "GIF", "M4A", "MP3", "WAV", "MKV", "WEBM", "AVI", "M4V"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    map
}

pub fn default_new_file_kinds() -> Vec<String> {
    vec![
        "folder",
        "txt",
        "md",
        "rtf",
        "docx",
        "xlsx",
        "html",
        "clipboard",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

pub fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(rest);
    }
    PathBuf::from(trimmed)
}

impl AppPreferences {
    pub fn app_support_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Application Support")
            .join("FLIP")
    }

    pub fn settings_path() -> PathBuf {
        Self::app_support_dir().join("settings.json")
    }

    pub fn templates_dir() -> PathBuf {
        Self::app_support_dir().join("Templates")
    }

    pub fn resolved_fallback_folder(&self) -> PathBuf {
        if let Some(custom) = &self.fallback_folder {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return expand_tilde_path(trimmed);
            }
        }
        dirs::desktop_dir().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Desktop")
        })
    }

    pub fn load() -> Self {
        Self::load_from(&Self::settings_path())
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<AppPreferences>(&raw) {
                Ok(mut prefs) => {
                    prefs.sanitize();
                    prefs
                }
                Err(err) => {
                    log_event("settings.load.error", Some(json!({ "error": err.to_string() })));
                    Self::default()
                }
            },
            Err(err) => {
                log_event("settings.load.error", Some(json!({ "error": err.to_string() })));
                Self::default()
            }
        }
    }

    pub fn sanitize(&mut self) {
        self.image_quality = self.image_quality.clamp(10, 100);
        if self.pdf_dpi < 72 || self.pdf_dpi > 600 {
            self.pdf_dpi = 144;
        }
        if !matches!(self.pdf_page_size.to_lowercase().as_str(), "a4" | "letter") {
            self.pdf_page_size = "a4".to_string();
        }
        let lang = self.language.trim().to_ascii_lowercase();
        self.language = if lang == "it" || lang.starts_with("it-") || lang.starts_with("it_") {
            "it".to_string()
        } else {
            "en".to_string()
        };
        if self.mouse_trigger_button.is_some_and(|b| !(2..=5).contains(&b)) {
            self.mouse_trigger_button = None;
        }
        if self.recent_conversions.len() > 10 {
            self.recent_conversions.truncate(10);
        }
        for (family, defaults) in default_preferred_formats() {
            let entry = self
                .preferred_formats
                .entry(family)
                .or_insert_with(|| defaults.clone());
            for fmt in defaults {
                if !entry.iter().any(|existing| existing.eq_ignore_ascii_case(&fmt)) {
                    entry.push(fmt);
                }
            }
        }
    }

    pub fn push_recent(&mut self, item: RecentConversion) {
        self.recent_conversions
            .retain(|existing| existing.output_path != item.output_path);
        self.recent_conversions.insert(0, item);
        if self.recent_conversions.len() > 10 {
            self.recent_conversions.truncate(10);
        }
        for (family, defaults) in default_preferred_formats() {
            let entry = self
                .preferred_formats
                .entry(family)
                .or_insert_with(|| defaults.clone());
            for fmt in defaults {
                if !entry.iter().any(|existing| existing.eq_ignore_ascii_case(&fmt)) {
                    entry.push(fmt);
                }
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::settings_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_file_name(format!(
            "{}.{}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("settings.json"),
            Uuid::new_v4().simple()
        ));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let write_res = fs::write(&temp, json);
        if let Err(e) = write_res {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        if let Err(e) = fs::rename(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mute_and_key_roundtrip_including_legacy_aliases() {
        let raw = r#"{"SoundEnabled":false,"Trigger":"Space"}"#;
        let prefs: AppPreferences = serde_json::from_str(raw).unwrap();
        assert!(!prefs.sound_enabled);
        assert_eq!(prefs.drag_trigger, "Space");
        assert!(prefs.auto_rename_on_create);
    }

    #[test]
    fn atomic_save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let prefs = AppPreferences {
            sound_enabled: false,
            notifications_enabled: false,
            drag_trigger: "F8".to_string(),
            image_quality: 85,
            ..Default::default()
        };
        prefs.save_to(&path).unwrap();

        let loaded = AppPreferences::load_from(&path);
        assert!(!loaded.sound_enabled);
        assert!(!loaded.notifications_enabled);
        assert_eq!(loaded.drag_trigger, "F8");
        assert_eq!(loaded.image_quality, 85);

        let legacy: AppPreferences = serde_json::from_str(r#"{"soundEnabled":true}"#).unwrap();
        assert!(legacy.notifications_enabled);
    }

    #[test]
    fn corrupt_settings_falls_back_to_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{not valid json").unwrap();
        let loaded = AppPreferences::load_from(&path);
        assert_eq!(loaded.drag_trigger, "Down");
        assert!(loaded.sound_enabled);
    }

    #[test]
    fn sanitize_preserves_custom_order_while_backfilling_new_defaults() {
        let mut prefs = AppPreferences::default();
        prefs.preferred_formats.insert(
            "document".to_string(),
            vec!["EPUB".into(), "MD".into(), "PDF".into()],
        );
        prefs.sanitize();
        let doc = &prefs.preferred_formats["document"];
        assert_eq!(&doc[0..3], &["EPUB", "MD", "PDF"]);
        assert!(doc.contains(&"HEIC".to_string()));
        assert!(prefs.preferred_formats["pdf"].contains(&"TIFF".to_string()));
        assert!(prefs.preferred_formats["pdf"].contains(&"OPTIMIZE_PDF".to_string()));
        assert!(prefs.preferred_formats["image"].contains(&"REMOVE_BG".to_string()));
        assert!(prefs.preferred_formats["image"].contains(&"ICO".to_string()));
        assert!(prefs.preferred_formats["image"].contains(&"ICNS".to_string()));
        assert!(prefs.preferred_formats["image"].contains(&"TXT".to_string()));
        assert!(prefs.preferred_formats["audio"].contains(&"CAF".to_string()));
    }

    #[test]
    fn language_defaults_to_en_and_sanitizes_cleanly() {
        let def = AppPreferences::default();
        assert_eq!(def.language, "en");

        let mut it_prefs = AppPreferences {
            language: " IT ".to_string(),
            ..Default::default()
        };
        it_prefs.sanitize();
        assert_eq!(it_prefs.language, "it");

        let mut bad_prefs = AppPreferences {
            language: "fr-FR".to_string(),
            ..Default::default()
        };
        bad_prefs.sanitize();
        assert_eq!(bad_prefs.language, "en");

        let mut empty_prefs = AppPreferences {
            language: "".to_string(),
            ..Default::default()
        };
        empty_prefs.sanitize();
        assert_eq!(empty_prefs.language, "en");
    }
}
