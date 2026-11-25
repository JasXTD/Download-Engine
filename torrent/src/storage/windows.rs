use super::directory::Creation;
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
            FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            LocalFree, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError,
            UNICODE_STRING,
        },
        Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        },
        Storage::FileSystem::*,
        System::{
            IO::{DeviceIoControl, IO_STATUS_BLOCK},
            Ioctl::FSCTL_SET_SPARSE,
            SystemServices::FILE_PERSISTENT_ACLS,
        },
    },
};

const DIR_ACCESS: u32 = FILE_TRAVERSE | FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE;

pub(super) fn open_directory(path: &Path, for_move: bool) -> io::Result<File> {
    let access = DIR_ACCESS | if for_move { DELETE } else { 0 };
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    check_type(&file, true)?;
    Ok(file)
}

fn component(name: &OsStr) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name
            .as_encoded_bytes()
            .iter()
            .any(|b| matches!(b, 0 | b':' | b'/' | b'\\'))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

/// RootDirectory and a single counted name keep lookup relative to the live parent.
/// OPEN_REPARSE_POINT also lets us reject a reparse point at the terminal component.
fn open_at(
    parent: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    creation: u32,
    kind: u32,
) -> io::Result<File> {
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let length = u16::try_from(wide.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    let string = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_mut_ptr(),
    };
    // Supply a protected inheritable DACL at creation, before the first child
    // can appear. OW denotes the owner of the object and SY denotes LocalSystem.
    let mut descriptor = ptr::null_mut();
    if creation == FILE_CREATE
        && kind == FILE_DIRECTORY_FILE
        && unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                windows_sys::core::w!("D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &string,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: descriptor.cast(),
        ..Default::default()
    };
    let mut handle = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // The parent and counted UTF-16 buffer remain live throughout the synchronous call.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            share,
            creation,
            kind | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if !descriptor.is_null() {
        unsafe {
            LocalFree(descriptor);
        }
    }
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    // NtCreateFile transferred a newly opened handle to the caller.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn check_type(file: &File, directory: bool) -> io::Result<()> {
    let info = file.metadata()?;
    if info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (directory && !info.is_dir())
        || (!directory && !info.is_file())
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) fn directory_at(parent: &File, name: &OsStr, creation: Creation) -> io::Result<File> {
    component(name)?;
    let disposition = match creation {
        Creation::Open => FILE_OPEN,
        Creation::New => {
            // A filesystem without persistent ACLs can ignore a supplied descriptor.
            let mut flags = 0;
            if unsafe {
                GetVolumeInformationByHandleW(
                    parent.as_raw_handle(),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut flags,
                    ptr::null_mut(),
                    0,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if flags & FILE_PERSISTENT_ACLS == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Choose a volume that supports private folder permissions",
                ));
            }
            FILE_CREATE
        }
        Creation::OpenOrCreate => FILE_OPEN_IF,
    };
    let file = open_at(
        parent,
        name,
        DIR_ACCESS,
        FILE_SHARE_READ,
        disposition,
        FILE_DIRECTORY_FILE,
    )?;
    check_type(&file, true)?;
    Ok(file)
}

pub(super) fn identity(file: &File) -> io::Result<String> {
    let mut info = FILE_ID_INFO::default();
    // This information class returns both the volume and full 128-bit file ID.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "{:x}:{:032x}",
        info.VolumeSerialNumber,
        u128::from_le_bytes(info.FileId.Identifier)
    ))
}

fn check_single_link(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.nNumberOfLinks != 1 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(info)
}

pub(super) fn file_at(
    parent: &File,
    name: &OsStr,
    size: Option<u64>,
    seeding: bool,
) -> io::Result<File> {
    component(name)?;
    // Other peer workers can open the same data. Omitting delete sharing pins the leaf
    // while checking it and opening its Internet-zone stream through the same parent.
    let share = if seeding {
        FILE_SHARE_READ
    } else {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    };
    let (access, disposition) = if size.is_some() {
        (FILE_GENERIC_READ | FILE_GENERIC_WRITE, FILE_OPEN_IF)
    } else {
        (FILE_GENERIC_READ, FILE_OPEN)
    };
    let file = open_at(
        parent,
        name,
        access,
        share,
        disposition,
        FILE_NON_DIRECTORY_FILE,
    )?;
    check_type(&file, false)?;
    let info = check_single_link(&file)?;
    if let Some(len) = size {
        mark_internet(parent, name, &file, share)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_SPARSE_FILE == 0 {
            // Sparse allocation is optional. This operation takes no data buffers.
            let mut returned = 0;
            unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    FSCTL_SET_SPARSE,
                    ptr::null(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut returned,
                    ptr::null_mut(),
                )
            };
        }
        if file.metadata()?.len() != len {
            file.set_len(len)?;
        }
    }
    Ok(file)
}

fn mark_internet(parent: &File, name: &OsStr, file: &File, share: u32) -> io::Result<()> {
    let mut stream = name.to_owned();
    stream.push(":Zone.Identifier");
    let mut marker = open_at(
        parent,
        &stream,
        FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES,
        share,
        FILE_OPEN_IF,
        FILE_NON_DIRECTORY_FILE,
    )
    .map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Cannot mark the file as an Internet download: {e}"),
        )
    })?;
    if identity(&marker)? != identity(file)? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    const ZONE: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    marker.write_all(ZONE)?;
    marker.set_len(ZONE.len() as u64)
}

pub(super) fn create_file(parent: &File, name: &OsStr) -> io::Result<File> {
    component(name)?;
    let file = open_at(
        parent,
        name,
        FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES | DELETE,
        FILE_SHARE_READ,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE,
    )?;
    // A marking failure must not leave an unmarked file available to open.
    if let Err(error) = mark_internet(
        parent,
        name,
        &file,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    ) {
        dispose(&file)?;
        return Err(error);
    }
    Ok(file)
}

pub(super) fn file_for_move(parent: &File, name: &OsStr) -> io::Result<File> {
    component(name)?;
    let file = open_at(
        parent,
        name,
        FILE_GENERIC_READ | DELETE,
        FILE_SHARE_READ,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )?;
    check_type(&file, false)?;
    check_single_link(&file)?;
    Ok(file)
}

/// Rename the checked handle into a single name under the open destination directory.
pub fn rename_into(parent: &File, file: &File, leaf: &OsStr) -> io::Result<()> {
    component(leaf)?;
    let name: Vec<_> = leaf.encode_wide().collect();
    let bytes = u16::try_from(name.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    let size = size_of::<FILE_RENAME_INFORMATION>()
        .max(std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName) + usize::from(bytes));
    let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status = IO_STATUS_BLOCK::default();
    // Pointer-aligned storage includes the variable-length counted name. The zeroed
    // ReplaceIfExists field makes the collision check part of the rename itself.
    let result = unsafe {
        (*info).RootDirectory = parent.as_raw_handle();
        (*info).FileNameLength = u32::from(bytes);
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            info.cast(),
            size as u32,
            FileRenameInformation,
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    Ok(())
}

pub(super) fn remove_at(parent: &File, name: &OsStr, directory: bool) -> io::Result<()> {
    component(name)?;
    let kind = if directory {
        FILE_DIRECTORY_FILE
    } else {
        FILE_NON_DIRECTORY_FILE
    };
    let file = open_at(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ,
        FILE_OPEN,
        kind,
    )?;
    check_type(&file, directory)?;
    dispose(&file)
}

fn dispose(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // The checked handle owns DELETE access. The kernel requires directories to be empty.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
