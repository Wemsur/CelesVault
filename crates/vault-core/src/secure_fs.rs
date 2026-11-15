//! 所有持久写入均限于已锁定、无重解析点的本地目录。句柄不共享 DELETE，
//! 防止检查后祖先目录被改名/换成 junction；只读源绝不创建或改变源属性。
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io, os::windows::{fs::OpenOptionsExt, io::AsRawHandle}, path::{Component, Path, PathBuf}};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE}};

pub fn local_path(value: &str) -> Result<PathBuf, String> {
    let path = lexical_path(value)?;
    let value = path.to_string_lossy();
    let root = format!("{}:\\", value.chars().next().ok_or("路径为空")?.to_ascii_uppercase());
    let kind = unsafe { GetDriveTypeW(wide(&root).as_ptr()) };
    if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
        return Err("只支持当前已挂载的本地固定盘或可移动盘".into());
    }
    Ok(path)
}

pub fn lexical_path(value: &str) -> Result<PathBuf, String> {
    // 不接受 Win32 设备命名空间、UNC、盘符相对路径、ADS 或路径消歧义。
    let value = value.replace('/', "\\");
    let b = value.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\'
        || value.contains('\0') || value.len() > 30_000 {
        return Err("必须选择绝对本地盘符路径，不支持 UNC、设备路径或相对路径".into());
    }
    for part in value[3..].split('\\').filter(|s| !s.is_empty()) {
        if part == "." || part == ".." || part.ends_with(['.', ' '])
            || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c)) || reserved(part) {
            return Err("路径含有歧义、保留名称或非法组件".into());
        }
    }
    Ok(PathBuf::from(value))
}

fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (["COM", "LPT"].iter().any(|p| stem.strip_prefix(p).is_some_and(|s|
            matches!(s, "1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"¹"|"²"|"³"))))
}

pub fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }
fn fail(context: &str, e: impl std::fmt::Display) -> String { format!("{context}：{e}") }
fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut result = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut result) } == 0 {
        return Err(fail("读取文件身份失败", io::Error::last_os_error()));
    }
    if result.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("拒绝符号链接、junction 或其他重解析点".into());
    }
    Ok(result)
}

/// 仅内部接受从已打开本地卷句柄取得的 GUID 路径；外部命令仍必须 local_path。
fn trusted_path(path:&Path)->Result<PathBuf,String> {
    let value=path.to_string_lossy().replace('/',"\\");
    if let Some(rest)=value.strip_prefix("\\\\?\\Volume{") {
        let (id,tail)=rest.split_once("}\\").ok_or("内部卷 GUID 路径不完整")?;
        if id.len()!=36 || uuid::Uuid::parse_str(id).is_err() {return Err("内部卷 GUID 非法".into());}
        for part in tail.split('\\').filter(|s|!s.is_empty()) {
            if part=="." || part==".." || part.ends_with(['.',' ']) || reserved(part)
                || part.chars().any(|c|c<' ' || "<>:\"|?*".contains(c)) {return Err("内部归档路径含非法组件".into());}
        }
        let root=format!("\\\\?\\Volume{{{id}}}\\");
        let kind=unsafe {GetDriveTypeW(wide(&root).as_ptr())};
        if kind!=DRIVE_FIXED && kind!=DRIVE_REMOVABLE {return Err("原数据卷已不可用，不按盘符改读其他卷".into());}
        return Ok(PathBuf::from(value));
    }
    local_path(&value)
}
fn handle_path(file:&File,flags:u32)->Result<PathBuf,String> {
    let mut path=vec![0u16;32768];
    let len=unsafe {GetFinalPathNameByHandleW(file.as_raw_handle().cast(),path.as_mut_ptr(),path.len() as u32,flags)};
    if len==0 || len as usize>=path.len() {return Err(fail("取得原卷句柄路径失败",io::Error::last_os_error()));}
    Ok(PathBuf::from(String::from_utf16_lossy(&path[..len as usize])))
}
pub fn guid_path(file:&File)->Result<PathBuf,String> {
    trusted_path(&handle_path(file,VOLUME_NAME_GUID)?)
}

/// 从首个卷根句柄取得 GUID 路径，此后各级目录 IO 不再经可能重占的盘符。
#[derive(Clone)]
pub struct DirectoryGuard {
    _handles:std::sync::Arc<Vec<File>>,
    _identities:std::sync::Arc<Vec<Identity>>,
    pub path:PathBuf,
}
impl DirectoryGuard {
    pub fn acquire(path:&Path,create:bool)->Result<Self,String> {Self::acquire_inner(path,create,false)}
    pub fn for_publish(path:&Path)->Result<Self,String> {Self::acquire_inner(path,true,true)}
    fn acquire_inner(path:&Path,create:bool,publish:bool)->Result<Self,String> {
        let input=trusted_path(path)?;
        let components:Vec<_>=input.components().collect();
        let mut at=PathBuf::new();let mut handles=Vec::new();let mut identities=Vec::new();
        for (index,component) in components.iter().enumerate() {
            at.push(component.as_os_str());
            if matches!(component,Component::Prefix(_)) {continue;}
            // at 在卷根打开后立即换为原卷 GUID；创建子目录从不沿盘符重新解析。
            if create && matches!(component,Component::Normal(_)) {
                if let Some(previous)=handles.last() {let _:Snapshot=snapshot(previous)?;}
                match fs::create_dir(&at) {
                    Ok(())=>(),Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>(),
                    Err(e)=>return Err(fail("创建原卷持久目录失败",e)),
                }
            }
            let access=FILE_READ_ATTRIBUTES | if publish && index+1==components.len() {FILE_ADD_FILE | FILE_TRAVERSE} else {0};
            let handle=OpenOptions::new().access_mode(access).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(&at).map_err(|e|fail("锁定原卷目录失败",e))?;
            if info(&handle)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY==0 {return Err("目录路径中存在普通文件".into());}
            at=guid_path(&handle)?;
            identities.push(snapshot(&handle)?.identity);handles.push(handle);
        }
        let guard=Self {_handles:std::sync::Arc::new(handles),_identities:std::sync::Arc::new(identities),path:at};
        guard.validate()?;Ok(guard)
    }
    pub fn validate(&self)->Result<(),String> {
        for (file,expected) in self._handles.iter().zip(self._identities.iter()) {
            if snapshot(file)?.identity!=*expected {return Err("原数据目录/卷身份已变化，操作已停止".into());}
        }
        let last=self._identities.last().ok_or("目录没有可验证原卷身份")?;
        // 打开原 GUID 名称验证卷仍在线，不能用旧句柄的缓存元数据猜新盘可用。
        let probe=OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&self.path).map_err(|e|fail("原数据卷/目录已不可用",e))?;
        if snapshot(&probe)?.identity!=*last {return Err("GUID 目录已经换代，不向替代卷写入".into());}
        Ok(())
    }
    pub fn display_path(&self)->Result<PathBuf,String> {
        self.validate()?;
        let file=self._handles.last().ok_or("目录句柄不可用")?;
        let value=handle_path(file,VOLUME_NAME_DOS)?.to_string_lossy().into_owned();
        let value=value.strip_prefix("\\\\?\\").unwrap_or(&value);
        if let Ok(path)=local_path(value) {
            let probe=OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(&path);
            if let Ok(probe)=probe && snapshot(&probe)?.identity==*self._identities.last().ok_or("目录身份不可用")? {return Ok(path);}
        }
        Ok(self.path.clone())
    }
    fn final_handle(&self)->Result<HANDLE,String> {
        self.validate()?;
        self._handles.last().map(|h|h.as_raw_handle().cast()).ok_or("发布目录没有已锁定句柄".into())
    }
}

pub fn relative(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    if rel.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("数据库中的归档相对路径不安全".into());
    }
    let joined = root.join(p);
    trusted_path(&joined)
}

pub struct Opened { pub file: File, pub _parents: DirectoryGuard }
impl Opened {
    pub fn source(path: &Path, metadata_only: bool) -> Result<Self, String> {
        let path = trusted_path(path)?;
        let parent = path.parent().ok_or("源文件没有父目录")?;
        let parents = DirectoryGuard::acquire(parent, false)?;
        parents.validate()?;
        let path=parents.path.join(path.file_name().ok_or("源文件名为空")?);
        let mut options = OpenOptions::new();
        if metadata_only { options.access_mode(FILE_READ_ATTRIBUTES); } else { options.read(true); }
        let file = options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(&path).map_err(|e| fail("只读打开源文件失败", e))?;
        if info(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err("只接受普通文件".into());
        }
        Ok(Self { file, _parents: parents })
    }
    pub fn archive(path: &Path) -> Result<Self, String> {
        let path=trusted_path(path)?;
        let parent = path.parent().ok_or("归档没有父目录")?;
        let parents = DirectoryGuard::acquire(parent, false)?;
        parents.validate()?;
        let path=parents.path.join(path.file_name().ok_or("归档文件名为空")?);
        // 验证时禁止写共享，避免摘要验证途中修改；不共享 DELETE 防止路径换代。
        let file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path).map_err(|e| fail("打开归档失败", e))?;
        if info(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err("归档实体不是普通文件".into());
        }
        Ok(Self { file, _parents: parents })
    }
    pub fn owned(path:&Path,write_data:bool)->Result<Self,String> {
        let parents=DirectoryGuard::acquire(path.parent().ok_or("自有副本没有父目录")?,false)?;
        parents.validate()?;
        let path=parents.path.join(path.file_name().ok_or("自有副本名为空")?);
        let access=FILE_GENERIC_READ | FILE_WRITE_ATTRIBUTES | DELETE | if write_data {FILE_GENERIC_WRITE} else {0};
        let file=OpenOptions::new().access_mode(access)
            .share_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path).map_err(|e|fail("锁定恢复区自有副本失败",e))?;
        ensure_single_link(&file)?;
        if info(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {return Err("恢复区不是普通文件".into());}
        Ok(Self {file,_parents:parents})
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub volume_serial: u64,
    pub volume_name: String,
    pub file_id: [u8; 16],
    pub created: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot { pub identity: Identity, pub size: u64, pub modified: u64, pub changed: i64, pub attributes: u32 }
fn ticks(ft: FILETIME) -> u64 { ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64 }

pub fn snapshot(file: &File) -> Result<Snapshot, String> {
    let base = info(file)?;
    let mut basic=FILE_BASIC_INFO::default();
    if unsafe {GetFileInformationByHandleEx(file.as_raw_handle().cast(),FileBasicInfo,
        (&mut basic as *mut FILE_BASIC_INFO).cast(),std::mem::size_of::<FILE_BASIC_INFO>() as u32)}==0 {
        return Err(fail("无法读取文件变更时间",io::Error::last_os_error()));
    }
    let mut id = FILE_ID_INFO::default();
    let ok = unsafe { GetFileInformationByHandleEx(file.as_raw_handle().cast(), FileIdInfo,
        (&mut id as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) };
    let (serial, identifier) = if ok != 0 { (id.VolumeSerialNumber, id.FileId.Identifier) } else {
        let mut bytes = [0u8; 16];
        let index = ((base.nFileIndexHigh as u64) << 32) | base.nFileIndexLow as u64;
        bytes[..8].copy_from_slice(&index.to_le_bytes());
        (base.dwVolumeSerialNumber as u64, bytes)
    };
    let mut path = vec![0u16; 32768];
    let len = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle().cast(), path.as_mut_ptr(), path.len() as u32, VOLUME_NAME_GUID) };
    if len == 0 || len as usize >= path.len() { return Err(fail("无法取得源卷身份", io::Error::last_os_error())); }
    let path = String::from_utf16_lossy(&path[..len as usize]);
    let end = path.find('}').ok_or("源卷没有稳定卷 GUID，不能安全重试")?;
    if !path.starts_with("\\\\?\\Volume{") { return Err("源卷不是本地卷 GUID 路径".into()); }
    // 某些 FAT 卷根没有文件ID；卷根只用 GUID+serial 验证物理卷，不能用于候选文件。
    if identifier==[0;16] && !(base.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY!=0 && path[end+1..].trim_matches('\\').is_empty()) {
        return Err("文件系统返回了空文件标识，拒绝不可靠的安全重试".into());
    }
    Ok(Snapshot {
        identity: Identity { volume_serial: serial, volume_name: path[..=end].to_ascii_lowercase(), file_id: identifier, created: ticks(base.ftCreationTime) },
        size: ((base.nFileSizeHigh as u64) << 32) | base.nFileSizeLow as u64,
        modified: ticks(base.ftLastWriteTime),
        changed: basic.ChangeTime,
        attributes: basic.FileAttributes,
    })
}

pub fn create_owned(path: &Path) -> Result<File, String> {
    let parents=DirectoryGuard::acquire(path.parent().ok_or("自有文件没有父目录")?,false)?;
    parents.validate()?;
    let path=parents.path.join(path.file_name().ok_or("自有文件名为空")?);
    let file = OpenOptions::new().access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE).create_new(true).share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path).map_err(|e| fail("新建归档临时文件失败", e))?;
    info(&file)?;
    Ok(file)
}

pub fn reject_special_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let opened = Opened::source(path, true)?;
            if info(&opened.file)?.nNumberOfLinks > 1 { return Err("持久元数据不能是外部硬链接".into()); }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fail("检查持久文件失败", e)),
    }
}

pub fn lock_root(root: &Path) -> Result<File, String> {
    let path = root.join("vault.lock");
    reject_special_file(&path)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&path).map_err(|e| fail("数据目录正在被使用或无法锁定", e))?;
    if info(&lock)?.nNumberOfLinks > 1 { return Err("数据目录锁不是自有独立文件".into()); }
    Ok(lock)
}

pub fn ensure_single_link(file: &File) -> Result<(), String> {
    if info(file)?.nNumberOfLinks != 1 { return Err("归档/临时文件不是自有独立副本，拒绝外部硬链接".into()); }
    Ok(())
}

pub fn readonly_handle(file:&File)->Result<(),String> {
    ensure_single_link(file)?;
    let mut permissions = file.metadata().map_err(|e| fail("读取归档属性失败", e))?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions).map_err(|e| fail("保护归档副本失败", e))
}

pub fn publish_handle(file:&File,directory:&DirectoryGuard,name:&str)->Result<PathBuf,String> {
    ensure_single_link(file)?;
    if name.is_empty() || name.contains(['\\','/']) {return Err("发布文件名必须是单一安全组件".into());}
    directory.validate()?;
    let target=trusted_path(&directory.path.join(name))?;
    let units:Vec<u16>=name.encode_utf16().collect();
    let base=std::mem::offset_of!(FILE_RENAME_INFO,FileName);
    let bytes=base+units.len()*2;
    let mut buffer=vec![0u64;bytes.max(std::mem::size_of::<FILE_RENAME_INFO>()).div_ceil(8)];
    // u64 buffer 为 FILE_RENAME_INFO 保证对齐并含完整变长 UTF-16 尾部。
    // root 和 source 句柄全程有效；不替换已存在目标，源没有按路径关闭/重开。
    unsafe {
        let header=&mut *buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        header.Anonymous.ReplaceIfExists=false;
        header.RootDirectory=directory.final_handle()?;
        header.FileNameLength=(units.len()*2) as u32;
        std::ptr::copy_nonoverlapping(units.as_ptr(),buffer.as_mut_ptr().cast::<u8>().add(base).cast::<u16>(),units.len());
        if SetFileInformationByHandle(file.as_raw_handle().cast(),FileRenameInfo,buffer.as_ptr().cast(),bytes.max(std::mem::size_of::<FILE_RENAME_INFO>()) as u32)==0 {
            return Err(fail("按自有句柄发布归档失败（不覆盖）",io::Error::last_os_error()));
        }
    }
    file.sync_all().map_err(|e|fail("同步已发布归档失败",e))?;
    Ok(target)
}

pub fn remove_handle(file:&File)->Result<(),String> {
    ensure_single_link(file)?;
    if info(file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 { return Err("staging 不是普通文件".into()); }
    let mut permissions=file.metadata().map_err(|e|fail("读取临时文件属性失败",e))?.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        file.set_permissions(permissions).map_err(|e|fail("解除自有临时副本只读失败",e))?;
    }
    let disposition=FILE_DISPOSITION_INFO {DeleteFile:true};
    if unsafe {SetFileInformationByHandle(file.as_raw_handle().cast(),FileDispositionInfo,
        (&disposition as *const FILE_DISPOSITION_INFO).cast(),std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32)}==0 {
        return Err(fail("按句柄删除自有临时文件失败",io::Error::last_os_error()));
    }
    Ok(())
}

pub fn remove_owned(path: &Path) -> Result<(), String> {
    // 仅用于 task-known staging；按锁定句柄删除，避免检查后路径被换代。
    let parents = DirectoryGuard::acquire(path.parent().ok_or("staging 没有父目录")?,false)?;
    parents.validate()?;
    let path=parents.path.join(path.file_name().ok_or("staging 文件名为空")?);
    let file = match OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path) {
        Ok(file)=>file,
        Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(()),
        Err(e)=>return Err(fail("锁定自有临时文件失败",e)),
    };
    remove_handle(&file)
}

pub fn safe_filename(name: &str) -> String {
    let mut units=0usize;
    let mut result: String = name.chars().map(|c|
        if c < ' ' || "<>:\"/\\|?*".contains(c) { '_' } else { c }).take_while(|c| {
            units+=c.len_utf16();units<=180
        }).collect();
    result = result.trim_end_matches(['.', ' ']).to_owned();
    if result.is_empty() || reserved(&result) { result.insert(0, '_'); }
    result
}
