use super::*;

#[test]
fn codes_and_incomplete_scans_are_classified_conservatively() {
    let mut settings = Settings::default();
    let output = |code, text: &str| Output {
        code: Some(code),
        stdout: text.into(),
        stderr: String::new(),
        truncated: false,
        stopped: None,
    };
    assert_eq!(
        classify(&settings, output(0, "")).0,
        Verdict::NoThreatsFound
    );
    assert_eq!(
        classify(&settings, output(2, "Threat found")).0,
        Verdict::Attention
    );
    assert_eq!(
        classify(&settings, output(2, "Scan error")).0,
        Verdict::Attention
    );
    assert_eq!(classify(&settings, output(-1, "")).0, Verdict::Failed);
    settings.provider = Provider::ClamAv;
    assert_eq!(
        classify(&settings, output(0, "Scanned files: 1\n")).0,
        Verdict::NoThreatsFound
    );
    assert_eq!(
        classify(&settings, output(0, "Scanned files: 0\n")).0,
        Verdict::Attention
    );
    assert_eq!(
        classify(&settings, output(0, "GUI launched")).0,
        Verdict::Attention
    );
    assert_eq!(
        classify(&settings, output(1, "Heuristics.Limits.Exceeded FOUND")).0,
        Verdict::Detected
    );
    assert_eq!(classify(&settings, output(2, "")).0, Verdict::Failed);
    settings.provider = Provider::Custom;
    settings.clean_exit = Some(10);
    settings.detected_exit = Some(20);
    assert_eq!(
        classify(&settings, output(10, "")).0,
        Verdict::NoThreatsFound
    );
    assert_eq!(classify(&settings, output(20, "")).0, Verdict::Detected);
    assert_eq!(classify(&settings, output(0, "")).0, Verdict::Failed);
    let mut truncated = output(10, "");
    truncated.truncated = true;
    assert_eq!(classify(&settings, truncated).0, Verdict::Attention);
}
#[test]
fn file_paths_are_single_literal_arguments_and_builtin_scans_do_not_remediate() {
    let file = Path::new(r"C:\Downloads\a & $(whoami); literal file.txt");
    let mut settings = Settings::default();
    let args = arguments(&settings, file).unwrap();
    assert_eq!(args[4], file.to_str().unwrap());
    assert!(args.iter().any(|a| a == "-DisableRemediation"));
    settings.provider = Provider::ClamAv;
    let args = arguments(&settings, file).unwrap();
    assert_eq!(args.last().unwrap(), file.to_str().unwrap());
    assert!(args.contains(&"--alert-exceeds-max=yes".into()));
    assert!(args.contains(&"--alert-encrypted=yes".into()));
    assert!(
        !args
            .iter()
            .any(|a| a.starts_with("--remove") || a.starts_with("--move"))
    );
    settings.provider = Provider::Custom;
    settings.arguments = vec!["scan".into(), "--file={file}".into(), "&".into()];
    let args = arguments(&settings, file).unwrap();
    assert_eq!(
        args,
        vec![
            "scan".into(),
            format!("--file={}", file.display()),
            "&".into()
        ]
    );
}

// A real child process, restricted to the test binary. No live malware or AV settings.
#[test]
fn scanner_child_fixture() {
    let Ok(mode) = std::env::var("IDM_EX_AV_CHILD") else {
        return;
    };
    match mode.as_str() {
        "external" => {
            println!(
                "IDM_EX_AV_REQUESTED_V1:{}",
                std::env::var("IDM_EX_AV_PROVIDER").unwrap()
            );
            std::process::exit(0);
        }
        "clean" => {
            println!("Scanned files: 1");
            std::process::exit(0);
        }
        "detected" => std::process::exit(1),
        "error" => std::process::exit(7),
        "flood" => {
            use std::io::Write;
            std::io::stdout()
                .write_all(&vec![b'x'; 128 * 1024])
                .unwrap();
            std::io::stderr()
                .write_all(&vec![b'y'; 128 * 1024])
                .unwrap();
            std::process::exit(0);
        }
        "wait" => {
            if let Some(path) = std::env::var_os("IDM_EX_AV_READY") {
                std::fs::write(path, b"ready").unwrap();
            }
            std::thread::sleep(Duration::from_secs(20));
        }
        "descendant" => {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "antivirus::process::tests::scanner_child_fixture",
                    "--nocapture",
                ])
                .env("IDM_EX_AV_CHILD", "wait")
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(200));
            std::process::exit(0);
        }
        _ => panic!("Unknown test mode"),
    }
}
fn fixture_command(mode: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "antivirus::process::tests::scanner_child_fixture",
        "--nocapture",
    ])
    .env("IDM_EX_AV_CHILD", mode);
    cmd
}
#[test]
fn actual_process_runner_handles_codes_and_bounded_output_without_deadlock() {
    for (mode, code) in [("clean", 0), ("detected", 1), ("error", 7), ("flood", 0)] {
        let output = run(
            &mut fixture_command(mode),
            &CancellationToken::new(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output.code, Some(code));
        assert_eq!(output.truncated, mode == "flood");
        assert!(output.stdout.len() <= 16384 && output.stderr.len() <= 16384);
    }
}
#[test]
fn timeout_and_cancellation_stop_owned_processes() {
    let output = run(
        &mut fixture_command("wait"),
        &CancellationToken::new(),
        Duration::from_millis(200),
    )
    .unwrap();
    assert_eq!(output.stopped, Some(Verdict::TimedOut));
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("scanner-ready");
    let mut command = fixture_command("wait");
    command.env("IDM_EX_AV_READY", &ready);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let thread = std::thread::spawn(move || {
        // Readiness proves the suspended child has passed process setup and
        // resumed. A fixed delay could instead test cancellation before startup.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "Scanner did not signal readiness"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        trigger.cancel();
    });
    let output = run(&mut command, &cancel, Duration::from_secs(10));
    thread.join().unwrap();
    let output = output.unwrap();
    assert_eq!(output.stopped, Some(Verdict::Cancelled));
}

#[test]
fn cancellation_before_start_does_not_launch_the_scanner() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("scanner-ready");
    let mut command = fixture_command("wait");
    command.env("IDM_EX_AV_READY", &ready);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = run(&mut command, &cancel, Duration::from_secs(10))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "Scan cancelled");
    assert!(!ready.exists());
}
#[cfg(windows)]
#[test]
fn child_descendants_do_not_outlive_the_scan_or_hold_its_output_open() {
    let start = Instant::now();
    let output = run(
        &mut fixture_command("descendant"),
        &CancellationToken::new(),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(output.code, Some(0));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn external_handoff_requires_exact_acknowledgment_and_never_reports_clean() {
    let output = |code, text: &str| Output {
        code: Some(code),
        stdout: text.into(),
        stderr: String::new(),
        truncated: false,
        stopped: None,
    };
    let token = "IDM_EX_AV_REQUESTED_V1:malwarebytes\n";
    assert_eq!(
        external_request_outcome(Provider::Malwarebytes, output(0, token))
            .unwrap()
            .0,
        Verdict::ScanRequested
    );
    for (code, text) in [
        (0, ""),
        (0, "Scanned files: 1"),
        (1, token),
        (-1073741819, token),
    ] {
        assert!(external_request_outcome(Provider::Malwarebytes, output(code, text)).is_err());
    }
    let mut data = output(0, token);
    data.truncated = true;
    assert!(external_request_outcome(Provider::Malwarebytes, data).is_err());
    let mut data = output(0, token);
    data.stopped = Some(Verdict::TimedOut);
    assert_eq!(
        external_request_outcome(Provider::Malwarebytes, data)
            .unwrap()
            .0,
        Verdict::TimedOut
    );
    let settings = Settings {
        provider: Provider::Malwarebytes,
        ..Default::default()
    };
    assert_eq!(classify(&settings, output(0, token)).0, Verdict::Failed);
}

#[cfg(windows)]
#[test]
fn external_protocol_is_bound_to_the_selected_provider() {
    for provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        // A fixed echo fixture avoids libtest's own stdout preamble. The only
        // interpolated value is an enum constant, never a file path or user input.
        let shell = known_folder(&windows::Win32::UI::Shell::FOLDERID_System)
            .unwrap()
            .join("cmd.exe");
        let echo = |id: &str| {
            let mut command = Command::new(&shell);
            command.args(["/D", "/C", &format!("echo IDM_EX_AV_REQUESTED_V1:{id}")]);
            command
        };
        let mut command = echo(provider.helper_id().unwrap());
        assert_eq!(
            request_external(
                provider,
                &mut command,
                &CancellationToken::new(),
                Duration::from_secs(10)
            )
            .unwrap()
            .0,
            Verdict::ScanRequested
        );
        let other = if provider == Provider::Kaspersky {
            "bitdefender"
        } else {
            "kaspersky"
        };
        command = echo(other);
        assert!(
            request_external(
                provider,
                &mut command,
                &CancellationToken::new(),
                Duration::from_secs(10)
            )
            .is_err()
        );
        assert!(
            request_external(
                provider,
                &mut fixture_command("error"),
                &CancellationToken::new(),
                Duration::from_secs(10)
            )
            .is_err()
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            request_external(provider, &mut command, &cancel, Duration::from_secs(10)).is_err()
        );
        assert_eq!(
            request_external(
                provider,
                &mut fixture_command("wait"),
                &CancellationToken::new(),
                Duration::from_millis(150)
            )
            .unwrap()
            .0,
            Verdict::TimedOut
        );
    }
}
