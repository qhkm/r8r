//! REST client for loading initial data from a running r8r server.

use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{error, info};

use super::event::{ExecutionSummary, InitialData, TuiEvent, WorkflowSummary};

/// REST response for listing executions.
#[derive(Deserialize)]
struct ExecutionsResponse {
    executions: Vec<ExecutionItem>,
}

#[derive(Deserialize)]
struct ExecutionItem {
    id: String,
    workflow_name: String,
    status: String,
    trigger_type: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
    error: Option<String>,
}

/// REST response for listing workflows.
#[derive(Deserialize)]
struct WorkflowsResponse {
    workflows: Vec<WorkflowItem>,
}

#[derive(Deserialize)]
struct WorkflowItem {
    name: String,
}

/// REST response for getting a single workflow's details including definition.
#[derive(Deserialize)]
struct WorkflowDetailResponse {
    name: String,
    definition: String,
}

/// Load initial data from the REST API and send it via the channel.
pub async fn load_initial_data(base_url: &str, token: Option<&str>, tx: mpsc::Sender<TuiEvent>) {
    info!("Loading initial data from {}", base_url);

    let client = reqwest::Client::new();
    let base = base_url.trim_end_matches('/');

    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(t) = token {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", t)) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    let mut executions = Vec::new();
    let mut workflows = Vec::new();

    // Load executions
    let exec_url = format!("{}/api/executions?limit=50", base);
    match client.get(&exec_url).headers(headers.clone()).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<ExecutionsResponse>().await {
                    Ok(data) => {
                        for item in data.executions {
                            let started_at = chrono::DateTime::parse_from_rfc3339(&item.started_at)
                                .map(|d| d.with_timezone(&chrono::Utc))
                                .unwrap_or_else(|_| chrono::Utc::now());

                            let finished_at = item.finished_at.and_then(|f| {
                                chrono::DateTime::parse_from_rfc3339(&f)
                                    .map(|d| d.with_timezone(&chrono::Utc))
                                    .ok()
                            });

                            executions.push(ExecutionSummary {
                                id: item.id,
                                workflow_name: item.workflow_name,
                                status: item.status,
                                trigger_type: item.trigger_type,
                                started_at,
                                finished_at,
                                duration_ms: item.duration_ms,
                                error: item.error,
                            });
                        }
                        info!("Loaded {} executions", executions.len());
                    }
                    Err(e) => error!("Failed to parse executions response: {}", e),
                }
            } else {
                error!("Failed to load executions: HTTP {}", resp.status());
            }
        }
        Err(e) => error!("Failed to connect for executions: {}", e),
    }

    // Load workflow list
    let wf_url = format!("{}/api/workflows", base);
    match client.get(&wf_url).headers(headers.clone()).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<WorkflowsResponse>().await {
                    Ok(data) => {
                        // Load each workflow's definition for DAG building
                        for item in data.workflows {
                            let detail_url = format!("{}/api/workflows/{}", base, item.name);
                            match client
                                .get(&detail_url)
                                .headers(headers.clone())
                                .send()
                                .await
                            {
                                Ok(detail_resp) => {
                                    if detail_resp.status().is_success() {
                                        match detail_resp.json::<WorkflowDetailResponse>().await {
                                            Ok(detail) => {
                                                workflows.push(WorkflowSummary {
                                                    name: detail.name,
                                                    definition: detail.definition,
                                                });
                                            }
                                            Err(e) => {
                                                error!("Failed to parse workflow detail: {}", e);
                                            }
                                        }
                                    }
                                }
                                Err(e) => error!("Failed to load workflow {}: {}", item.name, e),
                            }
                        }
                        info!("Loaded {} workflows", workflows.len());
                    }
                    Err(e) => error!("Failed to parse workflows response: {}", e),
                }
            } else {
                error!("Failed to load workflows: HTTP {}", resp.status());
            }
        }
        Err(e) => error!("Failed to connect for workflows: {}", e),
    }

    let _ = tx
        .send(TuiEvent::InitialData(InitialData {
            executions,
            workflows,
        }))
        .await;
}
