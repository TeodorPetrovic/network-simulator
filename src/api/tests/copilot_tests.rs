use super::*;
use crate::api::copilot_llm::to_openai_messages;

#[test]
fn canonical_messages_convert_to_openai_tool_format() {
    let messages = vec![
        json!({"role":"user","content":[{"type":"text","text":"list"}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"ok"},{"type":"tool_use","id":"c1","name":"project_list","input":{}}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"[]","is_error":false}]}),
    ];
    let out = to_openai_messages("sys", &messages);
    assert_eq!(out[0]["role"], "system");
    assert_eq!(out[2]["tool_calls"][0]["function"]["name"], "project_list");
    assert_eq!(out[3]["role"], "tool");
    assert_eq!(out[3]["tool_call_id"], "c1");
}

#[test]
fn base_url_must_be_http() {
    assert!(validate_base_url("ftp://x").is_err());
    assert!(validate_base_url("http://localhost:11434/v1").is_ok());
    assert!(validate_base_url("").is_ok());
}
