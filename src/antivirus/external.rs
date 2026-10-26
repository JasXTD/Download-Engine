//! Requests the selected antivirus action through the running Explorer. A successful call
//! acknowledges a request, not scan completion or a malware verdict.
use super::Provider;
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command, time::Duration};
use tokio_util::sync::CancellationToken;

pub(super) fn request(
    provider: Provider,
    file: &Path,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<(super::Verdict, String)> {
    let exe = std::env::current_exe().context("Cannot locate the Download Engine scan helper")?;
    let mut command = Command::new(exe);
    command
        .arg("--antivirus-scan-request")
        .arg(
            provider
                .helper_id()
                .context("Unsupported external scanner")?,
        )
        .arg(file);
    super::process::request_external(provider, &mut command, cancel, timeout)
}

pub fn helper_request(provider: Provider, file: &Path) -> Result<()> {
    ensure!(provider.external_review(), "Unsupported external scanner");
    let _parents = super::process::guard_parents(file)?;
    let _file = crate::download_file::read_completed(file)?;
    #[cfg(windows)]
    {
        windows::request(provider, file)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Explorer antivirus integration requires Windows")
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use ::windows::Win32::{
        System::{
            Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IDispatch, IServiceProvider},
            Ole::{OleInitialize, OleUninitialize},
            Variant::VARIANT,
        },
        UI::Shell::{
            IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
            SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
        },
    };
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    use windows_core::{BSTR, Interface};
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                OleUninitialize();
            }
        }
    }
    pub(super) fn request(provider: Provider, file: &Path) -> Result<()> {
        let dll = super::super::installation::handler(provider)?;
        let _folders = super::super::process::guard_parents(&dll)?;
        let _program = crate::download_file::read_completed(&dll)?;
        let folder_name: Vec<u16> = file
            .parent()
            .context("Missing file folder")?
            .as_os_str()
            .encode_wide()
            .collect();
        let file_name: Vec<u16> = file
            .file_name()
            .context("Missing file name")?
            .encode_wide()
            .collect();
        unsafe {
            OleInitialize(None)?;
            let _apartment = Apartment;
            // The Malwarebytes handler returns while its worker is still
            // dispatching the scan and does not retain a Shell thread reference.
            // Host it in Explorer, which outlives this helper. Creating a local
            // Shell.Application or loading its DLL here can silently lose requests.
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
                .context("Cannot connect to Windows Explorer")?;
            let mut hwnd = 0;
            let desktop = windows
                .FindWindowSW(
                    &VARIANT::from(0i32),
                    &VARIANT::default(),
                    SWC_DESKTOP,
                    &mut hwnd,
                    SWFO_NEEDDISPATCH,
                )
                .context("Cannot find the running Explorer desktop")?;
            let desktop_provider: IServiceProvider = desktop
                .cast()
                .context("Explorer desktop services are unavailable")?;
            let browser: IShellBrowser = desktop_provider
                .QueryService(&SID_STopLevelBrowser)
                .context("Explorer desktop browser is unavailable")?;
            let dispatch: IDispatch = browser
                .QueryActiveShellView()
                .context("Explorer desktop view is unavailable")?
                .GetItemObject(SVGIO_BACKGROUND)
                .context("Explorer desktop automation is unavailable")?;
            let view: IShellFolderViewDual = dispatch
                .cast()
                .context("Explorer folder automation is unavailable")?;
            let shell: IShellDispatch2 = view
                .Application()?
                .cast()
                .context("Explorer application automation is unavailable")?;
            let folder = shell.NameSpace(&VARIANT::from(BSTR::from_wide(&folder_name)))?;
            let item = folder.ParseName(&BSTR::from_wide(&file_name))?;
            let resolved = OsString::from_wide(&item.Path()?);
            ensure!(
                Path::new(&resolved) == file,
                "Explorer resolved a different scan target"
            );
            let verbs = item.Verbs()?;
            let count = verbs.Count()?;
            ensure!(
                (1..=256).contains(&count),
                "Unexpected Explorer action count"
            );
            let mut actions = Vec::new();
            let mut names = Vec::new();
            for index in 0..count {
                let action = verbs.Item(&VARIANT::from(index))?;
                names.push(action.Name()?.to_string());
                actions.push(action);
            }
            actions[scan_action_index(provider, &names)?].DoIt()?;
        }
        Ok(())
    }
}

fn scan_action_index(provider: Provider, names: &[String]) -> Result<usize> {
    let expected = match provider {
        Provider::Malwarebytes => "Scan with Malwarebytes",
        Provider::Kaspersky => "Scan for viruses",
        Provider::Bitdefender => "Scan with Bitdefender",
        _ => anyhow::bail!("Unsupported external scanner"),
    };
    let mut matches = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.replace('&', "").trim() == expected);
    let (index, _) = matches.next().with_context(|| format!("{} scan action is unavailable in this language/version. The English Explorer scan action must be exposed as an individual command. Scan the file manually in the antivirus app.", provider.name()))?;
    ensure!(
        matches.next().is_none(),
        "Multiple {} scan actions found; no action was invoked",
        provider.name()
    );
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_actions_are_exact_unique_and_never_default_to_open() {
        for (provider, label) in [
            (Provider::Malwarebytes, "Scan with Malwarebytes"),
            (Provider::Kaspersky, "Scan for viruses"),
            (Provider::Bitdefender, "Scan with Bitdefender"),
        ] {
            let names = vec!["Open".into(), label.into(), "Delete".into()];
            assert_eq!(scan_action_index(provider, &names).unwrap(), 1);
            assert!(scan_action_index(provider, &["Open".into(), "Delete".into()]).is_err());
            assert!(
                scan_action_index(provider, &[label.into(), label.replace("Scan", "&Scan")])
                    .is_err()
            );
            assert!(scan_action_index(provider, &[format!("{label} (other)")]).is_err());
            assert!(scan_action_index(provider, &[provider.name().into()]).is_err());
            assert!(scan_action_index(provider, &[]).is_err());
        }
        assert!(scan_action_index(Provider::Defender, &["Scan for viruses".into()]).is_err());
    }
}
