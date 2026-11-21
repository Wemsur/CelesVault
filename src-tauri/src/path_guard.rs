// SPDX-License-Identifier: GPL-3.0-only
//! Windows 本地路径边界，只接受受校验的浏览与文件操作参数。
//! 不移植网络、链接解析、任意命令执行或原始资料归档修改业务。
//!
//! 每级目录都以 OPEN_REPARSE_POINT 打开，并持有不共享 DELETE 的 File。
//! 内部 I/O 使用来自句柄的卷 GUID 路径，不能因盘符重新分配而改写另一块盘。
//! 最终提交/清理按自己的文件句柄完成，不存在关闭暂存后按名称重新打开的窗口。
//! 崩溃遗留的 .celesvault-* 文件只隐藏，不扫描或自动删除；阶段 6 才做可信恢复。

#[cfg(windows)]
mod windows {
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, OpenOptions},
        io::{self, Read, Seek, SeekFrom},
        mem::{offset_of, size_of},
        os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle},
        path::{Path, PathBuf},
        ptr,
        sync::Arc,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_BASIC_INFO, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN,
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo, FileDispositionInfo, FileRenameInfo,
            GetDriveTypeW, GetFileInformationByHandle, GetFileInformationByHandleEx,
            GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
            SetFileInformationByHandle, VOLUME_NAME_GUID,
        },
        System::{SystemServices::FILE_READ_ONLY_VOLUME, WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE}},
    };

    const MAX_PATH_UNITS: usize = 4096;
    const MAX_DEPTH: usize = 128;
    const MAX_FINAL_PATH_UNITS: usize = MAX_PATH_UNITS + 128;
    pub(crate) const MAX_NAME_UNITS: usize = 180;

    pub(crate) fn io_error(context: &str, error: io::Error) -> String {
        // Win32 错误码保留用于诊断，不把系统本地化英文文本当作应用文案。
        match error.raw_os_error() {
            Some(code) => format!("{context}（系统错误 {code}）"),
            None => format!("{context}（文件系统错误）"),
        }
    }

    pub(crate) fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn reserved(name: &str) -> bool {
        let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
                })
            })
    }

    pub(crate) fn valid_component(value: &str) -> bool {
        !value.is_empty()
            && value != "."
            && value != ".."
            && !value.ends_with(['.', ' '])
            && value.encode_utf16().count() <= 255
            && !value.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            && !reserved(value)
    }

    /// 不进行 canonicalize 消歧义：UNC、\\?\、ADS、相对路径和保留组件一律拒绝。
    pub(crate) fn local_path(value: &str) -> Result<PathBuf, String> {
        if value.encode_utf16().count() > MAX_PATH_UNITS {
            return Err("目录路径过长，阶段 2 最多支持 4096 个 UTF-16 字符".into());
        }
        let value = value.replace('/', "\\");
        let bytes = value.as_bytes();
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':' || bytes[2] != b'\\'
        {
            return Err("必须使用本地盘符的绝对路径，不支持网络、设备命名空间或相对路径".into());
        }
        let rest = &value[3..];
        if rest.starts_with('\\') {
            return Err("路径含有重叠分隔符，无法安全消除歧义".into());
        }
        let rest = rest.strip_suffix('\\').unwrap_or(rest);
        let parts: Vec<&str> = if rest.is_empty() { Vec::new() } else { rest.split('\\').collect() };
        if parts.len() > MAX_DEPTH || parts.iter().any(|part| !valid_component(part)) {
            return Err("路径含有重叠分隔符、歧义组件、保留名称或非法字符".into());
        }
        let mut normalized = format!("{}:\\", (bytes[0] as char).to_ascii_uppercase());
        normalized.push_str(&parts.join("\\"));
        Ok(PathBuf::from(normalized))
    }

    /// 仅限宿主已锁定并校验的 ExportSource；绝不能用于 browse/target/data_root 输入。
    fn internal_volume_path(value: &str) -> Result<PathBuf, String> {
        if value.encode_utf16().count() > MAX_FINAL_PATH_UNITS {
            return Err("内部归档卷路径过长".into());
        }
        let rest = value.strip_prefix("\\\\?\\Volume{")
            .ok_or("内部归档来源必须是可信的本地卷 GUID 路径")?;
        let (guid, relative) = rest.split_once("}\\")
            .ok_or("内部归档来源缺少完整的卷 GUID 根")?;
        if relative.starts_with('\\') {
            return Err("内部归档路径含有重叠分隔符".into());
        }
        if guid.len() != 36 || !guid.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        }) {
            return Err("内部归档卷 GUID 格式无效".into());
        }
        let relative = relative.strip_suffix('\\').unwrap_or(relative);
        let parts: Vec<&str> = if relative.is_empty() { Vec::new() } else { relative.split('\\').collect() };
        if parts.len() > MAX_DEPTH || parts.iter().any(|part| !valid_component(part)) {
            return Err("内部归档路径含有歧义组件或非法名称".into());
        }
        let root = format!("\\\\?\\Volume{{{guid}}}\\");
        Ok(PathBuf::from(root).join(parts.join("\\")))
    }

    fn local_drive(root: &Path) -> Result<(), String> {
        let root = root.to_str().ok_or("目录根路径不是有效 Unicode")?;
        // SAFETY: root 是严格验证后的盘符根或仅内部使用的卷 GUID 根；缓冲 NUL 结尾。
        let kind = unsafe { GetDriveTypeW(wide(root).as_ptr()) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
            return Err("只支持当前挂载的本地固定盘和可移动盘，不支持网络盘或其他设备".into());
        }
        Ok(())
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub(crate) struct FileKey {
        pub(crate) volume: u32,
        pub(crate) index: u64,
        pub(crate) created: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub(crate) struct FileStamp {
        pub(crate) key: FileKey,
        pub(crate) size: u64,
        pub(crate) modified: u64,
        /// Windows ChangeTime 同时捕获属性/名称等变更；审批令牌不能只比较大小/mtime。
        pub(crate) changed: i64,
        pub(crate) attributes: u32,
        pub(crate) links: u32,
    }

    pub(crate) fn file_stamp(file: &File) -> Result<FileStamp, String> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: File 在调用期间拥有 HANDLE；info 是正确大小的可写输出结构。
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
            return Err(io_error("读取文件或目录身份失败", io::Error::last_os_error()));
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("拒绝符号链接、junction、挂载点及其他重解析点".into());
        }
        let mut basic = FILE_BASIC_INFO::default();
        // SAFETY: File 与完整初始化的 FILE_BASIC_INFO 输出均在同步调用期间存活。
        if unsafe { GetFileInformationByHandleEx(file.as_raw_handle().cast(), FileBasicInfo,
            ptr::from_mut(&mut basic).cast(), size_of::<FILE_BASIC_INFO>() as u32) } == 0 {
            return Err(io_error("读取文件变更时间失败，不能确认审批身份", io::Error::last_os_error()));
        }
        let combine = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
        Ok(FileStamp {
            key: FileKey {
                volume: info.dwVolumeSerialNumber,
                index: combine(info.nFileIndexHigh, info.nFileIndexLow),
                created: combine(info.ftCreationTime.dwHighDateTime, info.ftCreationTime.dwLowDateTime),
            },
            size: combine(info.nFileSizeHigh, info.nFileSizeLow),
            modified: combine(info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime),
            changed: basic.ChangeTime,
            attributes: info.dwFileAttributes,
            links: info.nNumberOfLinks,
        })
    }

    /// 总是校验整个文件并复位到开头；调用方另核验摘要读取前后的完整 stamp。
    pub(crate) fn file_hash(file: &mut File) -> Result<String, String> {
        file.seek(SeekFrom::Start(0)).map_err(|error| io_error("定位已固定文件失败", error))?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| io_error("校验已固定文件摘要失败", error))?;
            if count == 0 { break; }
            hash.update(&buffer[..count]);
        }
        file.seek(SeekFrom::Start(0)).map_err(|error| io_error("复位已固定文件失败", error))?;
        Ok(hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub(crate) fn final_path(file: &File) -> Result<String, String> {
        let mut buffer = vec![0u16; MAX_FINAL_PATH_UNITS + 1];
        // SAFETY: File 保持 HANDLE 存活；buffer 容量与传入长度一致。
        let count = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle().cast(), buffer.as_mut_ptr(), buffer.len() as u32, VOLUME_NAME_GUID)
        } as usize;
        if count == 0 {
            return Err(io_error("解析目录的卷 GUID 最终路径失败", io::Error::last_os_error()));
        }
        if count >= buffer.len() {
            return Err("句柄最终路径过长，无法安全确认本地目标".into());
        }
        let result = String::from_utf16(&buffer[..count]).map_err(|_| "句柄路径不是有效 Unicode")?;
        if !result.starts_with("\\\\?\\Volume{") || !result.contains("}\\") || result.contains('\0') {
            return Err("句柄没有可验证的本地卷 GUID 路径，拒绝继续操作".into());
        }
        Ok(result)
    }

    fn path_key(value: &str) -> String {
        value.trim_end_matches('\\').to_lowercase()
    }

    pub(crate) fn same_path(a: &str, b: &str) -> bool {
        path_key(a) == path_key(b)
    }

    pub(crate) fn at_or_below(path: &str, root: &str) -> bool {
        let path = path_key(path);
        let root = path_key(root);
        path == root || path.strip_prefix(&root).is_some_and(|suffix| suffix.starts_with('\\'))
    }

    fn volume_info(file: &File) -> Result<(u32, u32, String, String, u32), String> {
        let mut serial = 0;
        let mut component_length = 0;
        let mut label = [0u16; 261];
        let mut filesystem = [0u16; 64];
        let mut flags = 0u32;
        // SAFETY: 所有非空输出指针均指向存活的对应类型/长度缓冲；未请求的字段用空指针。
        let result = unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle().cast(), label.as_mut_ptr(), label.len() as u32,
                &mut serial, &mut component_length, &mut flags, filesystem.as_mut_ptr(), filesystem.len() as u32,
            )
        };
        if result == 0 {
            return Err(io_error("读取本地卷代际失败，设备可能已拔出", io::Error::last_os_error()));
        }
        let label_length = label.iter().position(|unit| *unit == 0).unwrap_or(label.len());
        let label = String::from_utf16(&label[..label_length]).map_err(|_| "卷标不是有效 Unicode")?;
        if component_length == 0 {
            return Err("文件系统未提供可靠的文件名长度限制".into());
        }
        let fs_length = filesystem.iter().position(|unit| *unit == 0).unwrap_or(filesystem.len());
        let filesystem = String::from_utf16(&filesystem[..fs_length]).map_err(|_| "文件系统名称编码无效")?.to_ascii_lowercase();
        if filesystem.is_empty() || filesystem.contains('|') { return Err("文件系统没有可验证的格式标識".into()); }
        Ok((serial, component_length, label, filesystem, flags))
    }

    fn open_directory(path: &Path) -> Result<File, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| io_error("读取目录元数据失败", e))?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("目录或祖先包含符号链接、junction 或挂载点，拒绝访问".into());
        }
        if !metadata.is_dir() {
            return Err("所选路径或其祖先不是目录".into());
        }
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path).map_err(|e| io_error("打开并保护目录失败", e))?;
        if file_stamp(&file)?.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("路径在检查期间不再是普通目录".into());
        }
        Ok(file)
    }

    struct HeldDirectory {
        file: File,
        key: FileKey,
        final_path: String,
    }

    pub(crate) struct DirectoryGuard {
        pub(crate) path: PathBuf,
        pub(crate) final_path: String,
        pub(crate) identity: String,
        mount_root: PathBuf,
        nodes: Vec<HeldDirectory>,
        write_anchor: Option<File>,
        serial: u32,
        component_length: usize,
        label: String,
        filesystem: String,
        volume_flags: u32,
    }

    impl DirectoryGuard {
        pub(crate) fn open(value: &str) -> Result<Self, String> {
            Self::open_guard(local_path(value)?, false)
        }

        /// 只在 Rust 归档来源分支调用，用户目录输入永远不能走此入口。
        pub(crate) fn open_internal(value: &str) -> Result<Self, String> {
            Self::open_guard(internal_volume_path(value)?, true)
        }

        fn open_guard(path: PathBuf, internal: bool) -> Result<Self, String> {
            let text = path.to_str().ok_or("目录路径不是有效 Unicode")?;
            let root_length = if internal {
                text.find("}\\").ok_or("内部目录缺少卷 GUID 根")? + 2
            } else { 3 };
            let mount_root = PathBuf::from(&text[..root_length]);
            local_drive(&mount_root)?;
            let mut at = mount_root.clone();
            let mut nodes = Vec::new();
            let mut append = |at: &Path| -> Result<String, String> {
                let file = open_directory(at)?;
                let key = file_stamp(&file)?.key;
                let final_path = final_path(&file)?;
                nodes.push(HeldDirectory { file, key, final_path: final_path.clone() });
                Ok(final_path)
            };
            let root_final = append(&at)?;
            // 根句柄取得后，后续祖先/叶目录只从原卷 GUID 解析；盘符仅做后置校验。
            at = PathBuf::from(root_final);
            for part in text[root_length..].split('\\').filter(|part| !part.is_empty()) {
                at.push(part);
                append(&at)?;
            }
            let root = nodes.first().ok_or("目录缺少根句柄")?;
            let leaf = nodes.last().ok_or("目录缺少目标句柄")?;
            let (serial, component_length, label, filesystem, volume_flags) = volume_info(&root.file)?;
            if serial != root.key.volume || nodes.iter().any(|node| node.key.volume != serial) {
                return Err("目录链的卷身份不一致，拒绝跨挂载点操作".into());
            }
            let identity = format!(
                "win-dir-v1|{:08x}:{:016x}:{:016x}|{:016x}:{:016x}|{}|{}",
                serial, root.key.index, root.key.created, leaf.key.index, leaf.key.created,
                path_key(&root.final_path), path_key(&leaf.final_path),
            );
            let final_path = leaf.final_path.clone();
            let result = Self {
                path, final_path, identity, mount_root, nodes, write_anchor: None, serial,
                component_length: component_length as usize, label, filesystem, volume_flags,
            };
            result.validate()?;
            Ok(result)
        }

        pub(crate) fn for_copy(value: &str) -> Result<Self, String> {
            Self::for_access(value, FILE_ADD_FILE)
        }

        pub(crate) fn for_create(value: &str) -> Result<Self, String> {
            Self::for_access(value, FILE_ADD_SUBDIRECTORY)
        }

        fn for_access(value: &str, access: u32) -> Result<Self, String> {
            let mut guard = Self::open(value)?;
            let anchor = guard.write_handle(access)?;
            guard.write_anchor = Some(anchor);
            guard.validate()?;
            Ok(guard)
        }

        fn write_handle(&self, access: u32) -> Result<File, String> {
            if self.volume_flags & FILE_READ_ONLY_VOLUME != 0 { return Err("该卷为只读介质，不能写入".into()); }
            // 实际申请目录写访问，不凭本地/可移动标签或文件只读属性猜测 ACL。
            let anchor = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | access)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&self.final_path).map_err(|e| io_error("目录未授予所需写访问权限", e))?;
            if file_stamp(&anchor)?.key != *self.leaf_key()
                || !same_path(&final_path(&anchor)?, &self.final_path)
            { return Err("可写目录句柄与已选择目录身份不一致".into()); }
            Ok(anchor)
        }

        pub(crate) fn writable(&self) -> Result<bool, String> {
            self.validate()?;
            match self.write_handle(FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY) {
                Ok(_) => { self.validate()?; Ok(true) }
                Err(_) => { self.validate()?; Ok(false) }
            }
        }

        pub(crate) fn volume_id(&self) -> Result<String, String> {
            let root = &self.nodes[0];
            let volume = crate::devices::normalize_volume_identifier(&root.final_path)?;
            // SUBST/同卷别名的显示根可能是子目录；卷代际必须取真正 GUID 卷根，
            // 不能把显示根文件 ID 或盘符隐式变成分区 ID。
            let key = if same_path(&root.final_path, &volume) { root.key.clone() } else {
                let file = open_directory(Path::new(&volume))?;
                let key = file_stamp(&file)?.key;
                if key.volume != self.serial { return Err("卷根代际与已固定目录不一致".into()); }
                key
            };
            Ok(format!("win-volume-v1|{volume}|{:08x}|{}|{:016x}:{:016x}",
                self.serial, self.filesystem, key.index, key.created))
        }

        pub(crate) fn volume_root(&self) -> Result<String, String> {
            crate::devices::normalize_volume_identifier(&self.nodes[0].final_path)
        }

        /// 浏览目录身份与作为复制目标打开的同一个 DirectoryGuard 严格一致。
        pub(crate) fn entry_identity(&self, stamp: &FileStamp, path: &str) -> Result<String, String> {
            let root = &self.nodes[0];
            if stamp.key.volume != self.serial { return Err("目录项跨越已固定卷，拒绝返回身份".into()); }
            if stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                Ok(format!("win-dir-v1|{:08x}:{:016x}:{:016x}|{:016x}:{:016x}|{}|{}",
                    self.serial, root.key.index, root.key.created, stamp.key.index, stamp.key.created,
                    path_key(&root.final_path), path_key(path)))
            } else {
                if stamp.key.index == 0 { return Err("文件系统没有提供非空文件身份".into()); }
                Ok(format!("win-entry-v1|{}|{:08x}:{:016x}:{:016x}|{}:{:016x}:{:016x}:{:08x}:{}|{}",
                    self.volume_root()?, stamp.key.volume, stamp.key.index, stamp.key.created,
                    stamp.size, stamp.modified, stamp.changed, stamp.attributes, stamp.links, path_key(path)))
            }
        }

        /// 现有目录项允许文件系统原生的 255 字符名；新建/导出仍用保守 name_limit。
        pub(crate) fn entry_path(&self, name: &str) -> Result<PathBuf, String> {
            if !valid_component(name) || name.encode_utf16().count() > self.component_length { return Err("目录项名称非法或超过文件系统限制".into()); }
            let path = PathBuf::from(&self.final_path).join(name);
            if path.to_string_lossy().encode_utf16().count() > MAX_FINAL_PATH_UNITS { return Err("目录项最终路径过长".into()); }
            Ok(path)
        }

        pub(crate) fn open_entry(&self, name: &str, read_data: bool) -> Result<File, io::Error> {
            let path = self.entry_path(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | if read_data { GENERIC_READ } else { 0 })
                .share_mode(if read_data { FILE_SHARE_READ } else { FILE_SHARE_READ | FILE_SHARE_WRITE })
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | if read_data { FILE_FLAG_SEQUENTIAL_SCAN } else { 0 })
                .open(&path)?;
            let stamp = file_stamp(&file).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            if stamp.key.volume != self.serial || !same_path(&final_path(&file).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?, &path.to_string_lossy()) {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            Ok(file)
        }

        pub(crate) fn file(&self) -> &File {
            // 构造成功时 nodes 至少包含盘根；不导出可构造的字段。
            self.write_anchor.as_ref().unwrap_or(&self.nodes[self.nodes.len() - 1].file)
        }

        pub(crate) fn label(&self) -> &str { &self.label }

        pub(crate) fn name_limit(&self) -> usize {
            self.component_length.min(MAX_NAME_UNITS)
        }

        pub(crate) fn child(&self, name: &str) -> Result<PathBuf, String> {
            if !valid_component(name) || name.encode_utf16().count() > self.name_limit() {
                return Err("导出文件名不合法或超过目标文件系统限制".into());
            }
            let result = PathBuf::from(&self.final_path).join(name);
            if result.as_os_str().to_string_lossy().encode_utf16().count() > MAX_FINAL_PATH_UNITS {
                return Err("导出文件的最终路径过长".into());
            }
            Ok(result)
        }

        pub(crate) fn open_bound_entry(&self, name: &str) -> Result<File, String> {
            let path = self.entry_path(name)?;
            let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | GENERIC_READ | DELETE)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_SEQUENTIAL_SCAN)
                .open(&path).map_err(|error| io_error("固定现有目标失败；文件正在写入、待删除或权限不足", error))?;
            if !same_path(&final_path(&file)?, &path.to_string_lossy()) { return Err("现有目标的句柄路径不匹配".into()); }
            Ok(file)
        }

        /// 验证保留的句柄与卷代际；外部目标另验证盘符，内部来源只验证 GUID 根。
        pub(crate) fn validate(&self) -> Result<(), String> {
            local_drive(&self.mount_root)?;
            let root = &self.nodes[0];
            let (serial, component_length, _, filesystem, flags) = volume_info(&root.file)?;
            if serial != self.serial || component_length as usize != self.component_length
                || filesystem != self.filesystem || flags != self.volume_flags {
                return Err("目标卷代际已变化或设备不可用，复制已停止".into());
            }
            for node in &self.nodes {
                let stamp = file_stamp(&node.file)?;
                if stamp.key != node.key || stamp.attributes & FILE_ATTRIBUTE_DIRECTORY == 0
                    || !same_path(&final_path(&node.file)?, &node.final_path)
                {
                    return Err("目录或祖先身份已变化，拒绝继续访问".into());
                }
            }
            let live_root = open_directory(&self.mount_root)?;
            if file_stamp(&live_root)?.key != root.key
                || !same_path(&final_path(&live_root)?, &root.final_path)
            {
                return Err("盘符已指向另一卷，禁止自动切换导出目标".into());
            }
            let live_leaf = open_directory(&self.path)?;
            let leaf = &self.nodes[self.nodes.len() - 1];
            if file_stamp(&live_leaf)?.key != leaf.key
                || !same_path(&final_path(&live_leaf)?, &self.final_path)
            {
                return Err("当前路径不再指向已选择的目录，必须重新选择目标".into());
            }
            Ok(())
        }

        pub(crate) fn entry_exists(&self, name: &str) -> Result<bool, String> {
            match fs::symlink_metadata(self.child(name)?) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(io_error("无法确认目标名称是否已存在", e)),
            }
        }

        fn leaf_key(&self) -> &FileKey { &self.nodes[self.nodes.len() - 1].key }

        fn has_key(&self, key: &FileKey) -> bool {
            self.nodes.iter().any(|node| &node.key == key)
        }
    }

    pub(crate) struct ProtectedRoot {
        guard: DirectoryGuard,
    }

    impl ProtectedRoot {
        pub(crate) fn open(value: &str) -> Result<Self, String> {
            Ok(Self { guard: DirectoryGuard::open(value)? })
        }

        pub(crate) fn validate(&self) -> Result<(), String> { self.guard.validate() }

        pub(crate) fn identity(&self) -> &str { &self.guard.identity }
        pub(crate) fn volume_root(&self) -> Result<String, String> { self.guard.volume_root() }

        pub(crate) fn reject_mutation(&self, path: &str) -> Result<(), String> {
            self.validate()?;
            if at_or_below(path, &self.guard.final_path) || at_or_below(&self.guard.final_path, path) {
                return Err("不能重命名或回收应用持久目录、内部资料或其祖先目录".into());
            }
            Ok(())
        }

        pub(crate) fn reject_target(&self, target: &DirectoryGuard) -> Result<(), String> {
            self.validate()?;
            let inside = target.has_key(self.guard.leaf_key())
                || at_or_below(&target.final_path, &self.guard.final_path);
            if inside {
                return Err("应用持久目录及其内部资料不能作为浏览或导出目标".into());
            }
            // data 的祖先可以用于导航和导出。所有导出仅创建一个经校验的直接子文件，
            // 并且按句柄严格不覆盖；同名 data 子目录只能跳过/另取名，不能穿入。
            Ok(())
        }

        pub(crate) fn hides_entry(&self, path: &Path) -> bool {
            at_or_below(&path.to_string_lossy(), &self.guard.final_path)
        }

        pub(crate) fn source_path(&self, path: &Path) -> Result<PathBuf, String> {
            let text = path.to_str().ok_or("归档来源路径不是有效 Unicode")?;
            // 只有 ExportSource 的 Rust-only 分支能到达这里。用户目标和 data_root
            // 从不调用此函数，仍严格拒绝所有设备命名空间。
            let path = if text.starts_with("\\\\?\\Volume{") {
                internal_volume_path(text)?
            } else {
                // 兼容先前内部 DOS 来源，但立即转换为已 pin 数据根的 GUID 路径，
                // 后续源父目录、文件和快照不再通过盘符重开。
                let legacy = local_path(text)?;
                let relative = legacy.strip_prefix(&self.guard.path)
                    .map_err(|_| "归档来源不在已配置的持久目录内")?;
                let final_source = PathBuf::from(&self.guard.final_path).join(relative);
                internal_volume_path(final_source.to_str().ok_or("归档最终来源不是有效 Unicode")?)?
            };
            let archive = PathBuf::from(&self.guard.final_path).join("archive");
            let path_text = path.to_str().ok_or("归档来源路径无效")?;
            let archive_text = archive.to_str().ok_or("归档目录路径无效")?;
            if !at_or_below(path_text, archive_text) || same_path(path_text, archive_text) {
                return Err("导出来源必须是持久目录 archive 内已验证的归档资料".into());
            }
            Ok(path)
        }

        pub(crate) fn open_archive(&self, path: &Path, parents: Arc<DirectoryGuard>) -> Result<ArchiveFile, String> {
            self.validate()?;
            let path = self.source_path(path)?;
            let parent = path.parent().ok_or("归档文件没有父目录")?;
            if !same_path(parent.to_str().ok_or("归档目录不是有效 Unicode")?, parents.path.to_str().ok_or("归档目录不是有效 Unicode")?) {
                return Err("归档目录锁与来源路径不匹配".into());
            }
            parents.validate()?;
            let archive = PathBuf::from(&self.guard.final_path).join("archive");
            if !at_or_below(&parents.final_path, archive.to_str().ok_or("归档最终路径无效")?) {
                return Err("归档来源最终路径不在受保护的 archive 目录内".into());
            }
            let name = path.file_name().and_then(|name| name.to_str()).ok_or("归档文件名无效")?;
            // 归档原名可超过导出名上限，不能套用目标 child 的 180 字符限制。
            let real_path = PathBuf::from(&parents.final_path).join(name);
            let file = OpenOptions::new().read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
                .open(&real_path).map_err(|e| io_error("只读打开归档失败，文件可能缺失或正被修改", e))?;
            let stamp = file_stamp(&file)?;
            if stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 || stamp.links != 1 {
                return Err("只允许导出普通、非硬链接的归档文件".into());
            }
            if !same_path(&final_path(&file)?, path.to_str().ok_or("归档最终来源无效")?) {
                return Err("归档文件最终路径与宿主的已验证来源租约不匹配".into());
            }
            parents.validate()?;
            Ok(ArchiveFile { file, stamp, _parents: parents })
        }
    }

    pub(crate) fn user_name(name: &str, limit: usize) -> Result<(), String> {
        if !valid_component(name) || name.encode_utf16().count() > limit
            || name.to_ascii_lowercase().starts_with(".celesvault-")
            || matches!(name.to_ascii_lowercase().as_str(), "$recycle.bin" | "system volume information") {
            return Err("名称必须是安全的单一 Windows 文件名；不能使用保留名称、路径或应用暂存名称".into());
        }
        Ok(())
    }

    /// 用户显式文件管理的来源：固定父链并校验来源快照。
    /// 重命名拒绝写/删共享；Shell 回收允许 DELETE 共享，否则会妨碍系统自己回收。
    /// 回收即时重核身份并核实同句柄进入回收站，不宣称恶意同用户并发绝对免疫。
    pub(crate) struct EntryGuard {
        pub(crate) file: File,
        pub(crate) parents: DirectoryGuard,
        pub(crate) stamp: FileStamp,
        pub(crate) final_path: String,
        pub(crate) display_path: PathBuf,
    }

    impl EntryGuard {
        pub(crate) fn open(value: &str, identity: &str, rename: bool) -> Result<Self, String> {
            if identity.is_empty() || identity.len() > 32_768 { return Err("缺少有效目录项身份，请刷新后重试".into()); }
            let display_path = local_path(value)?;
            let text = display_path.to_str().ok_or("目录项路径编码无效")?;
            if text.len() <= 3 { return Err("不能重命名或回收驱动器根目录".into()); }
            let parent = display_path.parent().and_then(|path| path.to_str()).ok_or("目录项没有安全父目录")?;
            let parents = if rename { DirectoryGuard::for_access(parent, FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY)? } else { DirectoryGuard::open(parent)? };
            let name = display_path.file_name().and_then(|name| name.to_str()).ok_or("目录项名称编码无效")?;
            let path = parents.entry_path(name)?;
            let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | GENERIC_READ | DELETE)
                .share_mode(if rename { FILE_SHARE_READ } else { FILE_SHARE_READ | FILE_SHARE_DELETE })
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(&path).map_err(|error| io_error("无法固定目录项；权限不足、文件正在写入或待删除", error))?;
            let stamp = file_stamp(&file)?;
            let final_path = final_path(&file)?;
            if !same_path(&final_path, &path.to_string_lossy()) { return Err("目录项最终身份与浏览路径不一致".into()); }
            if stamp.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 && stamp.links != 1 { return Err("不修改外部硬链接文件".into()); }
            if parents.entry_identity(&stamp, &final_path)? != identity { return Err("目录项已变化，必须刷新并重新确认，未执行操作".into()); }
            let entry = Self { file, parents, stamp, final_path, display_path };
            entry.validate()?;
            Ok(entry)
        }

        pub(crate) fn validate(&self) -> Result<(), String> {
            self.parents.validate()?;
            if file_stamp(&self.file)? != self.stamp || !same_path(&final_path(&self.file)?, &self.final_path) {
                return Err("已选择的目录项身份或内容已变化，操作已停止".into());
            }
            Ok(())
        }

        pub(crate) fn rename(&self, name: &str) -> Result<(), String> {
            user_name(name, self.parents.name_limit())?;
            self.validate()?;
            let destination = self.parents.child(name)?;
            if self.parents.entry_exists(name)? {
                let existing = self.parents.open_entry(name, false).map_err(|error| io_error("确认重命名目标失败", error))?;
                if file_stamp(&existing)?.key != self.stamp.key { return Err("重命名目标已存在，不会覆盖已有文件或目录".into()); }
            }
            rename_no_replace(&self.file, &self.parents, name).map_err(|error| io_error("按来源句柄重命名失败，未覆盖目标", error))?;
            let actual = file_stamp(&self.file)?;
            if actual.key != self.stamp.key || actual.size != self.stamp.size
                || !same_path(&final_path(&self.file)?, &destination.to_string_lossy()) {
                return Err("重命名已提交但身份复核失败，请刷新目录；不会再次按名称修改".into());
            }
            self.parents.validate()
        }
    }

    pub(crate) fn rename_no_replace(file: &File, target: &DirectoryGuard, name: &str) -> Result<(), io::Error> {
        target.validate().map_err(io::Error::other)?;
        let destination = target.child(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let filename: Vec<u16> = destination.as_os_str().encode_wide().collect();
        let filename_bytes = filename.len() * size_of::<u16>();
        let bytes = (offset_of!(FILE_RENAME_INFO, FileName) + filename_bytes).max(size_of::<FILE_RENAME_INFO>());
        let mut storage = vec![0u64; bytes.div_ceil(size_of::<u64>())];
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // SAFETY: u64 缓冲覆盖头和全部 UTF-16 尾数组且对齐。Win32 FileRenameInfo
        // 使用完整卷 GUID 路径和空 RootDirectory；不共享 DELETE 的祖先链全程保持锁定。
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = ptr::null_mut();
            (*info).FileNameLength = filename_bytes as u32;
            ptr::copy_nonoverlapping(filename.as_ptr(), ptr::addr_of_mut!((*info).FileName).cast::<u16>(), filename.len());
            if SetFileInformationByHandle(file.as_raw_handle().cast(), FileRenameInfo, info.cast(), bytes as u32) == 0 { return Err(io::Error::last_os_error()); }
        }
        Ok(())
    }

    pub(crate) struct ArchiveFile {
        pub(crate) file: File,
        pub(crate) stamp: FileStamp,
        _parents: Arc<DirectoryGuard>,
    }

    pub(crate) struct OwnedTemp {
        pub(crate) file: File,
        key: FileKey,
        original_path: String,
        published_path: Option<String>,
        active: bool,
    }

    impl OwnedTemp {
        pub(crate) fn create(target: &DirectoryGuard, name: &str) -> Result<Self, io::Error> {
            let path = target.child(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            let file = OpenOptions::new().read(true).write(true).create_new(true)
                .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ)
                .attributes(FILE_ATTRIBUTE_NORMAL)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
                .open(&path)?;
            // create_new 已原子证明 File 是本次新建对象；这里不关闭或按路径重新打开。
            let stamp = match file_stamp(&file) {
                Ok(stamp) => stamp,
                Err(_) => {
                    // 身份无法确认时不删除：保留 .celesvault-*，禁止假装安全清理。
                    return Err(io::Error::from(io::ErrorKind::Other));
                }
            };
            let mut result = Self {
                file, key: stamp.key, original_path: path.to_string_lossy().into_owned(), published_path: None, active: true,
            };
            if stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 || stamp.links != 1
                || result.verify().is_err()
            {
                let _ = result.cleanup();
                return Err(io::Error::from(io::ErrorKind::Other));
            }
            Ok(result)
        }

        pub(crate) fn verify(&self) -> Result<(), String> {
            let stamp = file_stamp(&self.file)?;
            if stamp.key != self.key || stamp.links != 1
                || stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                || !same_path(&final_path(&self.file)?, &self.original_path)
            {
                return Err("本任务暂存文件身份发生变化，拒绝提交或按路径删除".into());
            }
            Ok(())
        }

        /// 仅用于本次已知自有意图/备份记录；保留在磁盘供显式恢复，Drop 不删除。
        pub(crate) fn preserve(&mut self) { self.active = false; }

        pub(crate) fn key(&self) -> &FileKey { &self.key }

        pub(crate) fn cleanup(&mut self) -> Result<(), String> {
            if !self.active { return Ok(()); }
            self.verify()?;
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: 持有带 DELETE 权限的自有 File；删除信息结构大小/指针正确。
            // 按句柄标记删除，绝不调用 remove_file 清理可被调换的名称。
            if unsafe {
                SetFileInformationByHandle(
                    self.file.as_raw_handle().cast(), FileDispositionInfo,
                    ptr::from_ref(&disposition).cast(), size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0 {
                return Err(io_error("清理本任务暂存文件失败，文件已保留且不会自动删除", io::Error::last_os_error()));
            }
            self.active = false;
            Ok(())
        }

        /// 相对名称 + 已 pin 的目标目录句柄，严格不替换。失败时没有路径式 fallback。
        pub(crate) fn publish(&mut self, target: &DirectoryGuard, name: &str) -> Result<(), io::Error> {
            let destination = target.child(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            self.verify().map_err(|_| io::Error::from(io::ErrorKind::Other))?;
            rename_no_replace(&self.file, target, name)?;
            // 已是正式导出，后续复核失败也不把正式文件当暂存自动删除。
            self.active = false;
            self.published_path = Some(destination.to_string_lossy().into_owned());
            Ok(())
        }

        pub(crate) fn verify_published(&mut self, target: &DirectoryGuard, name: &str, expected_size: u64, expected_hash: &str) -> Result<(), String> {
            let stamp = file_stamp(&self.file)?;
            let destination = target.child(name)?;
            let destination = destination.to_str().ok_or("导出最终路径无效")?;
            // FAT/exFAT 的文件 ID 可能依赖目录项位置，改名时可变；创建时间也不能
            // 假定跨发布缓存/落盘精度不变。身份连续性来自本次成功按句柄 rename、
            // 始终持有且拒绝写/删共享的同一个 File，而非重开路径后只比较大小。
            // 此例外只用于自有提交边界；审批/暂存阶段仍比较完整 FileKey/Stamp。
            if self.active || !self.published_path.as_deref().is_some_and(|path| same_path(path, destination))
                || stamp.key.volume != self.key.volume || stamp.key.index == 0
                || stamp.size != expected_size || stamp.links != 1
                || stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                || !same_path(&final_path(&self.file)?, destination)
            {
                return Err("文件已提交，但最终导出身份或大小复核失败，请检查目标设备".into());
            }
            if file_hash(&mut self.file)? != expected_hash || file_stamp(&self.file)? != stamp {
                return Err("文件已提交，但最终导出摘要或元数据复核失败，请检查目标设备".into());
            }
            target.validate()
        }
    }

    impl Drop for OwnedTemp {
        fn drop(&mut self) {
            // 正常错误/取消走显式 cleanup 以反馈错误；这里只是 panic 的保守 RAII 兜底。
            let _ = self.cleanup();
        }
    }

    fn truncate_units(value: &str, limit: usize) -> String {
        let mut result = String::new();
        let mut count = 0;
        for character in value.chars() {
            let units = character.len_utf16();
            if count + units > limit { break; }
            result.push(character);
            count += units;
        }
        result
    }

    pub(crate) fn safe_name(display_name: &str, limit: usize) -> Result<String, String> {
        if display_name.is_empty() || display_name.encode_utf16().count() > 1024 {
            return Err("资料显示名为空或超过阶段 2 的 1024 字符限制".into());
        }
        let value: String = display_name.chars().map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }
        }).collect();
        let mut value = value.trim_matches(['.', ' ']).to_string();
        if value.is_empty() { value = "未命名资料".into(); }
        if reserved(&value) { value.insert(0, '_'); }
        let (stem, extension) = split_name(&value);
        // 不截断扩展名，也不移除扩展名；无法保留时拒绝这次请求。
        let extension_units = extension.encode_utf16().count();
        if extension_units + 16 >= limit {
            return Err("资料扩展名过长，无法安全保留原扩展名并处理重名".into());
        }
        let stem = truncate_units(stem, limit - extension_units);
        let stem = stem.trim_end_matches(['.', ' ']);
        let result = format!("{}{extension}", if stem.is_empty() { "资料" } else { stem });
        if !valid_component(&result) { return Err("资料显示名无法转换为安全的 Windows 文件名".into()); }
        Ok(result)
    }

    fn split_name(value: &str) -> (&str, &str) {
        match value.rfind('.') {
            Some(index) if index > 0 => (&value[..index], &value[index..]),
            _ => (value, ""),
        }
    }

    pub(crate) fn conflict_name(base: &str, ordinal: usize, limit: usize) -> Result<String, String> {
        if ordinal == 0 { return Ok(base.to_string()); }
        let (stem, extension) = split_name(base);
        let suffix = format!(" ({})", ordinal + 1);
        let reserved_units = extension.encode_utf16().count() + suffix.encode_utf16().count();
        if reserved_units >= limit { return Err("重名编号超过目标文件名长度限制".into()); }
        let stem = truncate_units(stem, limit - reserved_units);
        let stem = stem.trim_end_matches(['.', ' ']);
        let result = format!("{stem}{suffix}{extension}");
        if !valid_component(&result) { return Err("无法生成安全的保留两份文件名".into()); }
        Ok(result)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct TestDirectory(PathBuf);
        impl TestDirectory {
            fn new() -> Self {
                let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
                let path = std::env::temp_dir().join(format!("celesvault-path-guard-{}-{nonce}", std::process::id()));
                fs::create_dir(&path).unwrap();
                Self(path)
            }
            fn guard(&self) -> DirectoryGuard { DirectoryGuard::for_copy(self.0.to_str().unwrap()).unwrap() }
        }
        impl Drop for TestDirectory {
            fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
        }

        #[test]
        fn hash_reads_whole_file_and_resets_cursor_for_copy() {
            use std::io::Write;
            let directory = TestDirectory::new();
            let guard = directory.guard();
            let mut temp = OwnedTemp::create(&guard, ".celesvault-hash.tmp").unwrap();
            temp.file.write_all(b"abc").unwrap();
            temp.file.seek(SeekFrom::Start(2)).unwrap();
            let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
            assert_eq!(file_hash(&mut temp.file).unwrap(), expected);
            assert_eq!(temp.file.stream_position().unwrap(), 0);
            let mut bytes = Vec::new();
            temp.file.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"abc");
            assert_eq!(file_hash(&mut temp.file).unwrap(), expected);
            assert_eq!(temp.file.stream_position().unwrap(), 0);
            temp.cleanup().unwrap();
        }

        #[test]
        fn published_handle_allows_rename_key_transition_but_requires_digest_and_path() {
            use std::io::Write;
            let directory = TestDirectory::new();
            let guard = directory.guard();
            let mut temp = OwnedTemp::create(&guard, ".celesvault-long-staging-name.tmp").unwrap();
            temp.file.write_all(b"abc").unwrap();
            temp.file.sync_all().unwrap();
            let digest = file_hash(&mut temp.file).unwrap();
            // 暂存阶段仍拒绝 key 不同，不能把发布边界的规则用于普通重开。
            let original_key = temp.key.clone();
            temp.key.index ^= 1;
            assert!(temp.verify().is_err());
            temp.key = original_key;
            assert!(temp.verify_published(&guard, "a.txt", 3, &digest).is_err());
            temp.publish(&guard, "a.txt").unwrap();
            // 在 NTFS 测试机也明确覆盖 FAT/exFAT 改名后的 ID/创建时间变化分支。
            temp.key.index ^= 1;
            temp.key.created ^= 1;
            temp.verify_published(&guard, "a.txt", 3, &digest).unwrap();
            assert!(temp.verify_published(&guard, "b.txt", 3, &digest).is_err());
            let same_size_wrong_digest = format!("{:x}", Sha256::digest(b"xyz"));
            assert!(temp.verify_published(&guard, "a.txt", 3, &same_size_wrong_digest).is_err());
            assert!(temp.verify_published(&guard, "a.txt", 4, &digest).is_err());
            temp.key.volume ^= 1;
            assert!(temp.verify_published(&guard, "a.txt", 3, &digest).is_err());
        }

        #[test]
        fn three_successive_publications_keep_target_valid() {
            use std::io::Write;
            let directory = TestDirectory::new();
            let guard = directory.guard();
            for index in 0..3 {
                let mut temp = OwnedTemp::create(&guard, &format!(".celesvault-batch-{index}.tmp")).unwrap();
                let bytes = format!("课程资料 {index}").into_bytes();
                temp.file.write_all(&bytes).unwrap();
                temp.file.sync_all().unwrap();
                let digest = file_hash(&mut temp.file).unwrap();
                let name = format!("资料-{index}.txt");
                temp.publish(&guard, &name).unwrap();
                temp.verify_published(&guard, &name, bytes.len() as u64, &digest).unwrap();
                guard.validate().unwrap();
            }
            assert_eq!(fs::read_dir(&guard.final_path).unwrap().count(), 3);
        }

        #[test]
        fn rejects_ambiguous_and_reserved_user_names() {
            for name in ["", ".", "..", "CON", "con.docx", "LPT1", "COM¹.txt", "name.", "name ", "a/b", "a\\b", "x:stream", "a|b"] {
                assert!(!valid_component(name), "{name}");
            }
            assert!(valid_component("语文课件 2026.pptx"));
            assert!(user_name(".celesvault-export-1.backup", 180).is_err());
            assert!(user_name("$Recycle.Bin", 180).is_err());
        }
        #[test]
        fn exported_and_conflict_names_preserve_extension() {
            assert_eq!(safe_name("CON.docx", 180).unwrap(), "_CON.docx");
            assert_eq!(conflict_name("课件.pptx", 1, 180).unwrap(), "课件 (2).pptx");
            let long = format!("{}.pdf", "课程".repeat(200));
            let base = safe_name(&long, 180).unwrap();
            let second = conflict_name(&base, 999, 180).unwrap();
            assert!(second.ends_with(".pdf"));
            assert!(second.encode_utf16().count() <= 180);
            assert!(valid_component(&second));
        }
        #[test]
        fn path_input_never_accepts_device_unc_or_relative_namespaces() {
            for path in ["relative", "D:relative", "\\\\server\\share", "\\\\?\\C:\\folder", "D:\\a\\..\\b", "D:\\a:stream", "D:\\\\a"] { assert!(local_path(path).is_err()); }
            assert_eq!(local_path("e:/课程/课件").unwrap(), PathBuf::from("E:\\课程\\课件"));
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::*;
