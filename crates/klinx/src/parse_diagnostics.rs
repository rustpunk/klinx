//! Post-processing for engine parse-error strings.
//!
//! The engine crates are git-pinned (see `CLAUDE.md`); klinx cannot change
//! how `clinker_plan` / `serde-saphyr` word a parse error. What klinx *can* do
//! is augment the raw string before it reaches the editor's error bar.
//!
//! The motivating case: a mapping key that lost its colon —
//!
//! ```yaml
//!   - type: source
//!     name: orders
//!     conf            # <- `config:` with the colon (and tail) deleted
//!       name: orders
//! ```
//!
//! saphyr reads `conf` as a plain scalar and only fails one line *later*, when
//! the over-indented `name: orders` proves `conf` can't be continued. The
//! engine therefore reports `simple key expect ':'` at the *next* line — a
//! downstream symptom, not the fault. [`refine`] detects this shape and
//! prepends a hint naming the real culprit line, so the bar points the author
//! straight at the missing colon instead of at the line that merely exposed it.
//!
//! Refinement is purely additive: errors that don't match a known shape pass
//! through verbatim, so an unrecognized message is never made worse.

/// Augment each raw engine parse-error string with a klinx hint when its shape
/// is recognized. `yaml` is the source the errors were produced from (1-based
/// line numbers in the engine message index into it). Unrecognized errors are
/// returned unchanged.
pub fn refine(yaml: &str, errors: Vec<String>) -> Vec<String> {
    errors
        .into_iter()
        .map(|raw| refine_one(yaml, raw))
        .collect()
}

/// A detector inspects `(yaml, raw_error)` and returns a hint when it recognizes
/// the error's shape, or `None` to defer to the next one.
type Detector = fn(&str, &str) -> Option<String>;

/// The recognized shapes, each translating one opaque engine message into a
/// hint that names the true fault. Some parser messages can describe more than
/// one shape, so keep more specific structural detectors before fallbacks.
const DETECTORS: &[Detector] = &[
    missing_colon_hint,
    tab_hint,
    indentation_hint,
    unquoted_colon_value_hint,
];

/// Refine a single error string, or return it unchanged.
fn refine_one(yaml: &str, raw: String) -> String {
    for detector in DETECTORS {
        if let Some(hint) = detector(yaml, &raw) {
            // Blank line between the hint and the engine snippet so the two read
            // as distinct blocks once the bar renders them with preserved
            // whitespace.
            return format!("{hint}\n\n{raw}");
        }
    }
    raw
}

/// Detect the "mapping key missing its colon" shape and, if it fits, build a
/// hint naming the true culprit line.
///
/// saphyr's `simple key expect ':'` fires at the first line that can't continue
/// the preceding bare scalar — i.e. one line *past* the actual mistake. We
/// recover the culprit as the nearest preceding non-blank line that is indented
/// *less* than the flagged line and that carries no colon of its own (a bare
/// scalar where a `key:` was meant). When that line isn't found or already has a
/// colon, the shape doesn't fit and we add nothing.
fn missing_colon_hint(yaml: &str, raw: &str) -> Option<String> {
    if !raw.contains("simple key expect ':'") {
        return None;
    }

    let flagged_line = engine_line_number(raw)?;
    let lines: Vec<&str> = yaml.lines().collect();
    // Engine line numbers are 1-based; guard the index.
    let flagged_idx = flagged_line.checked_sub(1)?;
    let flagged = lines.get(flagged_idx)?;
    let flagged_indent = indent_width(flagged);

    // The culprit is the immediately preceding line with real content. Skipping
    // blanks/comments keeps the heuristic robust to stray spacing without
    // letting it wander far enough to mis-attribute the fault.
    let (culprit_idx, culprit) = lines[..flagged_idx]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, l)| !is_blank_or_comment(l))?;

    // The fault shape requires the culprit to *open* the over-indented block:
    // it must sit at a shallower indent than the flagged line.
    if indent_width(culprit) >= flagged_indent {
        return None;
    }

    let token = bare_scalar_token(culprit)?;

    Some(format!(
        "hint: line {culprit_line}: `{token}` has no colon — a YAML mapping key \
         must be written `{token}:`. (The engine flags line {flagged_line} \
         because the missing colon only becomes detectable there.)",
        culprit_line = culprit_idx + 1,
    ))
}

/// Extract the 1-based line number from the engine's `line {N} column {M}`
/// prefix (saphyr renders this at the head of every located error).
fn engine_line_number(raw: &str) -> Option<usize> {
    let after = raw.split("line ").nth(1)?;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Leading-space count. Clinker YAML is space-indented; a tab would be its own
/// (different) error, so counting spaces is sufficient here.
fn indent_width(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Whether a line carries no key/value content the heuristic should consider.
fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

/// If `line` is a bare scalar where a mapping key was meant — content with no
/// colon — return the offending token. A line that already contains a colon is
/// a well-formed `key:` (or `key: value`) and is rejected, so a real key never
/// draws a spurious hint.
fn bare_scalar_token(line: &str) -> Option<String> {
    // A list entry (`- conf`) is still a missing-colon candidate: strip the
    // dash so the token is reported as the author wrote it.
    let body = line.trim().strip_prefix("- ").unwrap_or(line.trim());
    // Drop a trailing inline comment before judging "has a colon".
    let content = body.split('#').next().unwrap_or(body).trim();
    if content.is_empty() || content.contains(':') {
        return None;
    }
    Some(content.to_string())
}

/// A tab character where YAML requires spaces. saphyr renders the tab as spaces
/// in its own snippet, so the offending character is *invisible* there and the
/// message ("tabs disallowed within this context" / "while scanning a plain
/// scalar, found a tab") is baffling. Worse, a tab that merely continues a plain
/// scalar is flagged on the line where the scalar *began*, not the line holding
/// the tab. We locate the first real tab at or after the flagged line and point
/// straight at it.
fn tab_hint(yaml: &str, raw: &str) -> Option<String> {
    if !(raw.contains("tabs disallowed") || raw.contains("found a tab")) {
        return None;
    }
    let flagged_line = engine_line_number(raw)?;
    let lines: Vec<&str> = yaml.lines().collect();
    // Engine line is 1-based; scan from it forward for the line bearing the tab.
    let start = flagged_line.checked_sub(1)?;
    let (idx, col) = lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(i, l)| l.find('\t').map(|byte| (i, byte + 1)))?;
    Some(format!(
        "hint: line {line} has a tab character at column {col} — YAML forbids \
         tabs for indentation (and in plain text here); use spaces instead. The \
         snippet below renders the tab as spaces, which is why it looks like \
         nothing is wrong.",
        line = idx + 1,
    ))
}

/// Indentation that doesn't line up with the enclosing block. saphyr surfaces
/// two notoriously opaque messages for this, neither of which says
/// "indentation":
///   - under-indented line → "while parsing a block mapping, did not find
///     expected key"
///   - over-indented line  → "mapping values are not allowed in this context"
///
/// The flagged line *is* the misindented one in both cases. We compare its
/// indent to the nearest preceding content line (the sibling it should align
/// with) and translate the message, naming the column to aim for.
fn indentation_hint(yaml: &str, raw: &str) -> Option<String> {
    let under = raw.contains("did not find expected key");
    let over = raw.contains("mapping values are not allowed in this context");
    if !under && !over {
        return None;
    }

    let flagged_line = engine_line_number(raw)?;
    let lines: Vec<&str> = yaml.lines().collect();
    let flagged_idx = flagged_line.checked_sub(1)?;
    let flagged_indent = indent_width(lines.get(flagged_idx)?);

    // The reference is the nearest preceding content line — the sibling level
    // the misindented line was probably trying to join.
    let (_, reference) = lines[..flagged_idx]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, l)| !is_blank_or_comment(l))?;
    let reference_indent = indent_width(reference);

    if over {
        // Genuine over-indent only when the flagged line is deeper than the
        // line above *and* that line doesn't open a block (no trailing `:`).
        // A trailing `:` makes deeper indentation legitimate nesting, so the
        // error is something else (e.g. an unquoted colon in a value) and we
        // must not mislabel it as over-indentation.
        if flagged_indent <= reference_indent || opens_block(reference) {
            return None;
        }
        return Some(format!(
            "hint: line {flagged_line} is indented {flagged_indent} spaces, deeper \
             than the key above it ({reference_indent}) — it looks over-indented. \
             Align it to {reference_indent} spaces (or the line above must end \
             with `:` to open a nested block). YAML reports this as \"mapping \
             values are not allowed in this context\"."
        ));
    }

    // under-indent: shallower than the block it sits in, but not back to a
    // valid outer level.
    if flagged_indent >= reference_indent {
        return None;
    }
    Some(format!(
        "hint: line {flagged_line} is indented {flagged_indent} spaces, which \
         doesn't line up with the block above it (its keys sit at \
         {reference_indent}). Re-indent it to a level that matches — usually \
         {reference_indent} spaces to stay a sibling. YAML reports this as \
         \"did not find expected key\"."
    ))
}

/// A plain scalar value that contains another `:` where YAML expects a mapping
/// boundary. saphyr reports this with the same "mapping values" message used
/// for over-indentation, so this detector intentionally runs after
/// [`indentation_hint`] has had a chance to claim real indentation mistakes.
fn unquoted_colon_value_hint(yaml: &str, raw: &str) -> Option<String> {
    if !raw.contains("mapping values are not allowed in this context") {
        return None;
    }

    let flagged_line = engine_line_number(raw)?;
    let lines: Vec<&str> = yaml.lines().collect();
    let flagged = lines.get(flagged_line.checked_sub(1)?)?;
    let (key, value) = key_and_plain_value_with_unquoted_colon(flagged)?;
    let quoted = value.replace('\\', "\\\\").replace('"', "\\\"");

    Some(format!(
        "hint: line {flagged_line}: `{key}` has an unquoted colon in its value \
         (`{value}`) — quote the whole value, for example `{key}: \"{quoted}\"`. \
         YAML reports this as \"mapping values are not allowed in this context\"."
    ))
}

/// Whether a line opens a nested block — its content ends with `:` (a mapping
/// key with no inline value), so deeper-indented lines below it are valid
/// children. Inline comments are stripped first.
fn opens_block(line: &str) -> bool {
    let content = line.split('#').next().unwrap_or(line).trim_end();
    content.ends_with(':')
}

/// Return `key` and `value` when a line is a mapping entry whose unquoted plain
/// scalar value contains a YAML mapping colon, e.g. `where: a: b`.
fn key_and_plain_value_with_unquoted_colon(line: &str) -> Option<(String, String)> {
    let body = line.trim().strip_prefix("- ").unwrap_or(line.trim()).trim();
    let key_colon = find_unquoted_colon(body)?;
    let key = body[..key_colon].trim();
    if key.is_empty() {
        return None;
    }

    let value = body[key_colon + 1..].trim();
    let value = plain_value_before_comment(value);
    if value.is_empty() || matches!(value.chars().next(), Some('"' | '\'' | '|' | '>')) {
        return None;
    }
    if !plain_value_has_mapping_colon(value) {
        return None;
    }

    Some((key.to_string(), value.to_string()))
}

/// Find the first colon that is outside a quoted key and before any inline
/// comment.
fn find_unquoted_colon(s: &str) -> Option<usize> {
    let mut quote = None;
    for (idx, ch) in s.char_indices() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(ch),
            (None, '#') => break,
            (None, ':') => return Some(idx),
            (None, _) => {}
        }
    }
    None
}

/// Strip a YAML inline comment from a plain scalar value.
fn plain_value_before_comment(value: &str) -> &str {
    for (idx, ch) in value.char_indices() {
        if ch == '#'
            && value[..idx]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
        {
            return value[..idx].trim_end();
        }
    }
    value.trim_end()
}

/// In a plain scalar, a colon followed by whitespace/end is parsed as a mapping
/// separator unless the value is quoted.
fn plain_value_has_mapping_colon(value: &str) -> bool {
    value.char_indices().any(|(idx, ch)| {
        ch == ':'
            && value[idx + ch.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Malformed inputs reused by the unit tests and the live-engine drift
    // guard. Line numbers in the comments are 1-based. ──────────────────────

    /// Tab as block indentation on line 7 (`\t  type: csv`).
    const TAB_INDENT_YAML: &str = "pipeline:\n  name: t\nnodes:\n  - type: source\n    name: orders\n    config:\n\t  type: csv\n";
    /// Tab that continues a plain scalar; the tab is on line 5 but the scalar
    /// began on line 4, so the engine flags line 4.
    const TAB_SCALAR_YAML: &str =
        "pipeline:\n  name: t\nnodes:\n  - type: source\n\t\tname: orders\n";
    /// `type: csv` under-indented to 5 spaces on line 8 (siblings are at 6).
    const UNDER_INDENT_YAML: &str = "pipeline:\n  name: t\nnodes:\n  - type: source\n    name: orders\n    config:\n      name: orders\n     type: csv\n";
    /// `type: csv` over-indented to 8 spaces on line 8 (siblings are at 6).
    const OVER_INDENT_YAML: &str = "pipeline:\n  name: t\nnodes:\n  - type: source\n    name: orders\n    config:\n      name: orders\n        type: csv\n";
    /// `where: a: b` has an unquoted colon inside the value on line 6.
    const EMBEDDED_COLON_VALUE_YAML: &str =
        "pipeline:\n  name: t\nnodes:\n  - type: combine\n    config:\n      where: a: b\n";

    #[test]
    fn tab_indentation_is_pointed_at_directly() {
        let raw = "YAML syntax error: error: line 7 column 4: tabs disallowed within this context";
        let out = refine(TAB_INDENT_YAML, vec![raw.to_string()]);
        // The tab is the first byte of line 7 → column 1, not the engine's col 4.
        assert!(
            out[0].starts_with("hint: line 7 has a tab character at column 1"),
            "{}",
            out[0]
        );
    }

    #[test]
    fn tab_continuing_a_scalar_is_located_on_its_real_line() {
        // Engine flags line 4 (scalar start); the tab is actually on line 5.
        let raw = "YAML syntax error: error: line 4 column 11: while scanning a plain scalar, found a tab";
        let out = refine(TAB_SCALAR_YAML, vec![raw.to_string()]);
        assert!(
            out[0].starts_with("hint: line 5 has a tab character at column 1"),
            "{}",
            out[0]
        );
    }

    #[test]
    fn under_indentation_is_translated() {
        let raw = "YAML syntax error: error: line 8 column 6: while parsing a block mapping, did not find expected key";
        let out = refine(UNDER_INDENT_YAML, vec![raw.to_string()]);
        assert!(
            out[0].starts_with("hint: line 8 is indented 5 spaces, which doesn't line up"),
            "{}",
            out[0]
        );
        assert!(out[0].contains("keys sit at 6"), "{}", out[0]);
    }

    #[test]
    fn over_indentation_is_translated() {
        let raw = "YAML syntax error: error: line 8 column 13: mapping values are not allowed in this context";
        let out = refine(OVER_INDENT_YAML, vec![raw.to_string()]);
        assert!(
            out[0]
                .starts_with("hint: line 8 is indented 8 spaces, deeper than the key above it (6)"),
            "{}",
            out[0]
        );
    }

    #[test]
    fn embedded_colon_under_a_block_opener_suggests_quoting_value() {
        // `where: a: b` is correctly nested under `config:` (which opens a
        // block); the "mapping values" error is the unquoted colon in the value,
        // NOT over-indentation. The over-indent detector must defer to the
        // later unquoted-value detector rather than mislabel it.
        let raw = "YAML syntax error: error: line 6 column 15: mapping values are not allowed in this context";
        let out = refine(EMBEDDED_COLON_VALUE_YAML, vec![raw.to_string()]);
        assert!(
            out[0].starts_with("hint: line 6: `where` has an unquoted colon in its value (`a: b`)"),
            "{}",
            out[0]
        );
        assert!(
            out[0].contains("for example `where: \"a: b\"`"),
            "{}",
            out[0]
        );
        assert!(
            !out[0].contains("it looks over-indented"),
            "must not mislabel an embedded-colon value: {}",
            out[0]
        );
    }

    /// A `config:` key whose colon (and tail) was deleted, leaving the bare
    /// scalar `conf` on line 6; its child `name: orders` over-indents on line
    /// 7, which is where saphyr reports `simple key expect ':'`.
    const CONF_YAML: &str = "pipeline:\n  name: t\nnodes:\n  - type: source\n    name: orders\n    conf\n      name: orders\n      type: csv\n";

    /// The engine message for `CONF_YAML`, flagged at line 7 (the over-indented
    /// child), one line past the real fault on line 6.
    const CONF_RAW: &str = "YAML syntax error: error: line 7 column 11: simple key expect ':'\n  --> <input>:7:11\n   |\n5 |     name: orders\n6 |     conf\n7 |       name: orders\n   |           ^ simple key expect ':'\n   |";

    #[test]
    fn names_the_missing_colon_line_not_the_flagged_line() {
        let out = refine(CONF_YAML, vec![CONF_RAW.to_string()]);
        let refined = &out[0];
        // Hint names line 6 (`conf`), the real fault — not line 7, the symptom.
        assert!(
            refined.starts_with("hint: line 6: `conf` has no colon"),
            "{refined}"
        );
        // The original engine snippet is preserved beneath the hint.
        assert!(refined.contains("--> <input>:7:11"), "{refined}");
        // The hint explains the off-by-one the author would otherwise chase.
        assert!(refined.contains("flags line 7"), "{refined}");
    }

    #[test]
    fn well_formed_key_above_draws_no_hint() {
        // The line above the flagged line is a proper `key:` (has a colon), so
        // the missing-colon shape doesn't fit and the error passes through.
        let raw = "YAML syntax error: error: line 3 column 5: simple key expect ':'\n  --> <input>:3:5\n   |\n2 |   config:\n3 |     name: x\n";
        let yaml = "nodes:\n  config:\n    name: x\n";
        let out = refine(yaml, vec![raw.to_string()]);
        assert_eq!(
            out[0], raw,
            "a key with a colon must not be flagged as bare"
        );
    }

    #[test]
    fn unrelated_errors_pass_through() {
        let raw = "config validation error: input 'orders': duplicate node name".to_string();
        let out = refine("pipeline:\n  name: x\n", vec![raw.clone()]);
        assert_eq!(out[0], raw);
    }

    #[test]
    fn list_item_bare_scalar_is_recognized() {
        // `- conf` (a bare scalar as a list entry) followed by an over-indented
        // mapping reproduces the same engine error; the dash is stripped from
        // the reported token.
        let raw = "error: line 3 column 9: simple key expect ':'\n";
        let yaml = "nodes:\n  - conf\n      name: x\n";
        let out = refine(yaml, vec![raw.to_string()]);
        assert!(
            out[0].starts_with("hint: line 2: `conf` has no colon"),
            "{}",
            out[0]
        );
    }

    /// Drift guard: drive a real `clinker_plan` parse of `CONF_YAML` through the
    /// live editor path and confirm (a) the engine still emits `simple key
    /// expect ':'` flagged at line 7, and (b) the full pipeline — parse →
    /// refine — surfaces a hint pointing at line 6. If the pinned engine
    /// re-words the error or moves the flagged line, this fails loudly rather
    /// than silently dropping the hint in production.
    #[test]
    fn live_engine_parse_yields_a_line_6_hint() {
        let result = crate::sync::try_parse_yaml(CONF_YAML, None);
        let crate::sync::ParseResult::Failed(errors) = result else {
            panic!("expected a hard syntax failure for a colon-less mapping key");
        };
        let joined = errors.join("\n");
        assert!(
            joined.contains("simple key expect ':'"),
            "engine message changed — re-pin the heuristic: {joined}"
        );
        assert!(
            joined.contains("hint: line 6: `conf` has no colon"),
            "refinement did not fire on live engine output: {joined}"
        );
    }

    /// Drift guard for the embedded-colon shape: parse through the real engine
    /// and confirm both the keyed parser message and the quote-the-value hint.
    #[test]
    fn live_engine_embedded_colon_value_suggests_quoting() {
        let result = crate::sync::try_parse_yaml(EMBEDDED_COLON_VALUE_YAML, None);
        let crate::sync::ParseResult::Failed(errors) = result else {
            panic!("expected a hard syntax failure for an unquoted colon in a value");
        };
        let joined = errors.join("\n");
        assert!(
            joined.contains("mapping values are not allowed in this context"),
            "engine message changed — re-pin the heuristic: {joined}"
        );
        assert!(
            joined.contains("hint: line 6: `where` has an unquoted colon in its value (`a: b`)"),
            "refinement did not fire on live engine output: {joined}"
        );
        assert!(
            joined.contains("for example `where: \"a: b\"`"),
            "hint does not point to quoting the value: {joined}"
        );
    }

    /// Drift guard for the indentation/tab shapes: parse each malformed input
    /// through the real engine and assert both the engine substring the detector
    /// keys on AND that the refined output carries the expected hint. Pins all
    /// four translations to the pinned engine's actual wording at once, so an
    /// engine bump that re-words any of them fails here instead of silently
    /// shipping an un-refined error.
    #[test]
    fn live_engine_indentation_and_tab_hints_fire() {
        let cases: &[(&str, &str, &str)] = &[
            (
                TAB_INDENT_YAML,
                "tabs disallowed",
                "hint: line 7 has a tab character",
            ),
            (
                TAB_SCALAR_YAML,
                "found a tab",
                "hint: line 5 has a tab character",
            ),
            (
                UNDER_INDENT_YAML,
                "did not find expected key",
                "hint: line 8 is indented 5 spaces, which doesn't line up",
            ),
            (
                OVER_INDENT_YAML,
                "mapping values are not allowed in this context",
                "hint: line 8 is indented 8 spaces, deeper than the key above it (6)",
            ),
        ];
        for (yaml, engine_substr, hint_prefix) in cases {
            let crate::sync::ParseResult::Failed(errors) = crate::sync::try_parse_yaml(yaml, None)
            else {
                panic!("expected a hard syntax failure for:\n{yaml}");
            };
            let joined = errors.join("\n");
            assert!(
                joined.contains(engine_substr),
                "engine message changed — re-pin the heuristic.\nwanted substring: {engine_substr}\ngot: {joined}"
            );
            assert!(
                joined.contains(hint_prefix),
                "refinement did not fire on live engine output.\nwanted hint: {hint_prefix}\ngot: {joined}"
            );
        }
    }
}
