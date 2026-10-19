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
