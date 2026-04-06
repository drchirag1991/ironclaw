//! Frontend extension API handlers.
//!
//! Provides endpoints for reading/writing layout configuration and
//! discovering/serving widget files from the workspace.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};

use ironclaw_frontend::{LayoutConfig, ResolvedWidget, WidgetManifest};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::handlers::memory::resolve_workspace;
use crate::channels::web::server::GatewayState;
use crate::workspace::Workspace;

/// `GET /api/frontend/layout` — return the current layout configuration.
///
/// Reads `frontend/layout.json` from the workspace. Returns an empty
/// default config if the file doesn't exist or is invalid (with a warning
/// logged for the invalid case).
pub async fn frontend_layout_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<LayoutConfig>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let layout = match workspace.read("frontend/layout.json").await {
        Ok(doc) => match serde_json::from_str(&doc.content) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "frontend/layout.json is invalid — returning default layout"
                );
                LayoutConfig::default()
            }
        },
        Err(_) => LayoutConfig::default(),
    };

    Ok(Json(layout))
}

/// `PUT /api/frontend/layout` — update the layout configuration.
///
/// Writes the provided layout config to `frontend/layout.json` in workspace.
pub async fn frontend_layout_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(layout): Json<LayoutConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let content = serde_json::to_string_pretty(&layout).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid layout config: {e}"),
        )
    })?;

    workspace
        .write("frontend/layout.json", &content)
        .await
        .map_err(|e| {
            tracing::error!("Failed to write layout config: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to write layout config".to_string(),
            )
        })?;

    Ok(StatusCode::OK)
}

/// `GET /api/frontend/widgets` — list all widget manifests.
///
/// Scans `frontend/widgets/` in workspace for directories containing
/// `manifest.json` and returns their parsed manifests.
pub async fn frontend_widgets_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<WidgetManifest>>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;
    let manifests = load_widget_manifests(&workspace).await;
    Ok(Json(manifests))
}

/// Discover every widget in `frontend/widgets/` and return its parsed
/// manifest. Malformed manifests are skipped with a `warn!` log.
pub(crate) async fn load_widget_manifests(workspace: &Workspace) -> Vec<WidgetManifest> {
    let entries = workspace
        .list("frontend/widgets/")
        .await
        .unwrap_or_default();

    let mut manifests = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        if let Some(manifest) = read_widget_manifest(workspace, entry.name()).await {
            manifests.push(manifest);
        }
    }
    manifests
}

/// Read and parse a single widget's `manifest.json`. Returns `None` (with a
/// `warn!`) for parse failures and `None` silently when the file is missing.
async fn read_widget_manifest(workspace: &Workspace, widget_name: &str) -> Option<WidgetManifest> {
    let manifest_path = format!("frontend/widgets/{}/manifest.json", widget_name);
    let doc = workspace.read(&manifest_path).await.ok()?;
    match serde_json::from_str::<WidgetManifest>(&doc.content) {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            tracing::warn!(
                path = %manifest_path,
                error = %e,
                "skipping widget with invalid manifest"
            );
            None
        }
    }
}

/// Discover every widget in `frontend/widgets/` and return the fully-resolved
/// set (manifest + `index.js` + optional `style.css`), filtered by the
/// `enabled` flag in the supplied layout. Widgets missing `index.js` are
/// skipped silently — they're assumed to be in-progress scaffolds.
///
/// This is the single source of truth for widget loading; both the gateway's
/// `/` handler and the `/api/frontend/widgets` handler delegate to it (the
/// latter via [`load_widget_manifests`]).
pub(crate) async fn load_resolved_widgets(
    workspace: &Workspace,
    layout: &LayoutConfig,
) -> Vec<ResolvedWidget> {
    let entries = workspace
        .list("frontend/widgets/")
        .await
        .unwrap_or_default();

    let mut widgets = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        let name = entry.name();
        let Some(manifest) = read_widget_manifest(workspace, name).await else {
            continue;
        };

        // Widgets without `index.js` are incomplete — skip quietly.
        let js_path = format!("frontend/widgets/{}/index.js", name);
        let js = match workspace.read(&js_path).await {
            Ok(doc) => doc.content,
            Err(_) => continue,
        };

        let css = workspace
            .read(&format!("frontend/widgets/{}/style.css", name))
            .await
            .ok()
            .map(|doc| doc.content)
            .filter(|c| !c.trim().is_empty());

        // Respect the layout's `enabled` flag; default is `true` when the
        // widget has no entry at all (see WidgetInstanceConfig::default).
        let enabled = layout
            .widgets
            .get(&manifest.id)
            .map(|w| w.enabled)
            .unwrap_or(true);
        if !enabled {
            continue;
        }

        widgets.push(ResolvedWidget { manifest, js, css });
    }
    widgets
}

/// `GET /api/frontend/widget/{id}/{*file}` — serve a widget file.
///
/// Serves JS/CSS files from `frontend/widgets/{id}/{file}` in workspace
/// with appropriate MIME types.
pub async fn frontend_widget_file_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((id, file)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // The widget id is a single path segment; it must not contain any
    // separator and must not be `.`, `..`, or empty.
    if !is_safe_segment(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid widget id".to_string()));
    }
    // The file parameter is a nested path (`*file` wildcard). Validate every
    // component independently so neither `a/../b` nor `a/./b` nor
    // `a/\..\b` slips through.
    if !is_safe_relative_path(&file) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid widget file path".to_string(),
        ));
    }

    let workspace = resolve_workspace(&state, &user).await?;
    let path = format!("frontend/widgets/{}/{}", id, file);

    let doc = workspace.read(&path).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("Widget file not found: {path}"),
        )
    })?;

    // Determine MIME type from the file extension (case-insensitive — the
    // browser doesn't care about `.JS` vs `.js`).
    let ext = file
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let content_type = match ext.as_str() {
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "map" => "application/json",
        _ => "text/plain",
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        doc.content,
    ))
}

/// True if `s` is a safe single path segment: non-empty, no separators, and
/// not a relative component (`.`/`..`). Also rejects backslash and NUL so
/// platform-specific separators and C-string terminators cannot sneak past.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
}

/// True if `s` is a safe relative path under the widget directory — every
/// `/`-separated component must itself pass `is_safe_segment`. Leading or
/// trailing slashes and empty components are rejected.
fn is_safe_relative_path(s: &str) -> bool {
    if s.is_empty() || s.starts_with('/') || s.contains('\0') {
        return false;
    }
    s.split('/').all(is_safe_segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_allows_normal_names() {
        assert!(is_safe_segment("widget-1"));
        assert!(is_safe_segment("dashboard_v2"));
        assert!(is_safe_segment("a.b.c"));
        assert!(is_safe_segment("foo..bar")); // `..` embedded in a longer name is fine
    }

    #[test]
    fn segment_rejects_traversal_and_separators() {
        assert!(!is_safe_segment(""));
        assert!(!is_safe_segment("."));
        assert!(!is_safe_segment(".."));
        assert!(!is_safe_segment("a/b"));
        assert!(!is_safe_segment("a\\b"));
        assert!(!is_safe_segment("nul\0byte"));
    }

    #[test]
    fn relative_path_allows_multi_component() {
        assert!(is_safe_relative_path("index.js"));
        assert!(is_safe_relative_path("assets/icon.svg"));
        assert!(is_safe_relative_path("i18n/en/strings.json"));
    }

    #[test]
    fn relative_path_rejects_traversal() {
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("assets/../secrets"));
        assert!(!is_safe_relative_path("./index.js"));
        assert!(!is_safe_relative_path("assets//icon.svg"));
        assert!(!is_safe_relative_path("assets\\..\\secrets"));
    }
}
