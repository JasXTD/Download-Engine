//! Queue commands and fair, bounded dispatch selection.
use super::*;

pub(super) fn apply(
    store: &Store,
    active: &mut HashMap<String, Active>,
    command: Command,
) -> Result<()> {
    match &command {
        Command::StartQueue(id) => {
            store.set_queue_running(id, true)?;
            return reconcile_workers(store, active);
        }
        Command::PauseQueue(id) => {
            store.set_queue_running(id, false)?;
            return reconcile_workers(store, active);
        }
        Command::SetQueueLimit(id, limit) => {
            store.set_queue_limit(id, *limit)?;
            return reconcile_workers(store, active);
        }
        Command::ScheduleQueue(id, at) => {
            store.schedule_queue(id, *at, chrono::Utc::now().timestamp())?;
            return reconcile_workers(store, active);
        }
        Command::ScheduleRecurringQueue(id, schedule) => {
            store.schedule_recurring_queue(id, schedule, chrono::Utc::now().timestamp())?;
            return reconcile_workers(store, active);
        }
        Command::EnableSchedule(id) => {
            store.enable_recurring_schedule(id, chrono::Utc::now().timestamp())?;
            return reconcile_workers(store, active);
        }
        Command::ClearSchedule(id) => {
            store.clear_schedule(id)?;
            return reconcile_workers(store, active);
        }
        Command::MoveJob(id, queue) => {
            anyhow::ensure!(
                !active.contains_key(id),
                "Pause the download before moving it"
            );
            store.finish_queue_pause(id)?;
            store.cancel_immediate(id)?;
            return store.move_job(id, queue);
        }
        Command::ReorderJob(id, earlier) => {
            anyhow::ensure!(
                !active.contains_key(id),
                "Pause the download before reordering it"
            );
            return store.reorder_job(id, *earlier);
        }
        _ => {}
    }
    if matches!(command, Command::PauseAll) {
        for job in store.list()? {
            // A queue-cancelled worker may already have saved Paused while its
            // automatic resume marker is still pending. Clear that ownership
            // through the same individual-pause path before the next tick.
            if matches!(
                job.status,
                Status::Queued | Status::Downloading | Status::Paused
            ) {
                apply(store, active, Command::Pause(job.id))?;
            }
        }
        return Ok(());
    }
    let id = match &command {
        Command::Pause(id)
        | Command::Resume(id)
        | Command::StartImmediately(id)
        | Command::Restart(id)
        | Command::SelectDiscoveredFile { id, .. }
        | Command::Remove(id)
        | Command::DeleteFile { id, .. }
        | Command::RenameFile { id, .. }
        | Command::MoveFile { id, .. } => id,
        _ => return Ok(()),
    };
    // A user pause overrides any pending queue pause, including while cancellation finishes.
    if matches!(command, Command::Pause(_)) {
        store.mark_queue_pause(id, false)?;
        store.cancel_immediate(id)?;
    } else if !active.contains_key(id) {
        store.finish_queue_pause(id)?;
    }
    if let Some(worker) = active.get_mut(id) {
        if matches!(command, Command::Pause(_)) {
            worker.user_paused = true;
            worker.token.cancel();
            return Ok(());
        }
        anyhow::bail!("Pause the download before changing it");
    }
    let Some(mut job) = store.get(id)? else {
        return Ok(());
    };
    match command {
        Command::SelectDiscoveredFile {
            discovery,
            selection,
            ..
        } => {
            crate::link_fallback::select(&mut job, discovery, selection)?;
            store.save_for_start(&job)?;
        }
        Command::StartImmediately(_) => store.start_immediately(&job.id)?,
        Command::Pause(_) => {
            if job.status == Status::Queued {
                job.status = Status::Paused;
                job.quota_waiting = false;
                job.retry.next_at = None;
                job.error = None;
                store.save(&job)?;
            }
        }
        Command::Resume(_) => {
            if matches!(job.status, Status::Paused | Status::Failed) {
                job.status = Status::Queued;
                job.quota_waiting = false;
                job.retry = retry::State::default();
                job.error = None;
                store.save_for_start(&job)?;
            }
        }
        Command::Restart(_) => {
            anyhow::ensure!(
                job.status != Status::Complete,
                "Completed downloads cannot be restarted; add the URL again"
            );
            crate::download_file::discard_partial_files(&store.paths, &job)?;
            job.retry = retry::State::default();
            job.quota_waiting = false;
            job.plan = None;
            job.link_fallback = None;
            if let Some(ftp) = &mut job.ftp {
                ftp.reset();
            }
            if let Some(hls) = &mut job.hls {
                hls.completed.clear();
                hls.completed_ahead.clear();
            }
            crate::synchronization::restarting(&mut job);
            job.expected_file = None;
            job.integrity.actual = None;
            job.downloaded = 0;
            job.total = None;
            job.error = None;
            job.saved_path = None;
            job.status = Status::Queued;
            store.save_for_start(&job)?;
        }
        Command::Remove(_) => {
            crate::download_file::discard_partial_files(&store.paths, &job)?;
            store.remove(&job.id)?;
        }
        Command::DeleteFile { path, .. } => {
            anyhow::ensure!(
                job.status == Status::Complete,
                "Only completed files can be deleted"
            );
            anyhow::ensure!(
                job.saved_path.as_ref() == Some(&path),
                "The saved file location changed. Open Delete file again to review it."
            );
            crate::download_file::discard_partial_files(&store.paths, &job)?;
            crate::download_file::delete_completed(&path)
                .with_context(|| format!("Cannot delete {}", job.name))?;
            store
                .remove(&job.id)
                .context("File deleted, but its history entry could not be removed")?;
        }
        Command::RenameFile { name, .. } => {
            anyhow::ensure!(
                !name.is_empty() && crate::model::safe_filename(&name) == name,
                "Choose a file name without \\ / : * ? \" < > |, control characters, a trailing dot or space, or a reserved name such as CON"
            );
            let from = saved_file(&job, "renamed")?;
            let to = from.with_file_name(&name);
            anyhow::ensure!(to != from, "The file already has this name");
            relocate(store, &mut job, &from, &to, |job| job.name = name)?;
        }
        Command::MoveFile { folder, .. } => {
            let from = saved_file(&job, "moved")?;
            let folder = movable_folder(store, &folder)?;
            let to = folder.join(from.file_name().context("Missing file name")?);
            anyhow::ensure!(to != from, "The file is already in this folder");
            relocate(store, &mut job, &from, &to, |job| job.directory = folder)?;
        }
        _ => {}
    }
    Ok(())
}

fn saved_file(job: &Job, verb: &str) -> Result<std::path::PathBuf> {
    anyhow::ensure!(
        job.status == Status::Complete,
        "Only finished downloads can be {verb}"
    );
    job.saved_path
        .clone()
        .context("This download has no saved file")
}

/// Move the file, then record where it is. If the record cannot be saved,
/// move the file back so the list never points at a missing file.
fn relocate(
    store: &Store,
    job: &mut Job,
    from: &std::path::Path,
    to: &std::path::Path,
    update: impl FnOnce(&mut Job),
) -> Result<()> {
    crate::download_file::move_completed(from, to)?;
    update(job);
    job.saved_path = Some(to.to_owned());
    if let Err(error) = store.save(job) {
        let _ = crate::download_file::move_completed(to, from);
        return Err(error
            .context("The file was not moved because its download record could not be updated"));
    }
    Ok(())
}

/// Scripts move files only into the download folder, a category folder, or a
/// folder inside one. Missing folders inside them are created.
fn movable_folder(store: &Store, folder: &std::path::Path) -> Result<std::path::PathBuf> {
    let _existing = store.guard_script_folder(folder)?;
    std::fs::create_dir_all(folder).context("Cannot create the destination folder")?;
    crate::download_file::guard_parents(&folder.join("check"))?;
    Ok(folder.to_owned())
}

// Cancel extra workers in queue order. A cancelled token may belong to an
// individual pause, so never overwrite its ownership with a queue marker.
fn reconcile_workers(store: &Store, active: &HashMap<String, Active>) -> Result<()> {
    let immediate = store.immediate_downloads()?;
    for queue in store.queues()? {
        let mut keep = if queue.running { queue.max_active } else { 0 };
        for job in store.queue_jobs(&queue.id)? {
            if let Some(worker) = active.get(&job.id) {
                if immediate.contains(&job.id) {
                    continue;
                }
                if worker.token.is_cancelled() {
                    continue;
                }
                if keep > 0 {
                    // Persist automatic ownership for recovery on exit/crash. A
                    // user pause clears this marker before cancelling its worker.
                    store.mark_queue_pause(&job.id, queue.automatic())?;
                    keep -= 1;
                } else {
                    store.mark_queue_pause(&job.id, true)?;
                    worker.token.cancel();
                }
            }
        }
    }
    Ok(())
}

pub(super) fn ready_jobs(
    store: &Store,
    active: &HashMap<String, Active>,
    turn: &mut usize,
) -> Result<Vec<(String, Job)>> {
    ready_jobs_at(store, active, turn, chrono::Utc::now().timestamp_millis())
}

pub(super) fn ready_jobs_at(
    store: &Store,
    active: &HashMap<String, Active>,
    turn: &mut usize,
    now: i64,
) -> Result<Vec<(String, Job)>> {
    store.apply_queue_schedules(now / 1000)?;
    reconcile_workers(store, active)?;
    for id in store.queue_paused_jobs()? {
        if !active.contains_key(&id) {
            store.finish_queue_pause(&id)?;
        }
    }
    if store.quota_status_at(now / 1000)?.blocked() {
        return Ok(Vec::new());
    }
    let mut queues = store.queues()?;
    let mut ready = Vec::new();
    let immediate = store.immediate_downloads()?;
    let slots = MAX_ACTIVE.saturating_sub(active.len());
    for id in &immediate {
        if ready.len() == slots {
            break;
        }
        if !active.contains_key(id)
            && let Some(job) = store.get(id)?
            && job.status == Status::Queued
            && job.retry.ready(now)
        {
            let queue = store
                .job_queue(id)?
                .context("Download queue no longer exists")?;
            store.mark_queue_pause(id, false)?;
            ready.push((queue, job));
        }
    }
    if queues.is_empty() {
        return Ok(ready);
    }
    let queue_count = queues.len();
    let offset = *turn % queue_count;
    queues.rotate_left(offset);
    let mut candidates = Vec::new();
    for (index, queue) in queues.into_iter().enumerate().filter(|(_, q)| q.running) {
        let count = active
            .values()
            .filter(|worker| worker.queue == queue.id)
            .count();
        let chosen = ready.iter().filter(|(id, _)| id == &queue.id).count();
        let queue_slots = queue.max_active.saturating_sub(count + chosen);
        let jobs = store
            .queue_jobs(&queue.id)?
            .into_iter()
            .filter(|j| {
                j.status == Status::Queued
                    && j.retry.ready(now)
                    && !active.contains_key(&j.id)
                    && !immediate.contains(&j.id)
            })
            .take(queue_slots)
            .collect::<Vec<_>>();
        candidates.push((
            queue.automatic(),
            queue.id,
            jobs.into_iter(),
            (offset + index + 1) % queue_count,
        ));
    }
    // Give every running queue a turn before choosing its next file.
    loop {
        let mut added = false;
        for (automatic, queue, jobs, next_turn) in &mut candidates {
            if ready.len() >= slots {
                return Ok(ready);
            }
            if let Some(job) = jobs.next() {
                // Set before publishing Downloading, so startup can recover even
                // if the process exits between dispatch and worker cancellation.
                store.mark_queue_pause(&job.id, *automatic)?;
                ready.push((queue.clone(), job));
                // Advance only on dispatch, so full-capacity timer ticks cannot
                // keep giving the same queue the next available slot.
                *turn = *next_turn;
                added = true;
            }
        }
        if !added {
            return Ok(ready);
        }
    }
}
