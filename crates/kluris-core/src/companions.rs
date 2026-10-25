//! The embedded Specmint companion playbooks (names only, for now).

/// Known companions, in canonical order (v2 `companions.KNOWN`).
pub const KNOWN: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];

/// Known companion names, de-duplicated, in canonical order; unknown names
/// are dropped (v2 `companions.normalize`).
pub fn normalize<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|known| names.iter().any(|n| n.as_ref() == **known))
        .map(|known| known.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_filters_dedupes_and_orders() {
        let names = [
            "specmint-tdd-html",
            "bogus",
            "specmint-core",
            "specmint-core",
        ];
        assert_eq!(
            normalize(&names),
            vec!["specmint-core", "specmint-tdd-html"]
        );
    }

    #[test]
    fn normalize_empty() {
        assert!(normalize::<&str>(&[]).is_empty());
    }
}
