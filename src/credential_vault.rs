//! Only app-owned, opaque UUID targets are accessed. Never enumerate credentials.
use anyhow::{Result, bail, ensure};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[cfg(windows)]
use windows_sys::Win32::Security::Credentials::*;

// Concurrent Windows vault calls can observe stale state even for different
// targets. Serialize the complete FFI operation, including releasing read buffers.
#[cfg(windows)]
static API_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(windows)]
fn api_guard() -> Result<std::sync::MutexGuard<'static, ()>> {
    API_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Windows credential access was interrupted. Restart the app."))
}

#[cfg(windows)]
fn target(id: Uuid) -> Vec<u16> {
    format!("IDM EX/Downloads/v1/{id}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

#[cfg(windows)]
pub(crate) fn write(id: Uuid, blob: &mut [u8]) -> Result<()> {
    let _guard = api_guard()?;
    ensure!(
        blob.len() <= CRED_MAX_CREDENTIAL_BLOB_SIZE as usize,
        "These credentials are too long for Windows Credential Manager"
    );
    let mut name = target(id);
    // The actual username belongs in the encrypted blob, not public metadata.
    let mut user: Vec<u16> = "IDM EX".encode_utf16().chain(Some(0)).collect();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: name.as_mut_ptr(),
        UserName: user.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // All pointers are live for this synchronous call; Windows copies the blob.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        bail!(
            "Windows could not save these credentials (error {}).",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn read(id: Uuid) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let _guard = api_guard()?;
    let name = target(id);
    let mut pointer = std::ptr::null_mut();
    // CredReadW allocates one buffer, owned below and freed with CredFree.
    if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == 1168 {
            return Ok(None);
        } // ERROR_NOT_FOUND
        bail!("Windows could not read these credentials (error {code}).");
    }
    ensure!(!pointer.is_null(), "Windows returned an invalid credential");
    struct Owned(*mut CREDENTIALW);
    impl Drop for Owned {
        fn drop(&mut self) {
            // Only zero the valid credential blob returned by Windows, then
            // release the single allocation. No borrowed views outlive Owned.
            unsafe {
                let c = &mut *self.0;
                if !c.CredentialBlob.is_null()
                    && c.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
                {
                    std::slice::from_raw_parts_mut(c.CredentialBlob, c.CredentialBlobSize as usize)
                        .zeroize();
                }
                CredFree(self.0.cast());
            }
        }
    }
    let owned = Owned(pointer);
    // The OS owns the layout; Owned keeps it alive through this copy.
    let c = unsafe { &*owned.0 };
    ensure!(
        c.Type == CRED_TYPE_GENERIC
            && !c.CredentialBlob.is_null()
            && c.CredentialBlobSize > 0
            && c.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE,
        "Windows returned an invalid credential"
    );
    let bytes =
        unsafe { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize) };
    Ok(Some(Zeroizing::new(bytes.to_vec())))
}

#[cfg(windows)]
pub(crate) fn delete(id: Uuid) -> Result<()> {
    let _guard = api_guard()?;
    let name = target(id);
    // Delete only this application's exact UUID target.
    if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code != 1168 {
            bail!("Windows could not remove saved credentials (error {code}).");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn write(_: Uuid, _: &mut [u8]) -> Result<()> {
    bail!("Saved authentication requires Windows");
}
#[cfg(not(windows))]
pub(crate) fn read(_: Uuid) -> Result<Option<Zeroizing<Vec<u8>>>> {
    bail!("Saved authentication requires Windows");
}
#[cfg(not(windows))]
pub(crate) fn delete(_: Uuid) -> Result<()> {
    bail!("Saved authentication requires Windows");
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    #[test]
    fn concurrent_vault_entries_round_trip_without_affecting_each_other() {
        let ids = Arc::new(Mutex::new(Vec::new()));
        let gate = Arc::new(Barrier::new(4));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let ids = ids.clone();
                let gate = gate.clone();
                let failures = failures.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        let id = Uuid::new_v4();
                        ids.lock().unwrap().push(id);
                        let mut data = Zeroizing::new(b"synthetic-vault-regression".to_vec());
                        let write_result = write(id, &mut data);
                        data.zeroize();
                        if let Err(e) = write_result {
                            failures.lock().unwrap().push(format!("write: {e}"));
                        }
                        gate.wait();
                        match read(id) {
                            Ok(Some(bytes))
                                if bytes.as_slice() == b"synthetic-vault-regression" => {}
                            result => failures
                                .lock()
                                .unwrap()
                                .push(format!("read: {:?}", result.map(|r| r.map(|b| b.len())))),
                        }
                        gate.wait();
                        if let Err(e) = delete(id) {
                            failures.lock().unwrap().push(format!("delete: {e}"));
                        }
                        gate.wait();
                        match read(id) {
                            Ok(None) => {}
                            Ok(Some(_)) => failures
                                .lock()
                                .unwrap()
                                .push("credential survived deletion".into()),
                            Err(error) => failures
                                .lock()
                                .unwrap()
                                .push(format!("read after deletion: {error}")),
                        }
                        gate.wait();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        for id in ids.lock().unwrap().iter() {
            delete(*id).unwrap();
        }
        assert!(
            failures.lock().unwrap().is_empty(),
            "{:?}",
            failures.lock().unwrap()
        );
    }
}
