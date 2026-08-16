use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crossbeam_channel::bounded;

use crate::sources::{
    FullProjectionReason, ProjectionCheckpoint, ProjectionOutcome, SourceCatalog, SourceFile,
};

use super::stage::{ProjectionStage, StagedProjection};

#[derive(Clone, Debug)]
pub(crate) struct ProjectionInput {
    pub file: SourceFile,
    pub checkpoint: Option<ProjectionCheckpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionFailure {
    pub file_path: String,
    pub message: String,
}

pub(crate) fn project_bounded(
    catalog: SourceCatalog,
    inputs: &[ProjectionInput],
    worker_count: usize,
    stage: &mut ProjectionStage,
) -> std::io::Result<Vec<ProjectionFailure>> {
    if inputs.is_empty() {
        stage.seal()?;
        return Ok(Vec::new());
    }
    let workers = worker_count.max(1).min(inputs.len());
    let (sender, receiver) = bounded::<Result<StagedProjection, ProjectionFailure>>(workers * 2);
    let next = AtomicUsize::new(0);
    let mut failures = Vec::new();

    thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(input) = inputs.get(index) else {
                        break;
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        project_one(catalog, input)
                    }))
                    .unwrap_or_else(|panic| {
                        Err(ProjectionFailure {
                            file_path: input.file.file_path.to_string_lossy().into_owned(),
                            message: format!(
                                "source projection panicked: {}",
                                panic_message(panic.as_ref())
                            ),
                        })
                    });
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for result in receiver {
            match result {
                Ok(record) => stage.push(&record)?,
                Err(error) => failures.push(error),
            }
        }
        Ok::<(), std::io::Error>(())
    })?;
    stage.seal()?;
    failures.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(failures)
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

fn project_one(
    catalog: SourceCatalog,
    input: &ProjectionInput,
) -> Result<StagedProjection, ProjectionFailure> {
    let outcome = catalog
        .project(&input.file, input.file.size, input.checkpoint.as_ref())
        .map_err(|error| ProjectionFailure {
            file_path: input.file.file_path.to_string_lossy().into_owned(),
            message: error.to_string(),
        })?;
    let outcome = if input.checkpoint.is_some()
        && matches!(outcome, ProjectionOutcome::FullRequired { .. })
    {
        catalog
            .project(&input.file, input.file.size, None)
            .map_err(|error| ProjectionFailure {
                file_path: input.file.file_path.to_string_lossy().into_owned(),
                message: error.to_string(),
            })?
    } else {
        outcome
    };
    if let ProjectionOutcome::FullRequired { reason, .. } = outcome {
        return Err(ProjectionFailure {
            file_path: input.file.file_path.to_string_lossy().into_owned(),
            message: full_required_message(reason),
        });
    }
    Ok(StagedProjection {
        file: input.file.clone(),
        outcome,
    })
}

fn full_required_message(reason: FullProjectionReason) -> String {
    format!("source changed while projecting ({reason:?})")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::identity::SourceId;
    use crate::selector::Selector;
    use crate::sources::SourceMetadataCache;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn bounded_pipeline_projects_many_files_into_a_replayable_stage() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions/2026/08/15");
        fs::create_dir_all(&root).unwrap();
        for index in 0..12 {
            let id = format!("00000000-0000-4000-8000-{index:012}");
            let lines = [
                serde_json::json!({
                    "timestamp":"2026-08-15T00:00:00Z",
                    "type":"session_meta",
                    "payload":{"id":id,"cwd":"/work"}
                }),
                serde_json::json!({
                    "timestamp":"2026-08-15T00:00:01Z",
                    "type":"event_msg",
                    "payload":{"type":"user_message","message":format!("message {index}")}
                }),
            ];
            fs::write(
                root.join(format!("rollout-2026-08-15T00-00-00-{id}.jsonl")),
                format!("{}\n{}\n", lines[0], lines[1]),
            )
            .unwrap();
        }
        let selector = Selector::All {
            source: SourceId::Codex,
            root: temp.path().join("sessions").to_string_lossy().into_owned(),
        };
        let catalog = SourceCatalog;
        let scan = catalog
            .scan(&selector, &SourceMetadataCache::default())
            .unwrap();
        let inputs = scan
            .files
            .into_iter()
            .map(|file| ProjectionInput {
                file,
                checkpoint: None,
            })
            .collect::<Vec<_>>();
        let mut stage = ProjectionStage::create(temp.path()).unwrap();
        let failures = project_bounded(catalog, &inputs, 3, &mut stage).unwrap();
        assert!(failures.is_empty());
        let mut count = 0;
        stage
            .read_all(|_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 12);
    }
}
