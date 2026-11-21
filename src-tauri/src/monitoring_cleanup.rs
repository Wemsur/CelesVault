//! 用户显式执行“清理并重启”时，结束所有其他同名 CelesVault 实例。
#[cfg(windows)]
pub fn terminate_other_instances() -> Result<u32,String> {
    use std::{mem::size_of, path::Path};
    use windows_sys::Win32::{Foundation::*, Storage::FileSystem::SYNCHRONIZE, System::{Diagnostics::ToolHelp::*, Threading::*}};
    struct Owned(HANDLE);
    impl Drop for Owned {fn drop(&mut self){unsafe {CloseHandle(self.0);}}}
    let snapshot=unsafe {CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0)};
    if snapshot==INVALID_HANDLE_VALUE {return Err(format!("无法枚举监测进程：{}",std::io::Error::last_os_error()));}
    let snapshot=Owned(snapshot);
    let mut entry=PROCESSENTRY32W::default();entry.dwSize=size_of::<PROCESSENTRY32W>() as u32;
    let mut found=unsafe {Process32FirstW(snapshot.0,&mut entry)};
    let mut count=0;let mut errors=Vec::new();
    loop {
        if found==0 {
            let code=unsafe {GetLastError()};
            if code!=ERROR_NO_MORE_FILES {errors.push(format!("枚举监测进程失败：Win32={code}"));}
            break;
        }
        let name=String::from_utf16_lossy(&entry.szExeFile[..entry.szExeFile.iter().position(|c|*c==0).unwrap_or(entry.szExeFile.len())]);
        let ours=|name:&str|name.eq_ignore_ascii_case("celesvault.exe")||name.eq_ignore_ascii_case("celesvault-probe.exe");
        if entry.th32ProcessID!=std::process::id() && ours(&name) {
            let pid=entry.th32ProcessID;
            let handle=unsafe {OpenProcess(PROCESS_TERMINATE|PROCESS_QUERY_LIMITED_INFORMATION|SYNCHRONIZE,0,pid)};
            if handle.is_null() {
                let code=unsafe {GetLastError()};
                if code!=ERROR_INVALID_PARAMETER {errors.push(format!("不能结束 {name} PID={pid}：Win32={code}"));}
            } else {
                let handle=Owned(handle);let mut path=vec![0u16;32768];let mut len=path.len() as u32;
                if unsafe {QueryFullProcessImageNameW(handle.0,0,path.as_mut_ptr(),&mut len)}==0 {
                    errors.push(format!("不能核对 PID={pid} 的映像：{}",std::io::Error::last_os_error()));
                } else {
                    let image=String::from_utf16_lossy(&path[..len as usize]);
                    let actual=Path::new(&image).file_name().and_then(|n|n.to_str()).unwrap_or("");
                    // 同一打开句柄核实名称并终止，避免PID退出复用后误杀别的进程。
                    if ours(actual) {
                        let mut exit_code=0;
                        if unsafe {GetExitCodeProcess(handle.0,&mut exit_code)}==0 {errors.push(format!("不能读取 PID={pid} 状态"));}
                        else if exit_code==STILL_ACTIVE as u32 {
                            if unsafe {TerminateProcess(handle.0,1)}==0 {errors.push(format!("结束 PID={pid} 失败：{}",std::io::Error::last_os_error()));}
                            else if unsafe {WaitForSingleObject(handle.0,5000)}!=WAIT_OBJECT_0 {errors.push(format!("PID={pid} 未在5秒内退出"));}
                            else {count+=1;}
                        }
                    }
                }
            }
        }
        found=unsafe {Process32NextW(snapshot.0,&mut entry)};
    }
    if errors.is_empty(){Ok(count)}else{Err(errors.join("；"))}
}
#[cfg(not(windows))]
pub fn terminate_other_instances()->Result<u32,String>{Err("监测进程清理仅支持Windows".into())}
