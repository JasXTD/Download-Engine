use super::*;
use crate::paths::AppPaths;

pub(super) fn fixture() -> (tempfile::TempDir, Store, Job, Settings) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let mut job = Job::new(
        "https://example.com/file.txt",
        None,
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    let file = job.directory.join("a space & literal file.txt");
    std::fs::write(&file, b"harmless scan fixture").unwrap();
    job.saved_path = Some(file);
    job.status = Status::Complete;
    store.enqueue(&job).unwrap();
    let settings = Settings {
        enabled: true,
        ..Default::default()
    };
    store.save_antivirus_settings(&settings).unwrap();
    (temp, store, job, settings)
}
#[test]
fn settings_default_off_and_round_trip_without_touching_jobs() {
    let (_temp, store, job, _) = fixture();
    store.save_antivirus_settings(&Settings::default()).unwrap();
    assert!(!store.antivirus_settings().unwrap().enabled);
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().saved_path,
        job.saved_path
    );
    let settings = Settings {
        enabled: true,
        provider: Provider::Custom,
        executable: r"C:\Program Files\AV\scanner.exe".into(),
        arguments: vec!["--scan-only".into(), "{file}".into()],
        clean_exit: Some(0),
        detected_exit: Some(10),
        custom_confirmed: true,
        timeout_seconds: 30,
    };
    store.save_antivirus_settings(&settings).unwrap();
    assert_eq!(store.antivirus_settings().unwrap(), settings);
}
#[test]
fn invalid_custom_configuration_cannot_enable_a_scanner() {
    let mut settings = Settings {
        provider: Provider::Custom,
        executable: r"C:\Program Files\AV\scanner.exe".into(),
        clean_exit: Some(0),
        detected_exit: Some(1),
        custom_confirmed: true,
        ..Default::default()
    };
    assert!(settings.validate().is_ok());
    for args in [
        vec![],
        vec!["--version".into()],
        vec!["{file}".into(), "{file}".into()],
        vec!["{file}\0".into()],
    ] {
        settings.arguments = args;
        assert!(settings.validate().is_err());
    }
    settings.arguments = vec!["{file}".into()];
    for path in [
        "scanner.exe",
        r"C:\AV\scan.cmd",
        r"C:\Windows\cmd.exe",
        r"C:\AV\powershell.exe",
        r"C:\AV\Malwarebytes.exe",
        r"C:\AV\mbbr.exe",
        r"\\server\share\av.exe",
        r"C:\AV\scan.exe:stream",
        r"C:\AV\..\scan.exe",
    ] {
        settings.executable = path.into();
        assert!(settings.validate().is_err(), "{path}");
    }
    settings.executable = r"C:\Program Files\AV\scanner.exe".into();
    settings.detected_exit = Some(0);
    assert!(settings.validate().is_err());
    settings.detected_exit = Some(1);
    settings.custom_confirmed = false;
    assert!(settings.validate().is_err());
}
#[test]
fn corrupt_settings_fail_closed_instead_of_turning_protection_off() {
    let (_temp, store, _, _) = fixture();
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE antivirus_settings SET data=?1",
            ["{\"enabled\":true,\"provider\":\"unknown\"}"],
        )
        .unwrap();
    assert!(store.antivirus_settings().is_err());
}
#[test]
fn clean_scan_persists_history_but_only_fresh_review_has_open_permit() {
    let (_temp, store, job, settings) = fixture();
    let cancel = CancellationToken::new();
    let outcome = scan_using(&store, &job, &settings, &cancel, false, |_, _, _| {
        Ok((Verdict::NoThreatsFound, "fixture".into()))
    })
    .unwrap();
    assert!(outcome.permit.is_none());
    assert_eq!(
        store.antivirus_report(&job.id).unwrap().unwrap().verdict,
        Verdict::NoThreatsFound
    );
    let outcome = scan_using(&store, &job, &settings, &cancel, true, |_, _, _| {
        Ok((Verdict::NoThreatsFound, "fixture".into()))
    })
    .unwrap();
    assert_eq!(
        outcome.permit.as_ref().unwrap().validate(&store).unwrap(),
        job.saved_path.as_deref().unwrap()
    );
    assert_eq!(outcome.report.sha256.as_ref().unwrap().len(), 64);
    drop(outcome);
    std::fs::write(job.saved_path.as_ref().unwrap(), b"different bytes").unwrap();
    let changed = scan_using(&store, &job, &settings, &cancel, true, |_, _, _| {
        Ok((Verdict::Detected, "fixture".into()))
    })
    .unwrap();
    assert!(changed.permit.is_none());
    assert_ne!(
        changed.report.sha256,
        outcome_hash(b"harmless scan fixture")
    );
}
fn outcome_hash(bytes: &[u8]) -> Option<String> {
    Some(format!("{:x}", Sha256::digest(bytes)))
}
#[test]
fn all_failure_verdicts_and_cancellation_block_opening() {
    let (_temp, store, job, settings) = fixture();
    for verdict in [
        Verdict::Detected,
        Verdict::Attention,
        Verdict::Failed,
        Verdict::TimedOut,
        Verdict::Cancelled,
    ] {
        let outcome = scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((verdict, "fixture".into())),
        )
        .unwrap();
        assert!(outcome.permit.is_none());
        assert_eq!(outcome.report.verdict, verdict);
    }
    let cancel = CancellationToken::new();
    let outcome = scan_using(&store, &job, &settings, &cancel, true, |_, _, token| {
        token.cancel();
        Ok((Verdict::NoThreatsFound, "late completion".into()))
    })
    .unwrap();
    assert_eq!(outcome.report.verdict, Verdict::Cancelled);
    assert!(outcome.permit.is_none());
    let outcome = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| anyhow::bail!("scanner missing"),
    )
    .unwrap();
    assert_eq!(outcome.report.verdict, Verdict::Failed);
    assert!(outcome.report.message.contains("scanner missing"));
    assert!(outcome.permit.is_none());
}
#[test]
fn removed_job_or_changed_configuration_cannot_accept_a_late_result() {
    let (_temp, store, job, settings) = fixture();
    let result = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| {
            store.save_antivirus_settings(&Settings::default()).unwrap();
            Ok((Verdict::NoThreatsFound, "fixture".into()))
        },
    );
    assert!(result.is_err());
    assert!(store.antivirus_report(&job.id).unwrap().is_none());
    store.save_antivirus_settings(&settings).unwrap();
    let result = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| {
            store.remove(&job.id).unwrap();
            Ok((Verdict::NoThreatsFound, "fixture".into()))
        },
    );
    assert!(result.is_err());
    assert!(store.antivirus_report(&job.id).unwrap().is_none());
}
#[test]
fn permit_checks_current_job_and_settings_on_explicit_open() {
    let (_temp, store, mut job, settings) = fixture();
    let outcome = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| Ok((Verdict::NoThreatsFound, "fixture".into())),
    )
    .unwrap();
    let permit = outcome.permit.unwrap();
    store.save_antivirus_settings(&Settings::default()).unwrap();
    assert!(permit.validate(&store).is_err());
    store.save_antivirus_settings(&settings).unwrap();
    job.status = Status::Paused;
    store.save(&job).unwrap();
    assert!(permit.validate(&store).is_err());
    job.status = Status::Complete;
    store.save(&job).unwrap();
    store.remove(&job.id).unwrap();
    assert!(permit.validate(&store).is_err());
    assert!(store.antivirus_report(&job.id).unwrap().is_none());
}
#[test]
fn expected_checksum_is_rechecked_before_scanning() {
    let (_temp, store, mut job, settings) = fixture();
    job.integrity.expected = Some("0".repeat(64));
    store.save(&job).unwrap();
    let outcome = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| panic!("Must not scan mismatched bytes"),
    )
    .unwrap();
    assert_eq!(outcome.report.verdict, Verdict::Failed);
    assert!(outcome.report.message.contains("expected checksum"));
    assert!(outcome.permit.is_none());
}
#[cfg(windows)]
#[test]
fn file_and_folder_cannot_be_replaced_between_scan_and_confirmation() {
    let (_temp, store, job, settings) = fixture();
    let outcome = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, path, _| {
            assert!(std::fs::write(path, b"changed").is_err());
            assert!(std::fs::remove_file(path).is_err());
            assert!(
                std::fs::rename(
                    path.parent().unwrap(),
                    path.parent().unwrap().with_extension("moved")
                )
                .is_err()
            );
            Ok((Verdict::NoThreatsFound, "fixture".into()))
        },
    )
    .unwrap();
    assert!(outcome.permit.is_some());
    assert!(std::fs::write(job.saved_path.as_ref().unwrap(), b"changed").is_err());
    drop(outcome);
    std::fs::write(job.saved_path.as_ref().unwrap(), b"changed").unwrap();
}
#[test]
fn missing_file_records_failure_and_queue_cancellation_removes_waiting_work() {
    let (_temp, store, job, settings) = fixture();
    std::fs::remove_file(job.saved_path.as_ref().unwrap()).unwrap();
    let outcome = scan_using(
        &store,
        &job,
        &settings,
        &CancellationToken::new(),
        true,
        |_, _, _| panic!("Missing files must not run a scanner"),
    )
    .unwrap();
    assert_eq!(outcome.report.verdict, Verdict::Failed);
    assert!(outcome.permit.is_none());
    let mut service = Service::default();
    service.enqueue(job.clone(), settings.clone(), false);
    service.enqueue(job.clone(), settings, true);
    assert!(service.busy());
    assert_eq!(service.status(&job.id), Some("Waiting to scan…"));
    service.cancel(&job.id);
    assert!(!service.busy());
}

#[test]
fn malwarebytes_requests_are_not_clean_verdicts_or_verified_open_permits() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, store, job, mut settings) = fixture();
        settings.provider = external_provider;
        store.save_antivirus_settings(&settings).unwrap();
        assert_eq!(
            store.antivirus_settings().unwrap().provider,
            external_provider
        );
        let outcome = scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((Verdict::ScanRequested, "Review in Malwarebytes".into())),
        )
        .unwrap();
        assert_eq!(outcome.report.verdict, Verdict::ScanRequested);
        assert!(outcome.permit.is_none());
        let permit =
            external_review_permit(&store, &job, &outcome.report, &CancellationToken::new())
                .unwrap();
        assert_eq!(
            permit.validate(&store).unwrap(),
            job.saved_path.as_deref().unwrap()
        );
        // A faulty adapter must never label a dispatch acknowledgment as clean.
        let bad = scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((Verdict::NoThreatsFound, "wrong adapter result".into())),
        )
        .unwrap();
        assert_eq!(bad.report.verdict, Verdict::Failed);
        assert!(bad.permit.is_none());
        assert!(
            external_review_permit(&store, &job, &bad.report, &CancellationToken::new()).is_err()
        );
        settings.provider = Provider::Defender;
        store.save_antivirus_settings(&settings).unwrap();
        assert!(
            external_review_permit(&store, &job, &outcome.report, &CancellationToken::new())
                .is_err()
        );
    }
}

#[test]
fn external_manual_review_rechecks_job_and_settings_and_rejects_failed_requests() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, store, mut job, mut settings) = fixture();
        settings.provider = external_provider;
        store.save_antivirus_settings(&settings).unwrap();
        let mut report = scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((Verdict::ScanRequested, "request".into())),
        )
        .unwrap()
        .report;
        for verdict in [
            Verdict::Failed,
            Verdict::Cancelled,
            Verdict::TimedOut,
            Verdict::NoThreatsFound,
        ] {
            report.verdict = verdict;
            assert!(
                external_review_permit(&store, &job, &report, &CancellationToken::new()).is_err()
            );
        }
        report.verdict = Verdict::ScanRequested;
        let original = job.clone();
        job.status = Status::Paused;
        store.save(&job).unwrap();
        assert!(
            external_review_permit(&store, &original, &report, &CancellationToken::new()).is_err()
        );
        job.status = Status::Complete;
        let original_hash = job.integrity.actual.clone();
        job.integrity.actual = Some("ab".repeat(32));
        store.save(&job).unwrap();
        assert!(!report.matches_file(&job));
        assert!(external_review_permit(&store, &job, &report, &CancellationToken::new()).is_err());
        job.integrity.actual = original_hash;
        job.saved_path = Some(job.directory.join("different.txt"));
        store.save(&job).unwrap();
        assert!(
            external_review_permit(&store, &original, &report, &CancellationToken::new()).is_err()
        );
        store.remove(&job.id).unwrap();
        assert!(
            external_review_permit(&store, &original, &report, &CancellationToken::new()).is_err()
        );
    }
}

#[test]
fn external_manual_review_rejects_bytes_changed_after_the_scan_request() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, store, job, mut settings) = fixture();
        settings.provider = external_provider;
        store.save_antivirus_settings(&settings).unwrap();
        let report = scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((Verdict::ScanRequested, "request".into())),
        )
        .unwrap()
        .report;

        std::fs::write(job.saved_path.as_ref().unwrap(), b"replacement bytes").unwrap();
        let error = external_review_permit(&store, &job, &report, &CancellationToken::new())
            .err()
            .expect("changed bytes must not receive an open permit");
        assert!(error.to_string().contains("file changed"));
    }
}
