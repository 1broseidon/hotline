//! Keeping a secret from leaking: a token compared without saying, through
//! timing, how much of it matched, and tokens cut out of text before it is
//! shown or kept.

/// Equal without leaking, through timing, how much of a wrong token matched.
pub fn same_secret(presented: &str, expected: &str) -> bool {
    let presented = presented.as_bytes();
    let expected = expected.as_bytes();
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}
/// `text` with each bearer token and API key in it replaced by `[redacted]`.
pub fn redact_text(text: &str) -> String {
    let mut output = text.to_string();
    for prefix in ["Bearer ", "bearer ", "sk-", "sk_", "ghp_", "github_pat_"] {
        let mut offset = 0;
        while let Some(found) = output[offset..].find(prefix) {
            let start = offset + found;
            let end = output[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
                .map_or(output.len(), |n| start + n);
            let end = if prefix.ends_with(' ') {
                output[start + prefix.len()..]
                    .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
                    .map_or(output.len(), |n| start + prefix.len() + n)
            } else {
                end
            };
            output.replace_range(start..end, "[redacted]");
            offset = start + "[redacted]".len();
        }
    }
    output
}
