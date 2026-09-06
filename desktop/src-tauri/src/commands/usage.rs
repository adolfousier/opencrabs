use crate::AppState;
use opencrabs::usage::data::{DashboardData, Period};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct DashboardDataInfo {
    pub summary: SummaryInfo,
    pub daily: Vec<DailyInfo>,
    pub projects: Vec<ProjectInfo>,
    pub models: Vec<ModelInfo>,
    pub tools: Vec<ToolInfo>,
    pub activities: Vec<ActivityInfo>,
    pub cache: Option<CacheInfo>,
}

#[derive(Serialize)]
pub struct SummaryInfo {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub session_count: i64,
    pub call_count: i64,
}

#[derive(Serialize)]
pub struct DailyInfo {
    pub date: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
}

#[derive(Serialize)]
pub struct ProjectInfo {
    pub project: String,
    pub cost: f64,
    pub tokens: i64,
    pub sessions: i64,
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub model: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
    pub estimated: bool,
    pub variants: Vec<VariantInfo>,
}

#[derive(Serialize)]
pub struct VariantInfo {
    pub name: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
}

#[derive(Serialize)]
pub struct ToolInfo {
    pub tool_name: String,
    pub call_count: i64,
}

#[derive(Serialize)]
pub struct ActivityInfo {
    pub category: String,
    pub cost: f64,
    pub turns: i64,
    pub one_shot_pct: f64,
}

#[derive(Serialize)]
pub struct CacheInfo {
    pub cache_hit_pct: f64,
    pub cached_tokens: i64,
    pub total_input_tokens: i64,
}

fn period_from_str(s: &str) -> Period {
    match s {
        "today" => Period::Today,
        "week" => Period::Week,
        "month" => Period::Month,
        _ => Period::AllTime,
    }
}

#[tauri::command]
pub async fn get_usage_data(
    state: State<'_, AppState>,
    period: String,
) -> Result<DashboardDataInfo, String> {
    let sm = state.service_manager.lock().await;
    let sm = sm.as_ref().ok_or("Service not initialized")?;
    let pool = sm.context().pool();
    let p = period_from_str(&period);

    let data = DashboardData::fetch(&pool, p)
        .await
        .map_err(|e| e.to_string())?;

    Ok(DashboardDataInfo {
        summary: SummaryInfo {
            total_tokens: data.summary.total_tokens,
            total_cost: data.summary.total_cost,
            session_count: data.summary.session_count,
            call_count: data.summary.call_count,
        },
        daily: data
            .daily
            .iter()
            .map(|d| DailyInfo {
                date: d.date.clone(),
                tokens: d.tokens,
                cost: d.cost,
                calls: d.calls,
            })
            .collect(),
        projects: data
            .projects
            .iter()
            .map(|p| ProjectInfo {
                project: p.project.clone(),
                cost: p.cost,
                tokens: p.tokens,
                sessions: p.sessions,
            })
            .collect(),
        models: data
            .models
            .iter()
            .map(|m| ModelInfo {
                model: m.model.clone(),
                tokens: m.tokens,
                cost: m.cost,
                calls: m.calls,
                estimated: m.estimated,
                variants: m
                    .variants
                    .iter()
                    .map(|v| VariantInfo {
                        name: v.name.clone(),
                        tokens: v.tokens,
                        cost: v.cost,
                        calls: v.calls,
                    })
                    .collect(),
            })
            .collect(),
        tools: data
            .tools
            .iter()
            .map(|t| ToolInfo {
                tool_name: t.tool_name.clone(),
                call_count: t.call_count,
            })
            .collect(),
        activities: data
            .activities
            .iter()
            .map(|a| ActivityInfo {
                category: a.category.clone(),
                cost: a.cost,
                turns: a.turns,
                one_shot_pct: a.one_shot_pct,
            })
            .collect(),
        cache: data.cache.map(|c| CacheInfo {
            cache_hit_pct: c.cache_hit_pct,
            cached_tokens: c.cached_tokens,
            total_input_tokens: c.total_input_tokens,
        }),
    })
}
