use r8r::cli::diagnostics::{classify_error, fuzzy_match, Diagnostic, DiagnosticKind};

#[test]
fn test_classify_workflow_not_found() {
    let kind = classify_error(
        "WORKFLOW_ERROR",
        "Workflow error: workflow 'my-wf' not found",
    );
    match kind {
        DiagnosticKind::WorkflowNotFound { name } => assert_eq!(name, "my-wf"),
        other => panic!("Expected WorkflowNotFound, got {:?}", other),
    }
}

#[test]
fn test_classify_no_llm_configured() {
    let kind = classify_error(
        "CONFIG_ERROR",
        "Configuration error: no LLM provider configured",
    );
    assert!(matches!(kind, DiagnosticKind::NoLlmConfigured));
}

#[test]
fn test_classify_yaml_parse_error() {
    let kind = classify_error(
        "YAML_ERROR",
        "YAML error: mapping values are not allowed in this context at line 5 column 3",
    );
    match kind {
        DiagnosticKind::YamlParseError { line, column } => {
            assert_eq!(line, Some(5));
            assert_eq!(column, Some(3));
        }
        other => panic!("Expected YamlParseError, got {:?}", other),
    }
}

#[test]
fn test_classify_generic_fallback() {
    let kind = classify_error("INTERNAL_ERROR", "something unexpected");
    assert!(matches!(kind, DiagnosticKind::Generic));
}

#[test]
fn test_fuzzy_match_finds_similar() {
    let candidates = vec!["my-workflow", "other-workflow", "test-wf"];
    let matches = fuzzy_match("my-workflo", &candidates, 0.7);
    assert!(matches.contains(&"my-workflow".to_string()));
}

#[test]
fn test_fuzzy_match_no_match_below_threshold() {
    let candidates = vec!["completely-different"];
    let matches = fuzzy_match("my-workflow", &candidates, 0.7);
    assert!(matches.is_empty());
}

#[test]
fn test_diagnostic_from_error_enriches_workflow_not_found() {
    let err = r8r::error::Error::Workflow("workflow 'test-wf' not found".into());
    let diag = Diagnostic::from_error(&err, &["test-workflow", "test-wf-2"]);
    assert!(!diag.hints.is_empty());
    assert!(diag.hints.iter().any(|h| h.contains("workflows list")));
}

#[test]
fn test_diagnostic_from_error_generic_has_no_suggestions() {
    let err = r8r::error::Error::Internal("something broke".into());
    let diag = Diagnostic::from_error(&err, &[]);
    assert!(diag.suggestions.is_empty());
}
