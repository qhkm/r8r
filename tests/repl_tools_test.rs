use r8r::repl::tools::{build_tool_definitions, parse_text_tool_call};

#[test]
fn test_build_tool_definitions_contains_all_tools() {
    let defs = build_tool_definitions();
    assert!(
        defs.len() >= 15,
        "Expected at least 15 tool definitions, got {}",
        defs.len()
    );

    let names: Vec<&str> = defs
        .iter()
        .filter_map(|d| d["function"]["name"].as_str())
        .collect();
    assert!(names.contains(&"r8r_execute"));
    assert!(names.contains(&"r8r_list_workflows"));
    assert!(names.contains(&"r8r_generate"));
}

#[test]
fn test_tool_definition_has_parameters() {
    let defs = build_tool_definitions();
    let execute = defs
        .iter()
        .find(|d| d["function"]["name"].as_str() == Some("r8r_execute"))
        .expect("r8r_execute tool not found");

    let params = &execute["function"]["parameters"];
    assert!(params["properties"]["workflow"].is_object());
}

#[test]
fn test_parse_text_tool_call_valid() {
    let text = r#"<tool_call>{"name": "r8r_list_workflows", "args": {}}</tool_call>"#;
    let result = parse_text_tool_call(text);
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "r8r_list_workflows");
    assert!(args.is_object());
}

#[test]
fn test_parse_text_tool_call_no_match() {
    let text = "Just regular text here.";
    let result = parse_text_tool_call(text);
    assert!(result.is_none());
}

#[test]
fn test_parse_text_tool_call_with_args() {
    let text = r#"<tool_call>{"name": "r8r_execute", "args": {"workflow": "test-wf", "input": {"key": "val"}}}</tool_call>"#;
    let result = parse_text_tool_call(text);
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "r8r_execute");
    assert_eq!(args["workflow"].as_str().unwrap(), "test-wf");
}
