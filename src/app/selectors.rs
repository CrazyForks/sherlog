use std::path::{Path, PathBuf};

use crate::cli::{FindArgs, ListArgs, StatusArgs, SyncArgs};
use crate::config::{ResolvedPaths, resolve_source_root};
use crate::error::AppError;
use crate::identity::{SessionRef, SourceId, parse_session_ref};
use crate::selector::{Selector, SelectorDefaults, parse_selector_json};

pub(super) fn status_source(args: &StatusArgs) -> Result<SourceId, AppError> {
    parse_source(args.source.as_deref().unwrap_or("codex"))
}

pub(super) fn list_source(args: &ListArgs) -> Result<SourceId, AppError> {
    parse_source(args.source.as_deref().unwrap_or("codex"))
}

pub(super) fn sync_selector(
    args: &SyncArgs,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Selector, AppError> {
    let source = parse_source(args.source.as_deref().unwrap_or("codex"))?;
    optional_selector(
        args.selector.as_deref(),
        args.root.as_deref(),
        args.cwd.as_deref(),
        source,
        paths,
        cwd,
        true,
    )?
    .map_or_else(|| all_selector(source, None, paths, cwd), Ok)
}

pub(super) fn find_sources(args: &FindArgs) -> Result<Vec<SourceId>, AppError> {
    let requested = args.source.as_deref().map(str::trim);
    if let Some(source) = requested.filter(|value| *value != "all") {
        return Ok(vec![parse_source(source)?]);
    }
    if let Some(source) = selector_source_hint(args.selector.as_deref())? {
        return Ok(vec![source]);
    }
    Ok(SourceId::ALL.to_vec())
}

pub(super) fn status_selector(
    args: &StatusArgs,
    source: SourceId,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Option<Selector>, AppError> {
    optional_selector(
        args.selector.as_deref(),
        args.root.as_deref(),
        args.cwd.as_deref(),
        source,
        paths,
        cwd,
        false,
    )
}

pub(super) fn find_selector(
    args: &FindArgs,
    source: SourceId,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Option<Selector>, AppError> {
    optional_selector(
        args.selector.as_deref(),
        args.root.as_deref(),
        args.cwd.as_deref(),
        source,
        paths,
        cwd,
        true,
    )
}

pub(super) fn find_has_explicit_scope(args: &FindArgs) -> bool {
    args.selector.is_some() || args.cwd.is_some() || args.root.is_some()
}

pub(super) fn find_bootstrap_selectors(
    args: &FindArgs,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Option<Vec<Selector>>, AppError> {
    let sources = find_sources(args)?;
    if sources.len() != 1 && !find_has_explicit_scope(args) {
        return Ok(None);
    }
    let mut selectors = Vec::with_capacity(sources.len());
    for source in sources {
        selectors.push(
            find_selector(args, source, paths, cwd)?.unwrap_or(all_selector(
                source,
                args.root.as_deref(),
                paths,
                cwd,
            )?),
        );
    }
    Ok(Some(selectors))
}

pub(super) fn list_selector(
    args: &ListArgs,
    source: SourceId,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Option<Selector>, AppError> {
    optional_selector(
        args.selector.as_deref(),
        args.root.as_deref(),
        None,
        source,
        paths,
        cwd,
        true,
    )
}

pub(super) fn all_selector(
    source: SourceId,
    root_override: Option<&Path>,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<Selector, AppError> {
    let root = resolve_source_root(source, root_override, paths, cwd);
    Ok(Selector::All {
        source,
        root: utf8_path(&root, "sessions root")?,
    })
}

pub(super) fn read_session_ref(
    value: &str,
    explicit_source: Option<&str>,
) -> Result<SessionRef, AppError> {
    let parsed = parse_session_ref(value);
    let Some(explicit_source) = explicit_source else {
        return Ok(parsed);
    };
    let explicit_source = parse_source(explicit_source)?;
    if value
        .split_once(':')
        .is_some_and(|(prefix, _)| prefix.parse::<SourceId>().is_ok())
    {
        if parsed.source_id != explicit_source {
            return Err(AppError::invalid_selector(
                "--source must match session source qualifier",
            ));
        }
        return Ok(parsed);
    }
    Ok(SessionRef {
        source_id: explicit_source,
        native_session_id: value.to_owned(),
    })
}

fn optional_selector(
    selector_json: Option<&str>,
    root_override: Option<&Path>,
    selected_cwd: Option<&Path>,
    source: SourceId,
    paths: &ResolvedPaths,
    cwd: &Path,
    root_only_selector: bool,
) -> Result<Option<Selector>, AppError> {
    if selector_json.is_some() && selected_cwd.is_some() {
        return Err(AppError::invalid_selector(
            "--selector and --cwd cannot be combined",
        ));
    }
    let root = resolve_source_root(source, root_override, paths, cwd);
    let root_text = utf8_path(&root, "sessions root")?;
    if let Some(selector_json) = selector_json {
        let selector = parse_selector_json(
            selector_json,
            &SelectorDefaults {
                default_root: Some(root_text),
                default_source: Some(source),
            },
            cwd,
        )
        .map_err(|error| AppError::invalid_selector(error.to_string()))?;
        if selector.source() != source {
            return Err(AppError::invalid_selector(
                "--source must match selector.source",
            ));
        }
        return Ok(Some(selector));
    }
    if let Some(selected_cwd) = selected_cwd {
        return Ok(Some(Selector::Cwd {
            source,
            root: root_text,
            cwd: utf8_path(selected_cwd, "selector cwd")?,
        }));
    }
    if root_only_selector && root_override.is_some() {
        return Ok(Some(Selector::All {
            source,
            root: root_text,
        }));
    }
    Ok(None)
}

fn selector_source_hint(selector_json: Option<&str>) -> Result<Option<SourceId>, AppError> {
    let Some(selector_json) = selector_json else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(selector_json)
        .map_err(|error| AppError::invalid_selector(format!("invalid selector JSON: {error}")))?;
    let Some(source) = value.get("source").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    parse_source(source).map(Some)
}

fn parse_source(value: &str) -> Result<SourceId, AppError> {
    value
        .trim()
        .parse::<SourceId>()
        .map_err(|_| AppError::unsupported_source(value))
}

fn utf8_path(path: &Path, label: &str) -> Result<String, AppError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_selector(format!("{label} must be valid UTF-8")))
}

#[allow(dead_code)]
pub(super) fn resolved_db_path(path: &Path, cwd: &Path) -> PathBuf {
    crate::config::resolve_lexical(path, cwd)
}
