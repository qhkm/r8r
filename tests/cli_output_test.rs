use r8r::cli::output::{Output, OutputMode};
use serde::Serialize;

#[derive(Serialize)]
struct TestItem {
    name: String,
    count: u32,
}

#[test]
fn test_output_human_success() {
    let out = Output::new(OutputMode::Human);
    out.success("done");
}

#[test]
fn test_output_json_success() {
    let out = Output::new(OutputMode::Json);
    out.success("done");
}

#[test]
fn test_output_json_list() {
    let out = Output::new(OutputMode::Json);
    let items = vec![
        TestItem { name: "a".into(), count: 1 },
        TestItem { name: "b".into(), count: 2 },
    ];
    out.list(
        &["NAME", "COUNT"],
        &items,
        |item| vec![item.name.clone(), item.count.to_string()],
    );
}

#[test]
fn test_output_suggest_suppressed_in_json() {
    let out = Output::new(OutputMode::Json);
    out.suggest(&[("r8r help", "show help")]);
}

#[test]
fn test_output_is_json() {
    let human = Output::new(OutputMode::Human);
    let json = Output::new(OutputMode::Json);
    assert!(!human.is_json());
    assert!(json.is_json());
}
