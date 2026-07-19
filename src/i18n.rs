use clap::ValueEnum;
use std::env;
use std::sync::atomic::{AtomicU8, Ordering};


const LANG_EN: u8 = 0;
const LANG_ZH: u8 = 1;

static SELECTED_LANGUAGE: AtomicU8 = AtomicU8::new(LANG_EN);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Chinese => "zh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum LanguageChoice {
    Auto,
    En,
    Zh,
}

pub fn init_language(choice: LanguageChoice) -> Language {
    let lang = match choice {
        LanguageChoice::Auto => {
            if is_chinese_locale() {
                Language::Chinese
            } else {
                Language::English
            }
        }
        LanguageChoice::En => Language::English,
        LanguageChoice::Zh => Language::Chinese,
    };
    rust_i18n::set_locale(lang.code());
    SELECTED_LANGUAGE.store(
        match lang {
            Language::English => LANG_EN,
            Language::Chinese => LANG_ZH,
        },
        Ordering::Relaxed,
    );
    lang
}

#[allow(dead_code)]
pub fn current_language() -> Language {
    match SELECTED_LANGUAGE.load(Ordering::Relaxed) {
        LANG_ZH => Language::Chinese,
        _ => Language::English,
    }
}

/// Inspect CLI args before Clap so --lang already affects help text rendering.
pub fn detect_language_choice_from_args() -> LanguageChoice {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        match arg.as_str() {
            "--lang" | "-L" => {
                if let Some(value) = args.next() {
                    if let Some(choice) = parse_choice(&value) {
                        return choice;
                    }
                }
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--lang=") {
                    if let Some(choice) = parse_choice(value) {
                        return choice;
                    }
                }
                if let Some(val) = arg.strip_prefix("-L") {
                    if !val.is_empty() {
                        if let Some(choice) = parse_choice(val) {
                            return choice;
                        }
                    }
                }
            }
        }
    }
    LanguageChoice::Auto
}

fn parse_choice(raw: &str) -> Option<LanguageChoice> {
    LanguageChoice::from_str(raw, true).ok()
}

/// Convenience function for inline bilingual strings.
/// Uses current_language() to choose between en and zh.
#[allow(dead_code)]
pub fn tr<'a>(en: &'a str, zh: &'a str) -> &'a str {
    match current_language() {
        Language::English => en,
        Language::Chinese => zh,
    }
}

fn get_system_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"))
}

pub fn is_chinese_locale() -> bool {
    let locale = get_system_locale();
    locale.starts_with("zh-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_english() {
        let lang = init_language(LanguageChoice::En);
        assert_eq!(lang, Language::English);
        assert_eq!(lang.code(), "en");
    }

    #[test]
    fn test_init_chinese() {
        let lang = init_language(LanguageChoice::Zh);
        assert_eq!(lang, Language::Chinese);
        assert_eq!(lang.code(), "zh");
    }

    #[test]
    fn test_init_auto() {
        // Auto should return based on system locale
        let lang = init_language(LanguageChoice::Auto);
        // Just verify it returns one of the two valid values
        assert!(lang == Language::English || lang == Language::Chinese);
    }

    #[test]
    fn test_tr_en() {
        init_language(LanguageChoice::En);
        assert_eq!(tr("hello", "你好"), "hello");
    }

    #[test]
    fn test_tr_zh() {
        init_language(LanguageChoice::Zh);
        assert_eq!(tr("hello", "你好"), "你好");
    }

    #[test]
    fn test_parse_choice() {
        assert_eq!(parse_choice("auto"), Some(LanguageChoice::Auto));
        assert_eq!(parse_choice("en"), Some(LanguageChoice::En));
        assert_eq!(parse_choice("zh"), Some(LanguageChoice::Zh));
        assert_eq!(parse_choice("fr"), None);
    }

    #[test]
    fn test_current_language() {
        init_language(LanguageChoice::En);
        assert_eq!(current_language(), Language::English);

        init_language(LanguageChoice::Zh);
        assert_eq!(current_language(), Language::Chinese);
    }
}
