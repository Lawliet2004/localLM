//! Exact-text editing with tolerant matching and line-ending preservation.
//!
//! Coding agents reconstruct strings from memory: one smart quote, one
//! trailing space or one CRLF is enough to make an exact edit fail even
//! though the intent is unambiguous. The matcher below locates the intended
//! region in three tiers and rewrites it without disturbing the rest of the
//! file.
//!
//! Tiers, in order:
//! 1. [`MatchKind::Exact`] — byte-for-byte occurrence of the search text.
//! 2. [`MatchKind::Normalized`] — occurrence after folding typographic
//!    variants (smart quotes, dash variants, exotic spaces) on both sides.
//!    Offsets are reported against the original source.
//! 3. [`MatchKind::LineTrimmed`] — the search text matches a run of whole
//!    lines once trailing whitespace is trimmed from each line.
//!
//! A tier is only consulted when the previous one found nothing, so an exact
//! match always wins.
//!
//! Replacements reuse the line endings of the region they replace: a CRLF
//! file stays CRLF, a file with mixed endings keeps each region's own
//! ending, and bytes outside the replaced region are never rewritten.

/// Where a match was found in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    /// Byte offset of the first byte of the match in the original source.
    pub start: usize,
    /// Byte offset one past the last byte of the match in the original source.
    pub end: usize,
}

/// How a replacement was located.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    /// The search text was found byte-for-byte.
    Exact,
    /// The search text was found after folding typographic variants.
    Normalized,
    /// The search text was found as whole lines with trailing whitespace ignored.
    LineTrimmed,
}

impl MatchKind {
    /// Stable, model-visible name of the tier.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::Exact => "exact",
            MatchKind::Normalized => "normalized",
            MatchKind::LineTrimmed => "line_trimmed",
        }
    }
}

/// The result of matching `old_text` against `source`.
#[derive(Clone, Debug)]
pub struct MatchReport {
    /// The tier that produced the matches.
    pub kind: MatchKind,
    /// Every occurrence, ordered by ascending offset.
    pub matches: Vec<Match>,
}

impl MatchReport {
    /// Number of occurrences found.
    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// Whether the search text was found at all.
    pub fn found(&self) -> bool {
        !self.matches.is_empty()
    }

    /// A human-readable description of how the match was made, for the tool result.
    pub fn describe(&self) -> String {
        match self.kind {
            MatchKind::Exact => "exact match".to_string(),
            MatchKind::Normalized => {
                "normalized match: smart quotes, dash variants and exotic spaces were folded".to_string()
            }
            MatchKind::LineTrimmed => {
                "line-trimmed match: trailing whitespace on each line was ignored".to_string()
            }
        }
    }
}

/// Find every occurrence of `old_text` in `source`, tolerating the
/// normalizations below. Returns an empty report when nothing matches.
pub fn find_matches(source: &str, old_text: &str) -> MatchReport {
    if old_text.is_empty() {
        return MatchReport {
            kind: MatchKind::Exact,
            matches: Vec::new(),
        };
    }
    // Tier 1: byte-for-byte.
    let exact = find_all(source, old_text);
    if !exact.is_empty() {
        return MatchReport {
            kind: MatchKind::Exact,
            matches: exact.into_iter().map(|(start, end)| Match { start, end }).collect(),
        };
    }
    // Tiers 2 and 3 work on a folded view of the source: line endings are
    // normalized to LF and typographic variants are folded, so a CRLF file or
    // a smart quote never blocks a match. Offsets are mapped back to the
    // original bytes.
    let view = View::build(source);
    let pattern = fold(old_text);
    // Tier 2: typographic normalization on both sides.
    let normalized = find_all(&view.text, &pattern);
    if !normalized.is_empty() {
        return MatchReport {
            kind: MatchKind::Normalized,
            matches: normalized
                .into_iter()
                .map(|(start, end)| Match {
                    start: view.source_start(start),
                    end: view.source_end(end),
                })
                .collect(),
        };
    }
    // Tier 3: whole lines, trailing whitespace ignored.
    MatchReport {
        kind: MatchKind::LineTrimmed,
        matches: find_line_trimmed(&view, &pattern),
    }
}

/// Replace occurrences of `old_text` with `new_text`.
///
/// `replace_all` false replaces exactly one occurrence and fails when the
/// search text is ambiguous (more than one match).
/// `replace_all` true replaces every occurrence.
///
/// Line endings: the replacement reuses the line endings of the region it
/// replaces, so a CRLF file stays CRLF and untouched bytes are never
/// rewritten. A file with mixed endings keeps each region's own ending.
///
/// Returns the new content and how many occurrences were replaced.
pub fn replace(
    source: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<(String, usize, MatchKind), String> {
    if old_text.is_empty() {
        return Err(
            "The text to replace must not be empty. Use write to create or overwrite a file.".into(),
        );
    }
    if old_text == new_text {
        return Err(
            "The text to replace is identical to the replacement text. Provide different replacement text."
                .into(),
        );
    }
    let report = find_matches(source, old_text);
    if !report.found() {
        return Err(
            "The text to replace was not found. Checked exact, typographic and trailing-whitespace variants."
                .into(),
        );
    }
    let count = report.count();
    if !replace_all && count > 1 {
        return Err(format!(
            "The text to replace occurs {count} times. Add more surrounding context to make it unique, or set replace_all to true."
        ));
    }
    // A search/replacement pair that differs only in carriage returns is a
    // request to change the line endings of the matched region: the
    // replacement bytes are applied verbatim.
    let verbatim = old_text.replace('\r', "") == new_text.replace('\r', "");
    let dominant = dominant_terminator(source);
    let mut updated = String::with_capacity(source.len() + new_text.len());
    let mut cursor = 0usize;
    for located in &report.matches {
        updated.push_str(&source[cursor..located.start]);
        if verbatim {
            updated.push_str(new_text);
        } else {
            updated.push_str(&build_replacement(
                &source[located.start..located.end],
                new_text,
                dominant,
            ));
        }
        cursor = located.end;
    }
    updated.push_str(&source[cursor..]);
    Ok((updated, count, report.kind))
}

/// Replace one already located region of `source`.
///
/// This is the primitive behind [`replace`]: the region's own line endings
/// are reused, so the rest of the file keeps its style. `dominant` is the
/// terminator used for lines that exist only in the replacement; callers that
/// resolve several edits against the same original text pass that text's
/// [`dominant_terminator`] so every edit keeps one style.
pub fn replace_at(source: &str, located: &Match, new_text: &str, dominant: &str) -> String {
    let mut updated = String::with_capacity(source.len() + new_text.len());
    updated.push_str(&source[..located.start]);
    updated.push_str(&build_replacement(
        &source[located.start..located.end],
        new_text,
        dominant,
    ));
    updated.push_str(&source[located.end..]);
    updated
}

/// The line ending used most often in `source`.
///
/// `\n` is used when the source has no line terminator at all or when both
/// endings are equally common.
pub fn dominant_terminator(source: &str) -> &str {
    dominant_terminator_with(source, "\n")
}

// ---------------------------------------------------------------------------
// Folding
// ---------------------------------------------------------------------------

const SMART_SINGLE_QUOTES: [char; 4] = ['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];
const SMART_DOUBLE_QUOTES: [char; 4] = ['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'];
const DASH_VARIANTS: [char; 7] = [
    '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
];
const EXOTIC_SPACES: [char; 13] = [
    '\u{00A0}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
    '\u{2008}', '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}',
];

/// Fold one character onto its plain ASCII equivalent.
fn fold_char(character: char) -> char {
    if SMART_SINGLE_QUOTES.contains(&character) {
        return '\'';
    }
    if SMART_DOUBLE_QUOTES.contains(&character) {
        return '"';
    }
    if DASH_VARIANTS.contains(&character) {
        return '-';
    }
    if EXOTIC_SPACES.contains(&character) {
        return ' ';
    }
    character
}

/// Fold a whole text: CRLF becomes LF and typographic variants become ASCII.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' {
            if let Some('\n') = characters.peek() {
                folded.push('\n');
                characters.next();
                continue;
            }
        }
        folded.push(fold_char(character));
    }
    folded
}

// ---------------------------------------------------------------------------
// The folded view of a source
// ---------------------------------------------------------------------------

/// One source character and the bytes it produced in the folded view.
struct Span {
    view_start: usize,
    source_start: usize,
    source_end: usize,
}

/// The source folded onto LF line endings and plain ASCII punctuation, with a
/// map back to the original byte offsets.
struct View {
    text: String,
    spans: Vec<Span>,
    source_len: usize,
}

impl View {
    fn build(source: &str) -> View {
        let mut text = String::with_capacity(source.len());
        let mut spans = Vec::new();
        let mut characters = source.char_indices().peekable();
        while let Some((index, character)) = characters.next() {
            let end = index + character.len_utf8();
            if character == '\r' {
                if let Some(&(_, '\n')) = characters.peek() {
                    let view_start = text.len();
                    text.push('\n');
                    spans.push(Span {
                        view_start,
                        source_start: index,
                        source_end: end + 1,
                    });
                    characters.next();
                    continue;
                }
            }
            let view_start = text.len();
            text.push(fold_char(character));
            spans.push(Span {
                view_start,
                source_start: index,
                source_end: end,
            });
        }
        View {
            text,
            spans,
            source_len: source.len(),
        }
    }

    /// Index of the span holding the byte at `offset`.
    fn span_at(&self, offset: usize) -> Option<usize> {
        let mut low = 0usize;
        let mut high = self.spans.len();
        while low < high {
            let middle = (low + high) / 2;
            if self.spans[middle].view_start <= offset {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low == 0 {
            None
        } else {
            Some(low - 1)
        }
    }

    /// Map a folded offset that starts a character back to the source.
    fn source_start(&self, offset: usize) -> usize {
        if offset >= self.text.len() {
            return self.source_len;
        }
        match self.span_at(offset) {
            Some(index) => self.spans[index].source_start,
            None => 0,
        }
    }

    /// Map a folded offset that ends a character back to the source.
    fn source_end(&self, offset: usize) -> usize {
        if offset >= self.text.len() {
            return self.source_len;
        }
        if offset == 0 {
            return 0;
        }
        match self.span_at(offset - 1) {
            Some(index) => self.spans[index].source_end,
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Every non-overlapping occurrence of `needle` in `haystack`.
fn find_all(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let mut cursor = 0usize;
    while let Some(position) = haystack[cursor..].find(needle) {
        let start = cursor + position;
        found.push((start, start + needle.len()));
        cursor = start + needle.len();
    }
    found
}

/// One line of a text: its content and the terminator that ended it.
struct Line<'a> {
    content: &'a str,
    terminator: &'a str,
}

/// Split a text into lines, keeping each line's terminator.
fn split_lines(text: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let Some(position) = text[start..].find('\n') else {
            lines.push(Line {
                content: &text[start..],
                terminator: "",
            });
            break;
        };
        let newline = start + position;
        let (content, terminator) = if newline > start && text.as_bytes()[newline - 1] == b'\r' {
            (&text[start..newline - 1], "\r\n")
        } else {
            (&text[start..newline], "\n")
        };
        lines.push(Line { content, terminator });
        start = newline + 1;
    }
    lines
}

/// The byte range of one line's content plus the start of the next line.
struct LineSpan {
    start: usize,
    end: usize,
    next: usize,
}

/// Split a folded text into line spans.
fn split_line_spans(text: &str) -> Vec<LineSpan> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let Some(position) = text[start..].find('\n') else {
            spans.push(LineSpan {
                start,
                end: text.len(),
                next: text.len(),
            });
            break;
        };
        let newline = start + position;
        let mut end = newline;
        if end > start && text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        spans.push(LineSpan {
            start,
            end,
            next: newline + 1,
        });
        start = newline + 1;
    }
    spans
}

/// Whole-line matches where trailing whitespace is ignored on both sides.
fn find_line_trimmed(view: &View, pattern: &str) -> Vec<Match> {
    let mut found = Vec::new();
    let view_lines = split_line_spans(&view.text);
    let pattern_lines = split_line_spans(pattern);
    if pattern_lines.is_empty() {
        return found;
    }
    let wanted = pattern_lines.len();
    let pattern_ends_with_newline = pattern.ends_with('\n');
    let mut index = 0usize;
    while index + wanted <= view_lines.len() {
        let mut aligned = true;
        for offset in 0..wanted {
            let line = &view_lines[index + offset];
            let search = &pattern_lines[offset];
            if view.text[line.start..line.end].trim_end()
                != pattern[search.start..search.end].trim_end()
            {
                aligned = false;
                break;
            }
        }
        if !aligned {
            index += 1;
            continue;
        }
        let last = index + wanted - 1;
        let end = if pattern_ends_with_newline {
            if view_lines[last].next <= view_lines[last].end {
                // The last matched line has no terminator, so a search text
                // ending in a newline cannot match here.
                index += 1;
                continue;
            }
            view_lines[last].next
        } else {
            view_lines[last].end
        };
        found.push(Match {
            start: view.source_start(view_lines[index].start),
            end: view.source_end(end),
        });
        index += wanted;
    }
    found
}

// ---------------------------------------------------------------------------
// Line-ending preservation
// ---------------------------------------------------------------------------

/// The line ending used most often in `text`; `fallback` breaks a tie.
fn dominant_terminator_with<'a>(text: &str, fallback: &'a str) -> &'a str {
    let bytes = text.as_bytes();
    let mut carriage = 0usize;
    let mut plain = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                carriage += 1;
            } else {
                plain += 1;
            }
        }
    }
    match carriage.cmp(&plain) {
        std::cmp::Ordering::Greater => "\r\n",
        std::cmp::Ordering::Less => "\n",
        std::cmp::Ordering::Equal => fallback,
    }
}

/// One aligned step between the replaced region and the replacement.
enum Block {
    /// The region line and the replacement line carry the same content.
    Keep(usize, usize),
    /// A run of region lines replaced by a run of replacement lines.
    Change {
        region: Vec<usize>,
        replacement: Vec<usize>,
    },
}

/// Longest-common-subsequence alignment over trimmed line content.
fn align_lines(region: &[Line<'_>], replacement: &[Line<'_>]) -> Vec<Block> {
    let left: Vec<&str> = region
        .iter()
        .map(|line| line.content.trim_end())
        .collect();
    let right: Vec<&str> = replacement
        .iter()
        .map(|line| line.content.trim_end())
        .collect();
    // A pathological region would make the quadratic table too large; the
    // positional pairing below keeps the region usable instead.
    const MAX_CELLS: usize = 1_000_000;
    if left.len().saturating_mul(right.len()) > MAX_CELLS {
        return vec![Block::Change {
            region: (0..left.len()).collect(),
            replacement: (0..right.len()).collect(),
        }];
    }
    let columns = right.len() + 1;
    let mut table = vec![0u32; (left.len() + 1) * columns];
    for row in (0..left.len()).rev() {
        for column in (0..right.len()).rev() {
            table[row * columns + column] = if left[row] == right[column] {
                table[(row + 1) * columns + column + 1] + 1
            } else {
                table[(row + 1) * columns + column].max(table[row * columns + column + 1])
            };
        }
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut region_pending: Vec<usize> = Vec::new();
    let mut replacement_pending: Vec<usize> = Vec::new();
    let mut row = 0usize;
    let mut column = 0usize;
    while row < left.len() && column < right.len() {
        if left[row] == right[column] {
            if !region_pending.is_empty() || !replacement_pending.is_empty() {
                blocks.push(Block::Change {
                    region: std::mem::take(&mut region_pending),
                    replacement: std::mem::take(&mut replacement_pending),
                });
            }
            blocks.push(Block::Keep(row, column));
            row += 1;
            column += 1;
        } else if table[(row + 1) * columns + column] >= table[row * columns + column + 1] {
            region_pending.push(row);
            row += 1;
        } else {
            replacement_pending.push(column);
            column += 1;
        }
    }
    while row < left.len() {
        region_pending.push(row);
        row += 1;
    }
    while column < right.len() {
        replacement_pending.push(column);
        column += 1;
    }
    if !region_pending.is_empty() || !replacement_pending.is_empty() {
        blocks.push(Block::Change {
            region: region_pending,
            replacement: replacement_pending,
        });
    }
    blocks
}

/// Build the replacement text for one region.
///
/// Lines that survive keep the region's terminator, lines replaced in place
/// inherit the terminator of the line they replace, and lines that only
/// exist in the replacement take the file's dominant terminator.
fn build_replacement(region: &str, replacement: &str, dominant: &str) -> String {
    let region_lines = split_lines(region);
    let replacement_lines = split_lines(replacement);
    let blocks = align_lines(&region_lines, &replacement_lines);
    // The region's own style is the first terminator it carries; a region
    // without any terminator falls back to the file's dominant one. Its tail
    // terminator is the terminator of its last line and decides whether the
    // replacement ends with a line break.
    let style = region_lines
        .iter()
        .find_map(|line| {
            if line.terminator.is_empty() {
                None
            } else {
                Some(line.terminator)
            }
        })
        .unwrap_or(dominant);
    let tail = region_lines.last().map(|line| line.terminator).unwrap_or("");
    let mut out: Vec<(String, String)> = Vec::new();
    for block in blocks {
        match block {
            Block::Keep(region_index, replacement_index) => {
                let carried = region_lines[region_index].terminator;
                let terminator = if carried.is_empty() { style } else { carried };
                out.push((
                    replacement_lines[replacement_index].content.to_string(),
                    terminator.to_string(),
                ));
            }
            Block::Change {
                region: region_indices,
                replacement: replacement_indices,
            } => {
                let steps = region_indices.len().max(replacement_indices.len());
                for step in 0..steps {
                    let terminator = if step < region_indices.len() {
                        let carried = region_lines[region_indices[step]].terminator;
                        if carried.is_empty() {
                            style
                        } else {
                            carried
                        }
                    } else {
                        style
                    };
                    if step < replacement_indices.len() {
                        out.push((
                            replacement_lines[replacement_indices[step]].content.to_string(),
                            terminator.to_string(),
                        ));
                    }
                }
            }
        }
    }
    // The replacement ends the way the region ended.
    if let Some(last) = out.last_mut() {
        last.1 = tail.to_string();
    }
    let mut text = String::new();
    for (content, terminator) in out {
        text.push_str(&content);
        text.push_str(&terminator);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_single_match_replaces() {
        let source = "alpha beta gamma";
        let report = find_matches(source, "beta");
        assert_eq!(report.kind, MatchKind::Exact);
        assert_eq!(report.count(), 1);
        assert!(report.found());
        assert_eq!(report.matches[0].start, 6);
        assert_eq!(report.matches[0].end, 10);
        let (updated, count, kind) = replace(source, "beta", "BETA", false).unwrap();
        assert_eq!(updated, "alpha BETA gamma");
        assert_eq!(count, 1);
        assert_eq!(kind, MatchKind::Exact);
    }

    #[test]
    fn three_occurrences_without_replace_all_is_an_error_naming_the_count() {
        let source = "a x a x a";
        let error = replace(source, "a", "b", false).unwrap_err();
        assert!(error.contains('3'), "{error}");
        assert!(error.contains("replace_all"), "{error}");
        assert!(error.contains("context"), "{error}");
    }

    #[test]
    fn three_occurrences_with_replace_all_replaces_all_three() {
        let source = "a x a x a";
        let (updated, count, kind) = replace(source, "a", "b", true).unwrap();
        assert_eq!(updated, "b x b x b");
        assert_eq!(count, 3);
        assert_eq!(kind, MatchKind::Exact);
    }

    #[test]
    fn replace_all_with_no_match_is_an_error() {
        let error = replace("alpha", "beta", "gamma", true).unwrap_err();
        assert!(error.contains("not found"), "{error}");
    }

    #[test]
    fn smart_quotes_match_after_normalization() {
        let source = "let s = \u{201C}hello\u{201D};\n";
        let report = find_matches(source, "\"hello\"");
        assert_eq!(report.kind, MatchKind::Normalized);
        assert_eq!(report.count(), 1);
        let (updated, count, kind) = replace(source, "\"hello\"", "\"world\"", false).unwrap();
        assert_eq!(updated, "let s = \"world\";\n");
        assert_eq!(count, 1);
        assert_eq!(kind, MatchKind::Normalized);
    }

    #[test]
    fn normalized_offsets_point_at_the_original_bytes() {
        let source = "let s = \u{201C}hello\u{201D};\n";
        let report = find_matches(source, "\"hello\"");
        let located = report.matches[0];
        // The smart quotes are three bytes each in the original source.
        assert_eq!(located.start, 8);
        assert_eq!(located.end, 19);
        assert_eq!(&source[located.start..located.end], "\u{201C}hello\u{201D}");
        // The bytes around the match are untouched.
        assert_eq!(&source[..located.start], "let s = ");
        assert_eq!(&source[located.end..], ";\n");
    }

    #[test]
    fn en_dash_matches_a_plain_hyphen() {
        let source = "range 1\u{2013}10 inclusive";
        let report = find_matches(source, "1-10");
        assert_eq!(report.kind, MatchKind::Normalized);
        assert_eq!(report.count(), 1);
        let (updated, _, _) = replace(source, "1-10", "1..=10", false).unwrap();
        assert_eq!(updated, "range 1..=10 inclusive");
    }

    #[test]
    fn non_breaking_space_matches_a_plain_space() {
        let source = "let\u{00A0}x = 1;";
        let report = find_matches(source, "let x = 1;");
        assert_eq!(report.kind, MatchKind::Normalized);
        let (updated, _, _) = replace(source, "let x = 1;", "let y = 1;", false).unwrap();
        assert_eq!(updated, "let y = 1;");
    }

    #[test]
    fn trailing_whitespace_only_difference_matches_line_trimmed() {
        let source = "alpha   \nbeta\n";
        let report = find_matches(source, "alpha\nbeta");
        assert_eq!(report.kind, MatchKind::LineTrimmed);
        assert_eq!(report.count(), 1);
        assert_eq!(report.matches[0].start, 0);
        // The trailing newline of the matched region is excluded because the
        // search text has none.
        assert_eq!(report.matches[0].end, 13);
        let (updated, _, kind) = replace(source, "alpha\nbeta", "ALPHA\nBETA", false).unwrap();
        assert_eq!(updated, "ALPHA\nBETA\n");
        assert_eq!(kind, MatchKind::LineTrimmed);
    }

    #[test]
    fn crlf_file_matches_lf_search_text_and_keeps_crlf() {
        let source = "let a = 1;\r\nlet b = 2;\r\n";
        let report = find_matches(source, "let a = 1;\nlet b = 2;");
        assert_eq!(report.kind, MatchKind::Normalized);
        assert_eq!(report.count(), 1);
        let (updated, _, _) =
            replace(source, "let a = 1;\nlet b = 2;", "let a = 1;\nlet b = 3;", false).unwrap();
        assert_eq!(updated, "let a = 1;\r\nlet b = 3;\r\n");
        assert!(!updated.contains('\n') || updated.matches('\n').count() == 2);
        assert_eq!(updated.matches("\r\n").count(), 2);
    }

    #[test]
    fn mixed_line_endings_keep_each_region_own_terminator() {
        let source = "one\r\ntwo\nthree\r\nfour\n";
        let (updated, _, _) = replace(source, "two\nthree", "TWO\nTHREE", false).unwrap();
        assert_eq!(updated, "one\r\nTWO\nTHREE\r\nfour\n");
        let (updated, _, _) = replace(source, "one\r\ntwo", "ONE\r\nTWO", false).unwrap();
        assert_eq!(updated, "ONE\r\nTWO\nthree\r\nfour\n");
    }

    #[test]
    fn empty_search_text_is_an_error() {
        let error = replace("alpha", "", "beta", false).unwrap_err();
        assert!(error.contains("must not be empty"), "{error}");
        assert!(error.contains("write"), "{error}");
    }

    #[test]
    fn identical_search_and_replacement_is_an_error() {
        let error = replace("alpha", "beta", "beta", false).unwrap_err();
        assert!(error.contains("identical"), "{error}");
    }

    #[test]
    fn missing_text_reports_the_normalizations_that_were_tried() {
        let error = replace("alpha", "beta", "gamma", false).unwrap_err();
        assert!(error.contains("not found"), "{error}");
        assert!(error.contains("typographic"), "{error}");
        assert!(error.contains("trailing-whitespace"), "{error}");
    }

    #[test]
    fn exact_match_wins_over_the_looser_tiers() {
        let source = "alpha beta";
        let report = find_matches(source, "alpha");
        assert_eq!(report.kind, MatchKind::Exact);
    }

    #[test]
    fn source_without_line_terminators_uses_the_fallback_terminator() {
        let source = "alpha";
        let (updated, _, _) = replace(source, "alpha", "one\ntwo", false).unwrap();
        assert_eq!(updated, "one\ntwo");
    }

    #[test]
    fn a_replacement_with_crlf_is_normalized_to_the_region_style() {
        let source = "alpha\r\nbeta\r\n";
        let (updated, _, _) = replace(source, "beta", "gamma\r\ndelta", false).unwrap();
        assert_eq!(updated, "alpha\r\ngamma\r\ndelta\r\n");
    }

    #[test]
    fn a_search_and_replacement_differing_only_in_carriage_returns_are_verbatim() {
        let source = "one\ntwo\n";
        let (updated, _, _) = replace(source, "one\ntwo\n", "one\r\ntwo\r\n", false).unwrap();
        assert_eq!(updated, "one\r\ntwo\r\n");
    }

    #[test]
    fn describe_reports_the_tier() {
        let exact = find_matches("alpha", "alpha");
        assert!(exact.describe().contains("exact"));
        let normalized = find_matches("\u{201C}alpha\u{201D}", "\"alpha\"");
        assert!(normalized.describe().contains("normalized"));
        let trimmed = find_matches("alpha  \nbeta", "alpha\nbeta");
        assert!(trimmed.describe().contains("line-trimmed"));
    }

    #[test]
    fn an_empty_report_reports_no_match() {
        let report = find_matches("alpha", "beta");
        assert!(!report.found());
        assert_eq!(report.count(), 0);
        assert!(report.matches.is_empty());
    }

    #[test]
    fn replace_at_rewrites_only_the_located_region() {
        let source = "one\r\ntwo\r\nthree\r\n";
        let located = Match {
            start: 5,
            end: 8,
        };
        assert_eq!(&source[located.start..located.end], "two");
        let updated = replace_at(source, &located, "TWO", dominant_terminator(source));
        assert_eq!(updated, "one\r\nTWO\r\nthree\r\n");
    }

    #[test]
    fn a_line_trimmed_match_spans_whole_lines_only() {
        let source = "say hello  \n";
        let report = find_matches(source, "hello");
        // The exact text is a substring, so the exact tier wins.
        assert_eq!(report.kind, MatchKind::Exact);
        assert_eq!(report.matches[0].start, 4);
        assert_eq!(report.matches[0].end, 9);
    }

    #[test]
    fn a_whole_line_search_replaces_the_trailing_whitespace_too() {
        let source = "alpha   \nbeta\n";
        let report = find_matches(source, "alpha");
        assert_eq!(report.kind, MatchKind::Exact);
        let report = find_matches(source, "alpha   ");
        assert_eq!(report.kind, MatchKind::Exact);
        assert_eq!(report.matches[0].start, 0);
        assert_eq!(report.matches[0].end, 8);
    }
}
