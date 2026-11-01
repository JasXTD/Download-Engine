//! A completed torrent is scanned as its selected files. Explorer-based
//! scanners receive one folder request and still require a human review.
use super::*;
use std::collections::HashSet;

struct Locked {
    parents: Vec<File>,
    files: Vec<(PathBuf, File)>,
    reports: Vec<FileReport>,
    digest: String,
}

fn lock_files(store: &Store, job: &Job, cancel: &CancellationToken) -> Result<Locked> {
    use sha2::{Digest, Sha256};
    let root = job.saved_path.as_ref().context("Missing torrent folder")?;
    let mut parents = process::guard_parents(&root.join(".scan-check"))?;
    let mut seen: HashSet<PathBuf> = root.ancestors().map(Path::to_owned).collect();
    let mut files = Vec::new();
    let mut reports = Vec::new();
    let mut combined = Sha256::new();
    for (path, size) in crate::torrent::completed_files(store, job)? {
        ensure!(path != *root, "This torrent is a single file");
        let ancestor_paths: Vec<_> = path
            .parent()
            .context("Missing file folder")?
            .ancestors()
            .collect();
        if !seen.contains(path.parent().unwrap()) {
            for (parent, handle) in ancestor_paths
                .iter()
                .rev()
                .zip(process::guard_parents(&path)?)
            {
                if seen.insert((*parent).to_owned()) {
                    parents.push(handle);
                }
            }
        }
        let (hash, file) = integrity::hash_file(&path, cancel)?;
        ensure!(
            file.metadata()?.len() == size,
            "A torrent file changed size; restore it before scanning"
        );
        let relative = path.strip_prefix(root)?.to_owned();
        let name = relative.to_str().context("Invalid torrent filename")?;
        combined.update((name.len() as u64).to_le_bytes());
        combined.update(name.as_bytes());
        combined.update(hash.as_bytes());
        reports.push(FileReport {
            path: relative,
            sha256: hash,
            verdict: Verdict::Cancelled,
            message: "Not scanned".into(),
        });
        files.push((path, file));
    }
    ensure!(
        !files.is_empty(),
        "This torrent has no selected files to scan"
    );
    Ok(Locked {
        parents,
        files,
        reports,
        digest: format!("{:x}", combined.finalize()),
    })
}

fn permit(job: &Job, settings: &Settings, locked: Locked) -> Result<OpenPermit> {
    let root = job.saved_path.as_ref().context("Missing torrent folder")?;
    let root_guards = process::guard_parents(&root.join(".scan-check"))?;
    let directory = root_guards
        .last()
        .context("Missing folder handle")?
        .try_clone()?;
    Ok(OpenPermit {
        job: job.clone(),
        settings: settings.clone(),
        _file: directory,
        _parents: locked.parents,
        _scanned_files: locked.files,
    })
}

pub(super) fn external_permit(
    store: &Store,
    job: &Job,
    report: &Report,
    cancel: &CancellationToken,
) -> Result<OpenPermit> {
    let (current, settings, _) = validate_external_review(store, job, report)?;
    let locked = lock_files(store, &current, cancel)?;
    ensure!(
        report.sha256.as_deref() == Some(&locked.digest),
        "A torrent file changed after the scan request; scan again"
    );
    validate_external_review(store, &current, report)?;
    ensure!(!cancel.is_cancelled(), "File verification cancelled");
    permit(&current, &settings, locked)
}

#[cfg(test)]
fn scan(
    store: &Store,
    job: &Job,
    settings: &Settings,
    cancel: &CancellationToken,
    for_open: bool,
    run: impl FnMut(&Settings, &Path, &CancellationToken) -> Result<(Verdict, String)>,
) -> Result<Outcome> {
    scan_progress(store, job, settings, cancel, for_open, run, |_, _, _| {})
}

pub(super) fn scan_progress(
    store: &Store,
    job: &Job,
    settings: &Settings,
    cancel: &CancellationToken,
    for_open: bool,
    mut run: impl FnMut(&Settings, &Path, &CancellationToken) -> Result<(Verdict, String)>,
    mut on_file: impl FnMut(usize, usize, &Path),
) -> Result<Outcome> {
    let root = job.saved_path.as_ref().context("Missing torrent folder")?;
    check_current(
        job,
        &store.get(&job.id)?.context("This download was removed")?,
    )?;
    let mut report = Report {
        job_id: job.id.clone(),
        path: root.clone(),
        scanner: settings.provider.label().into(),
        settings_key: settings.key()?,
        verdict: Verdict::Failed,
        message: String::new(),
        sha256: None,
        finished_at: 0,
        files: Vec::new(),
    };
    let mut open = None;
    let result = (|| -> Result<()> {
        settings.validate()?;
        let mut locked = lock_files(store, job, cancel)?;
        report.sha256 = Some(locked.digest.clone());
        if settings.provider.external_review() {
            // One request avoids overlapping Malwarebytes/Kaspersky/Bitdefender
            // scans. Their acknowledgement must never become a clean verdict.
            let (verdict, message) = run(settings, root, cancel)?;
            ensure!(
                verdict != Verdict::NoThreatsFound,
                "An external antivirus request cannot confirm a clean scan"
            );
            for file in &mut locked.reports {
                file.verdict = verdict;
                file.message = message.clone();
            }
            report.verdict = verdict;
            report.message = format!(
                "Folder scan requested for {} selected torrent files. {message}",
                locked.files.len()
            );
        } else {
            report.verdict = Verdict::NoThreatsFound;
            for (index, (path, _)) in locked.files.iter().enumerate() {
                if cancel.is_cancelled() {
                    break;
                }
                on_file(index, locked.files.len(), path.strip_prefix(root)?);
                let (verdict, message) = run(settings, path, cancel)
                    .unwrap_or_else(|error| (Verdict::Failed, format!("{error:#}")));
                on_file(index + 1, locked.files.len(), path.strip_prefix(root)?);
                locked.reports[index].verdict = verdict;
                locked.reports[index].message = message;
                if verdict != Verdict::NoThreatsFound {
                    report.verdict = verdict;
                }
                // Stop at a threat/error, preserving explicit unscanned results.
                if verdict != Verdict::NoThreatsFound {
                    break;
                }
            }
            let clean = locked
                .reports
                .iter()
                .filter(|file| file.verdict == Verdict::NoThreatsFound)
                .count();
            report.message = format!(
                "{clean} of {} selected torrent files scanned with no threats found.",
                locked.files.len()
            );
        }
        report.files = locked.reports.clone();
        if report.verdict == Verdict::NoThreatsFound && for_open && !cancel.is_cancelled() {
            open = Some(permit(job, settings, locked)?);
        }
        Ok(())
    })();
    if let Err(error) = result {
        report.verdict = Verdict::Failed;
        report.message = format!("{error:#}");
        open = None;
    }
    if cancel.is_cancelled() {
        report.verdict = Verdict::Cancelled;
        report.message = "Torrent scan cancelled. Remaining files were not scanned.".into();
        open = None;
    }
    report.finished_at = chrono::Utc::now().timestamp();
    store.save_antivirus_report(job, settings, &report)?;
    Ok(Outcome {
        report,
        permit: open,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(provider: Provider) -> (tempfile::TempDir, Store, Job, Settings) {
        let (temp, store, job) = crate::torrent::completed_folder_fixture();
        let settings = Settings {
            enabled: true,
            provider,
            ..Default::default()
        };
        store.save_antivirus_settings(&settings).unwrap();
        (temp, store, job, settings)
    }
    #[test]
    fn folder_scans_selected_files_one_at_a_time_and_locks_the_open_permit() {
        let (_temp, store, job, settings) = setup(Provider::Defender);
        let mut paths = Vec::new();
        let outcome = scan(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, path, _| {
                paths.push(path.to_owned());
                #[cfg(windows)]
                assert!(std::fs::write(path, b"changed").is_err());
                Ok((Verdict::NoThreatsFound, "Harmless fixture".into()))
            },
        )
        .unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(outcome.report.files.len(), 2);
        assert_eq!(outcome.report.verdict, Verdict::NoThreatsFound);
        let permit = outcome.permit.unwrap();
        assert_eq!(
            permit.validate(&store).unwrap(),
            job.saved_path.as_deref().unwrap()
        );
        #[cfg(windows)]
        assert!(std::fs::remove_file(&paths[1]).is_err());
        drop(permit);
        std::fs::write(&paths[1], b"changed!").unwrap();
    }
    #[test]
    fn folder_scan_stops_on_threat_and_cancel_without_scanning_remaining_files() {
        for cancel_scan in [false, true] {
            let (_temp, store, job, settings) = setup(Provider::Defender);
            let cancel = CancellationToken::new();
            let mut calls = 0;
            let outcome = scan(&store, &job, &settings, &cancel, true, |_, _, token| {
                calls += 1;
                if cancel_scan {
                    token.cancel();
                }
                Ok((
                    if cancel_scan {
                        Verdict::Cancelled
                    } else {
                        Verdict::Detected
                    },
                    "Fixture result".into(),
                ))
            })
            .unwrap();
            assert_eq!(calls, 1);
            assert!(outcome.permit.is_none());
            assert_eq!(outcome.report.files[1].message, "Not scanned");
            assert_eq!(
                outcome.report.verdict,
                if cancel_scan {
                    Verdict::Cancelled
                } else {
                    Verdict::Detected
                }
            );
        }
    }
    #[test]
    fn external_folder_handoff_is_one_request_and_never_a_clean_verdict() {
        let (_temp, store, job, settings) = setup(Provider::Malwarebytes);
        let mut calls = 0;
        let outcome = scan(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, path, _| {
                calls += 1;
                assert_eq!(Some(path), job.saved_path.as_deref());
                Ok((
                    Verdict::ScanRequested,
                    "Review the external application".into(),
                ))
            },
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert!(outcome.permit.is_none());
        assert_eq!(outcome.report.verdict, Verdict::ScanRequested);
        let permit =
            external_permit(&store, &job, &outcome.report, &CancellationToken::new()).unwrap();
        assert!(permit.validate(&store).is_ok());
        drop(permit);
        std::fs::write(job.saved_path.as_ref().unwrap().join("a.bin"), b"changed!").unwrap();
        assert!(external_permit(&store, &job, &outcome.report, &CancellationToken::new()).is_err());
    }
    #[test]
    fn missing_selected_file_fails_before_any_scanner_is_launched() {
        let (_temp, store, job, settings) = setup(Provider::Defender);
        std::fs::remove_file(job.saved_path.as_ref().unwrap().join("nested/c.bin")).unwrap();
        let outcome = scan(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| panic!("Must not scan an incomplete folder"),
        )
        .unwrap();
        assert_eq!(outcome.report.verdict, Verdict::Failed);
        assert!(outcome.permit.is_none());
    }
}

#[cfg(test)]
mod sharing_regression {
    use super::*;
    #[test]
    fn moving_a_folder_does_not_bypass_a_known_threat_report() {
        let (_dir, store, job) = crate::torrent::completed_folder_fixture();
        let settings = store.antivirus_settings().unwrap();
        let report = Report {
            job_id: job.id.clone(),
            path: job.saved_path.clone().unwrap(),
            scanner: "Owned test fixture".into(),
            settings_key: "fixture".into(),
            verdict: Verdict::Detected,
            message: "Synthetic detection; no harmful content".into(),
            sha256: None,
            finished_at: 1,
            files: vec![],
        };
        store
            .save_antivirus_report(&job, &settings, &report)
            .unwrap();
        store
            .save_torrent_settings(crate::torrent::Settings {
                seed_completed: true,
                ..store.torrent_settings().unwrap()
            })
            .unwrap();
        let target = report.path.with_file_name("Moved collection");
        crate::file_relocation::relocate(
            &store,
            &job,
            target.clone(),
            "Moved collection".into(),
            target.parent().unwrap().into(),
            &CancellationToken::new(),
        )
        .unwrap();
        let moved = store.get(&job.id).unwrap().unwrap();
        assert!(!report.matches_file(&moved));
        assert!(
            store
                .set_torrent_sharing(&job.id, true)
                .unwrap_err()
                .to_string()
                .contains("antivirus reported a threat")
        );
        assert!(!store.torrents.is_running());
    }
}
