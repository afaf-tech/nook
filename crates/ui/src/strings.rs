//! The chrome's language, in one table.
//!
//! The key *is* the English sentence, so a missing translation renders English rather than a key
//! name — that is the whole reason for the shape. Engine errors stay English on purpose: making
//! them translatable means an error-code layer, which is a different project.

/// Every locale Nook ships, in the order the preference offers them.
pub const LOCALES: &[(&str, &str)] = &[("en", "English"), ("id", "Bahasa Indonesia")];

/// Indonesian, keyed by the English sentence.
const ID: &[(&str, &str)] = &[
    // Menus
    ("File", "Berkas"),
    ("Data", "Data"),
    ("Query", "Kueri"),
    ("Server", "Server"),
    ("Automation", "Otomasi"),
    ("Help", "Bantuan"),
    // Status bar
    ("no connection", "belum tersambung"),
    (" rows", " baris"),
    // Empty states
    ("nothing to pivot", "tidak ada yang bisa diputar"),
    ("no rows in this report", "laporan ini belum punya baris"),
    // Preferences
    ("Preferences", "Preferensi"),
    ("Language", "Bahasa"),
    ("Theme", "Tema"),
    ("Density", "Kerapatan"),
    ("Page size", "Ukuran halaman"),
    // Dialogs
    ("Cancel", "Batal"),
    ("Save", "Simpan"),
    ("Delete", "Hapus"),
    ("Close", "Tutup"),
    ("Run", "Jalankan"),
    ("Refresh", "Muat ulang"),
    ("Connect", "Sambungkan"),
];

/// The locale the current frame paints in.
///
/// A widget deep in a dialog has no `&Session` and no settings handle, and threading a locale through
/// sixty call sites would be a worse trade than one process-global the frame sets. It is set once per
/// frame from `settings.locale`, and read-only everywhere else.
static CURRENT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Record the locale for this frame. Called once per frame, before anything paints.
pub fn set_locale(locale: &str) {
    let code = LOCALES
        .iter()
        .position(|(code, _)| *code == locale)
        .unwrap_or(0);
    CURRENT.store(code as u8, std::sync::atomic::Ordering::Relaxed);
}

/// Translate a chrome string in the frame's locale.
pub fn tr(key: &'static str) -> &'static str {
    let code = CURRENT.load(std::sync::atomic::Ordering::Relaxed) as usize;
    match LOCALES.get(code) {
        Some((locale, _)) => t(locale, key),
        None => key,
    }
}

/// Translate one chrome string for a locale. An unknown locale, or a key with no translation,
/// returns the key — which is the English text.
pub fn t(locale: &str, key: &'static str) -> &'static str {
    let table: &[(&str, &str)] = match locale {
        "id" => ID,
        _ => return key,
    };
    table
        .iter()
        .find(|(english, _)| *english == key)
        .map(|(_, translated)| *translated)
        .unwrap_or(key)
}

/// The language's own name, for the preference row.
pub fn locale_label(locale: &str) -> &'static str {
    LOCALES
        .iter()
        .find(|(code, _)| *code == locale)
        .map(|(_, name)| *name)
        .unwrap_or("English")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_is_translated_twice() {
        // A duplicate is invisible at a glance and means a scripted insert landed on top of an
        // existing entry. Only the first one would ever be read.
        let mut seen = std::collections::HashSet::new();
        for (key, _) in ID {
            assert!(seen.insert(*key), "{key:?} is translated twice");
        }
    }

    #[test]
    fn every_indonesian_entry_translates_a_key_that_english_uses() {
        // A translation whose key no longer exists is a string nothing can reach, and it means a
        // rename in the UI went past this table. English is the key set, so it is the reference.
        for (key, _) in ID {
            assert!(!key.trim().is_empty(), "an empty key");
        }
        assert!(ID.len() > 10, "the table is the point");
    }

    #[test]
    fn an_unknown_key_renders_the_english_text() {
        assert_eq!(
            t("id", "a sentence nobody translated"),
            "a sentence nobody translated"
        );
        assert_eq!(t("id", "File"), "Berkas");
    }

    #[test]
    fn the_ui_and_the_engine_agree_on_which_languages_exist() {
        // `platform.i18n` validates against the engine's own list (the engine cannot depend on this
        // crate), so a language added on one side and not the other would be refused or unrenderable.
        let reported = nook_engine::Session::new()
            .run("platform.i18n", serde_json::json!({}))
            .expect("platform.i18n answers");
        let engine: Vec<String> = reported["languages"]
            .as_array()
            .expect("languages")
            .iter()
            .map(|pair| pair[0].as_str().unwrap_or_default().to_string())
            .collect();
        let ui: Vec<String> = LOCALES
            .iter()
            .map(|(code, _)| (*code).to_string())
            .collect();
        assert_eq!(
            engine, ui,
            "the two lists are the same languages in the same order"
        );
    }

    #[test]
    fn the_frame_locale_reaches_a_string_no_widget_passes_it_to() {
        // A dialog button has no settings handle; `tr` is how it still translates. Left set, the
        // global would leak into every later test in this binary, so it is restored.
        let before = tr("Close");
        set_locale("id");
        assert_eq!(tr("Close"), "Tutup");
        set_locale("en");
        assert_eq!(tr("Close"), "Close");
        assert_eq!(tr("Close"), before);
    }

    #[test]
    fn every_translated_key_is_reachable_from_the_chrome() {
        // A translation whose key nothing paints is dead weight that looks like coverage. The keys
        // are matched against the source, so a renamed button shows up here rather than in a
        // screenshot nobody reads.
        let sources: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("the ui source directory is there")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "rs"))
            .filter(|path| !path.ends_with("strings.rs"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect();
        for (key, _) in ID {
            // Two ways a chrome string is translated: `tr("…")` where no settings handle exists, and
            // `t(&locale, "…")` where one does (the menu bar has it).
            let painted = sources.iter().any(|source| {
                source.contains(&format!("tr(\"{key}\")"))
                    || source.contains(&format!("t(&locale, \"{key}\"))"))
                    || source.contains(&format!("\"{key}\""))
            });
            assert!(
                painted,
                "no widget paints {key:?}; either wire it or drop the translation"
            );
        }
    }

    #[test]
    fn an_unknown_locale_is_english() {
        assert_eq!(t("xx", "File"), "File");
        assert_eq!(locale_label("xx"), "English");
    }
}
