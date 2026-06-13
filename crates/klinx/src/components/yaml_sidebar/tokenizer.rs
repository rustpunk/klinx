//! YAML syntax tokenizer for the static highlight overlay.
//!
//! The primary path walks the `saphyr-parser-bw` event stream — the same
//! span-tracking YAML parser `serde-saphyr` already pulls in — to classify
//! every byte of the source as a key, a value, punctuation, or a comment, then
//! reconstructs each source line into a `Vec<Token>`. Walking a real parser
//! (rather than the old `find_key_colon` heuristic) fixes highlighting of
//! quoted colons, block scalars, flow collections, anchors/aliases and merge
//! keys.
//!
//! Because the user edits this YAML live, the parser frequently sees invalid
//! input mid-keystroke. `tokenize` is therefore infallible: on any parse error
//! it falls back to the line-based lexer (`tokenize_line_based`) so the overlay
//! never goes blank while typing.
//!
//! Reconstruction invariant: every token's `text` is sliced verbatim from the
//! source, and the concatenation of a line's token texts is byte-for-byte equal
//! to that source line. The overlay is positioned underneath a transparent
//! `<textarea>`, so any drift between token text and source text would misalign
//! the colour layer from the glyphs the user sees.

use saphyr_parser_bw::{Event, Parser};

/// A single coloured span within a YAML line.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
}

/// Semantic token kind — maps directly to CSS `data-token` attribute values
/// which drive the syntax colour rules in `klinx.css`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    /// `#` comment — text-floor colour.
    Comment,
    /// Mapping key before the colon — verdigris.
    Key,
    /// Scalar value / sequence item content — phosphor.
    Value,
    /// Punctuation: `:`, `- `, `"`, `'` — iron.
    Punct,
    /// Whitespace-only indent — rendered as-is, no colour class needed.
    Indent,
    /// CXL statement keyword (`emit`, `let`, `set`, `if`, etc.) — verdigris.
    CxlKeyword,
    /// CXL operator (`==`, `!=`, `and`, `or`, `not`, `+`, etc.) — iron.
    CxlOp,
    /// CXL string literal (`"active"`, `'foo'`) — ember.
    CxlString,
    /// CXL identifier / field reference — phosphor.
    CxlIdent,
    /// CXL literal value (`true`, `false`, `null`, numbers) — phosphor-dim.
    CxlLiteral,
    /// CXL punctuation (parens, brackets, dots, commas) — iron.
    CxlPunct,
}

impl TokenKind {
    /// The `data-token` attribute value used by `klinx.css` to colour the span.
    pub fn as_data_attr(self) -> &'static str {
        match self {
            TokenKind::Comment => "comment",
            TokenKind::Key => "key",
            TokenKind::Value => "value",
            TokenKind::Punct => "punct",
            TokenKind::Indent => "indent",
            TokenKind::CxlKeyword => "cxl-keyword",
            TokenKind::CxlOp => "cxl-op",
            TokenKind::CxlString => "cxl-string",
            TokenKind::CxlIdent => "cxl-ident",
            TokenKind::CxlLiteral => "cxl-literal",
            TokenKind::CxlPunct => "cxl-punct",
        }
    }
}

impl Token {
    fn new(kind: TokenKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// CXL keywords that start statements or control flow.
const CXL_KEYWORDS: &[&str] = &[
    "emit", "let", "trace", "use", "if", "then", "else", "match", "fn", "as",
];

/// CXL keyword-style operators.
const CXL_KEYWORD_OPS: &[&str] = &["and", "or", "not"];

/// CXL literal keywords.
const CXL_LITERALS: &[&str] = &["true", "false", "null", "now", "it", "window", "pipeline"];

/// Tokenise a YAML document into a `Vec` of lines, each line a `Vec<Token>`.
///
/// Infallible: walks the `saphyr` event stream when the document parses, and
/// falls back to the line-based lexer when it does not (the common case while
/// the user is mid-edit). Either way the per-line text reconstructs the source
/// exactly.
pub fn tokenize(yaml: &str) -> Vec<Vec<Token>> {
    match classify_segments(yaml) {
        Some(segments) => render_lines(yaml, &segments),
        None => tokenize_line_based(yaml),
    }
}

// ── saphyr event walk ───────────────────────────────────────────────────────

/// The semantic role of a parser-spanned byte range, used to pick a token kind
/// during line reconstruction.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SegKind {
    /// A mapping key scalar.
    Key,
    /// A scalar / alias value.
    Value,
    /// A value under a `cxl:` key — sub-tokenised with `tokenize_cxl`.
    CxlValue,
}

/// A classified, source-backed byte range produced by the event walk.
///
/// Ranges are non-overlapping and sorted by `start`. Everything *between*
/// segments (separators `:`/`-`, block-scalar indicators `|`/`>`, anchors,
/// comments, indentation) is glue that `render_lines` classifies directly from
/// the source text.
#[derive(Clone, Debug)]
struct Segment {
    start: usize,
    end: usize,
    kind: SegKind,
}

/// One frame of the container nesting stack while walking events.
///
/// YAML mappings interleave key and value events; sequences emit only values.
/// Tracking this is what lets the walker label a scalar as a key vs. a value —
/// the structural fact the old `find_key_colon` heuristic could only guess at.
#[derive(Clone, Copy)]
enum Frame {
    /// Inside a mapping; `expect_key` flips on each child node.
    Mapping {
        expect_key: bool,
        /// The next value is the content of a `cxl:` key.
        next_value_is_cxl: bool,
    },
    /// Inside a sequence; every child node is a value.
    Sequence,
}

/// Walk the parser event stream and classify spanned scalar/alias nodes.
///
/// Returns `None` on a parse error so `tokenize` can fall back; this is the
/// expected outcome for incomplete YAML during editing, not an exceptional one.
fn classify_segments(yaml: &str) -> Option<Vec<Segment>> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    for event in Parser::new_from_str(yaml) {
        let (event, span) = event.ok()?;
        // We only attribute byte-backed spans; structural events without a
        // source range (document/stream bookends) carry no glyphs to colour.
        let Some(range) = span.byte_range() else {
            continue;
        };

        match event {
            Event::Scalar(value, _style, _anchor, _tag) => {
                let role = node_role(&mut stack);
                // The parser synthesises a zero-width null scalar (`~`) for an
                // omitted mapping value (`key:`); it has no source text, so
                // emitting a token would desync the reconstruction.
                if range.start == range.end {
                    continue;
                }
                let kind = match role {
                    NodeRole::Key => {
                        // Record whether this key opens a CXL value so the
                        // matching value node is sub-tokenised.
                        if value.as_ref() == "cxl"
                            && let Some(Frame::Mapping {
                                next_value_is_cxl, ..
                            }) = stack.last_mut()
                        {
                            *next_value_is_cxl = true;
                        }
                        SegKind::Key
                    }
                    // A block scalar (`|`/`>`) under `cxl:` is reported as
                    // `CxlValue` by `node_role`, so block and inline CXL values
                    // route to `tokenize_cxl` through the same arm.
                    NodeRole::CxlValue => SegKind::CxlValue,
                    NodeRole::Value => SegKind::Value,
                };
                segments.push(Segment {
                    start: range.start,
                    end: range.end,
                    kind,
                });
            }
            Event::Alias(_) => {
                // An alias (`*ref`) is always a value position.
                let role = node_role(&mut stack);
                let kind = match role {
                    NodeRole::CxlValue => SegKind::CxlValue,
                    _ => SegKind::Value,
                };
                segments.push(Segment {
                    start: range.start,
                    end: range.end,
                    kind,
                });
            }
            Event::MappingStart(..) => {
                // A nested mapping occupies the current node position; advance
                // the parent's key/value cursor before pushing the new frame.
                let _ = node_role(&mut stack);
                stack.push(Frame::Mapping {
                    expect_key: true,
                    next_value_is_cxl: false,
                });
            }
            Event::SequenceStart(..) => {
                let _ = node_role(&mut stack);
                stack.push(Frame::Sequence);
            }
            Event::MappingEnd | Event::SequenceEnd => {
                stack.pop();
            }
            _ => {}
        }
    }

    Some(segments)
}

/// The role a node plays in its enclosing container.
enum NodeRole {
    Key,
    Value,
    CxlValue,
}

/// Resolve the current node's role and advance the enclosing mapping's cursor.
///
/// Top-level nodes (no enclosing container) are treated as values. Each call
/// consumes one node position, so it must be invoked exactly once per node
/// event (scalar, alias, or the start of a nested collection).
fn node_role(stack: &mut [Frame]) -> NodeRole {
    match stack.last_mut() {
        Some(Frame::Mapping {
            expect_key,
            next_value_is_cxl,
        }) => {
            if *expect_key {
                *expect_key = false;
                NodeRole::Key
            } else {
                *expect_key = true;
                if std::mem::take(next_value_is_cxl) {
                    NodeRole::CxlValue
                } else {
                    NodeRole::Value
                }
            }
        }
        Some(Frame::Sequence) | None => NodeRole::Value,
    }
}

// ── line reconstruction ───────────────────────────────────────────────────

/// Reconstruct each source line into tokens, using the classified segments to
/// colour scalar/alias text and treating the gaps between them as glue.
fn render_lines(yaml: &str, segments: &[Segment]) -> Vec<Vec<Token>> {
    let mut lines = Vec::new();
    // Index of the first segment that might intersect the current line.
    let mut seg_idx = 0usize;

    for (line_start, line) in line_spans(yaml) {
        let line_end = line_start + line.len();
        let mut tokens = Vec::new();
        // Cursor (absolute byte offset) within the line as we emit tokens.
        let mut cursor = line_start;

        // Skip segments that ended at or before this line's start.
        while seg_idx < segments.len() && segments[seg_idx].end <= line_start {
            seg_idx += 1;
        }

        // A block scalar (`|`/`>`) body is a single segment spanning several
        // lines, so on a continuation line the segment already covers
        // `line_start`. In that case the leading whitespace is *content* of the
        // scalar (the parser's slice includes it), so it must flow through the
        // segment rather than be split off as a standalone `Indent` token —
        // otherwise it would be emitted twice and the line would no longer
        // reconstruct the source. On all other lines the indent is glue and
        // gets its own `Indent` token (the old lexer did the same, and CSS
        // leaves it uncoloured).
        let continues_into_line = seg_idx < segments.len()
            && segments[seg_idx].start < line_start
            && segments[seg_idx].end > line_start;
        if !continues_into_line {
            let indent_end = line_start + (line.len() - line.trim_start().len());
            if indent_end > cursor {
                tokens.push(Token::new(TokenKind::Indent, &yaml[cursor..indent_end]));
                cursor = indent_end;
            }
        }

        let mut i = seg_idx;
        while i < segments.len() && segments[i].start < line_end {
            let seg = &segments[i];
            let seg_start = seg.start.max(line_start);
            let seg_end = seg.end.min(line_end);
            if seg_start >= seg_end {
                i += 1;
                continue;
            }

            // Glue between the cursor and this segment.
            if seg_start > cursor {
                push_gap(&mut tokens, &yaml[cursor..seg_start]);
            }

            push_segment(&mut tokens, seg.kind, &yaml[seg_start..seg_end]);
            cursor = seg_end;

            // A segment that runs past this line (a block scalar body) is only
            // partially consumed here; leave it for the next line.
            if seg.end > line_end {
                break;
            }
            i += 1;
        }
        // Advance the shared segment cursor past segments fully consumed on
        // this line so the next line starts its scan in the right place.
        seg_idx = i;

        // Trailing glue (e.g. a `# comment`, or a `:` after an empty value).
        if cursor < line_end {
            push_gap(&mut tokens, &yaml[cursor..line_end]);
        }

        if tokens.is_empty() {
            tokens.push(Token::new(TokenKind::Indent, ""));
        }
        lines.push(tokens);
    }

    lines
}

/// Yield each line of `yaml` as `(start_byte, content)`, where `content`
/// excludes the line terminator.
///
/// Mirrors `str::lines` (a trailing newline yields no final empty line) but
/// also reports the byte offset of each line so the renderer can index into the
/// original source using the parser's absolute byte spans. Recognises both
/// `\n` and `\r\n`; the `\r` is excluded from `content`, so the terminator
/// width (1 or 2 bytes) is accounted for when advancing — this is what keeps
/// reconstruction byte-exact on CRLF input.
fn line_spans(yaml: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        if pos >= yaml.len() {
            return None;
        }
        let start = pos;
        match yaml[start..].find('\n') {
            Some(rel_nl) => {
                let nl = start + rel_nl;
                // Strip a CR that immediately precedes the LF.
                let content_end = if nl > start && yaml.as_bytes()[nl - 1] == b'\r' {
                    nl - 1
                } else {
                    nl
                };
                pos = nl + 1;
                Some((start, &yaml[start..content_end]))
            }
            None => {
                // Final line with no trailing newline.
                pos = yaml.len();
                Some((start, &yaml[start..]))
            }
        }
    })
}

/// Emit a glue run: a leading `#…` comment becomes a `Comment`, everything else
/// (separators, indicators, anchors, inter-token spaces) is `Punct`. The text
/// is preserved verbatim so the line still reconstructs the source.
fn push_gap(tokens: &mut Vec<Token>, text: &str) {
    if text.is_empty() {
        return;
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        let lead = &text[..text.len() - trimmed.len()];
        if !lead.is_empty() {
            tokens.push(Token::new(TokenKind::Punct, lead));
        }
        tokens.push(Token::new(TokenKind::Comment, trimmed));
    } else {
        tokens.push(Token::new(TokenKind::Punct, text));
    }
}

/// Emit a classified segment slice, sub-tokenising CXL values.
fn push_segment(tokens: &mut Vec<Token>, kind: SegKind, text: &str) {
    match kind {
        SegKind::Key => tokens.push(Token::new(TokenKind::Key, text)),
        SegKind::Value => tokens.push(Token::new(TokenKind::Value, text)),
        SegKind::CxlValue => tokens.extend(tokenize_cxl(text)),
    }
}

// ── CXL sub-lexer ───────────────────────────────────────────────────────────

/// Tokenize a CXL expression into coloured spans.
///
/// This is a lightweight lexer for syntax colouring — not a full parser.
/// It handles keywords, operators, string literals, numbers, identifiers,
/// and punctuation.
pub fn tokenize_cxl(cxl: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = cxl.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Whitespace
        if ch.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            tokens.push(Token::new(TokenKind::Indent, &cxl[start..i]));
            continue;
        }

        // Line comment
        if ch == '#' {
            tokens.push(Token::new(TokenKind::Comment, &cxl[i..]));
            break;
        }

        // String literals
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let start = i;
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1; // skip escaped char
                }
                i += 1;
            }
            if i < len {
                i += 1; // closing quote
            }
            tokens.push(Token::new(TokenKind::CxlString, &cxl[start..i]));
            continue;
        }

        // Date literal #YYYY-MM-DD#
        if ch == '#' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '#' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token::new(TokenKind::CxlLiteral, &cxl[start..i]));
            continue;
        }

        // Multi-char operators
        if i + 1 < len {
            let two = &cxl[i..i + 2];
            match two {
                "==" | "!=" | ">=" | "<=" | "=>" | "??" | "::" => {
                    tokens.push(Token::new(TokenKind::CxlOp, two));
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }

        // Single-char operators
        if matches!(ch, '+' | '-' | '*' | '/' | '%' | '>' | '<' | '=' | '|') {
            tokens.push(Token::new(TokenKind::CxlOp, &cxl[i..i + 1]));
            i += 1;
            continue;
        }

        // Punctuation
        if matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | '.' | ',' | ':' | '_'
        ) && !(ch == '_'
            && i + 1 < len
            && (chars[i + 1].is_alphanumeric() || chars[i + 1] == '_'))
        {
            tokens.push(Token::new(TokenKind::CxlPunct, &cxl[i..i + 1]));
            i += 1;
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            tokens.push(Token::new(TokenKind::CxlLiteral, &cxl[start..i]));
            continue;
        }

        // Identifiers and keywords
        if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word = &cxl[start..i];
            let kind = if CXL_KEYWORDS.contains(&word) {
                TokenKind::CxlKeyword
            } else if CXL_KEYWORD_OPS.contains(&word) {
                TokenKind::CxlOp
            } else if CXL_LITERALS.contains(&word) {
                TokenKind::CxlLiteral
            } else {
                TokenKind::CxlIdent
            };
            tokens.push(Token::new(kind, word));
            continue;
        }

        // Fallback: single character
        tokens.push(Token::new(TokenKind::CxlIdent, &cxl[i..i + 1]));
        i += 1;
    }

    tokens
}

// ── line-based fallback ───────────────────────────────────────────────────

/// Line-by-line tokenizer used when the document does not parse.
///
/// This is the original heuristic lexer, retained as the graceful-degradation
/// path: while the user types, the YAML is frequently invalid and the parser
/// errors, so this keeps the overlay populated rather than blank. It tracks
/// `cxl: |` block-scalar context so CXL continuation lines still highlight.
fn tokenize_line_based(yaml: &str) -> Vec<Vec<Token>> {
    let mut result = Vec::new();
    let mut cxl_block: Option<usize> = None; // indent level of the cxl block body

    for line in yaml.lines() {
        let trimmed = line.trim();

        // Check if we're exiting a CXL block
        if let Some(block_indent) = cxl_block {
            if trimmed.is_empty() {
                // Blank lines inside block scalars are kept
                result.push(vec![Token::new(TokenKind::Indent, "")]);
                continue;
            }
            let line_indent = line.len() - line.trim_start().len();
            if line_indent < block_indent {
                // Dedented — we've left the CXL block
                cxl_block = None;
            }
        }

        // If we're inside a CXL block, tokenize as CXL
        if cxl_block.is_some() {
            let indent = &line[..line.len() - line.trim_start().len()];
            let body = line.trim_start();
            let mut tokens = Vec::new();
            if !indent.is_empty() {
                tokens.push(Token::new(TokenKind::Indent, indent));
            }
            tokens.extend(tokenize_cxl(body));
            result.push(tokens);
            continue;
        }

        // Normal YAML tokenization
        let tokens = tokenize_line(line);

        // Detect `cxl: |` pattern to enter CXL block mode
        if is_cxl_block_start(&tokens) {
            let line_indent = line.len() - line.trim_start().len();
            // The block body will be indented further than the key
            cxl_block = Some(line_indent + 2); // typical YAML indent is +2
        }

        // Detect inline `cxl: <expr>` (single-line CXL value)
        let final_tokens = maybe_inline_cxl(tokens);
        result.push(final_tokens);
    }

    result
}

/// Check if a tokenized line represents `cxl: |` (block scalar start).
fn is_cxl_block_start(tokens: &[Token]) -> bool {
    let mut saw_cxl_key = false;
    for token in tokens {
        if token.kind == TokenKind::Key && token.text == "cxl" {
            saw_cxl_key = true;
        }
        if saw_cxl_key && token.kind == TokenKind::Value && token.text.trim() == "|" {
            return true;
        }
    }
    false
}

/// If the line has a `cxl` key with an inline value (not `|`), replace the
/// value token with CXL-highlighted tokens.
fn maybe_inline_cxl(tokens: Vec<Token>) -> Vec<Token> {
    let mut saw_cxl_key = false;
    let mut cxl_value_idx = None;

    for (i, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Key && token.text == "cxl" {
            saw_cxl_key = true;
        }
        if saw_cxl_key && token.kind == TokenKind::Value {
            let trimmed = token.text.trim();
            if trimmed != "|" && trimmed != ">" && !trimmed.is_empty() {
                cxl_value_idx = Some(i);
            }
            break;
        }
    }

    if let Some(idx) = cxl_value_idx {
        let mut result = Vec::with_capacity(tokens.len() + 4);
        for (i, token) in tokens.into_iter().enumerate() {
            if i == idx {
                result.extend(tokenize_cxl(&token.text));
            } else {
                result.push(token);
            }
        }
        result
    } else {
        tokens
    }
}

/// Tokenize a single YAML line with the heuristic lexer (fallback path).
fn tokenize_line(line: &str) -> Vec<Token> {
    if line.trim().is_empty() {
        return vec![Token::new(TokenKind::Indent, "")];
    }

    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];

    // Comment line
    if rest.starts_with('#') {
        return vec![Token::new(TokenKind::Comment, line)];
    }

    let mut tokens: Vec<Token> = Vec::new();
    if indent_len > 0 {
        tokens.push(Token::new(TokenKind::Indent, indent));
    }

    // Strip list-item prefix "- "
    let (after_prefix, had_prefix) = if let Some(stripped) = rest.strip_prefix("- ") {
        (stripped, true)
    } else if rest == "-" {
        ("", true)
    } else {
        (rest, false)
    };

    if had_prefix {
        tokens.push(Token::new(TokenKind::Punct, "- "));
    }

    // Key: value  or  key:\n
    if let Some(colon_pos) = find_key_colon(after_prefix) {
        let key = &after_prefix[..colon_pos];
        let after_colon = &after_prefix[colon_pos + 1..];

        tokens.push(Token::new(TokenKind::Key, key));
        tokens.push(Token::new(TokenKind::Punct, ":"));

        if !after_colon.is_empty() {
            // "  value" — emit a space in the punct, then the value
            let value_trimmed = after_colon.trim_start();
            let leading = &after_colon[..after_colon.len() - value_trimmed.len()];
            if !leading.is_empty() {
                tokens.push(Token::new(TokenKind::Punct, leading));
            }
            tokens.push(Token::new(TokenKind::Value, value_trimmed));
        }
    } else {
        // Pure scalar value (sequence item body, bare word, etc.)
        tokens.push(Token::new(TokenKind::Value, after_prefix));
    }

    tokens
}

/// Finds the position of the `:` that ends a YAML mapping key, ignoring colons
/// that appear inside quoted strings. Returns `None` if the line is not a key.
fn find_key_colon(s: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;

    for (i, ch) in s.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' if !in_single && !in_double => {
                // Colon must be followed by whitespace, end-of-string, or EOF
                // to be a key colon (not an arbitrary colon inside a value).
                let next = s[i + 1..].chars().next();
                if matches!(next, None | Some(' ') | Some('\t')) {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Concatenated token text must reproduce the source line exactly — the
    /// overlay sits under a transparent textarea, so any drift misaligns the
    /// colour layer from the glyphs.
    fn assert_reconstructs(yaml: &str) {
        let lines = tokenize(yaml);
        let rebuilt: Vec<String> = lines
            .iter()
            .map(|toks| toks.iter().map(|t| t.text.as_str()).collect())
            .collect();
        let expected: Vec<&str> = yaml.lines().collect();
        assert_eq!(rebuilt, expected, "reconstruction drift for input {yaml:?}");
    }

    /// The ordered token kinds for line `idx`.
    fn kinds(yaml: &str, idx: usize) -> Vec<TokenKind> {
        tokenize(yaml)[idx].iter().map(|t| t.kind).collect()
    }

    /// The token text for line `idx` whose kind is `kind`.
    fn texts_of(yaml: &str, idx: usize, kind: TokenKind) -> Vec<String> {
        tokenize(yaml)[idx]
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.text.clone())
            .collect()
    }

    #[test]
    fn simple_mapping() {
        let yaml = "name: hello\ncount: 3\n";
        assert_reconstructs(yaml);
        assert_eq!(
            kinds(yaml, 0),
            vec![TokenKind::Key, TokenKind::Punct, TokenKind::Value]
        );
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["name"]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["hello"]);
    }

    #[test]
    fn quoted_string_with_colon() {
        // The `find_key_colon` bug: the inner `:` must not split the value.
        let yaml = "url: \"http://x:8080\"\n";
        assert_reconstructs(yaml);
        assert_eq!(
            kinds(yaml, 0),
            vec![TokenKind::Key, TokenKind::Punct, TokenKind::Value]
        );
        // The whole quoted scalar (quotes included) is one value token.
        assert_eq!(
            texts_of(yaml, 0, TokenKind::Value),
            vec!["\"http://x:8080\""]
        );
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["url"]);
    }

    #[test]
    fn single_quoted_colon() {
        let yaml = "msg: 'a: b'\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["msg"]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["'a: b'"]);
    }

    #[test]
    fn block_literal_scalar() {
        let yaml = "desc: |\n  line one\n  line two\nnext: 2\n";
        assert_reconstructs(yaml);
        // Key line.
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["desc"]);
        // The first body line splits its indent off as a standalone token; the
        // continuation line's leading whitespace is scalar *content* and folds
        // into the value (the parser's slice includes it), so it stays attached.
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["line one"]);
        assert_eq!(texts_of(yaml, 2, TokenKind::Value), vec!["  line two"]);
        // We're back to normal mapping afterwards.
        assert_eq!(texts_of(yaml, 3, TokenKind::Key), vec!["next"]);
    }

    #[test]
    fn folded_block_scalar() {
        let yaml = "desc: >\n  long text\n  more\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["desc"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["long text"]);
        // Continuation-line indent folds into the scalar content (see
        // `block_literal_scalar`).
        assert_eq!(texts_of(yaml, 2, TokenKind::Value), vec!["  more"]);
    }

    #[test]
    fn flow_sequence() {
        let yaml = "nums: [1, 2, 3]\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["nums"]);
        // The flow scalars are values; brackets/commas are punctuation glue.
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["1", "2", "3"]);
    }

    #[test]
    fn flow_mapping() {
        let yaml = "obj: {a: 1, b: 2}\n";
        assert_reconstructs(yaml);
        // Inner flow keys are keys; inner values are values.
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["obj", "a", "b"]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["1", "2"]);
    }

    #[test]
    fn anchor_and_alias() {
        let yaml = "base: &a 1\nref: *a\n";
        assert_reconstructs(yaml);
        // `&a` is glue (punct) before the value `1`.
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["base"]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["1"]);
        assert!(
            texts_of(yaml, 0, TokenKind::Punct)
                .iter()
                .any(|t| t.contains("&a"))
        );
        // The alias `*a` is the whole value on the second line.
        assert_eq!(texts_of(yaml, 1, TokenKind::Key), vec!["ref"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["*a"]);
    }

    #[test]
    fn merge_key() {
        let yaml = "base: &a\n  x: 1\nderived:\n  <<: *a\n  y: 2\n";
        assert_reconstructs(yaml);
        // The merge key `<<` is highlighted as a key (line index 3).
        assert_eq!(texts_of(yaml, 3, TokenKind::Key), vec!["<<"]);
        assert_eq!(texts_of(yaml, 3, TokenKind::Value), vec!["*a"]);
    }

    #[test]
    fn trailing_comment() {
        let yaml = "# top\nkey: val # trailing\n";
        assert_reconstructs(yaml);
        // Full-line comment.
        assert_eq!(kinds(yaml, 0), vec![TokenKind::Comment]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Comment), vec!["# top"]);
        // Trailing comment on a key/value line.
        assert_eq!(texts_of(yaml, 1, TokenKind::Key), vec!["key"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["val"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Comment), vec!["# trailing"]);
    }

    #[test]
    fn empty_value() {
        let yaml = "key:\nother: 1\n";
        assert_reconstructs(yaml);
        // No value token on the empty line; just key + colon.
        assert_eq!(kinds(yaml, 0), vec![TokenKind::Key, TokenKind::Punct]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["key"]);
    }

    #[test]
    fn sequence_items() {
        let yaml = "items:\n  - one\n  - two\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["one"]);
        assert_eq!(texts_of(yaml, 2, TokenKind::Value), vec!["two"]);
        // The `- ` marker is punctuation glue.
        assert!(
            texts_of(yaml, 1, TokenKind::Punct)
                .iter()
                .any(|t| t.contains('-'))
        );
    }

    #[test]
    fn cxl_inline_value() {
        let yaml = "cxl: emit foo == 1\n";
        assert_reconstructs(yaml);
        let line_kinds = kinds(yaml, 0);
        // The value is sub-tokenised into CXL kinds, not a single Value.
        assert!(line_kinds.contains(&TokenKind::CxlKeyword));
        assert!(line_kinds.contains(&TokenKind::CxlOp));
        assert!(!line_kinds.contains(&TokenKind::Value));
        // `emit` is a CXL keyword; `==` a CXL op.
        assert!(texts_of(yaml, 0, TokenKind::CxlKeyword).contains(&"emit".to_string()));
        assert!(texts_of(yaml, 0, TokenKind::CxlOp).contains(&"==".to_string()));
    }

    #[test]
    fn cxl_block_scalar() {
        let yaml = "cxl: |\n  emit foo\n  let x = 1\nnext: 2\n";
        assert_reconstructs(yaml);
        // Body lines highlight as CXL.
        assert!(texts_of(yaml, 1, TokenKind::CxlKeyword).contains(&"emit".to_string()));
        assert!(texts_of(yaml, 2, TokenKind::CxlKeyword).contains(&"let".to_string()));
        // `=` inside the block is a CXL op.
        assert!(texts_of(yaml, 2, TokenKind::CxlOp).contains(&"=".to_string()));
        // The trailing plain mapping is not treated as CXL.
        assert_eq!(texts_of(yaml, 3, TokenKind::Key), vec!["next"]);
    }

    #[test]
    fn cxl_nested_in_sequence() {
        let yaml = "stages:\n  - cxl: emit foo\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 1, TokenKind::Key), vec!["cxl"]);
        assert!(texts_of(yaml, 1, TokenKind::CxlKeyword).contains(&"emit".to_string()));
    }

    #[test]
    fn invalid_yaml_falls_back() {
        // Unbalanced flow bracket: the parser errors mid-edit, so we must fall
        // back to the line lexer and still reconstruct every line.
        let yaml = "key: [1, 2\nother: ok\n";
        let lines = tokenize(yaml);
        assert_eq!(lines.len(), 2);
        assert_reconstructs(yaml);
    }

    #[test]
    fn blank_lines_preserved() {
        let yaml = "a: 1\n\nb: 2\n";
        assert_reconstructs(yaml);
        // The blank middle line is a single empty Indent token.
        assert_eq!(tokenize(yaml)[1].len(), 1);
        assert_eq!(tokenize(yaml)[1][0].text, "");
    }

    #[test]
    fn empty_input() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn document_marker() {
        let yaml = "---\nkey: 1\n";
        assert_reconstructs(yaml);
        // `---` is glue punctuation on its own line.
        assert_eq!(texts_of(yaml, 1, TokenKind::Key), vec!["key"]);
    }

    #[test]
    fn crlf_line_endings() {
        // Per-line byte accounting must step over `\r\n`, not just `\n`, or the
        // value spans would drift one byte per line. `assert_reconstructs`
        // compares against `str::lines`, which strips the `\r`.
        let yaml = "name: hello\r\ncount: 3\r\n";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 0, TokenKind::Key), vec!["name"]);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["hello"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Key), vec!["count"]);
        assert_eq!(texts_of(yaml, 1, TokenKind::Value), vec!["3"]);
    }

    #[test]
    fn no_trailing_newline() {
        let yaml = "key: value";
        assert_reconstructs(yaml);
        assert_eq!(texts_of(yaml, 0, TokenKind::Value), vec!["value"]);
    }
}
