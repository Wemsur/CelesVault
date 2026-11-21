// SPDX-License-Identifier: GPL-3.0-only
//! Windows 物理磁盘、卷代际和正常 PnP 安全弹出。
//! 磁盘号仅用于本次 IOCTL/SetupAPI 关联，绝不进入持久设备 ID。
//! 无序列号、重复序列号、缺少可靠磁盘标识或拓扑不完整时保守禁用身份/弹出。

use crate::fs_jobs::DriveInfo;

const UNSUPPORTED: &str = "物理设备识别与安全弹出仅支持 Windows 本地磁盘，未回退到模拟模式";

/// 内部统一为小写卷 GUID 根。接受 core 的 GUID 路径和本模块的卷/目录身份，
/// 不接受盘符、磁盘编号、任意嵌入 GUID 的文本或网络路径作为卷标识。
pub fn normalize_volume_identifier(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
        return Err("卷标识为空、过长或含非法字符".into());
    }
    let value = if value.starts_with("win-dir-v1|") {
        let parts: Vec<_> = value.split('|').collect();
        if parts.len() != 5 { return Err("目录身份结构无效".into()); }
        parts[3]
    } else if value.starts_with("win-volume-v1|") || value.starts_with("win-entry-v1|") {
        value.split('|').nth(1).ok_or("卷身份结构无效")?
    } else { value };
    let value = value.replace('/', "\\").to_ascii_lowercase();
    let rest = value.strip_prefix("\\\\?\\volume{").ok_or("必须提供卷 GUID 标识，不能用盘符或磁盘号替代")?;
    let end = rest.find('}').ok_or("卷 GUID 标识不完整")?;
    let guid = &rest[..end];
    if guid.len() != 36 || !guid.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
    }) { return Err("卷 GUID 格式无效".into()); }
    let tail = &rest[end + 1..];
    if !tail.is_empty() && !tail.starts_with('\\') { return Err("卷 GUID 后存在歧义字符".into()); }
    if tail.contains('|') || tail.contains(':') || tail.split('\\').any(|part| part == "." || part == "..") {
        return Err("卷 GUID 路径含非法组件".into());
    }
    Ok(format!("\\\\?\\volume{{{guid}}}\\"))
}

pub fn list() -> Result<Vec<DriveInfo>, String> {
    #[cfg(windows)]
    { enrich(crate::fs_jobs::raw_drives()?) }
    #[cfg(not(windows))]
    { Err(UNSUPPORTED.into()) }
}

pub fn enrich(drives: Vec<DriveInfo>) -> Result<Vec<DriveInfo>, String> {
    #[cfg(windows)]
    { native::enrich(drives) }
    #[cfg(not(windows))]
    { let _ = drives; Err(UNSUPPORTED.into()) }
}

/// blocked_volumes 必须含 data、capture 和 copy 的全部活动卷，允许 core GUID
/// 与本模块卷 ID 混用。无法解析的阻塞项直接拒绝，不悄悄过滤。
pub fn eject(device_id: String, blocked_volumes: Vec<String>) -> Result<(), String> {
    #[cfg(windows)]
    { native::eject(device_id, blocked_volumes) }
    #[cfg(not(windows))]
    { let _ = (device_id, blocked_volumes); Err(UNSUPPORTED.into()) }
}

#[cfg(windows)]
pub(crate) use native::{Activity, begin_activity, related_volumes, require_recycle_volume, system_volume};

#[cfg(windows)]
mod native {
    use super::*;
    use crate::path_guard::{DirectoryGuard, io_error, wide};
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        fs::{File, OpenOptions}, io, mem::{offset_of, size_of},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle}, ptr,
        sync::{Mutex, OnceLock},
    };
    use windows_sys::Win32::{
        Devices::DeviceAndDriverInstallation::{
            CM_DEVCAP_EJECTSUPPORTED, CM_DEVCAP_REMOVABLE, CM_DEVCAP_UNIQUEID,
            CM_DRP_CAPABILITIES, CM_Get_DevNode_Registry_PropertyW, CM_Get_DevNode_Status,
            CM_Get_Device_IDW, CM_Get_Parent, CM_Request_Device_EjectW, CR_SUCCESS,
            DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DN_HAS_PROBLEM, DN_REMOVABLE, DN_STARTED,
            HDEVINFO, PNP_VetoOutstandingOpen, PNP_VetoPendingClose, PNP_VetoWindowsApp,
            PNP_VetoWindowsService, PNP_VetoInsufficientRights, SP_DEVICE_INTERFACE_DATA,
            SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList,
            SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
        },
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_NO_MORE_FILES, ERROR_NO_MORE_ITEMS, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BusTypeUsb, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDriveTypeW,
            GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{
                DISK_EXTENT, DRIVE_LAYOUT_INFORMATION_EX, GUID_DEVINTERFACE_DISK,
                IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_STORAGE_GET_DEVICE_NUMBER,
                IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX, PARTITION_INFORMATION_GPT,
                PARTITION_INFORMATION_MBR, PARTITION_STYLE_GPT, PARTITION_STYLE_MBR,
                PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_DEVICE_ID_DESCRIPTOR,
                STORAGE_DEVICE_NUMBER, STORAGE_IDENTIFIER, STORAGE_PROPERTY_QUERY,
                StorageDeviceIdProperty, StorageDeviceProperty, StorageIdAssocDevice,
                StorageIdCodeSetAscii, StorageIdCodeSetBinary, StorageIdCodeSetUtf8,
                StorageIdTypeEUI64, StorageIdTypeFCPHName, StorageIdTypeScsiNameString,
                VOLUME_DISK_EXTENTS,
            },
            SystemInformation::GetSystemWindowsDirectoryW,
            WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
        },
    };

    const MAX_VOLUMES: usize = 512;
    const MAX_DISKS: usize = 256;
    const MAX_EXTENTS: usize = 256;
    const MAX_IOCTL_BYTES: usize = 1024 * 1024;
    const MAX_PNP_DEPTH: usize = 16;

    #[derive(Default)]
    struct Activities { next: u64, volumes: BTreeMap<u64, Vec<String>> }
    static ACTIVITIES: OnceLock<Mutex<Activities>> = OnceLock::new();
    fn activities() -> &'static Mutex<Activities> { ACTIVITIES.get_or_init(|| Mutex::new(Activities::default())) }

    /// 新任务的注册与 eject 使用同一把锁；eject 调用期间不能增加新活动。
    /// 自有句柄先获取且验证身份，再注册，实际写入只在注册后开始。
    pub(crate) struct Activity { id: u64 }
    pub(crate) fn begin_activity(volumes: &[String]) -> Result<Activity, String> {
        let volumes = volumes.iter().map(|value| normalize_volume_identifier(value)).collect::<Result<Vec<_>, _>>()?;
        let mut state = activities().lock().map_err(|_| "设备活动锁不可用，拒绝开始文件操作")?;
        if state.volumes.len() >= 64 { return Err("并发文件操作过多，请稍后重试".into()); }
        state.next = state.next.checked_add(1).ok_or("设备活动计数已耗尽")?;
        let id = state.next;
        state.volumes.insert(id, volumes);
        Ok(Activity { id })
    }
    impl Drop for Activity {
        fn drop(&mut self) {
            activities().lock().unwrap_or_else(|poison| poison.into_inner()).volumes.remove(&self.id);
        }
    }

    fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
        let end = at.checked_add(4).ok_or("设备描述长度溢出")?;
        Ok(u32::from_le_bytes(bytes.get(at..end).ok_or("设备描述被截断")?.try_into().map_err(|_| "设备描述长度错误")?))
    }
    fn read_i32(bytes: &[u8], at: usize) -> Result<i32, String> { Ok(read_u32(bytes, at)? as i32) }
    fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
        Ok(u16::from_le_bytes(bytes.get(at..at + 2).ok_or("设备描述被截断")?.try_into().map_err(|_| "设备描述长度错误")?))
    }
    fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }
    fn digest(parts: &[String]) -> String {
        let mut hash = Sha256::new();
        for part in parts { hash.update((part.len() as u64).to_le_bytes()); hash.update(part.as_bytes()); }
        format!("win-device-v1:{}", hex(&hash.finalize()))
    }

    fn open_device(path: &str) -> Result<File, String> {
        OpenOptions::new().access_mode(0).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(path).map_err(|error| io_error("只读查询磁盘设备失败", error))
    }
    fn ioctl(file: &File, code: u32, input: Option<&STORAGE_PROPERTY_QUERY>, capacity: usize) -> Result<Vec<u8>, io::Error> {
        if capacity == 0 || capacity > MAX_IOCTL_BYTES { return Err(io::Error::from(io::ErrorKind::InvalidInput)); }
        let mut buffer = vec![0u64; capacity.div_ceil(8)];
        let mut returned = 0u32;
        let (input_ptr, input_len) = input.map_or((ptr::null(), 0), |query| (ptr::from_ref(query).cast(), size_of::<STORAGE_PROPERTY_QUERY>() as u32));
        // SAFETY: 对齐的可写输出覆盖 capacity 字节；输入是完整初始化的查询，File
        // 在同步调用结束前存活；没有 OVERLAPPED 或设备执行/写入命令。
        let ok = unsafe { DeviceIoControl(file.as_raw_handle().cast(), code, input_ptr, input_len,
            buffer.as_mut_ptr().cast(), capacity as u32, &mut returned, ptr::null_mut()) };
        if ok == 0 { return Err(io::Error::last_os_error()); }
        if returned as usize > capacity { return Err(io::Error::from(io::ErrorKind::InvalidData)); }
        // SAFETY: u64 缓冲中的所有字节都已初始化；复制出的 Vec 不含裸指针。
        Ok(unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), returned as usize) }.to_vec())
    }
    fn property(file: &File, id: i32) -> Result<Vec<u8>, String> {
        let query = STORAGE_PROPERTY_QUERY { PropertyId: id, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
        let header = ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, Some(&query), 8).map_err(|error| io_error("读取存储描述头失败", error))?;
        let size = read_u32(&header, 4)? as usize;
        if !(8..=MAX_IOCTL_BYTES).contains(&size) { return Err("存储设备描述长度超出安全限制".into()); }
        let bytes = ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, Some(&query), size).map_err(|error| io_error("读取完整存储描述失败", error))?;
        if bytes.len() < size || read_u32(&bytes, 4)? as usize != size { return Err("存储描述在两次查询之间变化或被截断".into()); }
        Ok(bytes)
    }
    fn descriptor_string(bytes: &[u8], offset: usize) -> Result<String, String> {
        let at = read_u32(bytes, offset)? as usize;
        if at == 0 { return Ok(String::new()); }
        if at < offset_of!(STORAGE_DEVICE_DESCRIPTOR, RawDeviceProperties) || at >= bytes.len() { return Err("存储字符串偏移非法".into()); }
        let tail = &bytes[at..];
        let len = tail.iter().position(|byte| *byte == 0).ok_or("存储字符串缺少结束符")?;
        if len > 512 { return Err("存储序列/型号字段过长".into()); }
        let value = std::str::from_utf8(&tail[..len]).map_err(|_| "存储字符串编码不可验证")?.trim().to_ascii_uppercase();
        if value.chars().any(char::is_control) { return Err("存储序列包含控制字符".into()); }
        Ok(value)
    }
    fn reliable_serial(value: &str) -> bool {
        let compact: String = value.chars().filter(|character| character.is_ascii_alphanumeric()).collect();
        compact.len() >= 4 && compact.len() <= 128
            && !compact.bytes().all(|byte| byte == b'0') && !compact.bytes().all(|byte| byte == b'F')
            && compact.bytes().collect::<HashSet<_>>().len() > 1
            && !["123456789", "1234567890", "0123456789", "0123456789ABCDEF", "SERIALNUMBER", "NOSERIAL", "UNKNOWN", "DEFAULT", "TOBEFILLEDBYOEM", "NONE"].contains(&compact.as_str())
    }
    fn storage_ids(file: &File) -> Result<Vec<String>, String> {
        let bytes = property(file, StorageDeviceIdProperty)?;
        let count = read_u32(&bytes, offset_of!(STORAGE_DEVICE_ID_DESCRIPTOR, NumberOfIdentifiers))? as usize;
        if count > 128 { return Err("存储标识数量超过安全限制".into()); }
        let mut at = offset_of!(STORAGE_DEVICE_ID_DESCRIPTOR, Identifiers);
        let mut ids = Vec::new();
        for index in 0..count {
            let code = read_i32(&bytes, at + offset_of!(STORAGE_IDENTIFIER, CodeSet))?;
            let kind = read_i32(&bytes, at + offset_of!(STORAGE_IDENTIFIER, Type))?;
            let association = read_i32(&bytes, at + offset_of!(STORAGE_IDENTIFIER, Association))?;
            let length = read_u16(&bytes, at + offset_of!(STORAGE_IDENTIFIER, IdentifierSize))? as usize;
            let next = read_u16(&bytes, at + offset_of!(STORAGE_IDENTIFIER, NextOffset))? as usize;
            let start = at.checked_add(offset_of!(STORAGE_IDENTIFIER, Identifier)).ok_or("存储标识偏移溢出")?;
            let end = start.checked_add(length).ok_or("存储标识长度溢出")?;
            let value = bytes.get(start..end).ok_or("存储标识被截断")?;
            if association == StorageIdAssocDevice && (4..=512).contains(&length)
                && matches!(kind, StorageIdTypeEUI64 | StorageIdTypeFCPHName | StorageIdTypeScsiNameString)
                && !value.iter().all(|byte| *byte == 0) && !value.iter().all(|byte| *byte == 0xff)
            {
                let value = if code == StorageIdCodeSetBinary { Some(hex(value)) }
                    else if code == StorageIdCodeSetAscii || code == StorageIdCodeSetUtf8 {
                        std::str::from_utf8(value).ok().filter(|text| !text.chars().any(char::is_control)).map(|text| text.trim().to_ascii_uppercase())
                    } else { None };
                if let Some(value) = value.filter(|value| !value.is_empty()) { ids.push(format!("{code}:{kind}:{value}")); }
            }
            if index + 1 < count {
                if next < offset_of!(STORAGE_IDENTIFIER, Identifier) + length { return Err("存储标识链重叠或循环".into()); }
                at = at.checked_add(next).ok_or("存储标识链偏移溢出")?;
            }
        }
        ids.sort(); ids.dedup(); Ok(ids)
    }

    fn volume_names() -> Result<Vec<String>, String> {
        let mut buffer = [0u16; 128];
        // SAFETY: 固定输出缓冲存活；查找句柄由 RAII 精确关闭一次。
        let handle = unsafe { FindFirstVolumeW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if handle == INVALID_HANDLE_VALUE { return Err(io_error("枚举本地卷失败", io::Error::last_os_error())); }
        struct Search(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for Search { fn drop(&mut self) { unsafe { FindVolumeClose(self.0); } } }
        let _search = Search(handle);
        let mut names = Vec::new();
        loop {
            let len = buffer.iter().position(|unit| *unit == 0).ok_or("卷名输出缺少结束符")?;
            let name = String::from_utf16(&buffer[..len]).map_err(|_| "卷 GUID 输出编码无效")?;
            let name = normalize_volume_identifier(&name)?;
            let kind = unsafe { GetDriveTypeW(wide(&name).as_ptr()) };
            if kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE { names.push(name); }
            if names.len() >= MAX_VOLUMES { return Err("本地卷数量超过 512，拒绝不完整设备拓扑".into()); }
            buffer.fill(0);
            if unsafe { FindNextVolumeW(handle, buffer.as_mut_ptr(), buffer.len() as u32) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) { break; }
                return Err(io_error("继续枚举本地卷失败", error));
            }
        }
        names.sort(); names.dedup(); Ok(names)
    }
    fn extents(volume: &str) -> Result<Vec<u32>, String> {
        let volume = normalize_volume_identifier(volume)?;
        let file = open_device(volume.trim_end_matches('\\'))?;
        let mut capacity = size_of::<VOLUME_DISK_EXTENTS>();
        loop {
            match ioctl(&file, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None, capacity) {
                Ok(bytes) => {
                    let count = read_u32(&bytes, offset_of!(VOLUME_DISK_EXTENTS, NumberOfDiskExtents))? as usize;
                    if count == 0 || count > MAX_EXTENTS { return Err("卷返回零个或过多磁盘区段，不能判定物理关系".into()); }
                    let start = offset_of!(VOLUME_DISK_EXTENTS, Extents);
                    let required = start.checked_add(count.checked_mul(size_of::<DISK_EXTENT>()).ok_or("卷区段长度溢出")?).ok_or("卷区段长度溢出")?;
                    if bytes.len() < required { return Err("卷磁盘区段描述被截断".into()); }
                    let mut numbers = Vec::new();
                    for index in 0..count {
                        let at = start + index * size_of::<DISK_EXTENT>();
                        numbers.push(read_u32(&bytes, at + offset_of!(DISK_EXTENT, DiskNumber))?);
                    }
                    numbers.sort_unstable(); numbers.dedup(); return Ok(numbers);
                }
                Err(error) if matches!(error.raw_os_error(), Some(code) if code == ERROR_MORE_DATA as i32 || code == ERROR_INSUFFICIENT_BUFFER as i32) => {
                    let limit = offset_of!(VOLUME_DISK_EXTENTS, Extents) + MAX_EXTENTS * size_of::<DISK_EXTENT>();
                    if capacity == limit { return Err("卷磁盘区段超过 256，不能安全关联".into()); }
                    capacity = capacity.checked_mul(2).ok_or("卷区段缓冲长度溢出")?.min(limit);
                }
                Err(error) => return Err(io_error("读取卷的物理磁盘区段失败", error)),
            }
        }
    }

    pub(crate) fn system_volume() -> Result<String, String> {
        let mut windows = vec![0u16; 4097];
        let len = unsafe { GetSystemWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) } as usize;
        if len == 0 || len >= windows.len() { return Err(io_error("无法确定 Windows 系统卷，拒绝危险操作", io::Error::last_os_error())); }
        let mut mount = vec![0u16; 4097];
        if unsafe { GetVolumePathNameW(windows.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
            return Err(io_error("无法确定系统目录所在卷", io::Error::last_os_error()));
        }
        let mut root = [0u16; 128];
        if unsafe { GetVolumeNameForVolumeMountPointW(mount.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return Err(io_error("无法确定系统卷 GUID", io::Error::last_os_error()));
        }
        let len = root.iter().position(|unit| *unit == 0).ok_or("系统卷标识缺少结束符")?;
        normalize_volume_identifier(&String::from_utf16(&root[..len]).map_err(|_| "系统卷 GUID 编码无效")?)
    }

    #[derive(Clone)]
    struct Node { number: u32, instance: String, caps: u32, status: u32, problem: u32 }
    fn chain(devnode: u32) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new(); let mut current = devnode; let mut visited = HashSet::new();
        for _ in 0..MAX_PNP_DEPTH {
            if !visited.insert(current) { return Err("PnP 祖先关系出现循环".into()); }
            let mut text = [0u16; 1024];
            let code = unsafe { CM_Get_Device_IDW(current, text.as_mut_ptr(), text.len() as u32, 0) };
            if code != CR_SUCCESS { return Err(format!("读取 PnP 设备实例失败（配置管理器错误 {code}）")); }
            let len = text.iter().position(|unit| *unit == 0).ok_or("PnP 实例标识未终止")?;
            let instance = String::from_utf16(&text[..len]).map_err(|_| "PnP 实例标识编码无效")?.to_ascii_uppercase();
            let mut caps = 0u32; let mut length = size_of::<u32>() as u32; let mut kind = 0u32;
            let code = unsafe { CM_Get_DevNode_Registry_PropertyW(current, CM_DRP_CAPABILITIES, &mut kind,
                ptr::from_mut(&mut caps).cast(), &mut length, 0) };
            if code != CR_SUCCESS || length != 4 { caps = 0; }
            let mut status = 0; let mut problem = 0;
            let code = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, current, 0) };
            if code != CR_SUCCESS { return Err("无法验证 PnP 设备状态，禁用安全弹出".into()); }
            nodes.push(Node { number: current, instance, caps, status, problem });
            let mut parent = 0;
            if unsafe { CM_Get_Parent(&mut parent, current, 0) } != CR_SUCCESS { return Ok(nodes); }
            current = parent;
        }
        Err("PnP 祖先超过 16 层，拒绝推测设备关系".into())
    }
    fn stable_pnp(nodes: &[Node], serial: &str) -> Option<String> {
        let serial: String = serial.chars().filter(|character| character.is_ascii_alphanumeric()).collect();
        nodes.iter().find_map(|node| {
            if node.caps & CM_DEVCAP_UNIQUEID == 0 { return None; }
            let last = node.instance.rsplit('\\').next()?;
            // Windows 生成的 6&端口地址不能充当永久标识；USBSTOR 的 序列&LUN 可保留。
            let last = last.strip_suffix("&0").unwrap_or(last);
            if last.contains('&') { return None; }
            let compact: String = last.chars().filter(|character| character.is_ascii_alphanumeric()).collect();
            if !compact.contains(&serial) { return None; }
            Some(node.instance.clone())
        })
    }
    fn eject_node(nodes: &[Node]) -> Option<u32> {
        let node = nodes.iter().find(|node| node.instance.starts_with("USB\\VID_") && !node.instance.contains("&MI_"))?;
        let removable = node.caps & (CM_DEVCAP_REMOVABLE | CM_DEVCAP_EJECTSUPPORTED) != 0 || node.status & DN_REMOVABLE != 0;
        (removable && node.status & DN_STARTED != 0 && node.status & DN_HAS_PROBLEM == 0 && node.problem == 0).then_some(node.number)
    }
    fn system_partition(file: &File) -> Result<bool, String> {
        let bytes = ioctl(file, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None, 128 * 1024).map_err(|error| io_error("不能确认磁盘引导分区，禁用弹出", error))?;
        let count = read_u32(&bytes, offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionCount))? as usize;
        if count > 256 { return Err("磁盘分区数量超过安全限制".into()); }
        let base = offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
        if bytes.len() < base + count * size_of::<PARTITION_INFORMATION_EX>() { return Err("磁盘分区布局被截断".into()); }
        // EFI 系统分区 GUID，按 Win32 GUID 内存布局编码。USB 上的引导分区也保守禁用。
        let efi = [0x28,0x73,0x2a,0xc1,0x1f,0xf8,0xd2,0x11,0xba,0x4b,0x00,0xa0,0xc9,0x3e,0xc9,0x3b];
        for index in 0..count {
            let at = base + index * size_of::<PARTITION_INFORMATION_EX>();
            let style = read_i32(&bytes, at + offset_of!(PARTITION_INFORMATION_EX, PartitionStyle))?;
            if bytes[at + offset_of!(PARTITION_INFORMATION_EX, IsServicePartition)] != 0 { return Ok(true); }
            let union = at + offset_of!(PARTITION_INFORMATION_EX, Anonymous);
            if style == PARTITION_STYLE_MBR && bytes[union + offset_of!(PARTITION_INFORMATION_MBR, BootIndicator)] != 0 { return Ok(true); }
            if style == PARTITION_STYLE_GPT {
                let start = union + offset_of!(PARTITION_INFORMATION_GPT, PartitionType);
                if bytes.get(start..start + 16) == Some(efi.as_slice()) { return Ok(true); }
            }
        }
        Ok(false)
    }

    struct Disk {
        number: u32, id: Option<String>, serial: Option<String>, usb: bool, bus_known: bool, node: u32, ancestors: Vec<u32>,
        pnp_signature: Vec<(u32, String)>, eject: Option<u32>, layout_safe: bool,
    }
    struct DeviceSet(HDEVINFO);
    impl Drop for DeviceSet { fn drop(&mut self) { unsafe { SetupDiDestroyDeviceInfoList(self.0); } } }
    fn disks() -> Result<(HashMap<u32, Disk>, bool), String> {
        let handle = unsafe { SetupDiGetClassDevsW(&GUID_DEVINTERFACE_DISK, ptr::null(), ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) };
        if handle == -1 { return Err(io_error("枚举磁盘设备接口失败", io::Error::last_os_error())); }
        let _set = DeviceSet(handle); let mut disks = HashMap::new(); let mut complete = true;
        for index in 0..=MAX_DISKS {
            let mut interface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if unsafe { SetupDiEnumDeviceInterfaces(handle, ptr::null(), &GUID_DEVINTERFACE_DISK, index as u32, &mut interface) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_ITEMS as i32) { break; }
                return Err(io_error("继续枚举磁盘接口失败", error));
            }
            if index == MAX_DISKS { return Err("磁盘接口超过 256，拒绝不完整设备清单".into()); }
            let mut required = 0u32;
            unsafe { SetupDiGetDeviceInterfaceDetailW(handle, &interface, ptr::null_mut(), 0, &mut required, ptr::null_mut()); }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) || !(8..=8192).contains(&required) {
                complete = false; continue;
            }
            let mut buffer = vec![0u64; (required as usize).div_ceil(8)];
            let detail = buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            let mut dev = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            // SAFETY: SetupAPI 的 cbSize 在 x64 为 8、x86 Unicode 为 6；变长路径缓冲
            // 由 required 决定且对齐。没有把设备路径暴露给前端。
            unsafe { (*detail).cbSize = if cfg!(target_pointer_width = "64") { 8 } else { 6 }; }
            if unsafe { SetupDiGetDeviceInterfaceDetailW(handle, &interface, detail, required, &mut required, &mut dev) } == 0 { complete = false; continue; }
            let start = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
            let max_units = (buffer.len() * 8 - start) / 2;
            let units = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(start).cast::<u16>(), max_units) };
            let Some(len) = units.iter().position(|unit| *unit == 0) else { complete = false; continue; };
            let Ok(path) = String::from_utf16(&units[..len]) else { complete = false; continue; };
            let Ok(file) = open_device(&path) else { complete = false; continue; };
            let Ok(number_bytes) = ioctl(&file, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, size_of::<STORAGE_DEVICE_NUMBER>()) else { complete = false; continue; };
            let number = read_u32(&number_bytes, offset_of!(STORAGE_DEVICE_NUMBER, DeviceNumber))?;
            let nodes = chain(dev.DevInst).ok();
            let descriptor = property(&file, StorageDeviceProperty).ok();
            let mut id = None; let mut usb = false; let mut serial_key = None;
            let bus_known = descriptor.as_ref().is_some_and(|bytes| bytes.len() >= offset_of!(STORAGE_DEVICE_DESCRIPTOR, RawDeviceProperties));
            if let Some(bytes) = descriptor.as_ref().filter(|bytes| bytes.len() >= offset_of!(STORAGE_DEVICE_DESCRIPTOR, RawDeviceProperties)) {
                usb = read_i32(bytes, offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType))? == BusTypeUsb;
                let vendor = descriptor_string(bytes, offset_of!(STORAGE_DEVICE_DESCRIPTOR, VendorIdOffset)).ok();
                let product = descriptor_string(bytes, offset_of!(STORAGE_DEVICE_DESCRIPTOR, ProductIdOffset)).ok();
                let serial = descriptor_string(bytes, offset_of!(STORAGE_DEVICE_DESCRIPTOR, SerialNumberOffset)).ok();
                serial_key = serial.as_ref().filter(|serial| reliable_serial(serial)).map(|serial| serial.chars().filter(|character| character.is_ascii_alphanumeric()).collect::<String>());
                if let (Some(vendor), Some(product), Some(serial), Some(nodes)) = (vendor, product, serial, nodes.as_ref()) {
                    if reliable_serial(&serial) && (!vendor.is_empty() || !product.is_empty()) {
                        serial_key = Some(serial.chars().filter(|character| character.is_ascii_alphanumeric()).collect::<String>());
                        let ids = storage_ids(&file).unwrap_or_default();
                        let pnp = stable_pnp(nodes, &serial);
                        if !ids.is_empty() || pnp.is_some() {
                            // 设备级标识优先；PNP 只用带 UNIQUEID 且与硬件序列匹配的实例。
                            let discriminator = if ids.is_empty() { pnp.unwrap_or_default() } else { ids.join(";") };
                            id = Some(digest(&[vendor, product, serial, discriminator]));
                        }
                    }
                }
            }
            if let Some(nodes) = &nodes { usb |= nodes.iter().any(|node| node.instance.starts_with("USB\\VID_")); }
            let eject = if usb { nodes.as_ref().and_then(|nodes| eject_node(nodes)) } else { None };
            let layout_safe = matches!(system_partition(&file), Ok(false));
            if descriptor.is_none() || nodes.is_none() { complete = false; }
            let pnp_signature = nodes.as_ref().map_or_else(Vec::new, |nodes| nodes.iter().map(|node| (node.number, node.instance.clone())).collect());
            let ancestors = nodes.map_or_else(Vec::new, |nodes| nodes.iter().map(|node| node.number).collect());
            if disks.insert(number, Disk { number, id, serial: serial_key, usb, bus_known, node: dev.DevInst, ancestors, pnp_signature, eject, layout_safe }).is_some() { complete = false; }
        }
        // 不同物理磁盘返回相同身份时，双方一律降级，不能把克隆/桥接器重复号当唯一。
        let mut counts = HashMap::new(); let mut serial_counts = HashMap::new();
        for disk in disks.values() {
            if let Some(id) = &disk.id { *counts.entry(id.clone()).or_insert(0usize) += 1; }
            if let Some(serial) = &disk.serial { *serial_counts.entry(serial.clone()).or_insert(0usize) += 1; }
        }
        for disk in disks.values_mut() {
            if disk.id.as_ref().is_some_and(|id| counts.get(id).copied().unwrap_or(0) != 1)
                || disk.serial.as_ref().is_some_and(|serial| serial_counts.get(serial).copied().unwrap_or(0) != 1) { disk.id = None; disk.eject = None; }
        }
        Ok((disks, complete))
    }

    struct Inventory { volumes: BTreeMap<String, Vec<u32>>, disks: HashMap<u32, Disk>, system: BTreeSet<u32>, complete: bool }
    fn inventory() -> Result<Inventory, String> {
        let (disks, mut complete) = disks()?; let mut volumes = BTreeMap::new();
        for volume in volume_names()? {
            match extents(&volume) {
                Ok(numbers) => { if numbers.iter().any(|number| !disks.contains_key(number)) { complete = false; } volumes.insert(volume, numbers); }
                Err(_) => { complete = false; volumes.insert(volume, Vec::new()); }
            }
        }
        let system_volume = system_volume()?;
        let mut system = BTreeSet::new();
        match volumes.get(&system_volume) {
            Some(numbers) if !numbers.is_empty() => system.extend(numbers),
            _ => complete = false,
        }
        // 程序所在设备也不可弹出；这不是安装/注册操作。
        if let Ok(executable) = std::env::current_exe() {
            if let Some(parent) = executable.parent().and_then(|path| path.to_str()) {
                match DirectoryGuard::open(parent).and_then(|guard| normalize_volume_identifier(&guard.final_path)) {
                    Ok(volume) => match volumes.get(&volume) { Some(numbers) => system.extend(numbers), None => complete = false },
                    Err(_) => complete = false,
                }
            } else { complete = false; }
        } else { complete = false; }
        for disk in disks.values() { if !disk.layout_safe { system.insert(disk.number); } }
        Ok(Inventory { volumes, disks, system, complete })
    }
    fn scope(inventory: &Inventory, disk: &Disk) -> Result<BTreeSet<u32>, String> {
        let node = disk.eject.ok_or("设备没有正常 PnP 可弹出能力，不按 DRIVE_FIXED/盘符猜测")?;
        let mut numbers = BTreeSet::new();
        for candidate in inventory.disks.values() {
            if candidate.node == node || candidate.ancestors.contains(&node) { numbers.insert(candidate.number); }
        }
        if numbers.is_empty() || !numbers.contains(&disk.number) { return Err("物理弹出节点与磁盘关系不完整".into()); }
        if numbers.iter().any(|number| inventory.system.contains(number)) { return Err("该物理设备包含系统、引导分区或应用程序，禁止弹出".into()); }
        Ok(numbers)
    }
    fn volume_scope(inventory: &Inventory, numbers: &BTreeSet<u32>) -> BTreeSet<String> {
        inventory.volumes.iter().filter(|(_, disks)| disks.iter().any(|number| numbers.contains(number))).map(|(volume, _)| volume.clone()).collect()
    }

    pub(super) fn enrich(mut drives: Vec<DriveInfo>) -> Result<Vec<DriveInfo>, String> {
        let inventory = inventory()?;
        for drive in &mut drives {
            let guard = DirectoryGuard::open(&drive.root)?;
            if guard.identity != drive.identity { return Err("驱动器在物理身份查询期间变化，请刷新设备列表".into()); }
            let volume = normalize_volume_identifier(&guard.final_path)?;
            drive.volume_id = guard.volume_id()?;
            drive.device_id = None; drive.identity_reliable = false; drive.is_usb = false; drive.can_eject = false;
            if let Some(numbers) = inventory.volumes.get(&volume) {
                drive.is_usb = numbers.iter().any(|number| inventory.disks.get(number).is_some_and(|disk| disk.usb));
                // 多盘区段卷（动态/跨区卷）没有一个可安全复用的单物理设备 ID。
                if numbers.len() == 1 {
                    if let Some(disk) = inventory.disks.get(&numbers[0]) {
                        drive.device_id = disk.id.clone(); drive.identity_reliable = disk.id.is_some();
                        drive.can_eject = inventory.complete && disk.id.is_some() && disk.usb && scope(&inventory, disk).is_ok();
                    }
                }
            }
            guard.validate()?;
        }
        Ok(drives)
    }

    /// 展开所有共享物理磁盘的卷，包含无盘符分区和多盘区段的传递关系。
    /// 选中卷无法解析时拒绝；其他未知卷令 eject 的完整性检查失败，不会强行弹出。
    pub(crate) fn related_volumes(values: &[String]) -> Result<Vec<String>, String> {
        let mut selected: BTreeSet<_> = values.iter().map(|value| normalize_volume_identifier(value)).collect::<Result<_, _>>()?;
        let mut numbers = BTreeSet::new();
        for volume in &selected { numbers.extend(extents(volume)?); }
        let mut all = BTreeMap::new();
        for volume in volume_names()? { if let Ok(disks) = extents(&volume) { all.insert(volume, disks); } }
        loop {
            let before = numbers.len();
            for (volume, disks) in &all {
                if disks.iter().any(|number| numbers.contains(number)) { selected.insert(volume.clone()); numbers.extend(disks); }
            }
            if before == numbers.len() { break; }
        }
        Ok(selected.into_iter().collect())
    }

    pub(crate) fn require_recycle_volume(volume: &str) -> Result<(), String> {
        let volume = normalize_volume_identifier(volume)?;
        if volume == system_volume()? { return Err("不能回收 Windows 系统卷中的浏览文件".into()); }
        let inventory = inventory()?;
        let numbers = inventory.volumes.get(&volume).filter(|numbers| !numbers.is_empty()).ok_or("无法验证回收来源的物理卷关系")?;
        for number in numbers {
            let disk = inventory.disks.get(number).ok_or("无法验证回收来源的磁盘类型")?;
            if !disk.bus_known { return Err("无法证明该磁盘不是 USB，不能使用系统回收操作".into()); }
            if disk.usb { return Err("第一版禁用 USB 设备文件回收；不会降级永久删除，请使用资源管理器明确处理".into()); }
        }
        // 还必须是当前挂载的固定盘；DRIVE_FIXED 不能单独证明不是 USB，前面已查 bus。
        if unsafe { GetDriveTypeW(wide(&volume).as_ptr()) } != DRIVE_FIXED { return Err("此介质不能证明支持系统回收站，禁止永久删除回退".into()); }
        Ok(())
    }

    pub(super) fn eject(device_id: String, blocked_volumes: Vec<String>) -> Result<(), String> {
        if !device_id.starts_with("win-device-v1:") || device_id.len() != "win-device-v1:".len() + 64
            || !device_id["win-device-v1:".len()..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("没有可靠物理设备标识，不能按盘符或磁盘编号安全弹出".into());
        }
        if blocked_volumes.len() > 4096 { return Err("活动卷数量超过安全限制，拒绝弹出".into()); }
        let state = activities().lock().map_err(|_| "设备活动锁不可用，拒绝弹出")?;
        let mut blocked: BTreeSet<_> = blocked_volumes.iter().map(|volume| normalize_volume_identifier(volume)).collect::<Result<_, _>>()?;
        for volumes in state.volumes.values() { for volume in volumes { blocked.insert(normalize_volume_identifier(volume)?); } }
        let inventory = inventory()?;
        if !inventory.complete { return Err("设备或卷拓扑不完整，不能证明全部分区均可安全释放；请刷新或检查设备权限".into()); }
        let mut matching = inventory.disks.values().filter(|disk| disk.id.as_deref() == Some(device_id.as_str()));
        let disk = matching.next().ok_or("该物理设备已拔出、身份不可靠或已变化，请刷新设备列表")?;
        if matching.next().is_some() { return Err("多块物理磁盘身份重复，禁止猜测要弹出的设备".into()); }
        if !disk.usb { return Err("该设备不是可安全弹出的 USB 存储设备".into()); }
        let numbers = scope(&inventory, disk)?;
        let volumes = volume_scope(&inventory, &numbers);
        if volumes.is_empty() { return Err("物理设备没有可核实的本地卷，拒绝弹出".into()); }
        // 阻塞卷若不在清单中（拔出/换代），也重新读取区段；无法验证则拒绝。
        for volume in &blocked {
            let blocked_disks = inventory.volumes.get(volume).cloned().map(Ok).unwrap_or_else(|| extents(volume))?;
            if volumes.contains(volume) || blocked_disks.iter().any(|number| numbers.contains(number)) {
                return Err("该物理设备的分区正被持久目录、收纳来源、复制或文件操作占用，请等待或取消并释放后再弹出".into());
            }
        }
        // 提交前重新关联每个卷，不使用过期磁盘号；所有 IOCTL 临时 File 已释放。
        for volume in &volumes {
            if extents(volume)? != *inventory.volumes.get(volume).ok_or("弹出卷关系缺失")? {
                return Err("卷与物理磁盘的关联已变化，取消本次安全弹出".into());
            }
        }
        let node = disk.eject.ok_or("设备没有正常安全弹出节点")?;
        let current = chain(disk.node)?;
        let signature: Vec<_> = current.iter().map(|node| (node.number, node.instance.clone())).collect();
        if signature != disk.pnp_signature || eject_node(&current) != Some(node) { return Err("物理 PnP 实例或节点已变化，取消本次弹出".into()); }
        let mut veto = 0i32; let mut name = [0u16; 1024];
        // SAFETY: node 是刚从当前磁盘接口重新验证的物理 USB devnode；输出有效。
        // 只调用正常 CM 请求：没有卸载卷、强杀进程、设备禁用、权限升级或安装操作。
        let code = unsafe { CM_Request_Device_EjectW(node, &mut veto, name.as_mut_ptr(), name.len() as u32, 0) };
        if code != CR_SUCCESS {
            let reason = match veto {
                PNP_VetoOutstandingOpen | PNP_VetoPendingClose => "仍有文件句柄或待关闭操作",
                PNP_VetoWindowsApp => "其他应用程序正在使用设备",
                PNP_VetoWindowsService => "Windows 服务正在使用设备",
                PNP_VetoInsufficientRights => "系统未授予弹出权限",
                _ => "系统或设备驱动拒绝安全弹出",
            };
            let len = name.iter().position(|unit| *unit == 0).unwrap_or(name.len());
            let detail = String::from_utf16_lossy(&name[..len]);
            let detail: String = detail.chars().filter(|character| !character.is_control()).take(160).collect();
            return Err(format!("安全弹出失败：{reason}（配置管理器错误 {code}，否决类型 {veto}{}）", if detail.is_empty() { String::new() } else { format!("，{detail}") }));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::reliable_serial;
        #[test]
        fn refuses_missing_or_placeholder_serials() {
            for value in ["", "0", "00000000", "FFFFFFFF", "1234567890", "NO SERIAL", "SERIAL NUMBER"] { assert!(!reliable_serial(value)); }
            assert!(reliable_serial("AB27C4D901"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_volume_identifier;
    #[test]
    fn normalizes_guid_paths_and_volume_generation_ids() {
        let guid = "ABCD1234-5678-9ABC-DEF0-1234567890AB";
        let expected = "\\\\?\\volume{abcd1234-5678-9abc-def0-1234567890ab}\\";
        assert_eq!(normalize_volume_identifier(&format!("\\\\?\\Volume{{{guid}}}" )).unwrap(), expected);
        assert_eq!(normalize_volume_identifier(&format!("win-volume-v1|\\\\?\\Volume{{{guid}}}\\|1234|ntfs|0:0")).unwrap(), expected);
        assert_eq!(normalize_volume_identifier(&format!("win-dir-v1|root|leaf|\\\\?\\Volume{{{guid}}}\\|\\\\?\\Volume{{{guid}}}\\课程")).unwrap(), expected);
    }
    #[test]
    fn refuses_letters_numbers_and_ambiguous_identifiers() {
        for value in ["E:\\", "PhysicalDrive2", "2", "\\\\server\\share", "junk\\\\?\\Volume{abcd1234-5678-9abc-def0-1234567890ab}", "\\\\?\\Volume{bad}\\", "\\\\?\\Volume{abcd1234-5678-9abc-def0-1234567890ab}\\..\\"] { assert!(normalize_volume_identifier(value).is_err()); }
    }
}
