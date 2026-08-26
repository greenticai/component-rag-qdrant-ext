//! Character-window text splitting. Pure — no WIT imports.

/// Split `text` into overlapping windows of at most `max_chars` **characters**
/// (not bytes — a byte window would split multi-byte UTF-8 mid-character).
///
/// Returns an empty vector for empty or whitespace-only input: there is nothing
/// worth embedding, and an empty chunk would cost an embedding call and occupy a
/// point id forever.
#[must_use]
pub fn chunk_text(text: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    if text.trim().is_empty() || max_chars == 0 {
        return Vec::new();
    }

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }

    // `max(1)` is load-bearing: an overlap >= max_chars would give a step of 0
    // and spin forever. Clamping degrades the overlap rather than hanging.
    let step = max_chars.saturating_sub(overlap_chars).max(1);

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = start.saturating_add(max_chars).min(chars.len());
        chunks.push(chars[start..end].iter().collect::<String>());
        if end == chars.len() {
            break;
        }
        start = start.saturating_add(step);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_only_text_yields_no_chunks() {
        assert!(chunk_text("", 100, 10).is_empty());
        assert!(chunk_text("   \n\t  ", 100, 10).is_empty());
    }

    #[test]
    fn text_shorter_than_the_window_is_one_chunk_verbatim() {
        assert_eq!(chunk_text("hello world", 100, 10), vec!["hello world"]);
    }

    #[test]
    fn text_exactly_the_window_is_one_chunk() {
        let text = "a".repeat(100);
        assert_eq!(chunk_text(&text, 100, 10), vec![text]);
    }

    #[test]
    fn longer_text_splits_with_the_requested_overlap() {
        let text: String = ('a'..='z').cycle().take(25).collect();
        let chunks = chunk_text(&text, 10, 3);
        // step = 10 - 3 = 7 → starts at 0, 7, 14, 21
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].chars().count(), 10);
        // The overlap is real: chunk 1 begins with chunk 0's last 3 chars.
        let tail: String = chunks[0].chars().skip(7).collect();
        assert!(chunks[1].starts_with(&tail));
        // The final chunk is the remainder, not padded.
        assert_eq!(chunks[3].chars().count(), 4);
    }

    #[test]
    fn multibyte_text_is_never_split_mid_character() {
        // 3 bytes per char — a byte-based splitter would corrupt these.
        let text = "日本語".repeat(20); // 60 chars, 180 bytes
        let chunks = chunk_text(&text, 7, 2);
        for chunk in &chunks {
            assert!(chunk.chars().all(|c| "日本語".contains(c)));
        }
        assert!(chunks.iter().all(|c| c.chars().count() <= 7));
    }

    #[test]
    fn a_degenerate_overlap_still_makes_progress() {
        // overlap >= max would give step 0 and loop forever without the clamp.
        let chunks = chunk_text(&"x".repeat(20), 5, 99);
        assert!(chunks.len() >= 4);
    }
}
