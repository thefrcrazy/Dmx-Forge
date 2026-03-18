use std::{collections::HashSet, sync::Arc};

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use url::{Url, form_urlencoded};
use uuid::Uuid;

use crate::{
    AppError, AppState,
    auth::{self, AuthSession, CurrentUser},
    db,
    discord::{PreviewRequest, PreviewResponse, validate_webhook_url},
    webhook,
};

const LOGIN_RATE_LIMIT_WINDOW_SECONDS: i64 = 15 * 60;
const LOGIN_RATE_LIMIT_MAX_ATTEMPTS: usize = 5;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/dashboard", get(dashboard))
        .route("/sources", get(sources).post(create_source))
        .route("/sources/{id}/update", post(update_source))
        .route("/sources/{id}/delete", post(delete_source))
        .route("/sources/{id}/toggle", post(toggle_source))
        .route("/sources/{id}/regenerate", post(regenerate_source_token))
        .route("/destinations", get(destinations).post(create_destination))
        .route("/destinations/{id}/update", post(update_destination))
        .route("/destinations/{id}/delete", post(delete_destination))
        .route("/destinations/{id}/toggle", post(toggle_destination))
        .route(
            "/templates",
            get(message_templates_page).post(create_message_template),
        )
        .route("/templates/{id}/update", post(update_message_template))
        .route("/templates/{id}/delete", post(delete_message_template))
        .route("/templates/{id}/toggle", post(toggle_message_template))
        .route("/rules", get(rules).post(create_rule))
        .route("/rules/test", post(test_rule_webhook))
        .route("/rules/{id}/update", post(update_rule))
        .route("/rules/{id}/delete", post(delete_rule))
        .route("/rules/{id}/toggle", post(toggle_rule))
        .route("/deliveries", get(deliveries))
        .route("/deliveries/{id}", get(delivery_detail))
        .route("/deliveries/{id}/replay", post(replay_delivery))
        .route("/users", get(users).post(create_user))
        .route("/users/{id}/update", post(update_user))
        .route("/users/{id}/delete", post(delete_user))
        .route("/users/{id}/toggle", post(toggle_user))
        .route("/settings", get(settings_redirect))
        .route("/login", get(login).post(login_post))
        .route("/setup", get(setup).post(setup_post))
        .route("/logout", post(logout))
        .route("/health", get(health))
        .route("/api/preview", post(preview))
        .nest_service("/static", ServeDir::new(state.config.static_dir()))
        .with_state(state)
}

async fn root() -> Redirect {
    Redirect::temporary("/dashboard")
}

async fn settings_redirect() -> Redirect {
    Redirect::temporary("/dashboard")
}

async fn dashboard(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
) -> Result<impl IntoResponse, AppError> {
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let visible_source_ids =
        visible_source_ids_for_user(&state, &session.current_user, &scope_ids).await?;
    let snapshot = db::fetch_dashboard_snapshot(&state.db, visible_source_ids.as_deref()).await?;
    let activity_total_count = snapshot.activity.iter().map(|point| point.count).sum();
    let (activity_peak_count, activity_peak_label) = snapshot
        .activity
        .iter()
        .max_by_key(|point| point.count)
        .map(|point| (point.count, point.day_label.clone()))
        .unwrap_or_else(|| (0, "Aucune activite".to_string()));

    Ok(HtmlTemplate(DashboardTemplate {
        shell: shell(
            &state,
            "dashboard",
            "Command center",
            "Operational dashboard",
            "Premiere brique SSR pour superviser les livraisons et valider le rendu des templates.",
            session.current_user.clone(),
            FlashView::empty(),
        ),
        total_deliveries: snapshot.total_deliveries,
        processed_deliveries: snapshot.processed_deliveries,
        failed_deliveries: snapshot.failed_deliveries,
        discord_messages_sent: snapshot.discord_messages_sent,
        activity: snapshot.activity,
        top_repositories: empty_metric_fallback(snapshot.top_repositories, "Aucune activite"),
        top_events: empty_metric_fallback(snapshot.top_events, "Aucun evenement"),
        recent_deliveries: snapshot.recent_deliveries,
        activity_total_count,
        activity_peak_count,
        activity_peak_label,
    }))
}

async fn sources(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_sources(),
        "sources access is not allowed for this account",
    )?;
    let request_base_url = request_base_url(&headers);
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_sources = db::list_sources(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let filtered = filter_sources(&all_sources, &query, &session.current_user, &scope_ids);
    let edit_source = match query.edit.as_deref() {
        Some(id) => db::find_source_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };
    let delete_source = match query.delete.as_deref() {
        Some(id) => db::find_source_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };
    let source_rows = filtered
        .into_iter()
        .map(|item| source_row_view(&request_base_url, item))
        .collect::<Vec<_>>();

    Ok(HtmlTemplate(SourcesTemplate {
        shell: shell(
            &state,
            "sources",
            "Ingestion Git",
            "Sources webhook",
            "Configuration des providers, tokens, secrets et filtres d'entree.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        filters: SourceFilterView {
            q: query.q.clone().unwrap_or_default(),
            provider: query.provider.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
        },
        total_count: all_sources.len(),
        active_count: all_sources
            .iter()
            .filter(|item| item.is_active == 1)
            .count(),
        inactive_count: all_sources
            .iter()
            .filter(|item| item.is_active == 0)
            .count(),
        sources: source_rows,
        show_create_modal: query.modal.as_deref() == Some("create"),
        create_form: default_source_form(),
        show_edit_modal: edit_source.is_some(),
        edit_form: edit_source
            .as_ref()
            .map(|item| source_form_from_record(&request_base_url, item))
            .unwrap_or_else(default_source_form),
        show_delete_modal: delete_source.is_some(),
        delete_target_name: delete_source
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        delete_target_action: delete_source
            .as_ref()
            .map(|item| format!("/sources/{}/delete", item.id))
            .unwrap_or_default(),
    }))
}

async fn create_source(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Form(form): Form<SourceFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_sources(),
        "source creation is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/sources",
            "modal",
            "create",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let input = match build_source_payload(form) {
        Ok(input) => input,
        Err(message) => {
            return Ok(redirect_with_modal_notice(
                "/sources", "modal", "create", "danger", &message,
            ));
        }
    };

    db::create_source(
        &state.db,
        db::NewSource {
            user_id: Some(session.current_user.id.clone()),
            ..input
        },
    )
    .await?;

    Ok(redirect_with_notice(
        "/sources",
        "success",
        "Source webhook created.",
    ))
}

async fn update_source(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<SourceFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_sources(),
        "source update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/sources",
            "edit",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_source_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Source not found.",
        ));
    };
    if !resource_visible_to(existing.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("source update is not allowed for this account"));
    }

    let input = match build_source_payload(form) {
        Ok(input) => input,
        Err(message) => {
            return Ok(redirect_with_modal_notice(
                "/sources", "edit", &id, "danger", &message,
            ));
        }
    };

    db::update_source(&state.db, &id, input).await?;

    Ok(redirect_with_notice(
        "/sources",
        "success",
        "Source updated.",
    ))
}

async fn delete_source(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_sources(),
        "source deletion is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/sources",
            "delete",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_source_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Source not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "source deletion is not allowed for this account",
        ));
    }

    db::delete_source(&state.db, &id).await?;

    Ok(redirect_with_notice(
        "/sources",
        "success",
        &format!("Source '{}' deleted.", item.name),
    ))
}

async fn toggle_source(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_sources(),
        "source update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_source_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Source not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("source update is not allowed for this account"));
    }

    let next_state = item.is_active == 0;
    db::set_source_active(&state.db, &id, next_state).await?;

    Ok(redirect_with_notice(
        "/sources",
        "success",
        if next_state {
            "Source enabled."
        } else {
            "Source disabled."
        },
    ))
}

async fn regenerate_source_token(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_sources(),
        "source update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_source_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/sources",
            "danger",
            "Source not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("source update is not allowed for this account"));
    }

    db::regenerate_source_token(&state.db, &id).await?;

    Ok(redirect_with_notice(
        "/sources",
        "warning",
        "Webhook token regenerated. Update the provider configuration.",
    ))
}

async fn destinations(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_destinations(),
        "destinations access is not allowed for this account",
    )?;
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_destinations = db::list_destinations(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let filtered = filter_destinations(
        &all_destinations,
        &query,
        &session.current_user,
        &scope_ids,
    );
    let edit_destination = match query.edit.as_deref() {
        Some(id) => db::find_destination_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };
    let delete_destination = match query.delete.as_deref() {
        Some(id) => db::find_destination_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };

    Ok(HtmlTemplate(DestinationsTemplate {
        shell: shell(
            &state,
            "destinations",
            "Sorties webhook",
            "Destinations Discord",
            "Gestion des webhooks Discord, validation stricte et test de connectivite.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        filters: BasicFilterView {
            q: query.q.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
        },
        total_count: all_destinations.len(),
        active_count: all_destinations
            .iter()
            .filter(|item| item.is_active == 1)
            .count(),
        inactive_count: all_destinations
            .iter()
            .filter(|item| item.is_active == 0)
            .count(),
        destinations: filtered
            .into_iter()
            .map(destination_row_view)
            .collect::<Vec<_>>(),
        show_create_modal: query.modal.as_deref() == Some("create"),
        create_form: default_destination_form(),
        show_edit_modal: edit_destination.is_some(),
        edit_form: edit_destination
            .as_ref()
            .map(destination_form_from_record)
            .unwrap_or_else(default_destination_form),
        show_delete_modal: delete_destination.is_some(),
        delete_target_name: delete_destination
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        delete_target_action: delete_destination
            .as_ref()
            .map(|item| format!("/destinations/{}/delete", item.id))
            .unwrap_or_default(),
    }))
}

async fn create_destination(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Form(form): Form<DestinationFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_destinations(),
        "destination creation is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/destinations",
            "modal",
            "create",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let input = match build_destination_payload(form) {
        Ok(input) => input,
        Err(message) => {
            return Ok(redirect_with_modal_notice(
                "/destinations",
                "modal",
                "create",
                "danger",
                &message,
            ));
        }
    };

    db::create_destination(
        &state.db,
        db::NewDestination {
            user_id: Some(session.current_user.id.clone()),
            ..input
        },
    )
    .await?;

    Ok(redirect_with_notice(
        "/destinations",
        "success",
        "Destination created.",
    ))
}

async fn update_destination(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<DestinationFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_destinations(),
        "destination update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/destinations",
            "edit",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_destination_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/destinations",
            "danger",
            "Destination not found.",
        ));
    };
    if !resource_visible_to(existing.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "destination update is not allowed for this account",
        ));
    }

    let input = match build_destination_payload(form) {
        Ok(input) => input,
        Err(message) => {
            return Ok(redirect_with_modal_notice(
                "/destinations",
                "edit",
                &id,
                "danger",
                &message,
            ));
        }
    };

    db::update_destination(&state.db, &id, input).await?;

    Ok(redirect_with_notice(
        "/destinations",
        "success",
        "Destination updated.",
    ))
}

async fn delete_destination(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_destinations(),
        "destination deletion is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/destinations",
            "delete",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_destination_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/destinations",
            "danger",
            "Destination not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "destination deletion is not allowed for this account",
        ));
    }

    db::delete_destination(&state.db, &id).await?;

    Ok(redirect_with_notice(
        "/destinations",
        "success",
        &format!("Destination '{}' deleted.", item.name),
    ))
}

async fn toggle_destination(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_destinations(),
        "destination update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            "/destinations",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_destination_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/destinations",
            "danger",
            "Destination not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "destination update is not allowed for this account",
        ));
    }

    let next_state = item.is_active == 0;
    db::set_destination_active(&state.db, &id, next_state).await?;

    Ok(redirect_with_notice(
        "/destinations",
        "success",
        if next_state {
            "Destination enabled."
        } else {
            "Destination disabled."
        },
    ))
}

async fn message_templates_page(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_templates(),
        "templates access is not allowed for this account",
    )?;
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_templates = db::list_message_templates(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let show_create_modal = query.modal.as_deref() == Some("create");
    let edit_template = match query.edit.as_deref() {
        Some(id) => db::find_message_template_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };
    let delete_template = match query.delete.as_deref() {
        Some(id) => db::find_message_template_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };

    let create_form = if show_create_modal {
        default_message_template_form(&state)
    } else {
        empty_message_template_form()
    };
    let edit_form = match edit_template.as_ref() {
        Some(item) => message_template_form_from_record(&state, item)?,
        None => empty_message_template_form(),
    };

    Ok(HtmlTemplate(build_message_templates_page(
        shell(
            &state,
            "templates",
            "Rendu Minijinja",
            "Templates",
            "Edition des messages Discord, seed systeme et preview temps reel.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        &query,
        &state,
        &session.current_user,
        &scope_ids,
        &all_templates,
        show_create_modal,
        create_form,
        edit_template.is_some(),
        edit_form,
        delete_template.is_some(),
        delete_template
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        delete_template
            .as_ref()
            .map(|item| format!("/templates/{}/delete", item.id))
            .unwrap_or_default(),
    )))
}

async fn create_message_template(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Form(form): Form<MessageTemplateFormInput>,
) -> Result<Response, AppError> {
    ensure_allowed(
        session.current_user.can_write_templates(),
        "template creation is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/templates",
            "modal",
            "create",
            "danger",
            "Invalid CSRF token.",
        )
        .into_response());
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;

    let input = match build_message_template_payload(form.clone()) {
        Ok(input) => input,
        Err(message) => {
            let all_templates = db::list_message_templates(&state.db)
                .await?
                .into_iter()
                .filter(|item| {
                    resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)
                })
                .collect::<Vec<_>>();
            let query = PageQuery::default();

            return Ok(HtmlTemplate(build_message_templates_page(
                shell(
                    &state,
                    "templates",
                    "Rendu Minijinja",
                    "Templates",
                    "Edition des messages Discord, seed systeme et preview temps reel.",
                    session.current_user.clone(),
                    FlashView {
                        has_notice: true,
                        level_class: "danger".to_string(),
                        message,
                    },
                ),
                &query,
                &state,
                &session.current_user,
                &scope_ids,
                &all_templates,
                true,
                message_template_form_from_input(
                    &state,
                    "/templates".to_string(),
                    "Create template",
                    "Save template",
                    &form,
                ),
                false,
                default_message_template_form(&state),
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
        }
    };

    db::create_message_template(
        &state.db,
        db::NewMessageTemplate {
            user_id: Some(session.current_user.id.clone()),
            ..input
        },
    )
    .await?;

    Ok(redirect_with_notice("/templates", "success", "Template created.").into_response())
}

async fn update_message_template(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<MessageTemplateFormInput>,
) -> Result<Response, AppError> {
    ensure_allowed(
        session.current_user.can_write_templates(),
        "template update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/templates",
            "edit",
            &id,
            "danger",
            "Invalid CSRF token.",
        )
        .into_response());
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_message_template_by_id(&state.db, &id).await? else {
        return Ok(
            redirect_with_notice("/templates", "danger", "Template not found.").into_response(),
        );
    };
    if !resource_visible_to(existing.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "template update is not allowed for this account",
        ));
    }

    let input = match build_message_template_payload(form.clone()) {
        Ok(input) => input,
        Err(message) => {
            let all_templates = db::list_message_templates(&state.db)
                .await?
                .into_iter()
                .filter(|item| {
                    resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)
                })
                .collect::<Vec<_>>();
            let query = PageQuery::default();

            return Ok(HtmlTemplate(build_message_templates_page(
                shell(
                    &state,
                    "templates",
                    "Rendu Minijinja",
                    "Templates",
                    "Edition des messages Discord, seed systeme et preview temps reel.",
                    session.current_user.clone(),
                    FlashView {
                        has_notice: true,
                        level_class: "danger".to_string(),
                        message,
                    },
                ),
                &query,
                &state,
                &session.current_user,
                &scope_ids,
                &all_templates,
                false,
                default_message_template_form(&state),
                true,
                message_template_form_from_input(
                    &state,
                    format!("/templates/{id}/update"),
                    "Edit template",
                    "Update template",
                    &form,
                ),
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
        }
    };

    db::update_message_template(&state.db, &id, input).await?;

    Ok(redirect_with_notice("/templates", "success", "Template updated.").into_response())
}

async fn delete_message_template(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_templates(),
        "template deletion is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/templates",
            "delete",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_message_template_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/templates",
            "danger",
            "Template not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "template deletion is not allowed for this account",
        ));
    }

    db::delete_message_template(&state.db, &id).await?;

    Ok(redirect_with_notice(
        "/templates",
        "success",
        &format!("Template '{}' deleted.", item.name),
    ))
}

async fn toggle_message_template(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_templates(),
        "template update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            "/templates",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_message_template_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice(
            "/templates",
            "danger",
            "Template not found.",
        ));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden(
            "template update is not allowed for this account",
        ));
    }

    let next_state = item.is_active == 0;
    db::set_message_template_active(&state.db, &id, next_state).await?;

    Ok(redirect_with_notice(
        "/templates",
        "success",
        if next_state {
            "Template enabled."
        } else {
            "Template disabled."
        },
    ))
}

async fn rules(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_rules(),
        "rules access is not allowed for this account",
    )?;
    let request_base_url = request_base_url(&headers);
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_rules = db::list_routing_rules(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let show_create_modal = query.modal.as_deref() == Some("create");
    let show_edit_modal = query.edit.is_some();
    let needs_rule_forms = show_create_modal || show_edit_modal;
    let all_sources = if needs_rule_forms {
        db::list_sources(&state.db)
            .await?
            .into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
            .collect()
    } else {
        Vec::new()
    };
    let (all_destinations, all_templates, can_create_rule) = if needs_rule_forms {
        let all_destinations = db::list_destinations(&state.db)
            .await?
            .into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
            .collect::<Vec<_>>();
        let all_templates = db::list_message_templates(&state.db)
            .await?
            .into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
            .collect::<Vec<_>>();
        let can_create_rule = !all_destinations.is_empty() && !all_templates.is_empty();
        (all_destinations, all_templates, can_create_rule)
    } else {
        let visible_destinations = db::list_destinations(&state.db)
            .await?
            .into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
            .count();
        let visible_templates = db::list_message_templates(&state.db)
            .await?
            .into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
            .count();
        (
            Vec::new(),
            Vec::new(),
            visible_destinations > 0 && visible_templates > 0,
        )
    };
    let edit_rule = match query.edit.as_deref() {
        Some(id) => db::find_routing_rule_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };
    let delete_rule = match query.delete.as_deref() {
        Some(id) => db::find_routing_rule_by_id(&state.db, id)
            .await?
            .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids)),
        None => None,
    };

    Ok(HtmlTemplate(build_rules_page(
        shell(
            &state,
            "rules",
            "Decision engine",
            "Regles de routage",
            "Association source -> template -> destination avec filtres fins et ordre explicite.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        &query,
        &state,
        &request_base_url,
        &session.current_user,
        &scope_ids,
        &all_rules,
        &all_sources,
        &all_destinations,
        &all_templates,
        can_create_rule,
        show_create_modal,
        default_rule_form(),
        show_edit_modal,
        edit_rule
            .as_ref()
            .map(|item| {
                rule_form_from_record(item, &all_sources, &all_destinations, &all_templates)
            })
            .unwrap_or_else(default_rule_form),
        delete_rule.is_some(),
        delete_rule
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        delete_rule
            .as_ref()
            .map(|item| format!("/rules/{}/delete", item.id))
            .unwrap_or_default(),
    )))
}

fn build_rules_page(
    shell: AppShellView,
    query: &PageQuery,
    state: &Arc<AppState>,
    request_base_url: &str,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
    all_rules: &[db::RoutingRuleListItem],
    all_sources: &[db::SourceListItem],
    all_destinations: &[db::DestinationListItem],
    all_templates: &[db::MessageTemplateListItem],
    can_create_rule: bool,
    show_create_modal: bool,
    create_form: RuleFormView,
    show_edit_modal: bool,
    edit_form: RuleFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
) -> RulesTemplate {
    let filtered = filter_rules(all_rules, query, current_user, scope_ids);
    let show_rule_forms = show_create_modal || show_edit_modal;

    RulesTemplate {
        shell,
        filters: BasicFilterView {
            q: query.q.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
        },
        total_count: all_rules.len(),
        active_count: all_rules.iter().filter(|item| item.is_active == 1).count(),
        inactive_count: all_rules.iter().filter(|item| item.is_active == 0).count(),
        rules: filtered.into_iter().map(rule_row_view).collect::<Vec<_>>(),
        source_options: if show_rule_forms {
            source_options(request_base_url, all_sources)
        } else {
            Vec::new()
        },
        destination_options: if show_rule_forms {
            destination_options(all_destinations)
        } else {
            Vec::new()
        },
        template_options: if show_rule_forms {
            template_options(state, all_templates)
        } else {
            Vec::new()
        },
        show_create_modal,
        create_form,
        show_edit_modal,
        edit_form,
        show_delete_modal,
        delete_target_name,
        delete_target_action,
        can_create_rule,
    }
}

fn build_message_templates_page(
    shell: AppShellView,
    query: &PageQuery,
    state: &Arc<AppState>,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
    all_templates: &[db::MessageTemplateListItem],
    show_create_modal: bool,
    create_form: MessageTemplateFormView,
    show_edit_modal: bool,
    edit_form: MessageTemplateFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
) -> MessageTemplatesTemplate {
    let filtered = filter_message_templates(all_templates, query, current_user, scope_ids);

    MessageTemplatesTemplate {
        shell,
        filters: TemplateFilterView {
            q: query.q.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
            format_style: query.format_style.clone().unwrap_or_default(),
        },
        total_count: all_templates.len(),
        active_count: all_templates
            .iter()
            .filter(|item| item.is_active == 1)
            .count(),
        inactive_count: all_templates
            .iter()
            .filter(|item| item.is_active == 0)
            .count(),
        templates: filtered
            .into_iter()
            .map(|item| template_row_view(state, item))
            .collect::<Vec<_>>(),
        show_create_modal,
        create_form,
        show_edit_modal,
        edit_form,
        show_delete_modal,
        delete_target_name,
        delete_target_action,
    }
}

async fn create_rule(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    session: AuthSession,
    Form(form): Form<RuleFormInput>,
) -> Result<Response, AppError> {
    ensure_allowed(
        session.current_user.can_write_rules(),
        "rule creation is not allowed for this account",
    )?;
    let request_base_url = request_base_url(&headers);
    let create_form = rule_form_from_input("/rules".to_string(), "Create rule", "Save rule", &form);
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_sources = db::list_sources(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_destinations = db::list_destinations(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_templates = db::list_message_templates(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_rules = db::list_routing_rules(&state.db).await?;

    if !csrf_matches(&session, &form.csrf_token) {
        let query = PageQuery::default();

        return Ok(HtmlTemplate(build_rules_page(
            shell(
                &state,
                "rules",
                "Decision engine",
                "Regles de routage",
                "Association source -> template -> destination avec filtres fins et ordre explicite.",
                session.current_user.clone(),
                FlashView {
                    has_notice: true,
                    level_class: "danger".to_string(),
                    message: "Invalid CSRF token.".to_string(),
                },
            ),
            &query,
            &state,
            &request_base_url,
            &session.current_user,
            &scope_ids,
            &all_rules,
            &all_sources,
            &all_destinations,
            &all_templates,
            !all_destinations.is_empty() && !all_templates.is_empty(),
            true,
            create_form,
            false,
            default_rule_form(),
            false,
            String::new(),
            String::new(),
        ))
        .into_response());
    }

    let input = match build_rule_payload(
        form.clone(),
        &all_sources,
        &all_destinations,
        &all_templates,
    ) {
        Ok(input) => input,
        Err(message) => {
            let query = PageQuery::default();

            return Ok(HtmlTemplate(build_rules_page(
                shell(
                    &state,
                    "rules",
                    "Decision engine",
                    "Regles de routage",
                    "Association source -> template -> destination avec filtres fins et ordre explicite.",
                    session.current_user.clone(),
                    FlashView {
                        has_notice: true,
                        level_class: "danger".to_string(),
                        message,
                    },
                ),
                &query,
                &state,
                &request_base_url,
                &session.current_user,
                &scope_ids,
                &all_rules,
                &all_sources,
                &all_destinations,
                &all_templates,
                !all_destinations.is_empty() && !all_templates.is_empty(),
                true,
                create_form,
                false,
                default_rule_form(),
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
        }
    };

    db::create_routing_rule(
        &state.db,
        db::NewRoutingRule {
            user_id: Some(session.current_user.id.clone()),
            ..input
        },
    )
    .await?;

    Ok(redirect_with_notice("/rules", "success", "Rule created.").into_response())
}

async fn update_rule(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<RuleFormInput>,
) -> Result<Response, AppError> {
    ensure_allowed(
        session.current_user.can_write_rules(),
        "rule update is not allowed for this account",
    )?;
    let request_base_url = request_base_url(&headers);
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/rules",
            "edit",
            &id,
            "danger",
            "Invalid CSRF token.",
        )
        .into_response());
    }

    let Some(existing) = db::find_routing_rule_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/rules", "danger", "Rule not found.").into_response());
    };
    if !resource_visible_to(existing.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("rule update is not allowed for this account"));
    }

    let all_sources = db::list_sources(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_destinations = db::list_destinations(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_templates = db::list_message_templates(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_rules = db::list_routing_rules(&state.db).await?;
    let input = match build_rule_payload(
        form.clone(),
        &all_sources,
        &all_destinations,
        &all_templates,
    ) {
        Ok(input) => input,
        Err(message) => {
            let query = PageQuery::default();

            return Ok(HtmlTemplate(build_rules_page(
                shell(
                    &state,
                    "rules",
                    "Decision engine",
                    "Regles de routage",
                    "Association source -> template -> destination avec filtres fins et ordre explicite.",
                    session.current_user.clone(),
                    FlashView {
                        has_notice: true,
                        level_class: "danger".to_string(),
                        message,
                    },
                ),
                &query,
                &state,
                &request_base_url,
                &session.current_user,
                &scope_ids,
                &all_rules,
                &all_sources,
                &all_destinations,
                &all_templates,
                !all_destinations.is_empty() && !all_templates.is_empty(),
                false,
                default_rule_form(),
                true,
                rule_form_from_input(
                    format!("/rules/{id}/update"),
                    "Edit rule",
                    "Update rule",
                    &form,
                ),
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
        }
    };

    db::update_routing_rule(&state.db, &id, input).await?;

    Ok(redirect_with_notice("/rules", "success", "Rule updated.").into_response())
}

async fn delete_rule(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_rules(),
        "rule deletion is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/rules",
            "delete",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_routing_rule_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/rules", "danger", "Rule not found."));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("rule deletion is not allowed for this account"));
    }

    db::delete_routing_rule(&state.db, &id).await?;

    Ok(redirect_with_notice(
        "/rules",
        "success",
        &format!("Rule '{}' deleted.", item.name),
    ))
}

async fn toggle_rule(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_rules(),
        "rule update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            "/rules",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(item) = db::find_routing_rule_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/rules", "danger", "Rule not found."));
    };
    if !resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids) {
        return Err(AppError::forbidden("rule update is not allowed for this account"));
    }

    let next_state = item.is_active == 0;
    db::set_routing_rule_active(&state.db, &id, next_state).await?;

    Ok(redirect_with_notice(
        "/rules",
        "success",
        if next_state {
            "Rule enabled."
        } else {
            "Rule disabled."
        },
    ))
}

async fn test_rule_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    session: AuthSession,
    Form(form): Form<RuleFormInput>,
) -> Result<Response, AppError> {
    ensure_allowed(
        session.current_user.can_write_rules(),
        "rule testing is not allowed for this account",
    )?;
    let request_base_url = request_base_url(&headers);
    let query = PageQuery::default();
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_rules = db::list_routing_rules(&state.db).await?;
    let all_sources = db::list_sources(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_destinations = db::list_destinations(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_templates = db::list_message_templates(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let (show_create_modal, create_form, show_edit_modal, edit_form) = rule_test_form_state(&form);

    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(HtmlTemplate(build_rules_page(
            shell(
                &state,
                "rules",
                "Decision engine",
                "Regles de routage",
                "Association source -> template -> destination avec filtres fins et ordre explicite.",
                session.current_user.clone(),
                FlashView {
                    has_notice: true,
                    level_class: "danger".to_string(),
                    message: "Invalid CSRF token.".to_string(),
                },
            ),
            &query,
            &state,
            &request_base_url,
            &session.current_user,
            &scope_ids,
            &all_rules,
            &all_sources,
            &all_destinations,
            &all_templates,
            !all_destinations.is_empty() && !all_templates.is_empty(),
            show_create_modal,
            create_form,
            show_edit_modal,
            edit_form,
            false,
            String::new(),
            String::new(),
        ))
        .into_response());
    }

    let input = match build_rule_payload(
        form.clone(),
        &all_sources,
        &all_destinations,
        &all_templates,
    ) {
        Ok(input) => input,
        Err(message) => {
            return Ok(HtmlTemplate(build_rules_page(
                shell(
                    &state,
                    "rules",
                    "Decision engine",
                    "Regles de routage",
                    "Association source -> template -> destination avec filtres fins et ordre explicite.",
                    session.current_user.clone(),
                    FlashView {
                        has_notice: true,
                        level_class: "danger".to_string(),
                        message,
                    },
                ),
                &query,
                &state,
                &request_base_url,
                &session.current_user,
                &scope_ids,
                &all_rules,
                &all_sources,
                &all_destinations,
                &all_templates,
                !all_destinations.is_empty() && !all_templates.is_empty(),
                show_create_modal,
                create_form,
                show_edit_modal,
                edit_form,
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
        }
    };

    let destination = all_destinations
        .iter()
        .find(|item| item.id == input.destination_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("Selected destination does not exist."))?;
    let template = all_templates
        .iter()
        .find(|item| item.id == input.template_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("Selected template does not exist."))?;
    let source = input
        .source_id
        .as_deref()
        .and_then(|source_id| all_sources.iter().find(|item| item.id == source_id));

    if let Err(error) = webhook::send_test_route_webhook(
        &state,
        &destination,
        &template,
        build_rule_webhook_test_request(&input, source),
    )
    .await
    {
        return Ok(HtmlTemplate(build_rules_page(
            shell(
                &state,
                "rules",
                "Decision engine",
                "Regles de routage",
                "Association source -> template -> destination avec filtres fins et ordre explicite.",
                session.current_user.clone(),
                FlashView {
                    has_notice: true,
                    level_class: "danger".to_string(),
                    message: format!("Failed to send test webhook: {error}"),
                },
                ),
                &query,
                &state,
                &request_base_url,
                &session.current_user,
                &scope_ids,
                &all_rules,
                &all_sources,
                &all_destinations,
                &all_templates,
                !all_destinations.is_empty() && !all_templates.is_empty(),
                show_create_modal,
                create_form,
                show_edit_modal,
                edit_form,
                false,
                String::new(),
                String::new(),
            ))
            .into_response());
    }

    Ok(HtmlTemplate(build_rules_page(
        shell(
            &state,
            "rules",
            "Decision engine",
            "Regles de routage",
            "Association source -> template -> destination avec filtres fins et ordre explicite.",
            session.current_user.clone(),
            FlashView {
                has_notice: true,
                level_class: "success".to_string(),
                message: format!("Discord test webhook sent to {}.", destination.name),
            },
        ),
        &query,
        &state,
        &request_base_url,
        &session.current_user,
        &scope_ids,
        &all_rules,
        &all_sources,
        &all_destinations,
        &all_templates,
        !all_destinations.is_empty() && !all_templates.is_empty(),
        show_create_modal,
        create_form,
        show_edit_modal,
        edit_form,
        false,
        String::new(),
        String::new(),
    ))
    .into_response())
}

async fn deliveries(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_deliveries(),
        "deliveries access is not allowed for this account",
    )?;
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let visible_source_ids =
        visible_source_ids_for_user(&state, &session.current_user, &scope_ids).await?;
    let filters = delivery_filters_from_query(&query);
    let page_number = query.page.unwrap_or(1).max(1);
    let page = db::list_delivery_summaries(
        &state.db,
        &filters,
        page_number,
        50,
        visible_source_ids.as_deref(),
    )
    .await?;
    let all_sources = db::list_sources(&state.db)
        .await?
        .into_iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let total_pages = if page.total_count == 0 {
        1
    } else {
        ((page.total_count as usize - 1) / page.per_page) + 1
    };
    let current_page = page.page.min(total_pages.max(1));
    let offset = if page.total_count == 0 {
        0
    } else {
        (current_page - 1) * page.per_page + 1
    };
    let visible_to = if page.total_count == 0 {
        0
    } else {
        offset + page.items.len() - 1
    };

    Ok(HtmlTemplate(DeliveriesTemplate {
        shell: shell(
            &state,
            "deliveries",
            "Historique technique",
            "Livraisons",
            "Inspection des payloads entrants, etats de pipeline et messages Discord produits.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        filters: DeliveryFilterView {
            q: query.q.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
            source_id: query.source_id.clone().unwrap_or_default(),
            provider: query.provider.clone().unwrap_or_default(),
            event_type: query.event_type.clone().unwrap_or_default(),
            date_from: query.date_from.clone().unwrap_or_default(),
            date_to: query.date_to.clone().unwrap_or_default(),
        },
        total_count: page.total_count,
        visible_from: offset,
        visible_to,
        rows: page.items.iter().map(delivery_row_view).collect::<Vec<_>>(),
        source_options: all_sources
            .iter()
            .map(|item| SimpleOptionView {
                value: item.id.clone(),
                label: item.name.clone(),
            })
            .collect(),
        pagination: build_pagination_view(&query, current_page, total_pages),
    }))
}

async fn delivery_detail(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_deliveries(),
        "delivery detail is not allowed for this account",
    )?;
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let visible_source_ids =
        visible_source_ids_for_user(&state, &session.current_user, &scope_ids).await?;
    let detail = db::find_delivery_detail(&state.db, &id, visible_source_ids.as_deref())
        .await?
        .ok_or_else(|| AppError::not_found("delivery not found"))?;
    let messages = db::list_delivery_message_attempts(&state.db, &id).await?;

    Ok(HtmlTemplate(DeliveryDetailTemplate {
        shell: shell(
            &state,
            "deliveries",
            "Payload inspection",
            "Delivery detail",
            "Metadonnees de livraison, payload brut, evenement normalise, messages Discord et rejeu complet.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        detail: delivery_detail_view(&detail),
        messages: messages
            .iter()
            .map(delivery_message_view)
            .collect::<Vec<_>>(),
        replay_action: format!("/deliveries/{}/replay", id),
        back_url: build_delivery_list_url(&PageQuery::default(), 1),
    }))
}

async fn replay_delivery(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_replay_deliveries(),
        "delivery replay is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice(
            &format!("/deliveries/{id}"),
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let visible_source_ids =
        visible_source_ids_for_user(&state, &session.current_user, &scope_ids).await?;

    if db::find_delivery_replay_record(&state.db, &id, visible_source_ids.as_deref())
        .await?
        .is_none()
    {
        return Err(AppError::forbidden("delivery replay is not allowed for this account"));
    }

    match webhook::enqueue_replay_from_delivery(state, &id).await {
        Ok(new_delivery_id) => Ok(redirect_with_notice(
            &format!("/deliveries/{new_delivery_id}"),
            "success",
            "Replay queued. A new delivery entry has been created.",
        )),
        Err(error) => Ok(redirect_with_notice(
            &format!("/deliveries/{id}"),
            "danger",
            &format!("Replay failed: {error}"),
        )),
    }
}

async fn users(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Query(query): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_read_users(),
        "users access is not allowed for this account",
    )?;
    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let all_users = db::list_users(&state.db)
        .await?
        .into_iter()
        .filter(|item| user_visible_to(item, &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let all_sessions = db::list_active_sessions(&state.db, 24)
        .await?
        .into_iter()
        .filter(|item| session_visible_to(item, &session.current_user, &scope_ids))
        .collect::<Vec<_>>();
    let filtered_users = filter_users(&all_users, &query, &session.current_user, &scope_ids);
    let filtered_sessions = filter_active_sessions(&all_sessions, &session.current_user, &scope_ids);
    let can_manage_accounts =
        session.current_user.can_write_users() || session.current_user.can_create_subusers();
    let edit_target = match query.edit.as_deref() {
        Some(id) if can_manage_accounts => db::find_user_by_id(&state.db, id)
            .await?
            .filter(|item| user_manageable_by(&session.current_user, item, &scope_ids)),
        _ => None,
    };
    let delete_target = match query.delete.as_deref() {
        Some(id) if can_manage_accounts => db::find_user_by_id(&state.db, id)
            .await?
            .filter(|item| user_manageable_by(&session.current_user, item, &scope_ids)),
        _ => None,
    };
    let live_session_count = all_users
        .iter()
        .map(|item| item.active_session_count.max(0) as usize)
        .sum();
    let visible_parent_items = all_users
        .iter()
        .filter(|item| user_visible_to(item, &session.current_user, &scope_ids))
        .cloned()
        .collect::<Vec<_>>();

    Ok(HtmlTemplate(UsersTemplate {
        shell: shell(
            &state,
            "users",
            "Controle d'acces",
            "Utilisateurs",
            "Visibilite des comptes, des relations parent-enfant et des sessions actives.",
            session.current_user.clone(),
            flash_from_query(&query),
        ),
        filters: UsersFilterView {
            q: query.q.clone().unwrap_or_default(),
            status: query.status.clone().unwrap_or_default(),
            role: query.role.clone().unwrap_or_default(),
        },
        total_count: all_users.len(),
        visible_count: filtered_users.len(),
        active_count: all_users.iter().filter(|item| item.is_active == 1).count(),
        admin_count: all_users
            .iter()
            .filter(|item| matches!(item.role.as_str(), "superadmin" | "admin"))
            .count(),
        live_session_count,
        users: filtered_users
            .into_iter()
            .map(|item| {
                user_row_view(
                    item,
                    can_manage_accounts
                        && (session.current_user.is_superadmin()
                            || item.id == session.current_user.id
                            || scope_ids.contains(&item.id)),
                )
            })
            .collect::<Vec<_>>(),
        sessions: filtered_sessions
            .into_iter()
            .map(|item| active_session_row_view(item, &session.session_id))
            .collect::<Vec<_>>(),
        can_manage_users: session.current_user.can_write_users(),
        can_create_subusers: session.current_user.can_create_subusers(),
        show_create_modal: query.modal.as_deref() == Some("create") && can_manage_accounts,
        create_form: default_user_form(&session.current_user),
        show_edit_modal: edit_target.is_some(),
        edit_form: edit_target
            .as_ref()
            .map(user_form_from_record)
            .unwrap_or_else(|| default_user_form(&session.current_user)),
        show_delete_modal: delete_target.is_some(),
        delete_target_name: delete_target
            .as_ref()
            .map(|item| item.username.clone())
            .unwrap_or_default(),
        delete_target_action: delete_target
            .as_ref()
            .map(|item| format!("/users/{}/delete", item.id))
            .unwrap_or_default(),
        role_options: allowed_role_options(&session.current_user),
        parent_options: parent_user_options(&visible_parent_items, &session.current_user),
    }))
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Form(form): Form<UserFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_users() || session.current_user.can_create_subusers(),
        "user creation is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/users",
            "modal",
            "create",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let payload = match build_user_payload(&session.current_user, &form, true) {
        Ok(payload) => payload,
        Err(message) => {
            return Ok(redirect_with_modal_notice(
                "/users",
                "modal",
                "create",
                "danger",
                &message,
            ));
        }
    };

    if let Some(parent_user_id) = payload.parent_user_id.as_deref() {
        let scope_ids = user_scope_ids(&state, &session.current_user).await?;
        let Some(parent) = db::find_user_by_id(&state.db, parent_user_id).await? else {
            return Ok(redirect_with_modal_notice(
                "/users",
                "modal",
                "create",
                "danger",
                "Selected parent user does not exist.",
            ));
        };
        if !session.current_user.is_superadmin()
            && !user_manageable_by(&session.current_user, &parent, &scope_ids)
        {
            return Err(AppError::forbidden(
                "user creation is not allowed outside your delegated scope",
            ));
        }
    }

    match db::create_user(
        &state.db,
        db::NewUser {
            username: payload.username,
            email: payload.email,
            password_hash: payload
                .password_hash
                .ok_or_else(|| AppError::bad_request("password is required"))?,
            role: payload.role,
            is_active: payload.is_active,
            parent_user_id: payload.parent_user_id,
            created_by_user_id: payload.created_by_user_id,
            permissions_json: payload.permissions_json,
        },
    )
    .await
    {
        Ok(_) => Ok(redirect_with_notice(
            "/users",
            "success",
            "User created.",
        )),
        Err(error) => Ok(redirect_with_modal_notice(
            "/users",
            "modal",
            "create",
            "danger",
            &user_write_error_message(&error),
        )),
    }
}

async fn update_user(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<UserFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_users(),
        "user update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/users",
            "edit",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_user_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/users", "danger", "User not found."));
    };
    if !user_manageable_by(&session.current_user, &existing, &scope_ids) {
        return Err(AppError::forbidden("user update is not allowed for this account"));
    }

    let mut payload = match build_user_payload(&session.current_user, &form, false) {
        Ok(payload) => payload,
        Err(message) => {
            return Ok(redirect_with_modal_notice("/users", "edit", &id, "danger", &message));
        }
    };
    payload.created_by_user_id = existing.created_by_user_id.clone();

    if existing.id == session.current_user.id && !payload.is_active {
        return Ok(redirect_with_modal_notice(
            "/users",
            "edit",
            &id,
            "danger",
            "You cannot deactivate your current account.",
        ));
    }

    if existing.role == "superadmin" && payload.role != "superadmin" {
        let superadmin_count = db::count_users_with_role(&state.db, "superadmin").await?;
        if superadmin_count <= 1 {
            return Ok(redirect_with_modal_notice(
                "/users",
                "edit",
                &id,
                "danger",
                "The last superadmin cannot be downgraded.",
            ));
        }
    }

    if existing.role == "superadmin" && !payload.is_active {
        let superadmin_count = db::count_users_with_role(&state.db, "superadmin").await?;
        if superadmin_count <= 1 {
            return Ok(redirect_with_modal_notice(
                "/users",
                "edit",
                &id,
                "danger",
                "The last superadmin cannot be deactivated.",
            ));
        }
    }

    if let Some(parent_user_id) = payload.parent_user_id.as_deref() {
        if parent_user_id == existing.id {
            return Ok(redirect_with_modal_notice(
                "/users",
                "edit",
                &id,
                "danger",
                "A user cannot be its own parent.",
            ));
        }
        let Some(parent) = db::find_user_by_id(&state.db, parent_user_id).await? else {
            return Ok(redirect_with_modal_notice(
                "/users",
                "edit",
                &id,
                "danger",
                "Selected parent user does not exist.",
            ));
        };
        if !session.current_user.is_superadmin()
            && !user_manageable_by(&session.current_user, &parent, &scope_ids)
        {
            return Err(AppError::forbidden(
                "user update is not allowed outside your delegated scope",
            ));
        }
    }

    match db::update_user(
        &state.db,
        &id,
        &payload.username,
        &payload.email,
        &payload.role,
        payload.is_active,
        payload.parent_user_id,
        payload.created_by_user_id,
        payload.permissions_json,
        payload.password_hash,
    )
    .await
    {
        Ok(_) => Ok(redirect_with_notice("/users", "success", "User updated.")),
        Err(error) => Ok(redirect_with_modal_notice(
            "/users",
            "edit",
            &id,
            "danger",
            &user_write_error_message(&error),
        )),
    }
}

async fn toggle_user(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_users(),
        "user update is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_notice("/users", "danger", "Invalid CSRF token."));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_user_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/users", "danger", "User not found."));
    };
    if !user_manageable_by(&session.current_user, &existing, &scope_ids) {
        return Err(AppError::forbidden("user update is not allowed for this account"));
    }

    let next_state = existing.is_active == 0;
    if existing.id == session.current_user.id && !next_state {
        return Ok(redirect_with_notice(
            "/users",
            "danger",
            "You cannot deactivate your current account.",
        ));
    }
    if existing.role == "superadmin" && !next_state {
        let superadmin_count = db::count_users_with_role(&state.db, "superadmin").await?;
        if superadmin_count <= 1 {
            return Ok(redirect_with_notice(
                "/users",
                "danger",
                "The last superadmin cannot be deactivated.",
            ));
        }
    }

    db::set_user_active(&state.db, &id, next_state).await?;

    Ok(redirect_with_notice(
        "/users",
        "success",
        if next_state {
            "User enabled."
        } else {
            "User disabled."
        },
    ))
}

async fn delete_user(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfFormInput>,
) -> Result<impl IntoResponse, AppError> {
    ensure_allowed(
        session.current_user.can_write_users(),
        "user deletion is not allowed for this account",
    )?;
    if !csrf_matches(&session, &form.csrf_token) {
        return Ok(redirect_with_modal_notice(
            "/users",
            "delete",
            &id,
            "danger",
            "Invalid CSRF token.",
        ));
    }

    let scope_ids = user_scope_ids(&state, &session.current_user).await?;
    let Some(existing) = db::find_user_by_id(&state.db, &id).await? else {
        return Ok(redirect_with_notice("/users", "danger", "User not found."));
    };
    if !user_manageable_by(&session.current_user, &existing, &scope_ids) {
        return Err(AppError::forbidden("user deletion is not allowed for this account"));
    }
    if existing.id == session.current_user.id {
        return Ok(redirect_with_modal_notice(
            "/users",
            "delete",
            &id,
            "danger",
            "You cannot delete your current account.",
        ));
    }
    if existing.role == "superadmin" {
        let superadmin_count = db::count_users_with_role(&state.db, "superadmin").await?;
        if superadmin_count <= 1 {
            return Ok(redirect_with_modal_notice(
                "/users",
                "delete",
                &id,
                "danger",
                "The last superadmin cannot be deleted.",
            ));
        }
    }

    db::delete_user(&state.db, &id).await?;
    Ok(redirect_with_notice("/users", "success", "User deleted."))
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Response, AppError> {
    if !db::admin_user_exists(&state.db).await? {
        return Ok(Redirect::to("/setup").into_response());
    }

    if auth::load_auth_session_from_headers(&state, &headers)
        .await?
        .is_some()
    {
        return Ok(Redirect::to("/dashboard").into_response());
    }

    let guest_csrf_token = auth::new_guest_csrf_token();
    let mut response = HtmlTemplate(LoginTemplate {
        shell: shell(
            &state,
            "",
            "Authentication",
            "Login",
            "Connexion au shell d'administration via sessions SQLite et cookie HttpOnly.",
            CurrentUser::guest(),
            flash_from_query(&query),
        ),
        form: LoginFormView {
            login: String::new(),
            guest_csrf_token: guest_csrf_token.clone(),
        },
    })
    .into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        auth::guest_csrf_cookie(&state, &guest_csrf_token),
    );

    Ok(response)
}

async fn login_post(
    State(state): State<Arc<AppState>>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginFormInput>,
) -> Result<Response, AppError> {
    if !db::admin_user_exists(&state.db).await? {
        return Ok(Redirect::to("/setup").into_response());
    }

    if !auth::guest_csrf_matches(&state, &headers, &form.guest_csrf_token) {
        return Ok(redirect_with_notice_response(
            "/login",
            "danger",
            "Invalid form token.",
        ));
    }

    let login = form.login.trim();
    let password = form.password;
    if login.is_empty() || password.trim().is_empty() {
        return Ok(redirect_with_notice_response(
            "/login",
            "danger",
            "Login and password are required.",
        ));
    }

    let client_ip = remote_addr.ip().to_string();
    if is_rate_limited(&state, &client_ip) {
        return Ok(redirect_with_notice_response(
            "/login",
            "danger",
            "Too many login attempts from this IP. Try again later.",
        ));
    }

    let Some(user) = db::find_user_by_login(&state.db, login).await? else {
        register_failed_login(&state, &client_ip);
        return Ok(redirect_with_notice_response(
            "/login",
            "danger",
            "Invalid credentials.",
        ));
    };

    let password_ok =
        auth::verify_password(&password, &user.password_hash).map_err(AppError::internal)?;
    if !password_ok || user.is_active == 0 {
        register_failed_login(&state, &client_ip);
        return Ok(redirect_with_notice_response(
            "/login",
            "danger",
            "Invalid credentials.",
        ));
    }

    clear_failed_login(&state, &client_ip);
    db::delete_expired_sessions(&state.db).await?;

    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().simple().to_string();
    let expires_at = auth::session_expires_at(state.config.session_ttl_hours);

    db::create_session(
        &state.db,
        db::NewSession {
            id: session_id.clone(),
            user_id: user.id,
            csrf_token,
            expires_at,
            ip_address: Some(client_ip),
            user_agent: auth::user_agent_from_headers(&headers),
        },
    )
    .await?;

    let mut response =
        auth::redirect_with_cookie("/dashboard", auth::new_session_cookie(&state, &session_id));
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        auth::clearing_guest_csrf_cookie(&state),
    );

    Ok(response)
}

async fn setup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Response, AppError> {
    if db::admin_user_exists(&state.db).await? {
        if auth::load_auth_session_from_headers(&state, &headers)
            .await?
            .is_some()
        {
            return Ok(Redirect::to("/dashboard").into_response());
        }

        return Ok(Redirect::to("/login").into_response());
    }

    let guest_csrf_token = auth::new_guest_csrf_token();
    let mut response = HtmlTemplate(SetupTemplate {
        shell: shell(
            &state,
            "",
            "Bootstrap",
            "Setup",
            "Creation du premier compte superadmin avec mot de passe Argon2 et session serveur.",
            CurrentUser::guest(),
            flash_from_query(&query),
        ),
        form: SetupFormView {
            username: String::new(),
            email: String::new(),
            guest_csrf_token: guest_csrf_token.clone(),
        },
    })
    .into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        auth::guest_csrf_cookie(&state, &guest_csrf_token),
    );

    Ok(response)
}

async fn setup_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<SetupFormInput>,
) -> Result<Response, AppError> {
    if db::admin_user_exists(&state.db).await? {
        return Ok(Redirect::to("/login").into_response());
    }

    if !auth::guest_csrf_matches(&state, &headers, &form.guest_csrf_token) {
        return Ok(redirect_with_notice_response(
            "/setup",
            "danger",
            "Invalid form token.",
        ));
    }

    let username = form.username.trim();
    let email = form.email.trim();
    let password = form.password;
    let password_confirmation = form.password_confirmation;

    if username.is_empty() || email.is_empty() {
        return Ok(redirect_with_notice_response(
            "/setup",
            "danger",
            "Username and email are required.",
        ));
    }

    if let Err(message) = auth::validate_password_rules(&password, &password_confirmation) {
        return Ok(redirect_with_notice_response("/setup", "danger", &message));
    }

    if db::find_user_by_login(&state.db, email).await?.is_some()
        || db::find_user_by_login(&state.db, username).await?.is_some()
    {
        return Ok(redirect_with_notice_response(
            "/setup",
            "danger",
            "A user with this email or username already exists.",
        ));
    }

    let password_hash = auth::hash_password(&password).map_err(AppError::internal)?;
    let user_id = db::create_user(
        &state.db,
        db::NewUser {
            username: username.to_string(),
            email: email.to_string(),
            password_hash,
            role: "superadmin".to_string(),
            is_active: true,
            parent_user_id: None,
            created_by_user_id: None,
            permissions_json: None,
        },
    )
    .await?;
    db::assign_unowned_resources_to_user(&state.db, &user_id).await?;

    db::delete_expired_sessions(&state.db).await?;

    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().simple().to_string();

    db::create_session(
        &state.db,
        db::NewSession {
            id: session_id.clone(),
            user_id,
            csrf_token,
            expires_at: auth::session_expires_at(state.config.session_ttl_hours),
            ip_address: auth::client_ip_from_headers(&headers),
            user_agent: auth::user_agent_from_headers(&headers),
        },
    )
    .await?;

    let mut response =
        auth::redirect_with_cookie("/dashboard", auth::new_session_cookie(&state, &session_id));
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        auth::clearing_guest_csrf_cookie(&state),
    );

    Ok(response)
}

async fn logout(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Form(form): Form<LogoutFormInput>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Ok(redirect_with_notice_response(
            "/dashboard",
            "danger",
            "Invalid CSRF token.",
        ));
    }

    db::delete_session(&state.db, &session.session_id).await?;

    Ok(auth::redirect_with_cookie(
        "/login",
        auth::clearing_session_cookie(&state),
    ))
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<HealthResponse>, AppError> {
    db::ping(&state.db).await?;

    Ok(Json(HealthResponse {
        status: "ok",
        app_name: state.config.app_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        database: "sqlite".to_string(),
        timestamp: Utc::now().to_rfc3339(),
    }))
}

async fn preview(
    State(state): State<Arc<AppState>>,
    session: AuthSession,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<PreviewResponse>, AppError> {
    ensure_allowed(
        session.current_user.can_read_templates(),
        "template preview is not allowed for this account",
    )?;
    if request.template.trim().is_empty() {
        return Err(AppError::bad_request("template is required"));
    }

    let rendered = state.discord.render_preview(&request.template)?;

    Ok(Json(PreviewResponse {
        rendered,
        sample_payload: state.discord.sample_payload(),
    }))
}

fn shell(
    state: &Arc<AppState>,
    active: &str,
    eyebrow: &str,
    page_title: &str,
    page_summary: &str,
    current_user: CurrentUser,
    flash: FlashView,
) -> AppShellView {
    let mut items = vec![("Dashboard", "/dashboard", "dashboard")];
    if current_user.can_read_sources() {
        items.push(("Sources", "/sources", "sources"));
    }
    if current_user.can_read_destinations() {
        items.push(("Discord", "/destinations", "destinations"));
    }
    if current_user.can_read_templates() {
        items.push(("Templates", "/templates", "templates"));
    }
    if current_user.can_read_rules() {
        items.push(("Routes", "/rules", "rules"));
    }
    if current_user.can_read_deliveries() {
        items.push(("Livraisons", "/deliveries", "deliveries"));
    }
    if current_user.can_read_users() {
        items.push(("Utilisateurs", "/users", "users"));
    }

    AppShellView {
        app_name: state.config.app_name.clone(),
        page_eyebrow: eyebrow.to_string(),
        page_title: page_title.to_string(),
        page_summary: page_summary.to_string(),
        nav_items: items
            .into_iter()
            .map(|(label, href, key)| NavItemView {
                label: label.to_string(),
                href: href.to_string(),
                active: key == active,
            })
            .collect(),
        current_user,
        flash,
    }
}

fn flash_from_query(query: &PageQuery) -> FlashView {
    match query.notice.as_deref() {
        Some(message) if !message.trim().is_empty() => FlashView {
            has_notice: true,
            level_class: match query.notice_level.as_deref() {
                Some("success") => "success".to_string(),
                Some("danger") => "danger".to_string(),
                Some("warning") => "warning".to_string(),
                Some("pending") => "pending".to_string(),
                _ => "info".to_string(),
            },
            message: message.to_string(),
        },
        _ => FlashView::empty(),
    }
}

fn is_rate_limited(state: &Arc<AppState>, client_ip: &str) -> bool {
    let now = Utc::now().timestamp();
    let mut attempts_by_ip = state
        .login_rate_limit
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let attempts = attempts_by_ip.entry(client_ip.to_string()).or_default();
    attempts.retain(|timestamp| now - *timestamp < LOGIN_RATE_LIMIT_WINDOW_SECONDS);
    attempts.len() >= LOGIN_RATE_LIMIT_MAX_ATTEMPTS
}

fn register_failed_login(state: &Arc<AppState>, client_ip: &str) {
    let now = Utc::now().timestamp();
    let mut attempts_by_ip = state
        .login_rate_limit
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let attempts = attempts_by_ip.entry(client_ip.to_string()).or_default();
    attempts.retain(|timestamp| now - *timestamp < LOGIN_RATE_LIMIT_WINDOW_SECONDS);
    attempts.push(now);
}

fn clear_failed_login(state: &Arc<AppState>, client_ip: &str) {
    state
        .login_rate_limit
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(client_ip);
}

fn csrf_matches(session: &AuthSession, submitted_token: &str) -> bool {
    !submitted_token.trim().is_empty() && submitted_token == session.csrf_token
}

fn empty_metric_fallback(metrics: Vec<db::TopMetric>, label: &str) -> Vec<db::TopMetric> {
    if metrics.is_empty() {
        vec![db::TopMetric {
            label: label.to_string(),
            value: 0,
        }]
    } else {
        metrics
    }
}

fn default_preview_template() -> &'static str {
    r#"{{ actor.name }} pushed {{ commit_count }} commits to {{ repository.full_name }} on {{ branch }}
{% for commit in commits -%}
- {{ commit.id }} {{ commit.message }}
{% endfor -%}"#
}

async fn user_scope_ids(
    state: &Arc<AppState>,
    current_user: &CurrentUser,
) -> Result<HashSet<String>, AppError> {
    let mut ids = HashSet::new();
    ids.insert(current_user.id.clone());
    ids.extend(db::list_descendant_user_ids(&state.db, &current_user.id).await?);
    Ok(ids)
}

async fn visible_source_ids_for_user(
    state: &Arc<AppState>,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Result<Option<Vec<String>>, AppError> {
    if current_user.is_superadmin() {
        return Ok(None);
    }

    let items = db::list_sources(&state.db).await?;
    Ok(Some(
        items.into_iter()
            .filter(|item| resource_visible_to(item.user_id.as_deref(), current_user, scope_ids))
            .map(|item| item.id)
            .collect(),
    ))
}

fn resource_visible_to(
    owner_user_id: Option<&str>,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> bool {
    if current_user.is_superadmin() {
        return true;
    }

    owner_user_id.is_some_and(|owner_id| scope_ids.contains(owner_id))
}

fn ensure_allowed(allowed: bool, message: &str) -> Result<(), AppError> {
    if allowed {
        Ok(())
    } else {
        Err(AppError::forbidden(message))
    }
}

fn user_visible_to(
    item: &db::UserListItem,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> bool {
    if current_user.is_superadmin() {
        return true;
    }

    item.id == current_user.id || scope_ids.contains(&item.id)
}

fn session_visible_to(
    item: &db::ActiveSessionListItem,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> bool {
    if current_user.is_superadmin() {
        return true;
    }

    item.user_id == current_user.id || scope_ids.contains(&item.user_id)
}

fn user_manageable_by(
    current_user: &CurrentUser,
    target: &db::UserRecord,
    scope_ids: &HashSet<String>,
) -> bool {
    if !current_user.can_write_users() {
        return false;
    }

    if current_user.is_superadmin() {
        return true;
    }

    if target.id == current_user.id {
        return true;
    }

    scope_ids.contains(&target.id)
}

fn role_assignable_by(current_user: &CurrentUser, role: &str) -> bool {
    current_user.is_superadmin() || auth::role_rank(role) <= current_user.role_rank()
}

fn filter_sources<'a>(
    items: &'a [db::SourceListItem],
    query: &PageQuery,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::SourceListItem> {
    let search = query.q.as_deref().map(|value| value.trim().to_lowercase());
    let provider = query
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    items
        .iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), current_user, scope_ids))
        .filter(|item| {
            let matches_search = match &search {
                Some(search) => {
                    item.name.to_lowercase().contains(search)
                        || item.provider.to_lowercase().contains(search)
                        || item
                            .repository_filter
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(search)
                }
                None => true,
            };
            let matches_provider = match provider {
                Some(provider) => item.provider == provider,
                None => true,
            };
            let matches_status = match status {
                Some("active") => item.is_active == 1,
                Some("inactive") => item.is_active == 0,
                _ => true,
            };

            matches_search && matches_provider && matches_status
        })
        .collect()
}

fn filter_destinations<'a>(
    items: &'a [db::DestinationListItem],
    query: &PageQuery,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::DestinationListItem> {
    let search = query.q.as_deref().map(|value| value.trim().to_lowercase());
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    items
        .iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), current_user, scope_ids))
        .filter(|item| {
            let matches_search = match &search {
                Some(search) => {
                    item.name.to_lowercase().contains(search)
                        || item.webhook_url.to_lowercase().contains(search)
                }
                None => true,
            };
            let matches_status = match status {
                Some("active") => item.is_active == 1,
                Some("inactive") => item.is_active == 0,
                _ => true,
            };

            matches_search && matches_status
        })
        .collect()
}

fn filter_message_templates<'a>(
    items: &'a [db::MessageTemplateListItem],
    query: &PageQuery,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::MessageTemplateListItem> {
    let search = query.q.as_deref().map(|value| value.trim().to_lowercase());
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let format_style = query
        .format_style
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    items
        .iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), current_user, scope_ids))
        .filter(|item| {
            let matches_search = match &search {
                Some(search) => {
                    item.name.to_lowercase().contains(search)
                        || item.body_template.to_lowercase().contains(search)
                        || item.format_style.to_lowercase().contains(search)
                }
                None => true,
            };
            let matches_status = match status {
                Some("active") => item.is_active == 1,
                Some("inactive") => item.is_active == 0,
                _ => true,
            };
            let matches_format = match format_style {
                Some(value) => item.format_style == value,
                None => true,
            };

            matches_search && matches_status && matches_format
        })
        .collect()
}

fn filter_rules<'a>(
    items: &'a [db::RoutingRuleListItem],
    query: &PageQuery,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::RoutingRuleListItem> {
    let search = query.q.as_deref().map(|value| value.trim().to_lowercase());
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    items
        .iter()
        .filter(|item| resource_visible_to(item.user_id.as_deref(), current_user, scope_ids))
        .filter(|item| {
            let matches_search = match &search {
                Some(search) => {
                    item.name.to_lowercase().contains(search)
                        || item.destination_name.to_lowercase().contains(search)
                        || item.template_name.to_lowercase().contains(search)
                        || item
                            .source_name
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(search)
                }
                None => true,
            };
            let matches_status = match status {
                Some("active") => item.is_active == 1,
                Some("inactive") => item.is_active == 0,
                _ => true,
            };

            matches_search && matches_status
        })
        .collect()
}

fn filter_users<'a>(
    items: &'a [db::UserListItem],
    query: &PageQuery,
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::UserListItem> {
    let search = query.q.as_deref().map(|value| value.trim().to_lowercase());
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let role = query
        .role
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    items
        .iter()
        .filter(|item| user_visible_to(item, current_user, scope_ids))
        .filter(|item| {
            let matches_search = match &search {
                Some(search) => {
                    item.username.to_lowercase().contains(search)
                        || item.email.to_lowercase().contains(search)
                        || item.role.to_lowercase().contains(search)
                        || item
                            .creator_username
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(search)
                }
                None => true,
            };
            let matches_status = match status {
                Some("active") => item.is_active == 1,
                Some("inactive") => item.is_active == 0,
                _ => true,
            };
            let matches_role = match role {
                Some(role) => item.role == role,
                None => true,
            };

            matches_search && matches_status && matches_role
        })
        .collect()
}

fn filter_active_sessions<'a>(
    items: &'a [db::ActiveSessionListItem],
    current_user: &CurrentUser,
    scope_ids: &HashSet<String>,
) -> Vec<&'a db::ActiveSessionListItem> {
    items
        .iter()
        .filter(|item| session_visible_to(item, current_user, scope_ids))
        .collect()
}

fn delivery_filters_from_query(query: &PageQuery) -> db::DeliveryListFilters {
    db::DeliveryListFilters {
        status: optional_field(query.status.clone()),
        source_id: optional_field(query.source_id.clone()),
        provider: optional_field(query.provider.clone()),
        event_type: optional_field(query.event_type.clone()),
        date_from: optional_field(query.date_from.clone()),
        date_to: optional_field(query.date_to.clone()),
        search: optional_field(query.q.clone()),
    }
}

fn build_pagination_view(
    query: &PageQuery,
    current_page: usize,
    total_pages: usize,
) -> PaginationView {
    let safe_total_pages = total_pages.max(1);

    PaginationView {
        current_page,
        total_pages: safe_total_pages,
        has_prev: current_page > 1,
        has_next: current_page < safe_total_pages,
        prev_url: build_delivery_list_url(query, current_page.saturating_sub(1).max(1)),
        next_url: build_delivery_list_url(query, (current_page + 1).min(safe_total_pages)),
    }
}

fn build_delivery_list_url(query: &PageQuery, page: usize) -> String {
    let mut params = Vec::new();

    push_query_param(&mut params, "q", query.q.clone());
    push_query_param(&mut params, "status", query.status.clone());
    push_query_param(&mut params, "source_id", query.source_id.clone());
    push_query_param(&mut params, "provider", query.provider.clone());
    push_query_param(&mut params, "event_type", query.event_type.clone());
    push_query_param(&mut params, "date_from", query.date_from.clone());
    push_query_param(&mut params, "date_to", query.date_to.clone());

    if page > 1 {
        params.push(("page", page.to_string()));
    }

    build_location("/deliveries", &params)
}

fn push_query_param(
    params: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: Option<String>,
) {
    if let Some(value) = optional_field(value) {
        params.push((key, value));
    }
}

fn delivery_row_view(item: &db::DeliveryListItem) -> DeliveryRowView {
    DeliveryRowView {
        short_id: item.short_id.clone(),
        source_label: item
            .source_name
            .clone()
            .unwrap_or_else(|| "Deleted source".to_string()),
        provider: provider_label(&item.provider).to_string(),
        event_type: item.event_type.clone(),
        repository: item.repository.clone(),
        branch: item.branch.clone(),
        status_label: delivery_status_label(&item.status, item.failed_count),
        status_class: delivery_status_class(&item.status, item.failed_count).to_string(),
        dispatch_summary: delivery_dispatch_summary(item.sent_count, item.failed_count),
        failure_excerpt: item
            .failure_reason
            .as_deref()
            .map(|value| excerpt(value, 100))
            .unwrap_or_default(),
        has_failure_reason: item.failure_reason.is_some(),
        received_at: item.received_at.clone(),
        detail_url: format!("/deliveries/{}", item.id),
    }
}

fn delivery_detail_view(item: &db::DeliveryDetail) -> DeliveryDetailView {
    DeliveryDetailView {
        id: item.id.clone(),
        source_label: item
            .source_name
            .clone()
            .unwrap_or_else(|| "Deleted source".to_string()),
        provider: provider_label(&item.provider).to_string(),
        event_type: item.event_type.clone(),
        repository: item.repository.clone(),
        branch: item.branch.clone().unwrap_or_else(|| "n/a".to_string()),
        status_label: delivery_status_label(&item.status, item.failed_count),
        status_class: delivery_status_class(&item.status, item.failed_count).to_string(),
        dispatch_summary: delivery_dispatch_summary(item.sent_count, item.failed_count),
        received_at: item.received_at.clone(),
        processed_at: item
            .processed_at
            .clone()
            .unwrap_or_else(|| "not processed yet".to_string()),
        failure_reason: item.failure_reason.clone().unwrap_or_default(),
        has_failure_reason: item.failure_reason.is_some(),
        partial_failure: item.status == "processed" && item.failed_count > 0,
        raw_headers_pretty: pretty_json(&item.raw_headers),
        raw_payload_pretty: pretty_json(&item.raw_payload),
        normalized_event_pretty: item
            .normalized_event
            .as_deref()
            .map(pretty_json)
            .unwrap_or_else(|| "No normalized event stored.".to_string()),
        has_normalized_event: item.normalized_event.is_some(),
    }
}

fn delivery_message_view(item: &db::DeliveryMessageAttempt) -> DeliveryMessageView {
    DeliveryMessageView {
        destination_label: item
            .destination_name
            .clone()
            .unwrap_or_else(|| "Deleted destination".to_string()),
        status_label: item.status.clone(),
        status_class: message_status_class(&item.status).to_string(),
        response_status: item
            .response_status
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
        attempted_at: item.attempted_at.clone(),
        request_payload_pretty: pretty_json(&item.request_payload),
        response_body_pretty: item
            .response_body
            .as_deref()
            .map(pretty_json)
            .unwrap_or_else(|| "No response body captured.".to_string()),
    }
}

fn source_row_view(request_base_url: &str, item: &db::SourceListItem) -> SourceRowView {
    SourceRowView {
        name: item.name.clone(),
        provider: provider_label(&item.provider).to_string(),
        webhook_url: source_webhook_url(request_base_url, &item.provider, &item.token),
        filters_summary: source_filters_summary(item),
        secret_summary: mask_value(item.webhook_secret.as_deref(), "No secret"),
        status_label: active_label(item.is_active),
        status_class: active_class(item.is_active).to_string(),
        updated_at: item.updated_at.clone(),
        edit_url: format!("/sources?edit={}", item.id),
        delete_url: format!("/sources?delete={}", item.id),
        toggle_action: format!("/sources/{}/toggle", item.id),
        regenerate_action: format!("/sources/{}/regenerate", item.id),
        toggle_label: toggle_label(item.is_active).to_string(),
    }
}

fn destination_row_view(item: &db::DestinationListItem) -> DestinationRowView {
    let parsed = Url::parse(&item.webhook_url).ok();
    let host = parsed
        .as_ref()
        .and_then(Url::host_str)
        .unwrap_or("invalid")
        .to_string();

    DestinationRowView {
        name: item.name.clone(),
        host,
        masked_url: mask_webhook_url(&item.webhook_url),
        status_label: active_label(item.is_active),
        status_class: active_class(item.is_active).to_string(),
        updated_at: item.updated_at.clone(),
        edit_url: format!("/destinations?edit={}", item.id),
        delete_url: format!("/destinations?delete={}", item.id),
        toggle_action: format!("/destinations/{}/toggle", item.id),
        toggle_label: toggle_label(item.is_active).to_string(),
    }
}

fn template_row_view(
    state: &Arc<AppState>,
    item: &db::MessageTemplateListItem,
) -> MessageTemplateRowView {
    let preview_output = state
        .discord
        .render_preview(&item.body_template)
        .map(|rendered| excerpt(&rendered, 160))
        .unwrap_or_else(|_| excerpt(&item.body_template, 160));

    MessageTemplateRowView {
        name: item.name.clone(),
        format_style: item.format_style.clone(),
        excerpt: excerpt(&item.body_template, 120),
        preview_output,
        feature_summary: template_feature_summary(item),
        status_label: active_label(item.is_active),
        status_class: active_class(item.is_active).to_string(),
        updated_at: item.updated_at.clone(),
        edit_url: format!("/templates?edit={}", item.id),
        delete_url: format!("/templates?delete={}", item.id),
        toggle_action: format!("/templates/{}/toggle", item.id),
        toggle_label: toggle_label(item.is_active).to_string(),
    }
}

fn rule_row_view(item: &db::RoutingRuleListItem) -> RuleRowView {
    RuleRowView {
        name: item.name.clone(),
        pipeline_summary: format!(
            "{} -> {} -> {}",
            item.source_name
                .clone()
                .unwrap_or_else(|| "All sources".to_string()),
            item.template_name,
            item.destination_name
        ),
        filters_summary: rule_filters_summary(item),
        sort_order: item.sort_order,
        status_label: active_label(item.is_active),
        status_class: active_class(item.is_active).to_string(),
        updated_at: item.updated_at.clone(),
        edit_url: format!("/rules?edit={}", item.id),
        delete_url: format!("/rules?delete={}", item.id),
        toggle_action: format!("/rules/{}/toggle", item.id),
        toggle_label: toggle_label(item.is_active).to_string(),
    }
}

fn user_row_view(item: &db::UserListItem, can_manage: bool) -> UserRowView {
    UserRowView {
        username: item.username.clone(),
        email: item.email.clone(),
        role_label: role_label(&item.role).to_string(),
        role_class: role_class(&item.role).to_string(),
        status_label: active_label(item.is_active),
        status_class: active_class(item.is_active).to_string(),
        owner_summary: user_owner_summary(item),
        scope_summary: user_scope_summary(item),
        session_summary: user_session_summary(item),
        created_at: item.created_at.clone(),
        updated_at: item.updated_at.clone(),
        edit_url: format!("/users?edit={}", item.id),
        delete_url: format!("/users?delete={}", item.id),
        toggle_action: format!("/users/{}/toggle", item.id),
        toggle_label: toggle_label(item.is_active).to_string(),
        can_manage,
    }
}

fn active_session_row_view(
    item: &db::ActiveSessionListItem,
    current_session_id: &str,
) -> ActiveSessionRowView {
    ActiveSessionRowView {
        username: item.username.clone(),
        email: item.email.clone(),
        role_label: role_label(&item.role).to_string(),
        role_class: role_class(&item.role).to_string(),
        ip_address: item
            .ip_address
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        user_agent: item
            .user_agent
            .as_deref()
            .map(|value| excerpt(value, 72))
            .unwrap_or_else(|| "No user agent".to_string()),
        created_at: item.created_at.clone(),
        last_seen_at: item.last_seen_at.clone(),
        expires_at: item.expires_at.clone(),
        is_current: item.id == current_session_id,
    }
}

fn default_source_form() -> SourceFormView {
    SourceFormView {
        action: "/sources".to_string(),
        title: "Create source".to_string(),
        submit_label: "Save source".to_string(),
        name: String::new(),
        provider: "github".to_string(),
        webhook_secret: String::new(),
        repository_filter: String::new(),
        allowed_branches: String::new(),
        allowed_events: String::new(),
        is_active: true,
        token: String::new(),
        webhook_url: String::new(),
        regenerate_action: String::new(),
    }
}

fn default_user_form(current_user: &CurrentUser) -> UserFormView {
    let permissions = if current_user.is_superadmin() {
        auth::PermissionSet::for_role("viewer")
    } else {
        auth::PermissionSet::for_role("viewer").intersect(&current_user.permissions)
    };

    UserFormView {
        action: "/users".to_string(),
        title: "Create user".to_string(),
        submit_label: "Save user".to_string(),
        username: String::new(),
        email: String::new(),
        role: "viewer".to_string(),
        parent_user_id: current_user.id.clone(),
        is_active: true,
        sources_read: permissions.sources_read,
        sources_write: permissions.sources_write,
        destinations_read: permissions.destinations_read,
        destinations_write: permissions.destinations_write,
        templates_read: permissions.templates_read,
        templates_write: permissions.templates_write,
        rules_read: permissions.rules_read,
        rules_write: permissions.rules_write,
        deliveries_read: permissions.deliveries_read,
        deliveries_replay: permissions.deliveries_replay,
        users_read: permissions.users_read,
        users_write: permissions.users_write,
        subusers_create: permissions.subusers_create,
    }
}

fn user_form_from_record(item: &db::UserRecord) -> UserFormView {
    let permissions =
        auth::PermissionSet::from_json_or_role(&item.role, item.permissions_json.as_deref());

    UserFormView {
        action: format!("/users/{}/update", item.id),
        title: "Edit user".to_string(),
        submit_label: "Update user".to_string(),
        username: item.username.clone(),
        email: item.email.clone(),
        role: item.role.clone(),
        parent_user_id: item.parent_user_id.clone().unwrap_or_default(),
        is_active: item.is_active == 1,
        sources_read: permissions.sources_read,
        sources_write: permissions.sources_write,
        destinations_read: permissions.destinations_read,
        destinations_write: permissions.destinations_write,
        templates_read: permissions.templates_read,
        templates_write: permissions.templates_write,
        rules_read: permissions.rules_read,
        rules_write: permissions.rules_write,
        deliveries_read: permissions.deliveries_read,
        deliveries_replay: permissions.deliveries_replay,
        users_read: permissions.users_read,
        users_write: permissions.users_write,
        subusers_create: permissions.subusers_create,
    }
}

fn permissions_from_form(form: &UserFormInput) -> auth::PermissionSet {
    auth::PermissionSet {
        sources_read: form.sources_read.is_some(),
        sources_write: form.sources_write.is_some(),
        destinations_read: form.destinations_read.is_some(),
        destinations_write: form.destinations_write.is_some(),
        templates_read: form.templates_read.is_some(),
        templates_write: form.templates_write.is_some(),
        rules_read: form.rules_read.is_some(),
        rules_write: form.rules_write.is_some(),
        deliveries_read: form.deliveries_read.is_some(),
        deliveries_replay: form.deliveries_replay.is_some(),
        users_read: form.users_read.is_some(),
        users_write: form.users_write.is_some(),
        subusers_create: form.subusers_create.is_some(),
    }
}

fn allowed_role_options(current_user: &CurrentUser) -> Vec<SimpleOptionView> {
    ["viewer", "editor", "admin", "superadmin"]
        .into_iter()
        .filter(|role| role_assignable_by(current_user, role))
        .map(|role| SimpleOptionView {
            value: role.to_string(),
            label: role_label(role).to_string(),
        })
        .collect()
}

fn parent_user_options(items: &[db::UserListItem], current_user: &CurrentUser) -> Vec<SimpleOptionView> {
    let mut options = vec![SimpleOptionView {
        value: String::new(),
        label: "No parent".to_string(),
    }];

    options.extend(items.iter().map(|item| SimpleOptionView {
        value: item.id.clone(),
        label: format!("{} ({})", item.username, role_label(&item.role)),
    }));

    if !current_user.is_superadmin() {
        options.retain(|item| item.value.is_empty() || item.value == current_user.id);
    }

    options
}

fn source_form_from_record(request_base_url: &str, item: &db::SourceListItem) -> SourceFormView {
    SourceFormView {
        action: format!("/sources/{}/update", item.id),
        title: "Edit source".to_string(),
        submit_label: "Update source".to_string(),
        name: item.name.clone(),
        provider: item.provider.clone(),
        webhook_secret: item.webhook_secret.clone().unwrap_or_default(),
        repository_filter: item.repository_filter.clone().unwrap_or_default(),
        allowed_branches: item.allowed_branches.clone().unwrap_or_default(),
        allowed_events: item.allowed_events.clone().unwrap_or_default(),
        is_active: item.is_active == 1,
        token: item.token.clone(),
        webhook_url: source_webhook_url(request_base_url, &item.provider, &item.token),
        regenerate_action: format!("/sources/{}/regenerate", item.id),
    }
}

fn default_destination_form() -> DestinationFormView {
    DestinationFormView {
        action: "/destinations".to_string(),
        title: "Create destination".to_string(),
        submit_label: "Save destination".to_string(),
        name: String::new(),
        webhook_url: String::new(),
        is_active: true,
    }
}

fn destination_form_from_record(item: &db::DestinationListItem) -> DestinationFormView {
    DestinationFormView {
        action: format!("/destinations/{}/update", item.id),
        title: "Edit destination".to_string(),
        submit_label: "Update destination".to_string(),
        name: item.name.clone(),
        webhook_url: item.webhook_url.clone(),
        is_active: item.is_active == 1,
    }
}

fn empty_message_template_form() -> MessageTemplateFormView {
    MessageTemplateFormView {
        action: String::new(),
        title: String::new(),
        submit_label: String::new(),
        name: String::new(),
        format_style: "custom".to_string(),
        body_template: String::new(),
        embed_color: "#FF7000".to_string(),
        username_override: String::new(),
        avatar_url_override: String::new(),
        footer_text: String::new(),
        show_avatar: true,
        show_repo_link: true,
        show_branch: true,
        show_commits: true,
        show_status_badge: true,
        show_timestamp: true,
        is_active: true,
        preview_output: String::new(),
    }
}

fn default_message_template_form(state: &Arc<AppState>) -> MessageTemplateFormView {
    let body_template = default_preview_template().to_string();
    let preview_output = state
        .discord
        .render_preview(&body_template)
        .unwrap_or_else(|error| error.to_string());

    MessageTemplateFormView {
        action: "/templates".to_string(),
        title: "Create template".to_string(),
        submit_label: "Save template".to_string(),
        name: "Custom Template".to_string(),
        format_style: "custom".to_string(),
        body_template,
        embed_color: "#FF7000".to_string(),
        username_override: String::new(),
        avatar_url_override: String::new(),
        footer_text: String::new(),
        show_avatar: true,
        show_repo_link: true,
        show_branch: true,
        show_commits: true,
        show_status_badge: true,
        show_timestamp: true,
        is_active: true,
        preview_output,
    }
}

fn message_template_form_from_record(
    state: &Arc<AppState>,
    item: &db::MessageTemplateListItem,
) -> Result<MessageTemplateFormView, AppError> {
    let preview_output = state.discord.render_preview(&item.body_template)?;

    Ok(MessageTemplateFormView {
        action: format!("/templates/{}/update", item.id),
        title: "Edit template".to_string(),
        submit_label: "Update template".to_string(),
        name: item.name.clone(),
        format_style: item.format_style.clone(),
        body_template: item.body_template.clone(),
        embed_color: item.embed_color.clone().unwrap_or_default(),
        username_override: item.username_override.clone().unwrap_or_default(),
        avatar_url_override: item.avatar_url_override.clone().unwrap_or_default(),
        footer_text: item.footer_text.clone().unwrap_or_default(),
        show_avatar: item.show_avatar == 1,
        show_repo_link: item.show_repo_link == 1,
        show_branch: item.show_branch == 1,
        show_commits: item.show_commits == 1,
        show_status_badge: item.show_status_badge == 1,
        show_timestamp: item.show_timestamp == 1,
        is_active: item.is_active == 1,
        preview_output,
    })
}

fn message_template_form_from_input(
    state: &Arc<AppState>,
    action: String,
    title: &str,
    submit_label: &str,
    form: &MessageTemplateFormInput,
) -> MessageTemplateFormView {
    let body_template = form.body_template.clone();
    let preview_output = if body_template.trim().is_empty() {
        String::new()
    } else {
        state
            .discord
            .render_preview(&body_template)
            .unwrap_or_else(|error| error.to_string())
    };

    MessageTemplateFormView {
        action,
        title: title.to_string(),
        submit_label: submit_label.to_string(),
        name: form.name.clone(),
        format_style: form.format_style.clone(),
        body_template,
        embed_color: form.embed_color.clone().unwrap_or_default(),
        username_override: form.username_override.clone().unwrap_or_default(),
        avatar_url_override: form.avatar_url_override.clone().unwrap_or_default(),
        footer_text: form.footer_text.clone().unwrap_or_default(),
        show_avatar: form.show_avatar.is_some(),
        show_repo_link: form.show_repo_link.is_some(),
        show_branch: form.show_branch.is_some(),
        show_commits: form.show_commits.is_some(),
        show_status_badge: form.show_status_badge.is_some(),
        show_timestamp: form.show_timestamp.is_some(),
        is_active: form.is_active.is_some(),
        preview_output,
    }
}

fn default_rule_form() -> RuleFormView {
    RuleFormView {
        action: "/rules".to_string(),
        title: "Create rule".to_string(),
        submit_label: "Save rule".to_string(),
        test_context: "create".to_string(),
        test_rule_id: String::new(),
        name: String::new(),
        source_id: String::new(),
        destination_id: String::new(),
        template_id: String::new(),
        provider_filter: String::new(),
        event_type_filter: String::new(),
        branch_prefix_filter: String::new(),
        repository_filter: String::new(),
        skip_keyword: String::new(),
        sort_order: "0".to_string(),
        is_active: true,
        show_advanced: false,
    }
}

fn rule_form_from_record(
    item: &db::RoutingRuleListItem,
    sources: &[db::SourceListItem],
    destinations: &[db::DestinationListItem],
    templates: &[db::MessageTemplateListItem],
) -> RuleFormView {
    RuleFormView {
        action: format!("/rules/{}/update", item.id),
        title: "Edit rule".to_string(),
        submit_label: "Update rule".to_string(),
        test_context: "edit".to_string(),
        test_rule_id: item.id.clone(),
        name: item.name.clone(),
        source_id: item.source_id.clone().unwrap_or_default(),
        destination_id: item.destination_id.clone(),
        template_id: item.template_id.clone(),
        provider_filter: item.provider_filter.clone().unwrap_or_default(),
        event_type_filter: item.event_type_filter.clone().unwrap_or_default(),
        branch_prefix_filter: item.branch_prefix_filter.clone().unwrap_or_default(),
        repository_filter: item.repository_filter.clone().unwrap_or_default(),
        skip_keyword: item.skip_keyword.clone().unwrap_or_default(),
        sort_order: item.sort_order.to_string(),
        is_active: item.is_active == 1,
        show_advanced: rule_record_has_advanced_values(item, sources, destinations, templates),
    }
}

fn rule_form_from_input(
    action: String,
    title: &str,
    submit_label: &str,
    form: &RuleFormInput,
) -> RuleFormView {
    RuleFormView {
        action,
        title: title.to_string(),
        submit_label: submit_label.to_string(),
        test_context: form.test_context.clone().unwrap_or_else(|| {
            if title == "Edit rule" {
                "edit".to_string()
            } else {
                "create".to_string()
            }
        }),
        test_rule_id: form.test_rule_id.clone().unwrap_or_default(),
        name: form.name.clone().unwrap_or_default(),
        source_id: form.source_id.clone().unwrap_or_default(),
        destination_id: form.destination_id.clone(),
        template_id: form.template_id.clone(),
        provider_filter: form.provider_filter.clone().unwrap_or_default(),
        event_type_filter: form.event_type_filter.clone().unwrap_or_default(),
        branch_prefix_filter: form.branch_prefix_filter.clone().unwrap_or_default(),
        repository_filter: form.repository_filter.clone().unwrap_or_default(),
        skip_keyword: form.skip_keyword.clone().unwrap_or_default(),
        sort_order: form.sort_order.clone().unwrap_or_default(),
        is_active: form.is_active.is_some(),
        show_advanced: rule_form_has_advanced_values(form),
    }
}

fn rule_form_has_advanced_values(form: &RuleFormInput) -> bool {
    optional_field(form.name.clone()).is_some()
        || optional_field(form.provider_filter.clone()).is_some()
        || optional_field(form.event_type_filter.clone()).is_some()
        || optional_field(form.branch_prefix_filter.clone()).is_some()
        || optional_field(form.repository_filter.clone()).is_some()
        || optional_field(form.skip_keyword.clone()).is_some()
        || optional_field(form.sort_order.clone())
            .map(|value| value != "0")
            .unwrap_or(false)
        || form.is_active.is_none()
}

fn rule_record_has_advanced_values(
    item: &db::RoutingRuleListItem,
    sources: &[db::SourceListItem],
    destinations: &[db::DestinationListItem],
    templates: &[db::MessageTemplateListItem],
) -> bool {
    let generated_name = build_rule_name(
        item.source_id.as_deref(),
        &item.destination_id,
        &item.template_id,
        item.provider_filter.as_deref(),
        sources,
        destinations,
        templates,
    );

    item.name != generated_name
        || item.provider_filter.is_some()
        || item.event_type_filter.is_some()
        || item.branch_prefix_filter.is_some()
        || item.repository_filter.is_some()
        || item.skip_keyword.is_some()
        || item.sort_order != 0
        || item.is_active == 0
}

fn rule_test_form_state(form: &RuleFormInput) -> (bool, RuleFormView, bool, RuleFormView) {
    let is_edit = matches!(form.test_context.as_deref(), Some("edit"));
    let action = if is_edit {
        optional_field(form.test_rule_id.clone())
            .map(|id| format!("/rules/{id}/update"))
            .unwrap_or_else(|| "/rules".to_string())
    } else {
        "/rules".to_string()
    };
    let view = rule_form_from_input(
        action,
        if is_edit { "Edit rule" } else { "Create rule" },
        if is_edit { "Update rule" } else { "Save rule" },
        form,
    );

    if is_edit {
        (false, default_rule_form(), true, view)
    } else {
        (true, view, false, default_rule_form())
    }
}

fn build_rule_webhook_test_request(
    input: &db::NewRoutingRule,
    source: Option<&db::SourceListItem>,
) -> webhook::RouteWebhookTestRequest {
    let provider = source
        .map(|item| item.provider.clone())
        .or_else(|| input.provider_filter.clone())
        .unwrap_or_else(|| "github".to_string());
    let repository = input
        .repository_filter
        .clone()
        .or_else(|| source.and_then(|item| optional_field(item.repository_filter.clone())))
        .unwrap_or_else(|| "acme/dmxforge".to_string());
    let event_type = first_filter_token(input.event_type_filter.as_deref())
        .or_else(|| source.and_then(|item| first_filter_token(item.allowed_events.as_deref())))
        .unwrap_or_else(|| "push".to_string());
    let branch = input
        .branch_prefix_filter
        .as_deref()
        .map(sample_branch_from_pattern)
        .or_else(|| {
            source
                .and_then(|item| first_filter_token(item.allowed_branches.as_deref()))
                .map(|value| sample_branch_from_pattern(&value))
        })
        .unwrap_or_else(|| "main".to_string());

    webhook::RouteWebhookTestRequest {
        provider,
        event_type,
        repository,
        branch: Some(branch),
    }
}

fn source_options(request_base_url: &str, items: &[db::SourceListItem]) -> Vec<SelectOptionView> {
    let mut options = vec![SelectOptionView {
        value: String::new(),
        label: "All sources".to_string(),
        meta: String::new(),
    }];

    options.extend(items.iter().map(|item| SelectOptionView {
        value: item.id.clone(),
        label: item.name.clone(),
        meta: format!(
            "{} | {}",
            provider_label(&item.provider),
            source_webhook_url(request_base_url, &item.provider, &item.token)
        ),
    }));

    options
}

fn destination_options(items: &[db::DestinationListItem]) -> Vec<SelectOptionView> {
    items
        .iter()
        .map(|item| SelectOptionView {
            value: item.id.clone(),
            label: item.name.clone(),
            meta: item.webhook_url.clone(),
        })
        .collect()
}

fn template_options(
    state: &Arc<AppState>,
    items: &[db::MessageTemplateListItem],
) -> Vec<TemplateOptionView> {
    items
        .iter()
        .map(|item| {
            let preview_output = state
                .discord
                .render_preview(&item.body_template)
                .map(|rendered| excerpt(&rendered, 220))
                .unwrap_or_else(|_| excerpt(&item.body_template, 220));

            TemplateOptionView {
                value: item.id.clone(),
                label: item.name.clone(),
                meta: item.format_style.clone(),
                preview_output,
                format_style: item.format_style.clone(),
                embed_color: item
                    .embed_color
                    .clone()
                    .unwrap_or_else(|| "#FF7000".to_string()),
                footer_text: item.footer_text.clone().unwrap_or_default(),
                username_override: item.username_override.clone().unwrap_or_default(),
                avatar_url_override: item.avatar_url_override.clone().unwrap_or_default(),
                show_avatar: item.show_avatar == 1,
                show_repo_link: item.show_repo_link == 1,
                show_branch: item.show_branch == 1,
                show_commits: item.show_commits == 1,
                show_status_badge: item.show_status_badge == 1,
                show_timestamp: item.show_timestamp == 1,
            }
        })
        .collect()
}

fn build_source_payload(form: SourceFormInput) -> Result<db::NewSource, String> {
    let name = required_field(form.name, "Source name")?;
    let provider = validate_provider(required_field(form.provider, "Provider")?.as_str())?;

    Ok(db::NewSource {
        user_id: None,
        name,
        provider: provider.to_string(),
        webhook_secret: optional_field(form.webhook_secret),
        repository_filter: optional_field(form.repository_filter),
        allowed_branches: optional_field(form.allowed_branches),
        allowed_events: optional_field(form.allowed_events),
        is_active: form.is_active.is_some(),
    })
}

fn build_destination_payload(form: DestinationFormInput) -> Result<db::NewDestination, String> {
    let name = required_field(form.name, "Destination name")?;
    let webhook_url = required_field(form.webhook_url, "Discord webhook URL")?;
    validate_webhook_url(&webhook_url).map_err(|error| error.to_string())?;

    Ok(db::NewDestination {
        user_id: None,
        name,
        webhook_url,
        is_active: form.is_active.is_some(),
    })
}

fn build_message_template_payload(
    form: MessageTemplateFormInput,
) -> Result<db::NewMessageTemplate, String> {
    let name = required_field(form.name, "Template name")?;
    let format_style_raw = required_field(form.format_style, "Format style")?;
    let format_style = validate_format_style(&format_style_raw)?;
    let body_template = required_field(form.body_template, "Body template")?;
    let embed_color = optional_field(form.embed_color)
        .map(validate_hex_color)
        .transpose()?;
    let avatar_url_override = optional_field(form.avatar_url_override)
        .map(|value| validate_url(value, "Avatar URL"))
        .transpose()?;

    Ok(db::NewMessageTemplate {
        user_id: None,
        name,
        format_style: format_style.to_string(),
        body_template,
        embed_color,
        username_override: optional_field(form.username_override),
        avatar_url_override,
        footer_text: optional_field(form.footer_text),
        show_avatar: form.show_avatar.is_some(),
        show_repo_link: form.show_repo_link.is_some(),
        show_branch: form.show_branch.is_some(),
        show_commits: form.show_commits.is_some(),
        show_status_badge: form.show_status_badge.is_some(),
        show_timestamp: form.show_timestamp.is_some(),
        is_active: form.is_active.is_some(),
    })
}

fn build_rule_payload(
    form: RuleFormInput,
    sources: &[db::SourceListItem],
    destinations: &[db::DestinationListItem],
    templates: &[db::MessageTemplateListItem],
) -> Result<db::NewRoutingRule, String> {
    let destination_id = required_field(form.destination_id, "Destination")?;
    let template_id = required_field(form.template_id, "Template")?;

    if !destinations.iter().any(|item| item.id == destination_id) {
        return Err("Selected destination does not exist.".to_string());
    }

    if !templates.iter().any(|item| item.id == template_id) {
        return Err("Selected template does not exist.".to_string());
    }

    let source_id = optional_field(form.source_id);
    if let Some(source_id) = source_id.as_deref() {
        if !sources.iter().any(|item| item.id == source_id) {
            return Err("Selected source does not exist.".to_string());
        }
    }

    let provider_filter = optional_field(form.provider_filter)
        .map(|value| validate_provider(&value).map(|provider| provider.to_string()))
        .transpose()?;
    let sort_order = optional_field(form.sort_order)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| "Sort order must be a valid integer.".to_string())
        })
        .transpose()?
        .unwrap_or(0);
    let name = optional_field(form.name).unwrap_or_else(|| {
        build_rule_name(
            source_id.as_deref(),
            &destination_id,
            &template_id,
            provider_filter.as_deref(),
            sources,
            destinations,
            templates,
        )
    });

    Ok(db::NewRoutingRule {
        user_id: None,
        name,
        source_id,
        destination_id,
        template_id,
        provider_filter,
        event_type_filter: optional_field(form.event_type_filter),
        branch_prefix_filter: optional_field(form.branch_prefix_filter),
        repository_filter: optional_field(form.repository_filter),
        skip_keyword: optional_field(form.skip_keyword),
        sort_order,
        is_active: form.is_active.is_some(),
    })
}

fn build_user_payload(
    current_user: &CurrentUser,
    form: &UserFormInput,
    password_required: bool,
) -> Result<UserMutationPayload, String> {
    let username = required_field(form.username.clone(), "Username")?;
    let email = required_field(form.email.clone(), "Email")?;
    let role = validate_user_role(&required_field(form.role.clone(), "Role")?)?.to_string();

    if !role_assignable_by(current_user, &role) {
        return Err("You cannot assign a role higher than your own.".to_string());
    }

    let raw_permissions = permissions_from_form(form);
    let role_permissions = auth::PermissionSet::for_role(&role);
    let delegated_permissions = if current_user.is_superadmin() {
        raw_permissions.intersect(&role_permissions)
    } else {
        raw_permissions
            .intersect(&current_user.permissions)
            .intersect(&role_permissions)
    };
    let permissions_json = Some(
        delegated_permissions
            .to_json()
            .map_err(|error| error.to_string())?,
    );

    let password = form.password.clone().unwrap_or_default();
    let password_confirmation = form.password_confirmation.clone().unwrap_or_default();
    let password_hash = if password_required || !password.trim().is_empty() {
        auth::validate_password_rules(&password, &password_confirmation)?;
        Some(auth::hash_password(&password).map_err(|error| error.to_string())?)
    } else {
        None
    };

    let parent_user_id = if current_user.is_superadmin() {
        optional_field(form.parent_user_id.clone())
    } else {
        Some(current_user.id.clone())
    };

    Ok(UserMutationPayload {
        username,
        email,
        role,
        is_active: form.is_active.is_some(),
        parent_user_id,
        created_by_user_id: Some(current_user.id.clone()),
        permissions_json,
        password_hash,
    })
}

fn build_rule_name(
    source_id: Option<&str>,
    destination_id: &str,
    template_id: &str,
    provider_filter: Option<&str>,
    sources: &[db::SourceListItem],
    destinations: &[db::DestinationListItem],
    templates: &[db::MessageTemplateListItem],
) -> String {
    let source_label = source_id
        .and_then(|id| sources.iter().find(|item| item.id == id))
        .map(|item| item.name.clone())
        .unwrap_or_else(|| "All sources".to_string());
    let destination_label = destinations
        .iter()
        .find(|item| item.id == destination_id)
        .map(|item| item.name.clone())
        .unwrap_or_else(|| "Destination".to_string());
    let template_label = templates
        .iter()
        .find(|item| item.id == template_id)
        .map(|item| item.name.clone())
        .unwrap_or_else(|| "Template".to_string());

    match provider_filter {
        Some(provider) if !provider.is_empty() => format!(
            "{} -> {} -> {} ({})",
            source_label,
            template_label,
            destination_label,
            provider_label(provider)
        ),
        _ => format!("{source_label} -> {template_label} -> {destination_label}"),
    }
}

fn redirect_with_notice(path: &str, level: &str, message: &str) -> Redirect {
    let location = build_location(
        path,
        &[
            ("notice", message.to_string()),
            ("notice_level", level.to_string()),
        ],
    );
    Redirect::to(&location)
}

fn redirect_with_notice_response(path: &str, level: &str, message: &str) -> Response {
    redirect_with_notice(path, level, message).into_response()
}

fn redirect_with_modal_notice(
    path: &str,
    key: &str,
    value: &str,
    level: &str,
    message: &str,
) -> Redirect {
    let location = build_location(
        path,
        &[
            (key, value.to_string()),
            ("notice", message.to_string()),
            ("notice_level", level.to_string()),
        ],
    );
    Redirect::to(&location)
}

fn build_location(path: &str, params: &[(&str, String)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        serializer.append_pair(key, value);
    }
    let query = serializer.finish();

    if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    }
}

fn request_base_url(headers: &HeaderMap) -> String {
    let scheme = first_header_value(headers, "x-forwarded-proto")
        .or_else(|| forwarded_header_value(headers, "proto"))
        .unwrap_or_else(|| "http".to_string());
    let host = first_header_value(headers, "x-forwarded-host")
        .or_else(|| forwarded_header_value(headers, "host"))
        .or_else(|| first_header_value(headers, "host"))
        .unwrap_or_else(|| "127.0.0.1:3000".to_string());

    format!("{}://{}", scheme.trim(), host.trim())
}

fn first_header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn forwarded_header_value(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get("forwarded")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|entry| {
            entry.split(';').find_map(|part| {
                let (candidate_key, candidate_value) = part.trim().split_once('=')?;
                if candidate_key.eq_ignore_ascii_case(key) {
                    let cleaned = candidate_value.trim().trim_matches('"');
                    (!cleaned.is_empty()).then(|| cleaned.to_string())
                } else {
                    None
                }
            })
        })
}

fn source_webhook_url(base_url: &str, provider: &str, token: &str) -> String {
    format!(
        "{}/webhooks/{provider}/{token}",
        base_url.trim_end_matches('/')
    )
}

fn source_filters_summary(item: &db::SourceListItem) -> String {
    let mut parts = Vec::new();

    if let Some(value) = item
        .repository_filter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("repo: {}", value.trim()));
    }
    if let Some(value) = item
        .allowed_branches
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("branches: {}", compact_multiline(value)));
    }
    if let Some(value) = item
        .allowed_events
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("events: {}", compact_multiline(value)));
    }

    if parts.is_empty() {
        "No filters".to_string()
    } else {
        parts.join(" | ")
    }
}

fn template_feature_summary(item: &db::MessageTemplateListItem) -> String {
    let mut flags = Vec::new();

    if item.show_avatar == 1 {
        flags.push("avatar");
    }
    if item.show_repo_link == 1 {
        flags.push("repo link");
    }
    if item.show_branch == 1 {
        flags.push("branch");
    }
    if item.show_commits == 1 {
        flags.push("commits");
    }
    if item.show_status_badge == 1 {
        flags.push("status");
    }
    if item.show_timestamp == 1 {
        flags.push("timestamp");
    }

    if flags.is_empty() {
        "No optional widgets".to_string()
    } else {
        flags.join(", ")
    }
}

fn rule_filters_summary(item: &db::RoutingRuleListItem) -> String {
    let mut parts = Vec::new();

    if let Some(value) = item.provider_filter.as_deref() {
        parts.push(format!("provider: {}", value));
    }
    if let Some(value) = item.event_type_filter.as_deref() {
        parts.push(format!("event: {}", value));
    }
    if let Some(value) = item.branch_prefix_filter.as_deref() {
        parts.push(format!("branch: {}", value));
    }
    if let Some(value) = item.repository_filter.as_deref() {
        parts.push(format!("repo: {}", value));
    }
    if let Some(value) = item.skip_keyword.as_deref() {
        parts.push(format!("skip: {}", value));
    }

    if parts.is_empty() {
        "No additional filters".to_string()
    } else {
        parts.join(" | ")
    }
}

fn user_owner_summary(item: &db::UserListItem) -> String {
    let creator = item
        .creator_username
        .clone()
        .unwrap_or_else(|| "system".to_string());
    let parent = item
        .parent_username
        .clone()
        .unwrap_or_else(|| "none".to_string());

    format!("created by {creator} | parent {parent}")
}

fn user_scope_summary(item: &db::UserListItem) -> String {
    format!(
        "{} child | {} src | {} dst | {} tpl | {} rules",
        item.child_count,
        item.source_count,
        item.destination_count,
        item.template_count,
        item.rule_count
    )
}

fn user_session_summary(item: &db::UserListItem) -> String {
    match item.active_session_count {
        0 => "No live session".to_string(),
        count => {
            let suffix = item
                .last_seen_at
                .clone()
                .map(|value| format!(" | last seen {value}"))
                .unwrap_or_default();
            format!("{count} live{suffix}")
        }
    }
}

fn compact_multiline(value: &str) -> String {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn mask_value(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => {
            let value = value.trim();
            let tail = &value[value.len().saturating_sub(4)..];
            format!("••••{}", tail)
        }
        _ => fallback.to_string(),
    }
}

fn mask_webhook_url(value: &str) -> String {
    match Url::parse(value) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("discord.com");
            let segments = url
                .path_segments()
                .map(|segments| segments.collect::<Vec<_>>())
                .unwrap_or_default();
            let id = segments.get(2).copied().unwrap_or("...");
            format!("https://{host}/api/webhooks/{id}/••••••")
        }
        Err(_) => value.to_string(),
    }
}

fn excerpt(value: &str, limit: usize) -> String {
    let collapsed = value.replace('\n', " ");
    if collapsed.len() <= limit {
        collapsed
    } else {
        format!("{}...", &collapsed[..limit])
    }
}

fn pretty_json(value: &str) -> String {
    serde_json::from_str::<serde_json::Value>(value)
        .and_then(|json| serde_json::to_string_pretty(&json))
        .unwrap_or_else(|_| value.to_string())
}

fn active_label(is_active: i64) -> String {
    if is_active == 1 {
        "active".to_string()
    } else {
        "inactive".to_string()
    }
}

fn active_class(is_active: i64) -> &'static str {
    if is_active == 1 { "success" } else { "warning" }
}

fn toggle_label(is_active: i64) -> &'static str {
    if is_active == 1 { "Disable" } else { "Enable" }
}

fn delivery_status_label(status: &str, failed_count: i64) -> String {
    if status == "processed" && failed_count > 0 {
        "partial".to_string()
    } else {
        status.to_string()
    }
}

fn delivery_status_class(status: &str, failed_count: i64) -> &'static str {
    match status {
        "processed" if failed_count > 0 => "warning",
        "processed" => "success",
        "failed" => "danger",
        "skipped" => "warning",
        "pending" => "pending",
        _ => "info",
    }
}

fn message_status_class(status: &str) -> &'static str {
    match status {
        "sent" => "success",
        "failed" => "danger",
        _ => "info",
    }
}

fn delivery_dispatch_summary(sent_count: i64, failed_count: i64) -> String {
    match (sent_count, failed_count) {
        (0, 0) => "No Discord attempt".to_string(),
        (_, 0) => format!("{sent_count} sent"),
        (0, _) => format!("{failed_count} failed"),
        _ => format!("{sent_count} sent / {failed_count} failed"),
    }
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "github" => "GitHub",
        "gitlab" => "GitLab",
        "gitea" => "Gitea",
        _ => "Unknown",
    }
}

fn role_label(role: &str) -> &'static str {
    match role {
        "superadmin" => "Superadmin",
        "admin" => "Admin",
        "editor" => "Editor",
        "viewer" => "Viewer",
        _ => "Unknown",
    }
}

fn role_class(role: &str) -> &'static str {
    match role {
        "superadmin" => "success",
        "admin" => "warning",
        "editor" => "info",
        "viewer" => "pending",
        _ => "info",
    }
}

fn required_field(value: String, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{field_name} is required."))
    } else {
        Ok(trimmed.to_string())
    }
}

fn optional_field(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn first_filter_token(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        raw.split([',', '\n'])
            .map(str::trim)
            .find(|item| !item.is_empty())
            .map(str::to_string)
    })
}

fn sample_branch_from_pattern(pattern: &str) -> String {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return "main".to_string();
    }

    if let Some(prefix) = trimmed.strip_suffix("/*") {
        return format!("{prefix}/test");
    }

    if trimmed.ends_with('*') {
        return format!("{}test", trimmed.trim_end_matches('*'));
    }

    if trimmed.ends_with('/') {
        return format!("{trimmed}test");
    }

    trimmed.to_string()
}

fn validate_provider(input: &str) -> Result<&'static str, String> {
    match input {
        "github" => Ok("github"),
        "gitlab" => Ok("gitlab"),
        "gitea" | "forgejo" => Ok("gitea"),
        _ => Err("Provider must be github, gitlab or gitea.".to_string()),
    }
}

fn validate_user_role(input: &str) -> Result<&'static str, String> {
    match input {
        "superadmin" => Ok("superadmin"),
        "admin" => Ok("admin"),
        "editor" => Ok("editor"),
        "viewer" => Ok("viewer"),
        _ => Err("Role must be superadmin, admin, editor or viewer.".to_string()),
    }
}

fn validate_format_style(input: &str) -> Result<&str, String> {
    match input {
        "compact" | "detailed" | "release" | "alert" | "custom" => Ok(input),
        _ => Err("Format style is invalid.".to_string()),
    }
}

fn validate_hex_color(input: String) -> Result<String, String> {
    let normalized = input.trim();
    let is_valid = normalized.len() == 7
        && normalized.starts_with('#')
        && normalized
            .chars()
            .skip(1)
            .all(|value| value.is_ascii_hexdigit());

    if is_valid {
        Ok(normalized.to_uppercase())
    } else {
        Err("Embed color must use the #RRGGBB format.".to_string())
    }
}

fn validate_url(input: String, field_name: &str) -> Result<String, String> {
    Url::parse(&input)
        .map(|_| input)
        .map_err(|_| format!("{field_name} must be a valid absolute URL."))
}

fn user_write_error_message(error: &anyhow::Error) -> String {
    let message = error.to_string();
    if message.contains("UNIQUE constraint failed") {
        "Username or email is already in use.".to_string()
    } else {
        "Failed to persist user changes.".to_string()
    }
}

struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(rendered) => Html(rendered).into_response(),
            Err(error) => AppError::internal(error).into_response(),
        }
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    app_name: String,
    version: String,
    database: String,
    timestamp: String,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct PageQuery {
    q: Option<String>,
    status: Option<String>,
    role: Option<String>,
    source_id: Option<String>,
    provider: Option<String>,
    event_type: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    page: Option<usize>,
    format_style: Option<String>,
    modal: Option<String>,
    edit: Option<String>,
    delete: Option<String>,
    notice: Option<String>,
    notice_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginFormInput {
    guest_csrf_token: String,
    login: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct SetupFormInput {
    guest_csrf_token: String,
    username: String,
    email: String,
    password: String,
    password_confirmation: String,
}

#[derive(Debug, Deserialize, Clone)]
struct UserFormInput {
    csrf_token: String,
    username: String,
    email: String,
    role: String,
    password: Option<String>,
    password_confirmation: Option<String>,
    parent_user_id: Option<String>,
    is_active: Option<String>,
    sources_read: Option<String>,
    sources_write: Option<String>,
    destinations_read: Option<String>,
    destinations_write: Option<String>,
    templates_read: Option<String>,
    templates_write: Option<String>,
    rules_read: Option<String>,
    rules_write: Option<String>,
    deliveries_read: Option<String>,
    deliveries_replay: Option<String>,
    users_read: Option<String>,
    users_write: Option<String>,
    subusers_create: Option<String>,
}

#[derive(Debug, Clone)]
struct UserMutationPayload {
    username: String,
    email: String,
    role: String,
    is_active: bool,
    parent_user_id: Option<String>,
    created_by_user_id: Option<String>,
    permissions_json: Option<String>,
    password_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LogoutFormInput {
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct CsrfFormInput {
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct SourceFormInput {
    csrf_token: String,
    name: String,
    provider: String,
    webhook_secret: Option<String>,
    repository_filter: Option<String>,
    allowed_branches: Option<String>,
    allowed_events: Option<String>,
    is_active: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DestinationFormInput {
    csrf_token: String,
    name: String,
    webhook_url: String,
    is_active: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MessageTemplateFormInput {
    csrf_token: String,
    name: String,
    format_style: String,
    body_template: String,
    embed_color: Option<String>,
    username_override: Option<String>,
    avatar_url_override: Option<String>,
    footer_text: Option<String>,
    show_avatar: Option<String>,
    show_repo_link: Option<String>,
    show_branch: Option<String>,
    show_commits: Option<String>,
    show_status_badge: Option<String>,
    show_timestamp: Option<String>,
    is_active: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct RuleFormInput {
    csrf_token: String,
    test_context: Option<String>,
    test_rule_id: Option<String>,
    name: Option<String>,
    source_id: Option<String>,
    destination_id: String,
    template_id: String,
    provider_filter: Option<String>,
    event_type_filter: Option<String>,
    branch_prefix_filter: Option<String>,
    repository_filter: Option<String>,
    skip_keyword: Option<String>,
    sort_order: Option<String>,
    is_active: Option<String>,
}

#[derive(Debug, Clone)]
struct AppShellView {
    app_name: String,
    page_eyebrow: String,
    page_title: String,
    page_summary: String,
    nav_items: Vec<NavItemView>,
    current_user: CurrentUser,
    flash: FlashView,
}

#[derive(Debug, Clone)]
struct NavItemView {
    label: String,
    href: String,
    active: bool,
}

#[derive(Debug, Clone)]
struct FlashView {
    has_notice: bool,
    level_class: String,
    message: String,
}

#[derive(Debug, Clone)]
struct LoginFormView {
    login: String,
    guest_csrf_token: String,
}

#[derive(Debug, Clone)]
struct SetupFormView {
    username: String,
    email: String,
    guest_csrf_token: String,
}

impl FlashView {
    fn empty() -> Self {
        Self {
            has_notice: false,
            level_class: "info".to_string(),
            message: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct BasicFilterView {
    q: String,
    status: String,
}

#[derive(Debug, Clone)]
struct UsersFilterView {
    q: String,
    status: String,
    role: String,
}

#[derive(Debug, Clone)]
struct DeliveryFilterView {
    q: String,
    status: String,
    source_id: String,
    provider: String,
    event_type: String,
    date_from: String,
    date_to: String,
}

#[derive(Debug, Clone)]
struct SourceFilterView {
    q: String,
    provider: String,
    status: String,
}

#[derive(Debug, Clone)]
struct TemplateFilterView {
    q: String,
    status: String,
    format_style: String,
}

#[derive(Debug, Clone)]
struct SourceRowView {
    name: String,
    provider: String,
    webhook_url: String,
    filters_summary: String,
    secret_summary: String,
    status_label: String,
    status_class: String,
    updated_at: String,
    edit_url: String,
    delete_url: String,
    toggle_action: String,
    regenerate_action: String,
    toggle_label: String,
}

#[derive(Debug, Clone)]
struct DestinationRowView {
    name: String,
    host: String,
    masked_url: String,
    status_label: String,
    status_class: String,
    updated_at: String,
    edit_url: String,
    delete_url: String,
    toggle_action: String,
    toggle_label: String,
}

#[derive(Debug, Clone)]
struct MessageTemplateRowView {
    name: String,
    format_style: String,
    excerpt: String,
    preview_output: String,
    feature_summary: String,
    status_label: String,
    status_class: String,
    updated_at: String,
    edit_url: String,
    delete_url: String,
    toggle_action: String,
    toggle_label: String,
}

#[derive(Debug, Clone)]
struct RuleRowView {
    name: String,
    pipeline_summary: String,
    filters_summary: String,
    sort_order: i64,
    status_label: String,
    status_class: String,
    updated_at: String,
    edit_url: String,
    delete_url: String,
    toggle_action: String,
    toggle_label: String,
}

#[derive(Debug, Clone)]
struct UserRowView {
    username: String,
    email: String,
    role_label: String,
    role_class: String,
    status_label: String,
    status_class: String,
    owner_summary: String,
    scope_summary: String,
    session_summary: String,
    created_at: String,
    updated_at: String,
    edit_url: String,
    delete_url: String,
    toggle_action: String,
    toggle_label: String,
    can_manage: bool,
}

#[derive(Debug, Clone)]
struct ActiveSessionRowView {
    username: String,
    email: String,
    role_label: String,
    role_class: String,
    ip_address: String,
    user_agent: String,
    created_at: String,
    last_seen_at: String,
    expires_at: String,
    is_current: bool,
}

#[derive(Debug, Clone)]
struct SelectOptionView {
    value: String,
    label: String,
    meta: String,
}

#[derive(Debug, Clone)]
struct TemplateOptionView {
    value: String,
    label: String,
    meta: String,
    preview_output: String,
    format_style: String,
    embed_color: String,
    footer_text: String,
    username_override: String,
    avatar_url_override: String,
    show_avatar: bool,
    show_repo_link: bool,
    show_branch: bool,
    show_commits: bool,
    show_status_badge: bool,
    show_timestamp: bool,
}

#[derive(Debug, Clone)]
struct SimpleOptionView {
    value: String,
    label: String,
}

#[derive(Debug, Clone)]
struct PaginationView {
    current_page: usize,
    total_pages: usize,
    has_prev: bool,
    has_next: bool,
    prev_url: String,
    next_url: String,
}

#[derive(Debug, Clone)]
struct SourceFormView {
    action: String,
    title: String,
    submit_label: String,
    name: String,
    provider: String,
    webhook_secret: String,
    repository_filter: String,
    allowed_branches: String,
    allowed_events: String,
    is_active: bool,
    token: String,
    webhook_url: String,
    regenerate_action: String,
}

#[derive(Debug, Clone)]
struct UserFormView {
    action: String,
    title: String,
    submit_label: String,
    username: String,
    email: String,
    role: String,
    parent_user_id: String,
    is_active: bool,
    sources_read: bool,
    sources_write: bool,
    destinations_read: bool,
    destinations_write: bool,
    templates_read: bool,
    templates_write: bool,
    rules_read: bool,
    rules_write: bool,
    deliveries_read: bool,
    deliveries_replay: bool,
    users_read: bool,
    users_write: bool,
    subusers_create: bool,
}

#[derive(Debug, Clone)]
struct DestinationFormView {
    action: String,
    title: String,
    submit_label: String,
    name: String,
    webhook_url: String,
    is_active: bool,
}

#[derive(Debug, Clone)]
struct MessageTemplateFormView {
    action: String,
    title: String,
    submit_label: String,
    name: String,
    format_style: String,
    body_template: String,
    embed_color: String,
    username_override: String,
    avatar_url_override: String,
    footer_text: String,
    show_avatar: bool,
    show_repo_link: bool,
    show_branch: bool,
    show_commits: bool,
    show_status_badge: bool,
    show_timestamp: bool,
    is_active: bool,
    preview_output: String,
}

#[derive(Debug, Clone)]
struct RuleFormView {
    action: String,
    title: String,
    submit_label: String,
    test_context: String,
    test_rule_id: String,
    name: String,
    source_id: String,
    destination_id: String,
    template_id: String,
    provider_filter: String,
    event_type_filter: String,
    branch_prefix_filter: String,
    repository_filter: String,
    skip_keyword: String,
    sort_order: String,
    is_active: bool,
    show_advanced: bool,
}

#[derive(Debug, Clone)]
struct DeliveryRowView {
    short_id: String,
    source_label: String,
    provider: String,
    event_type: String,
    repository: String,
    branch: String,
    status_label: String,
    status_class: String,
    dispatch_summary: String,
    failure_excerpt: String,
    has_failure_reason: bool,
    received_at: String,
    detail_url: String,
}

#[derive(Debug, Clone)]
struct DeliveryDetailView {
    id: String,
    source_label: String,
    provider: String,
    event_type: String,
    repository: String,
    branch: String,
    status_label: String,
    status_class: String,
    dispatch_summary: String,
    received_at: String,
    processed_at: String,
    failure_reason: String,
    has_failure_reason: bool,
    partial_failure: bool,
    raw_headers_pretty: String,
    raw_payload_pretty: String,
    normalized_event_pretty: String,
    has_normalized_event: bool,
}

#[derive(Debug, Clone)]
struct DeliveryMessageView {
    destination_label: String,
    status_label: String,
    status_class: String,
    response_status: String,
    attempted_at: String,
    request_payload_pretty: String,
    response_body_pretty: String,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    shell: AppShellView,
    form: LoginFormView,
}

#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    shell: AppShellView,
    form: SetupFormView,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    shell: AppShellView,
    total_deliveries: i64,
    processed_deliveries: i64,
    failed_deliveries: i64,
    discord_messages_sent: i64,
    activity: Vec<db::ActivityPoint>,
    top_repositories: Vec<db::TopMetric>,
    top_events: Vec<db::TopMetric>,
    recent_deliveries: Vec<db::RecentDelivery>,
    activity_total_count: i64,
    activity_peak_count: i64,
    activity_peak_label: String,
}

#[derive(Template)]
#[template(path = "sources.html")]
struct SourcesTemplate {
    shell: AppShellView,
    filters: SourceFilterView,
    total_count: usize,
    active_count: usize,
    inactive_count: usize,
    sources: Vec<SourceRowView>,
    show_create_modal: bool,
    create_form: SourceFormView,
    show_edit_modal: bool,
    edit_form: SourceFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
}

#[derive(Template)]
#[template(path = "destinations.html")]
struct DestinationsTemplate {
    shell: AppShellView,
    filters: BasicFilterView,
    total_count: usize,
    active_count: usize,
    inactive_count: usize,
    destinations: Vec<DestinationRowView>,
    show_create_modal: bool,
    create_form: DestinationFormView,
    show_edit_modal: bool,
    edit_form: DestinationFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
}

#[derive(Template)]
#[template(path = "message_templates.html")]
struct MessageTemplatesTemplate {
    shell: AppShellView,
    filters: TemplateFilterView,
    total_count: usize,
    active_count: usize,
    inactive_count: usize,
    templates: Vec<MessageTemplateRowView>,
    show_create_modal: bool,
    create_form: MessageTemplateFormView,
    show_edit_modal: bool,
    edit_form: MessageTemplateFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
}

#[derive(Template)]
#[template(path = "rules.html")]
struct RulesTemplate {
    shell: AppShellView,
    filters: BasicFilterView,
    total_count: usize,
    active_count: usize,
    inactive_count: usize,
    rules: Vec<RuleRowView>,
    source_options: Vec<SelectOptionView>,
    destination_options: Vec<SelectOptionView>,
    template_options: Vec<TemplateOptionView>,
    show_create_modal: bool,
    create_form: RuleFormView,
    show_edit_modal: bool,
    edit_form: RuleFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
    can_create_rule: bool,
}

#[derive(Template)]
#[template(path = "deliveries.html")]
struct DeliveriesTemplate {
    shell: AppShellView,
    filters: DeliveryFilterView,
    total_count: i64,
    visible_from: usize,
    visible_to: usize,
    rows: Vec<DeliveryRowView>,
    source_options: Vec<SimpleOptionView>,
    pagination: PaginationView,
}

#[derive(Template)]
#[template(path = "delivery_detail.html")]
struct DeliveryDetailTemplate {
    shell: AppShellView,
    detail: DeliveryDetailView,
    messages: Vec<DeliveryMessageView>,
    replay_action: String,
    back_url: String,
}

#[derive(Template)]
#[template(path = "users.html")]
struct UsersTemplate {
    shell: AppShellView,
    filters: UsersFilterView,
    total_count: usize,
    visible_count: usize,
    active_count: usize,
    admin_count: usize,
    live_session_count: usize,
    users: Vec<UserRowView>,
    sessions: Vec<ActiveSessionRowView>,
    can_manage_users: bool,
    can_create_subusers: bool,
    show_create_modal: bool,
    create_form: UserFormView,
    show_edit_modal: bool,
    edit_form: UserFormView,
    show_delete_modal: bool,
    delete_target_name: String,
    delete_target_action: String,
    role_options: Vec<SimpleOptionView>,
    parent_options: Vec<SimpleOptionView>,
}
