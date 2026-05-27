use std::path::Path;

/// Splits a path into (directory-part-including-trailing-slash, trailing-segment).
/// No '/' present → ("", text).
pub fn split_path(text: &str) -> (String, String) {
    match text.rfind('/') {
        Some(i) => (text[..=i].to_string(), text[i + 1..].to_string()),
        None => (String::new(), text.to_string()),
    }
}

/// Case-insensitive prefix filter over directory names. Hides names starting with
/// '.' unless `filter` itself starts with '.'. Sorted case-insensitively.
pub fn filter_subdirs(names: &[String], filter: &str) -> Vec<String> {
    let lower = filter.to_lowercase();
    let show_hidden = filter.starts_with('.');
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| show_hidden || !n.starts_with('.'))
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .cloned()
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Immediate subdirectory names of `base`. Missing/unreadable path → empty list.
pub fn read_subdirs(base: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Convenience: subdirectories of `base` filtered by `filter`.
pub fn list(base: &str, filter: &str) -> Vec<String> {
    filter_subdirs(&read_subdirs(Path::new(base)), filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_handles_mid_trailing_and_none() {
        assert_eq!(
            split_path("~/projects/pets/ag"),
            ("~/projects/pets/".to_string(), "ag".to_string())
        );
        assert_eq!(
            split_path("~/projects/pets/"),
            ("~/projects/pets/".to_string(), "".to_string())
        );
        assert_eq!(split_path("ag"), ("".to_string(), "ag".to_string()));
    }

    #[test]
    fn filter_subdirs_prefix_case_insensitive_sorted() {
        let names = vec![
            "notes".to_string(),
            "agents".to_string(),
            "Apps".to_string(),
            ".git".to_string(),
        ];
        assert_eq!(
            filter_subdirs(&names, "a"),
            vec!["agents".to_string(), "Apps".to_string()]
        );
    }

    #[test]
    fn filter_subdirs_hides_dotdirs_unless_filter_starts_with_dot() {
        let names = vec!["src".to_string(), ".git".to_string(), ".config".to_string()];
        assert_eq!(filter_subdirs(&names, ""), vec!["src".to_string()]);
        assert_eq!(filter_subdirs(&names, ".g"), vec![".git".to_string()]);
    }

    #[test]
    fn read_subdirs_returns_only_directories() {
        let base = std::env::temp_dir().join(format!("cm_browse_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("alpha")).unwrap();
        std::fs::create_dir_all(base.join("beta")).unwrap();
        std::fs::write(base.join("file.txt"), b"x").unwrap();

        let mut got = read_subdirs(&base);
        got.sort();
        assert_eq!(got, vec!["alpha".to_string(), "beta".to_string()]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn read_subdirs_missing_path_is_empty() {
        assert!(read_subdirs(Path::new("/no/such/path/xyz123")).is_empty());
    }
}
