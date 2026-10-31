//! Text cut for showing.

/// How much of a string a transcript line keeps, with an ellipsis standing
/// for the rest. A tool's title and a tool's output are cut the same way and
/// at very different lengths; what they share is that the cut is presentation
/// and never touches what the model was given.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

/// A message as one quiet line says it: its markdown read as plain words, its
/// lines run together, and cut at a word to at most `max` characters, an
/// ellipsis standing for the rest. A quiet line has no room for a list or a
/// code block, and `**bold**` drawn as typed reads as noise.
pub fn gist(text: &str, max: usize) -> String {
    let words: Vec<String> = text
        .lines()
        .map(plain_line)
        .filter(|line| !line.is_empty())
        .collect();
    let said = words.join(" ");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.chars().count() <= max {
        return said;
    }
    let kept: String = said.chars().take(max.saturating_sub(1)).collect();
    let at_a_word = said.chars().nth(kept.chars().count()) == Some(' ');
    let cut = match kept.rfind(' ') {
        Some(space) if !at_a_word => &kept[..space],
        _ => kept.as_str(),
    };
    format!("{}…", cut.trim_end_matches([' ', ',', ';', ':', '—', '-']))
}

/// One line of markdown as the words it says: no heading, quote or list
/// mark, no fence or rule, links as their text, and no emphasis or code
/// marks. An underscore inside a word (`bind_path`) is the word's own.
fn plain_line(line: &str) -> String {
    let line = line.trim();
    if line.starts_with("```") || line.starts_with("~~~") {
        return String::new();
    }
    if line.len() >= 3 && line.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')) {
        return String::new();
    }
    if line.starts_with('|') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
        return String::new();
    }
    let line = line.trim_start_matches(['#', '>', ' ']);
    let line = without_list_mark(line);
    let chars: Vec<char> = line.chars().collect();
    let mut plain = String::with_capacity(line.len());
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let inside_a_word = |at: usize| {
            at > 0
                && chars[at - 1].is_alphanumeric()
                && chars.get(at + 1).is_some_and(|next| next.is_alphanumeric())
        };
        match c {
            '*' | '`' => {}
            '~' if chars.get(at + 1) == Some(&'~') || (at > 0 && chars[at - 1] == '~') => {}
            '_' if !inside_a_word(at) => {}
            '|' => plain.push(' '),
            '!' if chars.get(at + 1) == Some(&'[') => {}
            '[' => {
                if let Some((text, next)) = link(&chars, at) {
                    plain.push_str(&text);
                    at = next;
                    continue;
                }
                plain.push(c);
            }
            _ => plain.push(c),
        }
        at += 1;
    }
    plain.trim().to_string()
}

/// A line without the bullet or number a list item starts with.
fn without_list_mark(line: &str) -> &str {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return rest;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return rest;
        }
    }
    line
}

/// A markdown link `[text](target)` starting at `open`: its text, and where
/// the line goes on after it. None when it is only a bracket.
fn link(chars: &[char], open: usize) -> Option<(String, usize)> {
    let close = open + chars[open..].iter().position(|&c| c == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 1 + chars[close + 1..].iter().position(|&c| c == ')')?;
    let text: String = chars[open + 1..close]
        .iter()
        .filter(|c| !matches!(c, '*' | '`' | '_'))
        .collect();
    Some((text, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gist_reads_markdown_as_plain_words_on_one_line() {
        let said = "## The fix\n\nThe 0.46 Windows fix is the **bind path fix** in `server.rs`:\n\n\
                    - it binds `0.0.0.0` once\n1. see [the notes](https://example.com/notes)\n\n\
                    ```rust\nlet x = 1;\n```\n---\n> _quoted_ bind_path ~~stays~~ in ~/work";
        assert_eq!(
            gist(said, 400),
            "The fix The 0.46 Windows fix is the bind path fix in server.rs: it binds 0.0.0.0 once \
             see the notes let x = 1; quoted bind_path stays in ~/work"
        );
    }

    #[test]
    fn a_gist_is_cut_at_a_word() {
        let said = "The release notes say the bind path was fixed for Windows in 0.46.";
        let cut = gist(said, 30);
        assert_eq!(cut, "The release notes say the…");
        assert!(cut.chars().count() <= 30, "{cut}");
        assert_eq!(gist("one two three", 9), "one two…");
        assert_eq!(gist("short", 30), "short");
        assert_eq!(gist("", 30), "");
        let word = "x".repeat(50);
        assert_eq!(gist(&word, 10).chars().count(), 10);
    }
}
