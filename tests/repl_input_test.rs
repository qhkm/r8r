use r8r::repl::input::{parse_input, InputCommand};

#[test]
fn test_slash_run() {
    let cmd = parse_input("/run my-workflow");
    assert!(matches!(cmd, InputCommand::Run { workflow } if workflow == "my-workflow"));
}

#[test]
fn test_slash_list() {
    let cmd = parse_input("/list");
    assert!(matches!(cmd, InputCommand::List));
}

#[test]
fn test_slash_dag() {
    let cmd = parse_input("/dag my-workflow");
    assert!(matches!(cmd, InputCommand::Dag { workflow } if workflow == "my-workflow"));
}

#[test]
fn test_slash_show() {
    let cmd = parse_input("/show my-workflow");
    assert!(matches!(cmd, InputCommand::Show { workflow } if workflow == "my-workflow"));
}

#[test]
fn test_slash_sessions() {
    let cmd = parse_input("/sessions");
    assert!(matches!(cmd, InputCommand::Sessions));
}

#[test]
fn test_slash_help() {
    let cmd = parse_input("/help");
    assert!(matches!(cmd, InputCommand::Help));
}

#[test]
fn test_slash_exit() {
    let cmd = parse_input("/exit");
    assert!(matches!(cmd, InputCommand::Exit));
}

#[test]
fn test_slash_clear() {
    let cmd = parse_input("/clear");
    assert!(matches!(cmd, InputCommand::Clear));
}

#[test]
fn test_natural_language() {
    let cmd = parse_input("create a workflow that checks bitcoin price");
    assert!(
        matches!(cmd, InputCommand::NaturalLanguage(text) if text == "create a workflow that checks bitcoin price")
    );
}

#[test]
fn test_unknown_slash_command() {
    let cmd = parse_input("/unknown");
    assert!(matches!(cmd, InputCommand::Unknown(text) if text == "/unknown"));
}

#[test]
fn test_empty_input() {
    let cmd = parse_input("");
    assert!(matches!(cmd, InputCommand::Empty));
}

#[test]
fn test_slash_trace() {
    let cmd = parse_input("/trace exec-123");
    assert!(matches!(cmd, InputCommand::Trace { execution_id } if execution_id == "exec-123"));
}

#[test]
fn test_slash_logs() {
    let cmd = parse_input("/logs my-workflow");
    assert!(matches!(cmd, InputCommand::Logs { workflow } if workflow == "my-workflow"));
}

#[test]
fn test_slash_model_only_model_name() {
    let cmd = parse_input("/model gpt-4o");
    assert!(matches!(cmd, InputCommand::SetModel { provider: None, model } if model == "gpt-4o"));
}

#[test]
fn test_slash_model_provider_and_model() {
    let cmd = parse_input("/model openai gpt-4o");
    assert!(
        matches!(cmd, InputCommand::SetModel { provider: Some(provider), model } if provider == "openai" && model == "gpt-4o")
    );
}

#[test]
fn test_slash_apikey() {
    let cmd = parse_input("/apikey sk-test");
    assert!(matches!(cmd, InputCommand::SetApiKey { api_key } if api_key == "sk-test"));
}

#[test]
fn test_slash_endpoint() {
    let cmd = parse_input("/endpoint http://localhost:11434/api/chat");
    assert!(
        matches!(cmd, InputCommand::SetEndpoint { endpoint } if endpoint == "http://localhost:11434/api/chat")
    );
}

#[test]
fn test_slash_llm() {
    let cmd = parse_input("/llm");
    assert!(matches!(cmd, InputCommand::ShowLlm));
}

#[test]
fn test_slash_view() {
    let cmd = parse_input("/view yaml");
    assert!(matches!(cmd, InputCommand::View { target } if target == "yaml"));
}

#[test]
fn test_slash_export_yaml() {
    let cmd = parse_input("/export yaml /tmp/hn.yaml");
    assert!(matches!(cmd, InputCommand::ExportYaml { path } if path == "/tmp/hn.yaml"));
}
