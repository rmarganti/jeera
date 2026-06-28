use super::SearchIntent;
use crate::cli::SearchArgs;
use crate::client::JiraClient;
use crate::error::AppError;

pub(super) fn merge_search_profile(
    client: &JiraClient,
    intent: &SearchIntent,
) -> Result<SearchIntent, AppError> {
    let args = intent.to_search_args();
    let Some(profile_name) = args.profile.as_deref() else {
        return Ok(intent.clone());
    };

    let profile = client
        .search_profile(profile_name)
        .ok_or_else(|| AppError::InvalidSearch {
            reason: format!("unknown search profile {profile_name:?}"),
        })?;

    let (assignee, unassigned) = if args.unassigned {
        (None, true)
    } else if let Some(assignee) = &args.assignee {
        (Some(assignee.clone()), false)
    } else {
        (profile.assignee.clone(), profile.unassigned)
    };

    let (asc, desc) = if args.asc {
        (true, false)
    } else if args.desc {
        (false, true)
    } else {
        (profile.asc, profile.desc)
    };

    SearchIntent::try_from(&SearchArgs {
        json: args.json,
        profile: None,
        query: args.query.clone(),
        jql: args.jql.clone().or_else(|| profile.jql.clone()),
        board: args.board.clone().or_else(|| profile.board.clone()),
        project: args.project.clone().or_else(|| profile.project.clone()),
        assignee,
        unassigned,
        reporter: args.reporter.clone().or_else(|| profile.reporter.clone()),
        status: merged_vec(&profile.status, &args.status),
        status_category: args
            .status_category
            .clone()
            .or_else(|| profile.status_category.clone()),
        issue_type: merged_vec(&profile.issue_type, &args.issue_type),
        component: merged_vec(&profile.component, &args.component),
        label: merged_vec(&profile.label, &args.label),
        text: args.text.clone().or_else(|| profile.text.clone()),
        open: args.open || profile.open,
        limit: args.limit.or(profile.limit),
        next_page_token: args.next_page_token.clone(),
        columns: args.columns.clone(),
        debug_jql: args.debug_jql,
        sort: args.sort.clone().or_else(|| profile.sort.clone()),
        asc,
        desc,
    })
}

fn merged_vec(profile_values: &[String], cli_values: &[String]) -> Vec<String> {
    profile_values
        .iter()
        .chain(cli_values.iter())
        .cloned()
        .collect()
}
