// Adapted from tools/wps-probe (MIT); independent diagnostic source remains unchanged.
use super::{
    filter::Scope,
    model::{DriveInfo, ProcessInfo},
};
use std::{
    collections::HashMap,
    mem::{size_of, zeroed},
    ptr,
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Security::*,
        Storage::FileSystem::*,
        System::{
            Com::CoTaskMemFree,
            Diagnostics::ToolHelp::*,
            IO::DeviceIoControl,
            Ioctl::*,
            Threading::*,
            WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
        },
        UI::Shell::*,
    },
    core::GUID,
};

pub static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub fn from_wide(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])
}
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn event_unix_ms(filetime: i64) -> u64 {
    ((filetime as u64).saturating_sub(116_444_736_000_000_000)) / 10_000
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub fn elevated() -> bool {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let token = Handle(token);
        let mut info: TOKEN_ELEVATION = zeroed();
        let mut len = 0;
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut info as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        ) != 0
            && info.TokenIsElevated != 0
    }
}

pub fn enable_trace_privilege() -> Result<(), u32> {
    unsafe {
        let mut raw = ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
            &mut raw,
        ) == 0
        {
            return Err(GetLastError());
        }
        let token = Handle(raw);
        let mut luid = zeroed();
        if LookupPrivilegeValueW(
            ptr::null(),
            wide("SeSystemProfilePrivilege").as_ptr(),
            &mut luid,
        ) == 0
        {
            return Err(GetLastError());
        }
        let mut privileges: TOKEN_PRIVILEGES = zeroed();
        privileges.PrivilegeCount = 1;
        privileges.Privileges[0] = LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        };
        SetLastError(0);
        if AdjustTokenPrivileges(token.0, 0, &privileges, 0, ptr::null_mut(), ptr::null_mut()) == 0
        {
            return Err(GetLastError());
        }
        let code = GetLastError();
        if code == 0 { Ok(()) } else { Err(code) }
    }
}

pub fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let mut raw = ptr::null_mut();
        if SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut raw) < 0 || raw.is_null() {
            return None;
        }
        let mut len = 0;
        while len < 32768 && *raw.add(len) != 0 {
            len += 1;
        }
        let path = from_wide(std::slice::from_raw_parts(raw, len));
        CoTaskMemFree(raw.cast());
        Some(path)
    }
}

fn device_bus(drive: &str) -> (Option<i32>, Option<u32>) {
    unsafe {
        let name = wide(&format!("\\\\.\\{}", drive.trim_end_matches('\\')));
        let raw = CreateFileW(
            name.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if raw == INVALID_HANDLE_VALUE {
            return (None, Some(GetLastError()));
        }
        let handle = Handle(raw);
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut buffer = [0u64; 128];
        let mut size = 0;
        let ok = DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&query as *const STORAGE_PROPERTY_QUERY).cast(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buffer.as_mut_ptr().cast(),
            size_of_val(&buffer) as u32,
            &mut size,
            ptr::null_mut(),
        );
        if ok == 0 {
            return (None, Some(GetLastError()));
        }
        if (size as usize) < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return (None, Some(ERROR_INVALID_DATA));
        }
        let descriptor = &*buffer.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>();
        (Some(descriptor.BusType), None)
    }
}

pub fn drives() -> Vec<DriveInfo> {
    let bits = unsafe { GetLogicalDrives() };
    let mut result = Vec::new();
    for index in 0..26 {
        if bits & (1 << index) == 0 {
            continue;
        }
        let drive = format!("{}:", char::from(b'A' + index));
        let root = format!("{drive}\\");
        let kind = unsafe { GetDriveTypeW(wide(&root).as_ptr()) };
        let mut buf = [0u16; 32768];
        let count =
            unsafe { QueryDosDeviceW(wide(&drive).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        let nt_device = (count > 0).then(|| from_wide(&buf));
        let (bus_type, device_query_error) = if kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE {
            device_bus(&drive)
        } else {
            (None, None)
        };
        result.push(DriveInfo {
            root,
            drive_type: kind,
            bus_type,
            selected_automatically: kind == DRIVE_REMOVABLE || bus_type == Some(BusTypeUsb),
            nt_device,
            device_query_error,
        });
    }
    result
}

pub fn update_scope(scope: &mut Scope, drives: &[DriveInfo], automatic: bool) {
    for drive in drives {
        if let Some(device) = &drive.nt_device {
            scope.add_device(device.clone(), drive.root.clone());
        }
        if automatic && (drive.selected_automatically || drive.root.eq_ignore_ascii_case("D:\\")) {
            scope.add_root(drive.root.clone());
        }
    }
}

pub fn default_roots() -> Vec<String> {
    [
        known_folder(&FOLDERID_Desktop),
        known_folder(&FOLDERID_Downloads),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub type ProcessTable = HashMap<u32, ProcessInfo>;
pub type ThreadTable = HashMap<u32, u32>;

pub fn snapshot() -> (ProcessTable, ThreadTable) {
    let mut processes = HashMap::new();
    let mut threads = HashMap::new();
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS | TH32CS_SNAPTHREAD, 0);
        if raw == INVALID_HANDLE_VALUE {
            return (processes, threads);
        }
        let handle = Handle(raw);
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(handle.0, &mut entry) != 0 {
            loop {
                processes.insert(
                    entry.th32ProcessID,
                    ProcessInfo {
                        pid: entry.th32ProcessID,
                        name: from_wide(&entry.szExeFile),
                        executable_path: None,
                        parent_pid: Some(entry.th32ParentProcessID),
                        source: "startup_toolhelp_snapshot".into(),
                    },
                );
                if Process32NextW(handle.0, &mut entry) == 0 {
                    break;
                }
            }
        }
        let mut thread: THREADENTRY32 = zeroed();
        thread.dwSize = size_of::<THREADENTRY32>() as u32;
        if Thread32First(handle.0, &mut thread) != 0 {
            loop {
                threads.insert(thread.th32ThreadID, thread.th32OwnerProcessID);
                if Thread32Next(handle.0, &mut thread) == 0 {
                    break;
                }
            }
        }
    }
    (processes, threads)
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if raw.is_null() {
            return None;
        }
        let handle = Handle(raw);
        let mut path = [0u16; 32768];
        let mut len = path.len() as u32;
        if QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut len) == 0 {
            None
        } else {
            Some(from_wide(&path[..len as usize]))
        }
    }
}

pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    process_path(pid).map(|path| ProcessInfo {
        pid,
        name: path.rsplit('\\').next().unwrap_or(&path).to_owned(),
        executable_path: Some(path),
        parent_pid: None,
        source: "live_process_query_not_historical_proof".into(),
    })
}

