//! One bounded, plain line for an agent completion.

pub const MAX_CHARS: usize = 132;

pub fn line(text: &str) -> Option<String> {
    let mut fence = None;
    for original in text.lines() {
        let original = original.trim();
        if let Some(marker) = ["```", "~~~"]
            .into_iter()
            .find(|marker| original.starts_with(marker))
        {
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let prose = original
            .trim_start_matches(['#', '>', ' '])
            .strip_prefix("- ")
            .or_else(|| original.strip_prefix("* "))
            .unwrap_or_else(|| original.trim_start_matches(['#', '>', ' ']))
            .trim_matches(['*', '_', '\r', ' ']);
        if prose.is_empty()
            || ["Summary", "Changes", "Verification", "Tests", "Next steps"]
                .iter()
                .any(|heading| prose.trim_end_matches(':').eq_ignore_ascii_case(heading))
        {
            continue;
        }
        let plain = prose.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut chars = plain.chars();
        let prefix = chars.by_ref().take(MAX_CHARS - 1).collect::<String>();
        if chars.nth(1).is_none() {
            return Some(plain);
        }
        return Some(format!("{}…", prefix.trim_end()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_progress_line_skips_generic_headings_and_source_code() {
        assert_eq!(
            line("# Summary\n\n```rs\nfn hello() {}\n```\n- Fixed   checkout validation."),
            Some("Fixed checkout validation.".into())
        );
        assert_eq!(line("```sh\ngit status\n```"), None);
    }

    #[test]
    fn a_progress_line_bounds_unicode_text() {
        let text = line(&"进展".repeat(100)).unwrap();
        assert_eq!(text.chars().count(), 132);
        assert!(text.ends_with('…'));
    }
}
