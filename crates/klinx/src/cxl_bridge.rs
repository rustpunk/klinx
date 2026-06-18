/// Thin adapter between the `cxl` compiler crate and the Klinx UI.
///
/// Wraps `cxl::parser::Parser::parse()` into a UI-friendly validation result
/// that components can render without depending on cxl types directly.
///
/// Placeholder for real schema validation — currently parser-only, skipping resolver/typechecker steps.
/// Future versions will add schema-aware validation via resolver and type checker.
use cxl::parser::Parser;

/// Result of validating a CXL expression, suitable for rendering.
#[derive(Clone, Debug, PartialEq)]
pub struct CxlValidation {
    pub is_valid: bool,
    pub errors: Vec<CxlDiagnostic>,
}

/// A single diagnostic message derived from a `ParseError`.
#[derive(Clone, Debug, PartialEq)]
pub struct CxlDiagnostic {
    /// Byte offset into the expression string (start of error span).
    pub start: usize,
    /// Byte offset into the expression string (end of error span).
    pub end: usize,
    pub message: String,
    pub severity: DiagSeverity,
    /// Actionable fix suggestion (may be empty).
    pub how_to_fix: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiagSeverity {
    Error,
}

/// Parse a CXL expression and return UI-friendly validation diagnostics.
///
/// Called synchronously on every keystroke in a CXL input field. The Pratt
/// parser is fast enough (sub-millisecond for <200-char expressions) that
/// no debounce is needed.
pub fn validate_expr(source: &str) -> CxlValidation {
    if source.trim().is_empty() {
        return CxlValidation {
            is_valid: true,
            errors: vec![],
        };
    }

    let result = Parser::parse(source);

    let errors: Vec<CxlDiagnostic> = result
        .errors
        .iter()
        .map(|e| CxlDiagnostic {
            start: e.span.start as usize,
            end: e.span.end as usize,
            message: e.message.clone(),
            severity: DiagSeverity::Error,
            how_to_fix: e.how_to_fix.clone(),
        })
        .collect();

    CxlValidation {
        is_valid: errors.is_empty(),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_input_is_valid() {
        for src in ["", "   ", "\n\t "] {
            let result = validate_expr(src);
            assert!(result.is_valid, "{src:?} should be valid");
            assert!(result.errors.is_empty(), "{src:?} should have no errors");
        }
    }

    #[test]
    fn well_formed_expression_is_valid() {
        // `sum(...)` over a column is the canonical aggregate form exercised by
        // the field-lineage tests, so it is a known-good parse target.
        let result = validate_expr("emit total = sum(amount)\n");
        assert!(result.is_valid, "valid CXL should parse clean: {result:?}");
        assert!(result.errors.is_empty());
    }

    #[test]
    fn malformed_expressions_are_flagged() {
        for src in ["emit x =", "sum(amount"] {
            let result = validate_expr(src);
            assert!(!result.is_valid, "{src:?} should be invalid");
            assert!(!result.errors.is_empty(), "{src:?} should report errors");
            assert!(
                !result.errors[0].message.is_empty(),
                "{src:?} first error should carry a message"
            );
        }
    }

    #[test]
    fn at_least_one_malformed_case_offers_a_fix() {
        // The parser emits actionable `how_to_fix` text for most syntax errors;
        // assert at least one of the two malformed cases surfaces a non-empty
        // suggestion so the inspector hint path stays exercised.
        let any_fix = ["emit x =", "sum(amount"].iter().any(|src| {
            validate_expr(src)
                .errors
                .iter()
                .any(|d| !d.how_to_fix.is_empty())
        });
        assert!(any_fix, "expected a non-empty how_to_fix from the parser");
    }
}
