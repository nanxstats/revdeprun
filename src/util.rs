use crate::progress::Progress;

/// Returns a single-quoted R string literal with minimal escaping.
///
/// # Examples
///
/// ```
/// use revdeprun::util::r_string_literal;
///
/// assert_eq!(r_string_literal("/tmp/pkg"), "'/tmp/pkg'");
/// assert_eq!(r_string_literal("O'Reilly"), "'O\\'Reilly'");
/// ```
pub fn r_string_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' => literal.push_str("\\'"),
            '\\' => literal.push_str("\\\\"),
            _ => literal.push(ch),
        }
    }
    literal.push('\'');
    literal
}

/// Extracts a plausible repository name from a git URL or path-like string.
///
/// The function strips trailing `.git` suffixes and handles SSH-style URLs.
pub fn guess_repo_name(spec: &str) -> Option<String> {
    let trimmed = spec.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let candidate = trimmed
        .rsplit_once(['/', ':'])
        .map(|(_, tail)| tail)
        .unwrap_or(trimmed);

    let candidate = candidate.strip_suffix(".git").unwrap_or(candidate);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// Emits stdout/stderr captured from a command to the progress renderer.
pub fn emit_command_output(progress: &Progress, label: &str, stdout: &[u8], stderr: &[u8]) {
    emit_stream(progress, label, "stdout", stdout);
    emit_stream(progress, label, "stderr", stderr);
}

fn emit_stream(progress: &Progress, label: &str, stream: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    progress.println(format!("{label} {stream}:\n{trimmed}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_r_string_literals() {
        assert_eq!(r_string_literal(r#"abc"#), "'abc'");
        assert_eq!(r_string_literal(r#"O'Reilly"#), "'O\\'Reilly'");
        assert_eq!(r_string_literal(r#"C:\R"#), "'C:\\\\R'");
    }

    #[test]
    fn infers_repository_name() {
        assert_eq!(
            guess_repo_name("https://github.com/nanxstats/ggsci.git"),
            Some("ggsci".to_string())
        );
        assert_eq!(
            guess_repo_name("git@github.com:nanxstats/ggsci.git"),
            Some("ggsci".to_string())
        );
        assert_eq!(guess_repo_name(""), None);
    }
}
