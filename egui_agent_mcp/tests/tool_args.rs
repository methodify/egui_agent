//! Regression tests for MCP argument handling: hosts that stringify
//! structured arguments, string node ids, and the quiet flag.

use egui_agent::{Command, Selector};
use egui_agent_mcp::command_for_tool;
use serde_json::json;

#[test]
fn stringified_args_and_targets_are_unwrapped() {
    // Some MCP hosts serialize object arguments as JSON-encoded strings.
    let cmd = command_for_tool(
        "invoke",
        &json!({
            "action": "click",
            "target": "{\"id\": \"submit\"}",
            "args": "{\"count\": 2}"
        }),
    )
    .unwrap();
    let Command::Invoke { target, args, .. } = cmd else {
        panic!()
    };
    assert_eq!(target, Some(Selector::Id("submit".into())));
    assert_eq!(args, Some(json!({ "count": 2 })));

    // Stringified predicates too.
    let cmd = command_for_tool(
        "wait_for",
        &json!({ "predicate": "{\"exists\": {\"id\": \"done\"}}" }),
    )
    .unwrap();
    assert!(matches!(cmd, Command::WaitFor { .. }));
}

#[test]
fn plain_string_args_are_left_alone() {
    let cmd = command_for_tool(
        "invoke",
        &json!({ "action": "add_task", "args": "walk dog" }),
    )
    .unwrap();
    let Command::Invoke { args, .. } = cmd else {
        panic!()
    };
    assert_eq!(args, Some(json!("walk dog")));

    // set_value never destringifies: a string is a first-class value.
    let cmd = command_for_tool(
        "set_value",
        &json!({ "target": {"id": "notes"}, "value": "{\"literal\": \"text\"}" }),
    )
    .unwrap();
    let Command::SetValue { value, .. } = cmd else {
        panic!()
    };
    assert_eq!(value, json!("{\"literal\": \"text\"}"));
}

#[test]
fn string_node_ids_parse_in_selectors() {
    let big = (u64::MAX - 99).to_string();
    let cmd = command_for_tool("query", &json!({ "target": { "node": big } })).unwrap();
    let Command::Query { target } = cmd else {
        panic!()
    };
    assert_eq!(target, Selector::Node(u64::MAX - 99));
}

#[test]
fn quiet_flag_is_parsed_including_stringified() {
    let cmd = command_for_tool(
        "invoke",
        &json!({ "action": "click", "target": {"id": "a"}, "quiet": true }),
    )
    .unwrap();
    assert!(matches!(cmd, Command::Invoke { quiet: true, .. }));

    let cmd = command_for_tool(
        "set_value",
        &json!({ "target": {"id": "a"}, "value": 1, "quiet": "true" }),
    )
    .unwrap();
    assert!(matches!(cmd, Command::SetValue { quiet: true, .. }));

    let cmd =
        command_for_tool("wait_for", &json!({ "predicate": {"exists": {"id": "a"}} })).unwrap();
    assert!(matches!(cmd, Command::WaitFor { quiet: false, .. }));
}
