// SPDX-License-Identifier: GPL-3.0-only
//! 阶段 2 的 Windows 本地路径边界。只供 fs_jobs 使用，不接受前端传来的设备路径。
//! 参考 Sigma dir_reader 的元数据分层设计；没有移植网络、链接解析或文件管理业务。
//!
//! 每级目录都以 OPEN_REPARSE_POINT 打开，并持有不共享 DELETE 的 File。
//! 内部 I/O 使用来自句柄的卷 GUID 路径，不能因盘符重新分配而改写另一块盘。
//! 最终提交/清理按自己的文件句柄完成，不存在关闭暂存后按名称重新打开的窗口。
//! 崩溃遗留的 .celesvault-* 文件只隐藏，不扫描或自动删除；阶段 6 才做可信恢复。

#[cfg(windows)]
mod windows {
    use std::{
        fs::{self, File, OpenOptions},
        io,
        mem::{offset_of, size_of},
        os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle},
        path::{Path, PathBuf},
        ptr,
        sync::Arc,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ADD_FILE, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN,
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo, FileRenameInfo,
            GetDriveTypeW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
            GetVolumeInformationByHandleW, SetFileInformationByHandle, VOLUME_NAME_GUID,
        },
        System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
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

    fn valid_component(value: &str) -> bool {
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

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FileKey {
        pub(crate) volume: u32,
        pub(crate) index: u64,
        pub(crate) created: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FileStamp {
        pub(crate) key: FileKey,
        pub(crate) size: u64,
        pub(crate) modified: u64,
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
        let combine = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
        Ok(FileStamp {
            key: FileKey {
                volume: info.dwVolumeSerialNumber,
                index: combine(info.nFileIndexHigh, info.nFileIndexLow),
                created: combine(info.ftCreationTime.dwHighDateTime, info.ftCreationTime.dwLowDateTime),
            },
            size: combine(info.nFileSizeHigh, info.nFileSizeLow),
            modified: combine(info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime),
            attributes: info.dwFileAttributes,
            links: info.nNumberOfLinks,
        })
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

    fn volume_info(file: &File) -> Result<(u32, u32, String), String> {
        let mut serial = 0;
        let mut component_length = 0;
        let mut label = [0u16; 261];
        // SAFETY: 所有非空输出指针均指向存活的对应类型/长度缓冲；未请求的字段用空指针。
        let result = unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle().cast(), label.as_mut_ptr(), label.len() as u32,
                &mut serial, &mut component_length, ptr::null_mut(), ptr::null_mut(), 0,
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
        Ok((serial, component_length, label))
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
            let mut append = |at: &Path| -> Result<(), String> {
                let file = open_directory(at)?;
                let key = file_stamp(&file)?.key;
                let final_path = final_path(&file)?;
                nodes.push(HeldDirectory { file, key, final_path });
                Ok(())
            };
            append(&at)?;
            for part in text[root_length..].split('\\').filter(|part| !part.is_empty()) {
                at.push(part);
                append(&at)?;
            }
            let root = nodes.first().ok_or("目录缺少根句柄")?;
            let leaf = nodes.last().ok_or("目录缺少目标句柄")?;
            let (serial, component_length, label) = volume_info(&root.file)?;
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
                component_length: component_length as usize, label,
            };
            result.validate()?;
            Ok(result)
        }

        pub(crate) fn for_copy(value: &str) -> Result<Self, String> {
            let mut guard = Self::open(value)?;
            // 浏览不要求写权限。复制额外获取同一目标目录的 FILE_ADD_FILE 句柄，
            // 既提前拒绝不可写 ACL，也为相对 FileRenameInfo 提供可写目录锚点。
            let anchor = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_ADD_FILE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&guard.final_path).map_err(|e| io_error("目标目录不可写，不能开始导出", e))?;
            if file_stamp(&anchor)?.key != *guard.leaf_key()
                || !same_path(&final_path(&anchor)?, &guard.final_path)
            {
                return Err("可写目标句柄与已选择目录身份不一致".into());
            }
            guard.write_anchor = Some(anchor);
            guard.validate()?;
            Ok(guard)
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

        /// 验证保留的句柄与卷代际；外部目标另验证盘符，内部来源只验证 GUID 根。
        pub(crate) fn validate(&self) -> Result<(), String> {
            local_drive(&self.mount_root)?;
            let root = &self.nodes[0];
            let (serial, component_length, _) = volume_info(&root.file)?;
            if serial != self.serial || component_length as usize != self.component_length {
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

    pub(crate) struct ArchiveFile {
        pub(crate) file: File,
        pub(crate) stamp: FileStamp,
        _parents: Arc<DirectoryGuard>,
    }

    pub(crate) struct OwnedTemp {
        pub(crate) file: File,
        key: FileKey,
        original_path: String,
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
                file, key: stamp.key, original_path: path.to_string_lossy().into_owned(), active: true,
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
            target.child(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            self.verify().map_err(|_| io::Error::from(io::ErrorKind::Other))?;
            let filename: Vec<u16> = name.encode_utf16().collect();
            let filename_bytes = filename.len() * size_of::<u16>();
            let bytes = (offset_of!(FILE_RENAME_INFO, FileName) + filename_bytes)
                .max(size_of::<FILE_RENAME_INFO>());
            // u64 保证 FILE_RENAME_INFO 在 x86/x64 下都满足对齐；整块缓冲初始化为零。
            let mut storage = vec![0u64; bytes.div_ceil(size_of::<u64>())];
            let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
            // SAFETY: 分配字节数覆盖头与全部 UTF-16 尾数组，指针对齐，源/目标不重叠。
            // RootDirectory 的 File 存活到同步 Win32 调用结束，不缓存或转移裸 HANDLE。
            unsafe {
                (*info).Anonymous.ReplaceIfExists = false;
                (*info).RootDirectory = target.file().as_raw_handle().cast();
                (*info).FileNameLength = filename_bytes as u32;
                ptr::copy_nonoverlapping(filename.as_ptr(), ptr::addr_of_mut!((*info).FileName).cast::<u16>(), filename.len());
                if SetFileInformationByHandle(self.file.as_raw_handle().cast(), FileRenameInfo, info.cast(), bytes as u32) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // 已是正式导出，后续复核失败也不把正式文件当暂存自动删除。
            self.active = false;
            Ok(())
        }

        pub(crate) fn verify_published(&self, target: &DirectoryGuard, name: &str, expected_size: u64) -> Result<(), String> {
            let stamp = file_stamp(&self.file)?;
            let destination = target.child(name)?;
            if stamp.key != self.key || stamp.size != expected_size || stamp.links != 1
                || !same_path(&final_path(&self.file)?, destination.to_str().ok_or("导出最终路径无效")?)
            {
                return Err("文件已提交，但最终导出身份或大小复核失败，请检查目标设备".into());
            }
            Ok(())
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
}

#[cfg(windows)]
pub(crate) use windows::*;
