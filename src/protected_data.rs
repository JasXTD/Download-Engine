//! User-scoped Windows DPAPI; no machine-wide flag and no plaintext fallback.
use anyhow::{Result, ensure};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(crate) fn seal(id: Uuid, bytes: &mut [u8]) -> Result<Zeroizing<Vec<u8>>> {
    transform(id, bytes, true)
}
pub(crate) fn open(id: Uuid, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let mut copy = Zeroizing::new(bytes.to_vec());
    transform(id, &mut copy, false)
}
#[cfg(windows)]
fn transform(id: Uuid, bytes: &mut [u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    use zeroize::Zeroize;
    ensure!(bytes.len() <= 512 * 1024, "Invalid protected data size");
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut entropy = *id.as_bytes();
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    // DPAPI borrows the input buffers only for this synchronous call and owns the
    // returned LocalAlloc buffer until we wipe and release it below.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    ensure!(
        ok != 0,
        "Windows could not protect or unlock this data. Use the original Windows account or supply the secret again."
    );
    struct Buffer(CRYPT_INTEGER_BLOB);
    impl Drop for Buffer {
        fn drop(&mut self) {
            if !self.0.pbData.is_null() {
                unsafe {
                    std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                    LocalFree(self.0.pbData.cast());
                }
            }
        }
    }
    let output = Buffer(output);
    ensure!(
        !output.0.pbData.is_null(),
        "Windows returned invalid protected data"
    );
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec(),
    ))
}
#[cfg(not(windows))]
fn transform(_id: Uuid, _bytes: &mut [u8], _encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    anyhow::bail!("Protected data requires Windows protected storage")
}
