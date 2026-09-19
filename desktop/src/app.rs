use opencrabs_desktop_ui::bridge::{invoke, invoke_unit};
use opencrabs_desktop_ui::models::*;

use dioxus::prelude::*;
use serde_json::json;

const NAV_ITEMS: &[NavItem] = &[
    NavItem {
        id: RouteId::Chat,
        icon: "💬",
    },
    NavItem {
        id: RouteId::Files,
        icon: "📁",
    },
    NavItem {
        id: RouteId::Brain,
        icon: "🧠",
    },
    NavItem {
        id: RouteId::Providers,
        icon: "⚙️",
    },
    NavItem {
        id: RouteId::Tools,
        icon: "🛠",
    },
    NavItem {
        id: RouteId::Skills,
        icon: "✨",
    },
    NavItem {
        id: RouteId::Cron,
        icon: "⏱",
    },
    NavItem {
        id: RouteId::Channels,
        icon: "📡",
    },
    NavItem {
        id: RouteId::Usage,
        icon: "📊",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteId {
    Chat,
    Files,
    Brain,
    Providers,
    Tools,
    Skills,
    Cron,
    Channels,
    Usage,
}

impl RouteId {
    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Files => "files",
            Self::Brain => "brain",
            Self::Providers => "providers",
            Self::Tools => "tools",
            Self::Skills => "skills",
            Self::Cron => "cron",
            Self::Channels => "channels",
            Self::Usage => "usage",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(Self::Chat),
            "files" => Some(Self::Files),
            "brain" => Some(Self::Brain),
            "providers" => Some(Self::Providers),
            "tools" => Some(Self::Tools),
            "skills" => Some(Self::Skills),
            "cron" => Some(Self::Cron),
            "channels" => Some(Self::Channels),
            "usage" => Some(Self::Usage),
            _ => None,
        }
    }
}

struct NavItem {
    id: RouteId,
    icon: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActionStatus {
    scope: &'static str,
    message: String,
}

impl ActionStatus {
    fn new(scope: &'static str, message: impl Into<String>) -> Self {
        Self {
            scope,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ChatRequestState {
    active: bool,
}

fn push_warning_signal(signal: &mut Signal<Vec<String>>, message: String) {
    signal.with_mut(|items| items.push(message));
}

fn set_action_error(signal: &mut Signal<Vec<ActionStatus>>, scope: &'static str, message: String) {
    signal.with_mut(|items| {
        if let Some(existing) = items.iter_mut().find(|item| item.scope == scope) {
            *existing = ActionStatus::new(scope, message);
        } else {
            items.push(ActionStatus::new(scope, message));
        }
    });
}

fn clear_action_error(signal: &mut Signal<Vec<ActionStatus>>, scope: &'static str) {
    signal.with_mut(|items| items.retain(|item| item.scope != scope));
}

fn format_token_count(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn compact_path(path: Option<&str>) -> String {
    let Some(path) = path else {
        return "No workspace".to_string();
    };
    let parts: Vec<_> = path.rsplit('/').take(2).collect();
    match parts.as_slice() {
        [last, parent] if !last.is_empty() => format!("…/{parent}/{last}"),
        [last] if !last.is_empty() => (*last).to_string(),
        _ => path.to_string(),
    }
}

fn session_activity_label(timestamp: &str) -> String {
    timestamp
        .split('T')
        .next()
        .map(|date| format!("Updated {date}"))
        .unwrap_or_else(|| "Updated recently".to_string())
}

fn session_matches(session: &SessionInfo, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    [
        Some(session.title.as_str()),
        session.project_name.as_deref(),
        session.working_directory.as_deref(),
        session.provider_name.as_deref(),
        session.model.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&query))
}

#[component]
pub fn App() -> Element {
    let mut route = use_signal(|| RouteId::Chat);
    let mut sessions = use_signal(Vec::<SessionInfo>::new);
    let mut session_query = use_signal(String::new);
    let mut renaming_session = use_signal(|| None::<String>);
    let mut rename_title = use_signal(String::new);
    let mut deleting_session = use_signal(|| None::<String>);
    let mut selected_session = use_signal(|| None::<String>);
    let mut session_messages = use_signal(Vec::<MessageInfo>::new);
    let mut composer = use_signal(String::new);
    let workspace_root = use_signal(String::new);
    let current_directory = use_signal(String::new);
    let file_entries = use_signal(Vec::<FileEntry>::new);
    let selected_file = use_signal(|| None::<String>);
    let file_content = use_signal(|| None::<FileContent>);
    let brain_files = use_signal(Vec::<BrainFile>::new);
    let mut active_brain_tab = use_signal(|| None::<String>);
    let mut brain_editor = use_signal(String::new);
    let config_info = use_signal(|| None::<ConfigInfo>);
    let tools = use_signal(Vec::<ToolInfo>::new);
    let selected_tool = use_signal(|| None::<ToolDetail>);
    let skills = use_signal(Vec::<SkillInfo>::new);
    let selected_skill = use_signal(|| None::<SkillDetail>);
    let cron_jobs = use_signal(Vec::<CronJobInfo>::new);
    let cron_runs = use_signal(Vec::<CronJobRunInfo>::new);
    let channels = use_signal(Vec::<ChannelStatus>::new);
    let usage = use_signal(|| None::<DashboardDataInfo>);
    let status = use_signal(|| "Loading…".to_string());
    let error = use_signal(|| None::<String>);
    let background_warnings = use_signal(Vec::<String>::new);
    let mut action_errors = use_signal(Vec::<ActionStatus>::new);
    let mut chat_request = use_signal(ChatRequestState::default);
    // Chat intentionally uses the completed request/response command. The prior
    // event stream required long-lived JavaScript closures whose ownership and
    // unlisten lifecycle were not safe in the WASM shell.

    use_future(move || {
        let mut route = route;
        let mut sessions = sessions;
        let mut selected_session = selected_session;
        let mut session_messages = session_messages;
        let mut status = status;
        let mut error = error;
        let mut background_warnings = background_warnings;
        let mut workspace_root = workspace_root;
        let mut current_directory = current_directory;
        let mut file_entries = file_entries;
        let mut selected_file = selected_file;
        let mut file_content = file_content;
        let mut brain_files = brain_files;
        let mut active_brain_tab = active_brain_tab;
        let mut brain_editor = brain_editor;
        let mut config_info = config_info;
        let mut tools = tools;
        let mut skills = skills;
        let mut cron_jobs = cron_jobs;
        let mut channels = channels;
        let mut usage = usage;
        let mut chat_request = chat_request;

        async move {
            status.set("Loading sessions…".to_string());
            error.set(None);
            chat_request.set(ChatRequestState::default());

            let persisted = invoke::<DesktopState, _>("get_desktop_state", json!({}))
                .await
                .unwrap_or(DesktopState {
                    route: RouteId::Chat.as_str().to_string(),
                    selected_session_id: None,
                });
            route.set(RouteId::parse(&persisted.route).unwrap_or(RouteId::Chat));

            match invoke::<Vec<SessionInfo>, _>("list_sessions", json!({})).await {
                Ok(list) => {
                    let persisted_id = persisted
                        .selected_session_id
                        .filter(|id| list.iter().any(|session| &session.id == id));
                    let session_id =
                        persisted_id.or_else(|| list.first().map(|session| session.id.clone()));
                    sessions.set(list);
                    selected_session.set(session_id.clone());
                    if let Some(session_id) = session_id {
                        match invoke::<Vec<MessageInfo>, _>(
                            "get_session_messages",
                            json!({"sessionId": session_id}),
                        )
                        .await
                        {
                            Ok(messages) => session_messages.set(messages),
                            Err(message) => error.set(Some(message)),
                        }
                    }
                    status.set("Ready".to_string());
                }
                Err(message) => {
                    status.set("Sessions unavailable".to_string());
                    error.set(Some(message));
                }
            }

            spawn(async move {
                let mut warnings = Vec::new();
                if let Ok(root) = invoke::<String, _>("get_workspace_root", json!({})).await {
                    workspace_root.set(root.clone());
                    current_directory.set(root.clone());
                    match invoke::<Vec<FileEntry>, _>("list_directory", json!({"path": root})).await
                    {
                        Ok(entries) => {
                            if let Some(first_file) = entries.iter().find(|entry| !entry.is_dir) {
                                selected_file.set(Some(first_file.path.clone()));
                                if let Ok(content) = invoke::<FileContent, _>(
                                    "read_file_content",
                                    json!({"path": first_file.path.clone()}),
                                )
                                .await
                                {
                                    file_content.set(Some(content));
                                }
                            }
                            file_entries.set(entries);
                        }
                        Err(message) => warnings.push(format!("Files unavailable: {message}")),
                    }
                } else {
                    warnings.push("Workspace root unavailable".to_string());
                }
                if !warnings.is_empty() {
                    background_warnings.with_mut(|items| items.extend(warnings));
                }
            });

            spawn(async move {
                match invoke::<Vec<BrainFile>, _>("list_brain_files", json!({})).await {
                    Ok(files) => {
                        if let Some(first) = files.first() {
                            active_brain_tab.set(Some(first.name.clone()));
                            brain_editor.set(first.content.clone());
                        }
                        brain_files.set(files);
                    }
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Brain files unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<ConfigInfo, _>("get_config", json!({})).await {
                    Ok(cfg) => config_info.set(Some(cfg)),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Config unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<Vec<ToolInfo>, _>("list_tools", json!({})).await {
                    Ok(list) => tools.set(list),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Tools unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<Vec<SkillInfo>, _>("list_skills", json!({})).await {
                    Ok(list) => skills.set(list),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Skills unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<Vec<CronJobInfo>, _>("list_cron_jobs", json!({})).await {
                    Ok(list) => cron_jobs.set(list),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Cron unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<Vec<ChannelStatus>, _>("get_channel_statuses", json!({})).await {
                    Ok(list) => channels.set(list),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Channels unavailable: {message}"),
                    ),
                }
            });

            spawn(async move {
                match invoke::<DashboardDataInfo, _>("get_usage_data", json!({"period": "week"}))
                    .await
                {
                    Ok(value) => usage.set(Some(value)),
                    Err(message) => push_warning_signal(
                        &mut background_warnings,
                        format!("Usage unavailable: {message}"),
                    ),
                }
            });
        }
    });

    let current_session_title = sessions
        .read()
        .iter()
        .find(|session| Some(&session.id) == selected_session.read().as_ref())
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "No session selected".to_string());

    let rendered_messages = session_messages.read().clone();

    let filtered_sessions: Vec<SessionInfo> = sessions
        .read()
        .iter()
        .filter(|session| session_matches(session, session_query.read().as_str()))
        .cloned()
        .collect();
    let delete_target_title = deleting_session.read().as_ref().and_then(|id| {
        sessions
            .read()
            .iter()
            .find(|session| &session.id == id)
            .map(|session| session.title.clone())
    });
    rsx! {
        div { class: "app-shell",
            header { class: "topbar",
                div { class: "brand",
                    div { class: "brand-mark", "🦀" }
                    div {
                        h1 { "OpenCrabs Desktop" }
                        p { "Dioxus + Tauri command center" }
                    }
                }
                div { class: "top-actions",
                    span { class: "badge", "Provider status wired" }
                    span { class: "badge success", "{status.read()}" }
                }
            }

            nav { class: "tabs",
                for item in NAV_ITEMS {
                    button {
                        class: if *route.read() == item.id { "tab active" } else { "tab" },
                        onclick: {
                            let next_route = item.id;
                            move |_| {
                                route.set(next_route);
                                let state = DesktopState {
                                    route: next_route.as_str().to_string(),
                                    selected_session_id: selected_session.read().clone(),
                                };
                                spawn(async move {
                                    match invoke_unit("save_desktop_state", json!({"state": state})).await {
                                        Ok(()) => clear_action_error(&mut action_errors, "desktop-state"),
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "desktop-state",
                                            format!("Failed to save desktop view: {message}"),
                                        ),
                                    }
                                });
                            }
                        },
                        "{item.icon}"
                    }
                }
            }

            aside { class: "sidebar",
                div { class: "sidebar-header",
                    h2 { "Sessions" }
                    button {
                        class: "btn-new",
                        onclick: move |_| {
                            spawn(async move {
                                let created = invoke::<SessionInfo, _>("create_session", json!({"title": "New desktop session"})).await;
                                match created {
                                    Ok(session) => {
                                        sessions.with_mut(|list| list.insert(0, session.clone()));
                                        selected_session.set(Some(session.id.clone()));
                                        chat_request.set(ChatRequestState::default());
                                        let state = DesktopState {
                                            route: route.read().as_str().to_string(),
                                            selected_session_id: Some(session.id.clone()),
                                        };
                                        match invoke_unit("save_desktop_state", json!({"state": state})).await {
                                            Ok(()) => clear_action_error(&mut action_errors, "desktop-state"),
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "desktop-state",
                                                format!("Failed to save selected session: {message}"),
                                            ),
                                        }
                                        match invoke::<Vec<MessageInfo>, _>(
                                            "get_session_messages",
                                            json!({"sessionId": session.id.clone()}),
                                        )
                                        .await
                                        {
                                            Ok(messages) => session_messages.set(messages),
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "sessions-create",
                                                format!("Session created, but messages could not load: {message}"),
                                            ),
                                        }
                                    }
                                    Err(message) => set_action_error(
                                        &mut action_errors,
                                        "sessions-create",
                                        format!("Failed to create session: {message}"),
                                    ),
                                }
                            });
                        },
                        "+ New"
                    }
                }
                div { class: "sidebar-nav",
                    label { class: "session-search-label", r#for: "session-search", "Find sessions" }
                    input {
                        id: "session-search",
                        class: "session-search",
                        value: "{session_query.read()}",
                        placeholder: "Search title, project, model…",
                        oninput: move |event| session_query.set(event.value()),
                    }
                    div { class: "session-count", "{filtered_sessions.len()} of {sessions.read().len()} sessions" }
                }
                div { class: "session-list", aria_label: "Session history",
                    if filtered_sessions.is_empty() {
                        div { class: "session-empty", "No sessions match this search." }
                    }
                    for session in filtered_sessions {
                        {
                            let session_id = session.id.clone();
                            let title = session.title.clone();
                            let selected = Some(&session_id) == selected_session.read().as_ref();
                            let project_or_path = session.project_name.clone().unwrap_or_else(|| compact_path(session.working_directory.as_deref()));
                            let provider_model = match (&session.provider_name, &session.model) {
                                (Some(provider), Some(model)) => format!("{provider} / {model}"),
                                (Some(provider), None) => provider.clone(),
                                (None, Some(model)) => model.clone(),
                                (None, None) => "Model unavailable".to_string(),
                            };
                            let usage = if session.total_cost > 0.0 {
                                format!("{} · ${:.2}", format_token_count(session.token_count), session.total_cost)
                            } else {
                                format_token_count(session.token_count)
                            };
                            let activity = session_activity_label(&session.updated_at);
                            rsx! {
                                div { class: if selected { "session-row selected" } else { "session-row" },
                                    button {
                                        class: "session-select",
                                        aria_current: if selected { "page" } else { "false" },
                                        onclick: {
                                            let selected_id = session_id.clone();
                                            let id_for_messages = session_id.clone();
                                            let id_for_state = session_id.clone();
                                            move |_| {
                                                let id_for_messages = id_for_messages.clone();
                                                selected_session.set(Some(selected_id.clone()));
                                                chat_request.set(ChatRequestState::default());
                                                let state = DesktopState {
                                                    route: route.read().as_str().to_string(),
                                                    selected_session_id: Some(id_for_state.clone()),
                                                };
                                                spawn(async move {
                                                    match invoke_unit("save_desktop_state", json!({"state": state})).await {
                                                        Ok(()) => clear_action_error(&mut action_errors, "desktop-state"),
                                                        Err(message) => set_action_error(&mut action_errors, "desktop-state", format!("Failed to save selected session: {message}")),
                                                    }
                                                    match invoke::<Vec<MessageInfo>, _>("get_session_messages", json!({"sessionId": id_for_messages})).await {
                                                        Ok(messages) => {
                                                            session_messages.set(messages);
                                                            clear_action_error(&mut action_errors, "sessions-load");
                                                        }
                                                        Err(message) => set_action_error(&mut action_errors, "sessions-load", format!("Failed to load selected session: {message}")),
                                                    }
                                                });
                                            }
                                        },
                                        div { class: "session-primary",
                                            span { class: "session-title", "{title}" }
                                            span { class: "session-activity", "{activity}" }
                                        }
                                        div { class: "session-secondary",
                                            span { class: "session-project", "{project_or_path}" }
                                            span { class: "session-model", "{provider_model}" }
                                        }
                                        div { class: "session-tertiary", "{usage}" }
                                    }
                                    div { class: "session-actions",
                                        button {
                                            class: "session-action",
                                            title: "Rename session",
                                            aria_label: "Rename {title}",
                                            onclick: {
                                                let id = session_id.clone();
                                                let current_title = title.clone();
                                                move |_| {
                                                    renaming_session.set(Some(id.clone()));
                                                    rename_title.set(current_title.clone());
                                                }
                                            },
                                            "✎"
                                        }
                                        button {
                                            class: "session-action danger",
                                            title: "Delete session",
                                            aria_label: "Delete {title}",
                                            onclick: {
                                                let id = session_id.clone();
                                                move |_| deleting_session.set(Some(id.clone()))
                                            },
                                            "×"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            main { class: "panel-container",
                if let Some(err) = error.read().clone() {
                    div { class: "card", style: "border-color: var(--text-error); margin-bottom: 16px;",
                        h3 { "Frontend bridge error" }
                        p { "{err}" }
                        p { class: "subtle", "Check Usage → Diagnostics for a redacted local support snapshot." }
                    }
                }
                if !background_warnings.read().is_empty() {
                    div { class: "card", style: "margin-bottom: 16px;",
                        h3 { "Some desktop panels are unavailable" }
                        for warning in background_warnings.read().iter() {
                            p { class: "subtle", "{warning}" }
                        }
                    }
                }
                if !action_errors.read().is_empty() {
                    div { class: "card", style: "border-color: var(--text-error); margin-bottom: 16px;",
                        h3 { "Desktop actions need attention" }
                        for item in action_errors.read().iter() {
                            p { class: "subtle", "{item.scope}: {item.message}" }
                        }
                    }
                }

                match *route.read() {
                    RouteId::Chat => rsx! {
                        ChatPanel {
                            title: current_session_title,
                            messages: rendered_messages,
                            composer: composer.read().clone(),
                            is_busy: chat_request.read().active,
                            on_input: move |value| composer.set(value),
                            on_send: move |_| {
                                let maybe_session = selected_session.read().clone();
                                let text = composer.read().trim().to_string();
                                if text.is_empty() {
                                    return;
                                }
                                if let Some(session_id) = maybe_session {
                                    composer.set(String::new());
                                    session_messages.with_mut(|msgs| {
                                        let next_sequence = msgs.last().map(|m| m.sequence + 1).unwrap_or(1);
                                        msgs.push(MessageInfo {
                                            id: format!("local-user-{next_sequence}"),
                                            role: "user".to_string(),
                                            content: text.clone(),
                                            sequence: next_sequence,
                                            token_count: None,
                                            cost: None,
                                            created_at: "now".to_string(),
                                            thinking: None,
                                        });
                                    });
                                    chat_request.set(ChatRequestState { active: true });
                                    clear_action_error(&mut action_errors, "chat-send");
                                    spawn(async move {
                                        let send_result = invoke::<serde_json::Value, _>(
                                            "send_message",
                                            json!({"sessionId": session_id.clone(), "message": text, "model": null}),
                                        )
                                        .await;
                                        match send_result {
                                            Ok(_) => match invoke::<Vec<MessageInfo>, _>(
                                                "get_session_messages",
                                                json!({"sessionId": session_id.clone()}),
                                            )
                                            .await
                                            {
                                                Ok(messages) => {
                                                    session_messages.set(messages);
                                                    clear_action_error(&mut action_errors, "chat-send");
                                                }
                                                Err(message) => set_action_error(
                                                    &mut action_errors,
                                                    "chat-send",
                                                    format!("Message sent, but the transcript could not refresh: {message}"),
                                                ),
                                            },
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "chat-send",
                                                format!("Failed to send chat message: {message}"),
                                            ),
                                        }
                                        chat_request.set(ChatRequestState::default());
                                    });
                                }
                            }
                        }
                    },
                    RouteId::Files => rsx! {
                        FilesPanel {
                            root: workspace_root.read().clone(),
                            current_directory: current_directory.read().clone(),
                            entries: file_entries.read().clone(),
                            selected_path: selected_file.read().clone(),
                            preview: file_content.read().clone(),
                            on_open: move |entry: FileEntry| {
                                let mut file_entries = file_entries;
                                let mut current_directory = current_directory;
                                let mut selected_file = selected_file;
                                let mut file_content = file_content;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    if entry.is_dir {
                                        match invoke::<Vec<FileEntry>, _>("list_directory", json!({"path": entry.path.clone()})).await {
                                            Ok(entries) => {
                                                clear_action_error(&mut action_errors, "files-open");
                                                current_directory.set(entry.path.clone());
                                                selected_file.set(None);
                                                file_content.set(None);
                                                file_entries.set(entries);
                                            }
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "files-open",
                                                format!("Failed to open directory {}: {message}", entry.path),
                                            ),
                                        }
                                    } else {
                                        selected_file.set(Some(entry.path.clone()));
                                        match invoke::<FileContent, _>("read_file_content", json!({"path": entry.path.clone()})).await {
                                            Ok(content) => {
                                                clear_action_error(&mut action_errors, "files-open");
                                                file_content.set(Some(content));
                                            }
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "files-open",
                                                format!("Failed to read file {}: {message}", entry.path),
                                            ),
                                        }
                                    }
                                });
                            },
                            on_up: move |_| {
                                let current = current_directory.read().clone();
                                let root = workspace_root.read().clone();
                                if current == root || current.is_empty() {
                                    return;
                                }
                                let mut file_entries = file_entries;
                                let mut current_directory = current_directory;
                                let mut selected_file = selected_file;
                                let mut file_content = file_content;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    let parent = std::path::Path::new(&current)
                                        .parent()
                                        .map(|path| path.to_string_lossy().to_string())
                                        .unwrap_or(root.clone());
                                    match invoke::<Vec<FileEntry>, _>("list_directory", json!({"path": parent.clone()})).await {
                                        Ok(entries) => {
                                            clear_action_error(&mut action_errors, "files-up");
                                            current_directory.set(parent);
                                            selected_file.set(None);
                                            file_content.set(None);
                                            file_entries.set(entries);
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "files-up",
                                            format!("Failed to open parent directory: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Brain => rsx! {
                        BrainPanel {
                            files: brain_files.read().clone(),
                            active_tab: active_brain_tab.read().clone(),
                            editor: brain_editor.read().clone(),
                            on_select: move |name: String| {
                                if let Some(file) = brain_files.read().iter().find(|file| file.name == name) {
                                    active_brain_tab.set(Some(file.name.clone()));
                                    brain_editor.set(file.content.clone());
                                }
                            },
                            on_edit: move |value| brain_editor.set(value),
                            on_save: move |_| {
                                let active = active_brain_tab.read().clone();
                                let content = brain_editor.read().clone();
                                if let Some(name) = active {
                                    let mut brain_files = brain_files;
                                    let mut action_errors = action_errors;
                                    let mut active_brain_tab = active_brain_tab;
                                    let mut brain_editor = brain_editor;
                                    spawn(async move {
                                        match invoke_unit("write_brain_file", json!({"name": name.clone(), "content": content.clone()})).await {
                                            Ok(()) => {
                                                clear_action_error(&mut action_errors, "brain-save");
                                                match invoke::<Vec<BrainFile>, _>("list_brain_files", json!({})).await {
                                                    Ok(files) => {
                                                        if let Some(file) = files.iter().find(|file| file.name == name) {
                                                            active_brain_tab.set(Some(file.name.clone()));
                                                            brain_editor.set(file.content.clone());
                                                        }
                                                        brain_files.set(files);
                                                    }
                                                    Err(message) => set_action_error(
                                                        &mut action_errors,
                                                        "brain-save",
                                                        format!("Brain file saved, but refresh failed: {message}"),
                                                    ),
                                                }
                                            }
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "brain-save",
                                                format!("Failed to save brain file {name}: {message}"),
                                            ),
                                        }
                                    });
                                }
                            }
                        }
                    },
                    RouteId::Providers => rsx! {
                        ProvidersPanel {
                            config: config_info.read().clone(),
                            on_select_model: move |(provider_name, model): (String, String)| {
                                let mut config_info = config_info;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit(
                                        "select_model",
                                        json!({"providerName": provider_name.clone(), "model": model.clone()}),
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            match invoke::<ConfigInfo, _>("get_config", json!({})).await {
                                                Ok(cfg) => {
                                                    config_info.set(Some(cfg));
                                                    clear_action_error(&mut action_errors, "providers-select");
                                                }
                                                Err(message) => set_action_error(
                                                    &mut action_errors,
                                                    "providers-select",
                                                    format!("Model selected, but provider refresh failed: {message}"),
                                                ),
                                            }
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "providers-select",
                                            format!("Failed to select model for {provider_name}: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Tools => rsx! {
                        ToolsPanel {
                            tools: tools.read().clone(),
                            selected: selected_tool.read().clone(),
                            on_open: move |tool_name: String| {
                                let mut selected_tool = selected_tool;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke::<ToolDetail, _>("get_tool_details", json!({"toolName": tool_name.clone()})).await {
                                        Ok(detail) => {
                                            clear_action_error(&mut action_errors, "tools-open");
                                            selected_tool.set(Some(detail));
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "tools-open",
                                            format!("Tool details failed for {tool_name}: {message}"),
                                        ),
                                    }
                                });
                            },
                            on_approve: move |tool_name: String| {
                                let session_id = selected_session.read().clone().unwrap_or_else(|| "desktop".to_string());
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit(
                                        "approve_tool",
                                        json!({
                                            "sessionId": session_id,
                                            "toolName": tool_name.clone(),
                                            "approved": true,
                                            "alwaysApprove": false
                                        }),
                                    )
                                    .await
                                    {
                                        Ok(()) => clear_action_error(&mut action_errors, "tools-approve"),
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "tools-approve",
                                            format!("Failed to update approval for {tool_name}: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Skills => rsx! {
                        SkillsPanel {
                            skills: skills.read().clone(),
                            selected: selected_skill.read().clone(),
                            on_open: move |name: String| {
                                let mut selected_skill = selected_skill;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke::<SkillDetail, _>("get_skill_details", json!({"name": name.clone()})).await {
                                        Ok(detail) => {
                                            clear_action_error(&mut action_errors, "skills-open");
                                            selected_skill.set(Some(detail));
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "skills-open",
                                            format!("Skill details failed for {name}: {message}"),
                                        ),
                                    }
                                });
                            },
                            on_toggle: move |(name, enabled): (String, bool)| {
                                let mut skills = skills;
                                let mut selected_skill = selected_skill;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit("toggle_skill", json!({"name": name.clone(), "enabled": enabled})).await {
                                        Ok(()) => {
                                            match invoke::<Vec<SkillInfo>, _>("list_skills", json!({})).await {
                                                Ok(list) => skills.set(list),
                                                Err(message) => {
                                                    set_action_error(
                                                        &mut action_errors,
                                                        "skills-toggle",
                                                        format!("Skill changed, but list refresh failed: {message}"),
                                                    );
                                                    return;
                                                }
                                            }
                                            match invoke::<SkillDetail, _>("get_skill_details", json!({"name": name.clone()})).await {
                                                Ok(detail) => {
                                                    selected_skill.set(Some(detail));
                                                    clear_action_error(&mut action_errors, "skills-toggle");
                                                }
                                                Err(message) => set_action_error(
                                                    &mut action_errors,
                                                    "skills-toggle",
                                                    format!("Skill changed, but detail refresh failed: {message}"),
                                                ),
                                            }
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "skills-toggle",
                                            format!("Failed to toggle skill {name}: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Cron => rsx! {
                        CronPanel {
                            jobs: cron_jobs.read().clone(),
                            runs: cron_runs.read().clone(),
                            on_toggle: move |(job_id, enabled): (String, bool)| {
                                let mut cron_jobs = cron_jobs;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit("toggle_cron_job", json!({"jobId": job_id.clone(), "enabled": enabled})).await {
                                        Ok(()) => match invoke::<Vec<CronJobInfo>, _>("list_cron_jobs", json!({})).await {
                                            Ok(list) => { cron_jobs.set(list); clear_action_error(&mut action_errors, "cron-toggle"); }
                                            Err(message) => set_action_error(&mut action_errors, "cron-toggle", format!("Cron changed, but refresh failed: {message}")),
                                        },
                                        Err(message) => set_action_error(&mut action_errors, "cron-toggle", format!("Failed to toggle cron job {job_id}: {message}")),
                                    }
                                });
                            },
                            on_run_now: move |job_id: String| {
                                let mut cron_runs = cron_runs;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit("trigger_cron_job", json!({"jobId": job_id.clone()})).await {
                                        Ok(()) => match invoke::<Vec<CronJobRunInfo>, _>("list_cron_runs", json!({"jobId": job_id})).await {
                                            Ok(runs) => { cron_runs.set(runs); clear_action_error(&mut action_errors, "cron-run"); }
                                            Err(message) => set_action_error(&mut action_errors, "cron-run", format!("Cron triggered, but history refresh failed: {message}")),
                                        },
                                        Err(message) => set_action_error(&mut action_errors, "cron-run", format!("Failed to trigger cron job {job_id}: {message}")),
                                    }
                                });
                            },
                            on_show_runs: move |job_id: String| {
                                let mut cron_runs = cron_runs;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke::<Vec<CronJobRunInfo>, _>("list_cron_runs", json!({"jobId": job_id.clone()})).await {
                                        Ok(runs) => { cron_runs.set(runs); clear_action_error(&mut action_errors, "cron-runs"); }
                                        Err(message) => set_action_error(&mut action_errors, "cron-runs", format!("Failed to load runs for {job_id}: {message}")),
                                    }
                                });
                            },
                            on_delete: move |job_id: String| {
                                let mut cron_jobs = cron_jobs;
                                let mut cron_runs = cron_runs;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit("delete_cron_job", json!({"jobId": job_id.clone()})).await {
                                        Ok(()) => match invoke::<Vec<CronJobInfo>, _>("list_cron_jobs", json!({})).await {
                                            Ok(list) => { cron_jobs.set(list); cron_runs.set(Vec::new()); clear_action_error(&mut action_errors, "cron-delete"); }
                                            Err(message) => set_action_error(&mut action_errors, "cron-delete", format!("Cron job deleted, but refresh failed: {message}")),
                                        },
                                        Err(message) => set_action_error(&mut action_errors, "cron-delete", format!("Failed to delete cron job {job_id}: {message}")),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Channels => rsx! {
                        ChannelsPanel {
                            channels: channels.read().clone(),
                            on_toggle: move |(name, enabled): (String, bool)| {
                                let mut channels = channels;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke_unit("toggle_channel", json!({"name": name.clone(), "enabled": enabled})).await {
                                        Ok(()) => match invoke::<Vec<ChannelStatus>, _>("get_channel_statuses", json!({})).await {
                                            Ok(list) => {
                                                channels.set(list);
                                                clear_action_error(&mut action_errors, "channels-toggle");
                                            }
                                            Err(message) => set_action_error(
                                                &mut action_errors,
                                                "channels-toggle",
                                                format!("Channel changed, but status refresh failed: {message}"),
                                            ),
                                        },
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "channels-toggle",
                                            format!("Failed to toggle channel {name}: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                    },
                    RouteId::Usage => rsx! {
                        UsagePanel {
                            data: usage.read().clone(),
                            on_refresh: move |_| {
                                let mut usage = usage;
                                let mut action_errors = action_errors;
                                spawn(async move {
                                    match invoke::<DashboardDataInfo, _>(
                                        "get_usage_data",
                                        json!({"period": "week"}),
                                    )
                                    .await
                                    {
                                        Ok(value) => {
                                            usage.set(Some(value));
                                            clear_action_error(&mut action_errors, "usage-refresh");
                                        }
                                        Err(message) => set_action_error(
                                            &mut action_errors,
                                            "usage-refresh",
                                            format!("Failed to refresh usage: {message}"),
                                        ),
                                    }
                                });
                            }
                        }
                        DiagnosticsPanel {}
                    },
                }
            }
            if let Some(session_id) = renaming_session.read().clone() {
                div { class: "modal-overlay", role: "dialog", aria_modal: "true", aria_label: "Rename session",
                    div { class: "modal",
                        h2 { "Rename session" }
                        input {
                            class: "rename-input",
                            value: "{rename_title.read()}",
                            autofocus: true,
                            oninput: move |event| rename_title.set(event.value()),
                        }
                        div { class: "form-actions",
                            button { class: "btn-secondary", onclick: move |_| renaming_session.set(None), "Cancel" }
                            button {
                                class: "btn-primary",
                                onclick: {
                                    let session_id = session_id.clone();
                                    move |_| {
                                        let session_id = session_id.clone();
                                        let title = rename_title.read().trim().to_string();
                                        if title.is_empty() {
                                            set_action_error(&mut action_errors, "sessions-rename", "Session title cannot be empty.".to_string());
                                            return;
                                        }
                                        spawn(async move {
                                            match invoke_unit("rename_session", json!({"sessionId": session_id, "title": title})).await {
                                                Ok(()) => match invoke::<Vec<SessionInfo>, _>("list_sessions", json!({})).await {
                                                    Ok(list) => {
                                                        sessions.set(list);
                                                        renaming_session.set(None);
                                                        clear_action_error(&mut action_errors, "sessions-rename");
                                                    }
                                                    Err(message) => set_action_error(&mut action_errors, "sessions-rename", format!("Session renamed, but refresh failed: {message}")),
                                                },
                                                Err(message) => set_action_error(&mut action_errors, "sessions-rename", format!("Failed to rename session: {message}")),
                                            }
                                        });
                                    }
                                },
                                "Save"
                            }
                        }
                    }
                }
            }
            if let Some(session_id) = deleting_session.read().clone() {
                div { class: "modal-overlay", role: "alertdialog", aria_modal: "true", aria_label: "Delete session",
                    div { class: "modal",
                        h2 { "Delete this session?" }
                        p { class: "subtle", "This permanently removes \"{delete_target_title.clone().unwrap_or_else(|| \"this session\".to_string())}\" and its chat history." }
                        div { class: "form-actions",
                            button { class: "btn-secondary", onclick: move |_| deleting_session.set(None), "Cancel" }
                            button {
                                class: "btn-small danger",
                                onclick: {
                                    let session_id = session_id.clone();
                                    move |_| {
                                        let session_id = session_id.clone();
                                        spawn(async move {
                                            match invoke_unit("delete_session", json!({"sessionId": session_id.clone()})).await {
                                                Ok(()) => {
                                                    let next_selected = if selected_session.read().as_deref() == Some(session_id.as_str()) {
                                                        None
                                                    } else {
                                                        selected_session.read().clone()
                                                    };
                                                    match invoke::<Vec<SessionInfo>, _>("list_sessions", json!({})).await {
                                                        Ok(list) => {
                                                            let resolved = next_selected.filter(|id| list.iter().any(|session| &session.id == id)).or_else(|| list.first().map(|session| session.id.clone()));
                                                            sessions.set(list);
                                                            selected_session.set(resolved.clone());
                                                            deleting_session.set(None);
                                                            clear_action_error(&mut action_errors, "sessions-delete");
                                                            if let Some(id) = resolved {
                                                                match invoke::<Vec<MessageInfo>, _>("get_session_messages", json!({"sessionId": id})).await {
                                                                    Ok(messages) => session_messages.set(messages),
                                                                    Err(message) => set_action_error(&mut action_errors, "sessions-delete", format!("Session deleted, but replacement messages failed to load: {message}")),
                                                                }
                                                            } else {
                                                                session_messages.set(Vec::new());
                                                            }
                                                        }
                                                        Err(message) => set_action_error(&mut action_errors, "sessions-delete", format!("Session deleted, but refresh failed: {message}")),
                                                    }
                                                }
                                                Err(message) => set_action_error(&mut action_errors, "sessions-delete", format!("Failed to delete session: {message}")),
                                            }
                                        });
                                    }
                                },
                                "Delete permanently"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MessageTraceItem(item: ChatDisplayItem) -> Element {
    match item {
        ChatDisplayItem::Text { content, reasoning } => rsx! {
            if let Some(reasoning) = reasoning {
                details { class: "trace-disclosure",
                    summary { class: "trace-summary",
                        span { class: "trace-icon", "◌" }
                        span { "Reasoning trace" }
                        span { class: "trace-hint", "Show details" }
                    }
                    pre { class: "trace-content", "{reasoning}" }
                }
            }
            if !content.is_empty() {
                div { class: "msg-text", "{content}" }
            }
        },
        ChatDisplayItem::ProtocolFallback { label, content } => rsx! {
            details { class: "tool-disclosure protocol-fallback",
                summary { class: "tool-summary",
                    span { class: "tool-summary-icon", "!" }
                    span { "{label}" }
                    span { class: "trace-hint", "Show raw data" }
                }
                pre { class: "tool-detail", "{content}" }
            }
        },
        ChatDisplayItem::Tools(calls) => {
            let count = calls.len();
            let label = if count == 1 { "call" } else { "calls" };
            rsx! {
                details { class: "tool-disclosure",
                    summary { class: "tool-summary",
                        span { class: "tool-summary-icon", "⌘" }
                        span { "{count} tool {label}" }
                        span { class: "trace-hint", "Show activity" }
                    }
                    div { class: "tool-call-list",
                        for call in calls {
                            details { class: if call.success { "tool-call-row success" } else { "tool-call-row failed" },
                                summary {
                                    span { class: "tool-status", if call.success { "✓" } else { "×" } }
                                    span { class: "tool-description", "{call.description}" }
                                    span { class: "tool-state", if call.success { "Complete" } else { "Failed" } }
                                }
                                if call.input != serde_json::Value::Null {
                                    div { class: "tool-detail-label", "Input" }
                                    pre { class: "tool-detail", "{call.input}" }
                                }
                                if let Some(output) = call.output {
                                    div { class: "tool-detail-label", "Output" }
                                    pre { class: "tool-detail", "{output}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
#[component]
fn ChatPanel(
    title: String,
    messages: Vec<MessageInfo>,
    composer: String,
    is_busy: bool,
    on_input: EventHandler<String>,
    on_send: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "chat-panel",
            div { class: "chat-header",
                div { class: "chat-title", "{title}" }
                div { class: "card-actions",
                    span { class: "badge", if is_busy { "waiting" } else { "idle" } }
                }
            }
            div { class: "message-list",
                if messages.is_empty() {
                    div { class: "empty-state",
                        h3 { "No messages in this session yet" }
                        p { "Send a message below to start the conversation. Chat uses a single completed request/response cycle per turn." }
                    }
                }
                for message in messages.iter() {
                    div {
                        class: if message.role == "user" { "message-bubble user" } else { "message-bubble assistant" },
                        div { class: "msg-avatar", if message.role == "user" { "U" } else { "A" } }
                        div {
                            for item in display_message_items(&message.content, message.thinking.as_deref()) {
                                MessageTraceItem { item }
                            }
                            div { class: "msg-meta", "{message.role} · {message.created_at}" }
                        }
                    }
                }
            }
            div { class: "input-bar",
                textarea {
                    class: "chat-input",
                    value: "{composer}",
                    placeholder: "Talk to OpenCrabs…",
                    oninput: move |evt| on_input.call(evt.value()),
                }
                button { class: "btn-send", disabled: is_busy, onclick: move |_| on_send.call(()), "➤" }
            }
        }
    }
}

#[component]
fn FilesPanel(
    root: String,
    current_directory: String,
    entries: Vec<FileEntry>,
    selected_path: Option<String>,
    preview: Option<FileContent>,
    on_open: EventHandler<FileEntry>,
    on_up: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "files-panel",
            div { class: "panel-header",
                h2 { "Workspace files" }
                code { "{root}" }
            }
            div { class: "card", style: "margin-bottom: 16px;",
                p { class: "subtle", "Workspace browsing is constrained to the active desktop workspace root." }
                p { class: "subtle", "Current directory: {current_directory}" }
                button { class: "btn-small", disabled: current_directory == root || current_directory.is_empty(), onclick: move |_| on_up.call(()), "Up one directory" }
            }
            div { class: "files-layout",
                div { class: "file-list card",
                    h3 { "Entries" }
                    for entry in entries.iter() {
                        button {
                            class: if selected_path.as_ref() == Some(&entry.path) { "session-card active" } else { "session-card" },
                            onclick: {
                                let entry = entry.clone();
                                move |_| on_open.call(entry.clone())
                            },
                            div { class: "session-title", "{entry.name}" }
                            if entry.is_dir {
                                div { class: "msg-meta", "directory" }
                            } else if let Some(size) = entry.size {
                                div { class: "msg-meta", "{size} bytes" }
                            }
                        }
                    }
                }
                div { class: "file-preview",
                    if let Some(path) = selected_path {
                        h3 { "{path}" }
                    }
                    if let Some(content) = preview {
                        pre { class: "code-block", "{content.content}" }
                    } else {
                        p { "Pick a file to preview." }
                    }
                }
            }
        }
    }
}

#[component]
fn BrainPanel(
    files: Vec<BrainFile>,
    active_tab: Option<String>,
    editor: String,
    on_select: EventHandler<String>,
    on_edit: EventHandler<String>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "editor-layout",
            div { class: "panel-header",
                h2 { "Brain files" }
                button { class: "btn-primary", onclick: move |_| on_save.call(()), "Save" }
            }
            div { class: "card", style: "margin-bottom: 16px;",
                p { class: "subtle", "Protected brain files keep ownership headers and reject empty writes." }
            }
            div { class: "editor-tabs",
                for file in files.iter() {
                    button {
                        class: if active_tab.as_ref() == Some(&file.name) { "editor-tab active" } else { "editor-tab" },
                        onclick: {
                            let name = file.name.clone();
                            move |_| on_select.call(name.clone())
                        },
                        "{file.name}"
                    }
                }
            }
            textarea {
                class: "brain-textarea",
                value: "{editor}",
                oninput: move |evt| on_edit.call(evt.value()),
            }
        }
    }
}

#[component]
fn ProvidersPanel(
    config: Option<ConfigInfo>,
    on_select_model: EventHandler<(String, String)>,
) -> Element {
    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Providers" }
                if let Some(cfg) = &config {
                    div { class: "badge key-ok", "Auto approve: {cfg.agent_auto_approve}" }
                }
            }
            if let Some(cfg) = config {
                div { class: "card-grid",
                    for provider in cfg.providers.iter() {
                        div { class: "card",
                            div { class: "card-body",
                                h3 { "{provider.name}" }
                                div { class: "card-tags",
                                    span { class: if provider.enabled { "badge enabled" } else { "badge disabled" }, if provider.enabled { "enabled" } else { "disabled" } }
                                    span { class: if provider.has_api_key { "badge key-ok" } else { "badge" }, if provider.has_api_key { "key present" } else { "no key" } }
                                }
                                if let Some(model) = provider.default_model.as_deref() {
                                    p { class: "subtle", "Default model: {model}" }
                                }
                                if !provider.models.is_empty() {
                                    div { class: "card-tags",
                                        for model in provider.models.iter().take(3) {
                                            button {
                                                class: "btn-small",
                                                onclick: {
                                                    let provider_name = provider.name.clone();
                                                    let model_name = model.clone();
                                                    move |_| on_select_model.call((provider_name.clone(), model_name.clone()))
                                                },
                                                "Use {model}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                p { "No provider data yet." }
            }
        }
    }
}

#[component]
fn ToolsPanel(
    tools: Vec<ToolInfo>,
    selected: Option<ToolDetail>,
    on_open: EventHandler<String>,
    on_approve: EventHandler<String>,
) -> Element {
    let mut pending_approve = use_signal(|| false);
    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Tools" }
                span { class: "badge", "{tools.len()} loaded" }
            }
            div { class: "card", style: "margin-bottom: 16px;",
                p { class: "subtle", "Tool approval sets the global agent approval policy (manual, on-request, or auto-always). The desktop preview does not yet store per-tool or per-session approval, and does not mirror the TUI's inline approval event flow." }
            }
            div { class: "card-grid",
                for tool in tools.iter() {
                    button {
                        class: "card",
                        onclick: {
                            let name = tool.name.clone();
                            move |_| on_open.call(name.clone())
                        },
                        div { class: "card-body",
                            h3 { "{tool.name}" }
                            p { "{tool.description}" }
                            div { class: "card-tags",
                                for cap in tool.capabilities.iter() {
                                    span { class: "tag", "{cap}" }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(detail) = selected {
                div { class: "modal",
                    h2 { "{detail.name}" }
                    p { "{detail.description}" }
                    button {
                        class: "btn-primary",
                        onclick: move |_| pending_approve.set(true),
                        "Approve — sets global policy…"
                    }
                    if *pending_approve.read() {
                        p { class: "subtle", "This changes the global agent approval policy for every session, not just this tool. Confirm to continue." }
                        div { class: "card-actions",
                            button {
                                class: "btn-primary",
                                onclick: {
                                    let tool_name = detail.name.clone();
                                    move |_| {
                                        pending_approve.set(false);
                                        on_approve.call(tool_name.clone());
                                    }
                                },
                                "Confirm — apply global policy"
                            }
                            button {
                                class: "btn-secondary",
                                onclick: move |_| pending_approve.set(false),
                                "Cancel"
                            }
                        }
                    }
                    pre { class: "code-block", "{detail.parameters}" }
                }
            }
        }
    }
}

#[component]
fn SkillsPanel(
    skills: Vec<SkillInfo>,
    selected: Option<SkillDetail>,
    on_open: EventHandler<String>,
    on_toggle: EventHandler<(String, bool)>,
) -> Element {
    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Skills" }
                span { class: "badge", "{skills.len()} discovered" }
            }
            div { class: "card-grid",
                for skill in skills.iter() {
                    div { class: "card",
                        div { class: "card-body",
                            button {
                                class: "btn-small",
                                onclick: {
                                    let name = skill.name.clone();
                                    move |_| on_open.call(name.clone())
                                },
                                "Open"
                            }
                            h3 { "{skill.name}" }
                            p { "{skill.description}" }
                            div { class: "card-tags",
                                span { class: "badge", "{skill.source}" }
                                span { class: if skill.enabled { "badge enabled" } else { "badge disabled" }, if skill.enabled { "enabled" } else { "disabled" } }
                                if skill.review_gate {
                                    span { class: "badge review-gate", "review gate" }
                                }
                            }
                            button {
                                class: "btn-small",
                                onclick: {
                                    let name = skill.name.clone();
                                    let next = !skill.enabled;
                                    move |_| on_toggle.call((name.clone(), next))
                                },
                                if skill.enabled { "Disable" } else { "Enable" }
                            }
                        }
                    }
                }
            }
            if let Some(detail) = selected {
                div { class: "modal",
                    h2 { "{detail.name}" }
                    p { "{detail.description}" }
                    p { class: "subtle", "Enabled: {detail.enabled}" }
                    pre { class: "code-block", "{detail.body}" }
                }
            }
        }
    }
}

#[component]
fn CronPanel(
    jobs: Vec<CronJobInfo>,
    runs: Vec<CronJobRunInfo>,
    on_toggle: EventHandler<(String, bool)>,
    on_run_now: EventHandler<String>,
    on_show_runs: EventHandler<String>,
    on_delete: EventHandler<String>,
) -> Element {
    let mut pending_delete = use_signal(|| None::<String>);

    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Cron jobs" }
                span { class: "badge", "{jobs.len()} scheduled" }
            }
            div { class: "card", style: "margin-bottom: 16px;",
                p { class: "subtle", "Run history is read-only. Deleting a job permanently removes its schedule." }
            }
            if jobs.is_empty() {
                div { class: "empty-state",
                    h3 { "No scheduled jobs" }
                    p { "Schedules created through OpenCrabs will appear here with their run history." }
                }
            } else {
                div { class: "table-container",
                    table { class: "data-table",
                        thead { tr { th { "Name" } th { "Schedule" } th { "Provider" } th { "Enabled" } th { "Actions" } } }
                        tbody {
                            for job in jobs.iter() {
                                tr {
                                    td { "{job.name}" }
                                    td { "{job.cron_expr}" }
                                    td { "{job.provider.as_deref().unwrap_or(\"default\")}" }
                                    td { if job.enabled { "yes" } else { "no" } }
                                    td {
                                        button { class: "btn-small", onclick: { let id = job.id.clone(); let next = !job.enabled; move |_| on_toggle.call((id.clone(), next)) }, if job.enabled { "Disable" } else { "Enable" } }
                                        button { class: "btn-small", onclick: { let id = job.id.clone(); move |_| on_run_now.call(id.clone()) }, "Run now" }
                                        button { class: "btn-small", onclick: { let id = job.id.clone(); move |_| on_show_runs.call(id.clone()) }, "History" }
                                        if pending_delete.read().as_deref() == Some(job.id.as_str()) {
                                            button { class: "btn-small danger", onclick: { let id = job.id.clone(); move |_| { on_delete.call(id.clone()); pending_delete.set(None); } }, "Confirm delete" }
                                            button { class: "btn-small", onclick: move |_| pending_delete.set(None), "Cancel" }
                                        } else {
                                            button { class: "btn-small danger", onclick: { let id = job.id.clone(); move |_| pending_delete.set(Some(id.clone())) }, "Delete…" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !runs.is_empty() {
                div { class: "card", style: "margin-top: 16px;",
                    h3 { "Recent runs" }
                    div { class: "table-container",
                        table { class: "data-table",
                            thead { tr { th { "Status" } th { "Started" } th { "Tokens" } th { "Error" } } }
                            tbody {
                                for run in runs.iter() {
                                    tr {
                                        td { "{run.status}" }
                                        td { "{run.started_at}" }
                                        td { "{run.input_tokens + run.output_tokens}" }
                                        td { "{run.error.as_deref().unwrap_or(\"—\")}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ChannelsPanel(channels: Vec<ChannelStatus>, on_toggle: EventHandler<(String, bool)>) -> Element {
    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Channels" }
                span { class: "badge", "{channels.len()} integrations" }
            }
            div { class: "card", style: "margin-bottom: 16px;",
                p { class: "subtle", "Channel status reports configuration and credential readiness only. It does not prove a live socket or external service connection." }
            }
            div { class: "card-grid",
                for channel in channels.iter() {
                    div { class: "card",
                        div { class: "card-body",
                            h3 { "{channel.display_name}" }
                            div { class: "card-actions",
                                span {
                                    class: if channel.enabled { "badge enabled" } else { "badge disabled" },
                                    if channel.enabled { "enabled" } else { "disabled" }
                                }
                                span {
                                    class: if channel.alive { "badge key-ok" } else { "badge" },
                                    if channel.alive { "credentials present" } else { "not ready" }
                                }
                                button {
                                    class: "btn-small",
                                    onclick: {
                                        let name = channel.name.clone();
                                        let enabled = !channel.enabled;
                                        move |_| on_toggle.call((name.clone(), enabled))
                                    },
                                    if channel.enabled { "Disable" } else { "Enable" }
                                }
                            }
                            if let Some(err) = &channel.error {
                                p { class: "subtle", "{err}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DiagnosticsPanel() -> Element {
    let mut snapshot = use_signal(|| None::<DiagnosticsSnapshot>);
    let mut error = use_signal(|| None::<String>);

    let load = move |_| {
        spawn(async move {
            error.set(None);
            match invoke::<DiagnosticsSnapshot, _>("get_diagnostics", json!({})).await {
                Ok(value) => snapshot.set(Some(value)),
                Err(message) => error.set(Some(message)),
            }
        });
    };

    rsx! {
        div { class: "card", style: "margin-top: 16px;",
            div { class: "panel-header",
                h3 { "Diagnostics" }
                button { class: "btn-secondary", onclick: load, "Refresh diagnostics" }
            }
            p { class: "subtle", "Read-only support snapshot. Credential-bearing log lines are redacted." }
            if let Some(message) = error.read().clone() {
                p { class: "error", "Diagnostics unavailable: {message}" }
            }
            if let Some(value) = snapshot.read().clone() {
                p { class: "subtle", "App {value.app_version} · config: {value.config_present} · database: {value.database_present}" }
                p { class: "subtle", "Log: {value.log_path}" }
                for note in value.notes {
                    p { class: "subtle", "{note}" }
                }
                if value.log_tail.is_empty() {
                    p { class: "subtle", "No safe log entries available." }
                } else {
                    pre { class: "file-preview", "{value.log_tail.join(\"\\n\")}" }
                }
            } else {
                p { class: "subtle", "Refresh to collect a safe local diagnostic snapshot." }
            }
        }
    }
}

#[component]
fn UsagePanel(data: Option<DashboardDataInfo>, on_refresh: EventHandler<()>) -> Element {
    rsx! {
        div {
            div { class: "panel-header",
                h2 { "Usage" }
                span { class: "badge", "week" }
                button { class: "btn-secondary", onclick: move |_| on_refresh.call(()), "Refresh" }
            }
            if let Some(data) = data {
                if data.projects.is_empty() && data.summary.total_tokens == 0 {
                    div { class: "empty-state",
                        h3 { "No usage recorded yet" }
                        p { "Token, cost, and session totals will appear here once the agent has been used." }
                    }
                } else {
                    div { class: "summary-grid",
                        SummaryCard { title: "Tokens", value: data.summary.total_tokens.to_string() }
                        SummaryCard { title: "Cost", value: format!("${:.2}", data.summary.total_cost) }
                        SummaryCard { title: "Sessions", value: data.summary.session_count.to_string() }
                        SummaryCard { title: "Calls", value: data.summary.call_count.to_string() }
                    }
                    div { class: "table-container",
                        table { class: "data-table",
                            thead {
                                tr {
                                    th { "Project" }
                                    th { "Tokens" }
                                    th { "Cost" }
                                }
                            }
                            tbody {
                                for project in data.projects.iter() {
                                    tr {
                                        td { "{project.project}" }
                                        td { class: "num", "{project.tokens}" }
                                        td { class: "num", "${project.cost}" }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "empty-state",
                    h3 { "Usage summary is loading or unavailable" }
                    p { "If a load error was reported above, use Refresh to try again." }
                    button { class: "btn-primary", onclick: move |_| on_refresh.call(()), "Retry load" }
                }
            }
        }
    }
}

#[cfg(test)]
mod session_history_tests {
    use super::{compact_path, format_token_count, session_matches};
    use opencrabs_desktop_ui::models::SessionInfo;

    fn session() -> SessionInfo {
        SessionInfo {
            id: "session-1".to_string(),
            title: "Polish desktop history".to_string(),
            model: Some("gpt-5.4".to_string()),
            provider_name: Some("surplus".to_string()),
            working_directory: Some("/Users/moe/Desktop/crabz".to_string()),
            token_count: 12_500,
            total_cost: 0.18,
            created_at: "2026-07-25T12:00:00Z".to_string(),
            updated_at: "2026-07-25T12:01:00Z".to_string(),
            is_archived: false,
            project_id: Some("project-1".to_string()),
            project_name: Some("Crabz Desktop".to_string()),
        }
    }

    #[test]
    fn session_search_matches_structured_metadata() {
        let session = session();
        assert!(session_matches(&session, "crabz"));
        assert!(session_matches(&session, "surplus"));
        assert!(session_matches(&session, "gpt-5.4"));
        assert!(!session_matches(&session, "unrelated"));
    }

    #[test]
    fn compact_session_metadata_stays_readable() {
        assert_eq!(format_token_count(12_500), "12.5k");
        assert_eq!(
            compact_path(Some("/Users/moe/Desktop/crabz")),
            "…/Desktop/crabz"
        );
        assert_eq!(compact_path(None), "No workspace");
    }
}
#[component]
fn SummaryCard(title: String, value: String) -> Element {
    rsx! {
        div { class: "stat-card",
            h4 { "{title}" }
            div { class: "stat-value", "{value}" }
        }
    }
}
