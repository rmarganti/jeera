use super::*;
use crate::cli::SearchArgs;
use crate::client::{JiraAuth, JiraClient, JiraClientConfig};
use crate::config::SearchProfileSettings;
use crate::issue_search::tests_support::spawn_server;
use std::collections::BTreeMap;
use std::time::Duration;

#[test]
fn execute_exposes_effective_intent_and_expands_continuation_from_it() {
    let body = r#"{"isLast":false,"nextPageToken":"next token","issues":[]}"#;
    let (base_url, rx) = spawn_server("200 OK", body);
    let mut searches = BTreeMap::new();
    searches.insert(
        "qqms".to_string(),
        SearchProfileSettings {
            project: Some("SAMPLE".to_string()),
            component: vec!["QQMS".to_string()],
            limit: Some(25),
            ..Default::default()
        },
    );
    let client = JiraClient::new(JiraClientConfig {
        base_url,
        auth: JiraAuth::Basic {
            email: "user@example.com".to_string(),
            api_token: "token".to_string(),
        },
        timeout: Duration::from_secs(5),
        default_board_id: None,
        searches,
    });
    let intent = SearchIntent::try_from(&SearchArgs {
        profile: Some("qqms".to_string()),
        ..Default::default()
    })
    .unwrap();

    let execution = execute(&client, intent).unwrap();

    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        execution.effective_intent().project.as_deref(),
        Some("SAMPLE")
    );
    assert_eq!(execution.effective_intent().component, vec!["QQMS"]);
    assert_eq!(execution.effective_intent().limit, Some(25));
    assert_eq!(
        execution.continuation().unwrap().next_page_token(),
        "next token"
    );

    let mut rendered = Vec::new();
    super::render_human(&mut rendered, &execution).unwrap();
    assert_eq!(
        String::from_utf8(rendered).unwrap(),
        "No issues found.\nNext page token: next token\nNext page command: jeera search --project SAMPLE --component QQMS --limit 25 --next-page-token 'next token'\n"
    );
}
