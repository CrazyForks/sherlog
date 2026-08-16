use std::io::Write;

use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;
use crate::model::{
    FindSummary, MessageRecord, ReadPageSummary, ReadRangeSummary, SessionListSummary,
    StatsSummary, StatusSummary,
};
use crate::retrieval::{EvidenceReadContext, build_evidence_read_action};
use crate::sync::SyncReport;

pub(super) fn write_json(writer: &mut dyn Write, value: &impl Serialize) -> Result<(), AppError> {
    serde_json::to_writer_pretty(&mut *writer, value).map_err(AppError::output)?;
    writeln!(writer).map_err(AppError::output)
}

pub(super) fn write_sync_text(writer: &mut dyn Write, report: &SyncReport) -> Result<(), AppError> {
    writeln!(writer, "shlog sync").map_err(AppError::output)?;
    writeln!(
        writer,
        "selector: {}",
        serde_json::to_string(&report.selector).map_err(AppError::output)?
    )
    .map_err(AppError::output)?;
    writeln!(writer, "scanned:  {}", report.scanned).map_err(AppError::output)?;
    writeln!(writer, "added:    {}", report.added).map_err(AppError::output)?;
    writeln!(writer, "updated:  {}", report.updated).map_err(AppError::output)?;
    writeln!(writer, "skipped:  {}", report.skipped).map_err(AppError::output)?;
    writeln!(writer, "filtered: {}", report.filtered).map_err(AppError::output)?;
    writeln!(writer, "removed:  {}", report.removed).map_err(AppError::output)?;
    if report.retained_cold > 0 {
        writeln!(writer, "retainedCold: {}", report.retained_cold).map_err(AppError::output)?;
    }
    writeln!(writer, "errors:   {}", report.errors).map_err(AppError::output)?;
    let coverage = if report.coverage.written {
        "written".to_owned()
    } else {
        format!(
            "not written ({})",
            report.coverage.reason.as_deref().unwrap_or("unknown")
        )
    };
    writeln!(writer, "coverage: {coverage}").map_err(AppError::output)?;
    if !report.error_details.is_empty() {
        writeln!(writer, "\nsync errors").map_err(AppError::output)?;
        for detail in &report.error_details {
            writeln!(writer, "{}\n  {}", detail.file_path, detail.message)
                .map_err(AppError::output)?;
        }
    }
    Ok(())
}

pub(super) fn write_find_json(
    writer: &mut dyn Write,
    summary: &FindSummary,
    elapsed_ms: u64,
    db_path: &str,
    json_output: bool,
) -> Result<(), AppError> {
    let mut value = serde_json::to_value(summary).map_err(AppError::output)?;
    let object = value
        .as_object_mut()
        .expect("FindSummary always serializes as an object");
    let context = EvidenceReadContext {
        db_path,
        json: json_output,
    };
    if let Some(results) = object.get_mut("results").and_then(Value::as_array_mut) {
        for (result_value, result) in results.iter_mut().zip(&summary.results) {
            result_value
                .as_object_mut()
                .expect("FindResult always serializes as an object")
                .insert(
                    "evidenceRead".to_owned(),
                    serde_json::to_value(build_evidence_read_action(
                        result,
                        Some(&summary.query),
                        &context,
                    ))
                    .map_err(AppError::output)?,
                );
        }
    }
    object.insert("elapsedMs".to_owned(), Value::from(elapsed_ms));
    write_json(writer, &value)
}

pub(super) fn write_elapsed_json<T: Serialize>(
    writer: &mut dyn Write,
    summary: &T,
    elapsed_ms: u64,
) -> Result<(), AppError> {
    let mut value = serde_json::to_value(summary).map_err(AppError::output)?;
    value
        .as_object_mut()
        .expect("command summary always serializes as an object")
        .insert("elapsedMs".to_owned(), Value::from(elapsed_ms));
    write_json(writer, &value)
}

pub(super) fn write_find_text(
    writer: &mut dyn Write,
    summary: &FindSummary,
    elapsed_ms: u64,
) -> Result<(), AppError> {
    writeln!(writer, "shlog find \"{}\"", summary.query).map_err(AppError::output)?;
    if summary.results.is_empty() {
        writeln!(writer, "没有找到结果").map_err(AppError::output)?;
        return write_next_action(writer, summary.next_action.as_ref());
    }
    for result in &summary.results {
        writeln!(writer).map_err(AppError::output)?;
        writeln!(
            writer,
            "[{}] {}",
            result.rank,
            nonempty(&result.title, "(no title)")
        )
        .map_err(AppError::output)?;
        writeln!(
            writer,
            "{} · {}",
            result.started_at,
            nonempty(&result.cwd, "-")
        )
        .map_err(AppError::output)?;
        let anchor = result
            .match_seq
            .map(|seq| format!("seq={seq}"))
            .unwrap_or_else(|| "session-level".to_owned());
        writeln!(
            writer,
            "source={} · uuid={} · {} · matches={}",
            result.source_id, result.session_uuid, anchor, result.match_count
        )
        .map_err(AppError::output)?;
        if !result.summary_text.is_empty() {
            writeln!(writer, "{}", collapse_and_trim(&result.summary_text, 220))
                .map_err(AppError::output)?;
        }
        writeln!(writer, "{}", strip_marks(&result.snippet)).map_err(AppError::output)?;
        if let Some(seq) = result.match_seq {
            writeln!(
                writer,
                "next: shlog read-range {} --seq {} --query {}",
                result.session_ref,
                seq,
                shell_arg(&summary.query)
            )
            .map_err(AppError::output)?;
        } else {
            writeln!(
                writer,
                "next: shlog read-page {} --offset 0 --limit 40",
                result.session_ref
            )
            .map_err(AppError::output)?;
        }
    }
    let _ = elapsed_ms;
    write_next_action(writer, summary.next_action.as_ref())
}

pub(super) fn write_range_text(
    writer: &mut dyn Write,
    summary: &ReadRangeSummary,
) -> Result<(), AppError> {
    writeln!(writer, "shlog read-range {}", summary.session.session_uuid)
        .map_err(AppError::output)?;
    writeln!(
        writer,
        "{} · {}",
        nonempty(&summary.session.title, "(no title)"),
        nonempty(&summary.session.cwd, "-")
    )
    .map_err(AppError::output)?;
    writeln!(
        writer,
        "anchor={} · range={}-{}",
        summary.anchor_seq, summary.range_start_seq, summary.range_end_seq
    )
    .map_err(AppError::output)?;
    writeln!(writer).map_err(AppError::output)?;
    for message in &summary.messages {
        let marker = if message.seq == summary.anchor_seq {
            ">>"
        } else {
            "  "
        };
        write_message(writer, marker, message)?;
    }
    Ok(())
}

pub(super) fn write_page_text(
    writer: &mut dyn Write,
    summary: &ReadPageSummary,
) -> Result<(), AppError> {
    writeln!(writer, "shlog read-page {}", summary.session.session_uuid)
        .map_err(AppError::output)?;
    writeln!(
        writer,
        "{} · total={} · offset={} · limit={} · hasMore={}",
        nonempty(&summary.session.title, "(no title)"),
        summary.total_count,
        summary.offset,
        summary.limit,
        summary.has_more
    )
    .map_err(AppError::output)?;
    writeln!(writer).map_err(AppError::output)?;
    for message in &summary.messages {
        write_message(writer, "", message)?;
    }
    Ok(())
}

pub(super) fn write_list_text(
    writer: &mut dyn Write,
    summary: &SessionListSummary,
) -> Result<(), AppError> {
    writeln!(writer, "shlog list").map_err(AppError::output)?;
    if summary.results.is_empty() {
        writeln!(writer, "没有匹配的 session").map_err(AppError::output)?;
        return write_next_action(writer, summary.next_action.as_ref());
    }
    for (index, entry) in summary.results.iter().enumerate() {
        writeln!(writer).map_err(AppError::output)?;
        writeln!(
            writer,
            "[{}] {}",
            index + 1,
            nonempty(&entry.title, "(no title)")
        )
        .map_err(AppError::output)?;
        writeln!(
            writer,
            "{} · {} · msgs={}",
            entry.ended_at,
            nonempty(&entry.cwd, "-"),
            entry.message_count
        )
        .map_err(AppError::output)?;
        writeln!(writer, "uuid={}", entry.session_uuid).map_err(AppError::output)?;
        if !entry.summary_text.is_empty() {
            writeln!(writer, "{}", collapse_and_trim(&entry.summary_text, 220))
                .map_err(AppError::output)?;
        }
    }
    Ok(())
}

pub(super) fn write_stats_text(
    writer: &mut dyn Write,
    stats: &StatsSummary,
) -> Result<(), AppError> {
    writeln!(writer, "shlog stats").map_err(AppError::output)?;
    writeln!(writer, "sessions:        {}", stats.session_count).map_err(AppError::output)?;
    writeln!(writer, "messages:        {}", stats.message_count).map_err(AppError::output)?;
    writeln!(
        writer,
        "earliest:        {}",
        stats.earliest_started_at.as_deref().unwrap_or("-")
    )
    .map_err(AppError::output)?;
    writeln!(
        writer,
        "latest:          {}",
        stats.latest_ended_at.as_deref().unwrap_or("-")
    )
    .map_err(AppError::output)?;
    writeln!(
        writer,
        "last_sync_at:    {}",
        stats.last_sync_at.as_deref().unwrap_or("-")
    )
    .map_err(AppError::output)?;
    writeln!(writer, "index_version:   {}", stats.index_version).map_err(AppError::output)?;
    writeln!(writer, "db_path:         {}", stats.db_path).map_err(AppError::output)?;
    writeln!(writer, "db_size_bytes:   {}", stats.db_size_bytes).map_err(AppError::output)?;
    writeln!(writer, "coverage_count:  {}", stats.coverage.len()).map_err(AppError::output)?;
    if !stats.top_cwds.is_empty() {
        writeln!(writer, "\ntop cwds").map_err(AppError::output)?;
        let width = stats
            .top_cwds
            .iter()
            .map(|row| row.cwd.chars().count())
            .max()
            .unwrap_or(0);
        for row in &stats.top_cwds {
            writeln!(writer, "  {:width$}  {}", row.cwd, row.count, width = width)
                .map_err(AppError::output)?;
        }
    }
    Ok(())
}

pub(super) fn write_status_text(
    writer: &mut dyn Write,
    status: &StatusSummary,
) -> Result<(), AppError> {
    writeln!(writer, "shlog status").map_err(AppError::output)?;
    writeln!(writer, "cwd:            {}", status.context.cwd).map_err(AppError::output)?;
    writeln!(writer, "root:           {}", status.context.root).map_err(AppError::output)?;
    writeln!(writer, "db_path:        {}", status.context.db_path).map_err(AppError::output)?;
    writeln!(
        writer,
        "source_files:   {}",
        status.source_inventory.total_files
    )
    .map_err(AppError::output)?;
    writeln!(
        writer,
        "source_dates:   {}..{}",
        status
            .source_inventory
            .path_date_range
            .from
            .as_deref()
            .unwrap_or("-"),
        status
            .source_inventory
            .path_date_range
            .to
            .as_deref()
            .unwrap_or("-")
    )
    .map_err(AppError::output)?;
    writeln!(writer, "index_exists:   {}", status.index.exists).map_err(AppError::output)?;
    writeln!(writer, "sessions:       {}", status.index.session_count).map_err(AppError::output)?;
    writeln!(writer, "messages:       {}", status.index.message_count).map_err(AppError::output)?;
    writeln!(writer, "coverage_count: {}", status.coverage_count).map_err(AppError::output)?;
    if let Some(requested) = &status.requested_coverage {
        writeln!(writer, "requested_coverage: {:?}", requested.freshness)
            .map_err(AppError::output)?;
        writeln!(writer, "stale_reason:       {:?}", requested.stale_reason)
            .map_err(AppError::output)?;
        writeln!(
            writer,
            "recommended_action:  {:?}",
            requested.recommended_action
        )
        .map_err(AppError::output)?;
        writeln!(
            writer,
            "source_file_count:   {}",
            requested.source_file_count
        )
        .map_err(AppError::output)?;
        writeln!(
            writer,
            "covering_selectors:  {}",
            requested.covering_selectors.len()
        )
        .map_err(AppError::output)?;
    }
    Ok(())
}

fn write_message(
    writer: &mut dyn Write,
    marker: &str,
    message: &MessageRecord,
) -> Result<(), AppError> {
    let role = match message.role {
        crate::model::MessageRole::User => "U",
        crate::model::MessageRole::Assistant => "A",
    };
    let timestamp = message
        .elision
        .as_ref()
        .map(|_| format!(" {}", message.timestamp))
        .unwrap_or_default();
    writeln!(
        writer,
        "{} [{}] {}{} {}",
        marker,
        message.seq,
        role,
        timestamp,
        collapse_and_trim(&message.content_text, 1_000)
    )
    .map_err(AppError::output)?;
    if let Some(elision) = &message.elision {
        writeln!(
            writer,
            "   elided {}/{} chars ({:?}); {}",
            elision.omitted_char_count, elision.original_char_count, elision.strategy, elision.hint
        )
        .map_err(AppError::output)?;
    }
    Ok(())
}

fn write_next_action(
    writer: &mut dyn Write,
    next_action: Option<&crate::model::QueryNextAction>,
) -> Result<(), AppError> {
    let Some(next_action) = next_action else {
        return Ok(());
    };
    writeln!(writer, "next:").map_err(AppError::output)?;
    for step in &next_action.steps {
        writeln!(writer, "  - {step}").map_err(AppError::output)?;
    }
    Ok(())
}

fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

fn strip_marks(value: &str) -> String {
    value.replace("<mark>", "").replace("</mark>", "")
}

fn collapse_and_trim(value: &str, budget: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = collapsed.chars().take(budget).collect::<String>();
    if collapsed.chars().count() > budget {
        output.push('…');
    }
    output
}

fn shell_arg(value: &str) -> String {
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '_' | '.' | '/' | ':' | '@' | '=' | '-')
    }) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
