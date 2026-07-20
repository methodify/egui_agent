//! M4: `#[derive(Snapshot)]` from `egui_agent_macros`.

use egui_agent::ToSnapshot as _;
use serde_json::json;

#[derive(egui_agent_macros::Snapshot)]
struct AppState {
    count: u32,
    #[snapshot(id = "user_name")]
    name: String,
    #[snapshot(skip)]
    #[allow(dead_code)]
    cache: Vec<u8>,
    flags: Flags,
}

#[derive(serde::Serialize)]
struct Flags {
    dark: bool,
}

#[test]
fn derive_snapshot_emits_value_nodes() {
    let state = AppState {
        count: 3,
        name: "Ada".into(),
        cache: vec![1, 2, 3],
        flags: Flags { dark: true },
    };
    let snap = state.to_snapshot();
    assert_eq!(snap.nodes.len(), 3, "skipped fields are omitted");

    let count = snap
        .select_unique(&egui_agent::Selector::Id("count".into()))
        .unwrap();
    assert_eq!(count.role, "value");
    assert_eq!(count.value, Some(json!(3)));

    let name = snap
        .select_unique(&egui_agent::Selector::Id("user_name".into()))
        .unwrap();
    assert_eq!(name.value, Some(json!("Ada")), "id override respected");

    let flags = snap
        .select_unique(&egui_agent::Selector::Id("flags".into()))
        .unwrap();
    assert_eq!(
        flags.value,
        Some(json!({ "dark": true })),
        "nested serde structs serialize"
    );
}
