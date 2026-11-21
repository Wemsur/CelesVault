use crate::api::{AppSettings, CaptureSettings, ConfigBundle, FixedFolder, CONFIG_VERSION};
use std::collections::HashSet;

pub const MIN_RATIO: f64 = 0.15;
pub const MAX_RATIO: f64 = 0.85;
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_FAVORITES: usize = 512;
pub const MAX_FAVORITE_POSITION: u32 = 4095;

pub(crate) fn text(value: &str, label: &str, max: usize, empty: bool) -> Result<(), String> {
    if (!empty && value.trim().is_empty()) || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("{label}为空、过长或含控制字符（最多 {max} 字）"));
    }
    Ok(())
}

pub(crate) fn identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 80 || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("{label}必须为 1–80 个英文字母、数字、短横线或下划线"));
    }
    Ok(())
}

fn reserved(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|prefix| stem.strip_prefix(prefix).is_some_and(|n|
            matches!(n, "1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"¹"|"²"|"³")))
}

pub(crate) fn path_component(value: &str) -> Result<(), String> {
    if value.is_empty() || value == "." || value == ".." || value.ends_with(['.', ' '])
        || value.encode_utf16().count() > 255 || reserved(value)
        || value.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)) {
        return Err("路径含歧义、保留名称或非法组件".into());
    }
    Ok(())
}

/// 纯词法校验，不要求设置中的源盘当前在线；不接受 UNC/ADS/设备路径。
pub(crate) fn windows_path(value: &str) -> Result<(), String> {
    let value = value.replace('/', "\\");
    let bytes = value.as_bytes();
    if bytes.len() < 3 || bytes.len() > 30_000 || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':' || bytes[2] != b'\\' || value.contains('\0') {
        return Err("监测路径必须是绝对本地盘符路径，不支持 UNC、设备路径或相对路径".into());
    }
    for part in value[3..].split('\\').filter(|p| !p.is_empty()) { path_component(part)?; }
    Ok(())
}

/// root/busy 使用的裸卷 GUID 规范化；不能把此结果用来匹配固定夹代际身份。
pub(crate) fn volume_id(value: &str) -> Result<String, String> {
    let value = value.trim_end_matches(['\\', '/']);
    let lower = value.to_ascii_lowercase();
    let id = lower.strip_prefix("\\\\?\\volume{").and_then(|s| s.strip_suffix('}'))
        .ok_or("固定夹卷身份必须是完整的本地卷 GUID，不得保存盘符")?;
    if id.len() != 36 || !id.bytes().enumerate().all(|(i, b)| {
        if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }
    }) { return Err("固定夹卷 GUID 格式不正确".into()); }
    Ok(format!("\\\\?\\Volume{{{id}}}"))
}

/// 新固定夹完整保存设备模块的格式化代际串；旧裸 GUID 只作为未重新关联的历史记录保留。
fn favorite_volume_id(value: &str) -> Result<String, String> {
    if !value.contains('|') {
        // 不查询当前盘，不补 serial/FS/root 身份；裸 GUID 永远不会变成新 generation ID。
        return volume_id(value);
    }
    if value.len()>160 {return Err("固定夹卷代际身份过长".into());}
    let parts:Vec<_>=value.split('|').collect();
    if parts.len()!=5 || parts[0]!="win-volume-v1" {
        return Err("固定夹卷代际须为 win-volume-v1|GUID|serial8hex|filesystem|rootIndex16hex:rootCreated16hex".into());
    }
    // Windows 设备层返回带单个尾反斜线的卷根；历史配置也允许无尾斜线。
    // 只验证这两种表示，完整代际串原样保留，避免改变设备/恢复意图身份。
    let guid = parts[1].strip_suffix('\\').unwrap_or(parts[1]);
    if guid.ends_with(['\\','/']) || parts[1].ends_with('/') {
        return Err("卷代际 GUID 段仅允许无尾斜线或单个尾反斜线".into());
    }
    volume_id(guid)?;
    let hex=|part:&str,width:usize| part.len()==width && part.bytes().all(|b|b.is_ascii_hexdigit());
    if !hex(parts[2],8) {return Err("卷代际 serial 必须为 8 位十六进制".into());}
    let filesystem=parts[3];
    if filesystem.is_empty() || filesystem.len()>32
        || !filesystem.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-' || b==b'_') {
        return Err("卷代际 filesystem 须为 1–32 个 ASCII 字母、数字、短横线或下划线".into());
    }
    let (index,created)=parts[4].split_once(':').ok_or("卷代际缺少 rootIndex:rootCreated")?;
    if !hex(index,16) || !hex(created,16) {return Err("卷代际 rootIndex/rootCreated 必须各为 16 位十六进制".into());}
    Ok(value.to_owned())
}

fn forbidden_process(name: &str) -> bool {
    ["system", "system.exe", "explorer.exe", "dllhost.exe", "prevhost.exe", "svchost.exe", "searchindexer.exe"]
        .contains(&name.to_ascii_lowercase().as_str())
}

pub fn validate_capture(mut settings: CaptureSettings) -> Result<CaptureSettings, String> {
    if !["strict", "loose"].contains(&settings.mode.as_str()) { return Err("监测模式只能是 strict 或 loose".into()); }
    for count in [settings.process_names.len(), settings.process_paths.len(), settings.wps_install_paths.len(),
        settings.sources.len(), settings.extensions.len(), settings.excluded_paths.len()] {
        if count > 128 { return Err("每类监测规则最多 128 项".into()); }
    }
    for name in &mut settings.process_names {
        *name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.len() > 255 || name.contains(['\\', '/', ':'])
            || name.chars().any(char::is_control) || !name.ends_with(".exe") || forbidden_process(name) {
            return Err("主程序规则必须是明确的应用 exe 名；不能使用 Explorer/dllhost/System 等通用来源".into());
        }
        path_component(name)?;
    }
    for ext in &mut settings.extensions { *ext = extension(ext)?; }
    for path in settings.process_paths.iter().chain(settings.wps_install_paths.iter())
        .chain(settings.excluded_paths.iter()).chain(settings.sources.iter().map(|s| &s.path)) { windows_path(path)?; }
    for path in &settings.process_paths {
        let name = path.rsplit(['\\', '/']).next().unwrap_or("");
        if forbidden_process(name) || !name.to_ascii_lowercase().ends_with(".exe") {
            return Err("程序路径必须指向应用 exe，不能指向通用系统进程".into());
        }
    }
    for path in &settings.wps_install_paths {
        if path.replace('/', "\\").trim_end_matches('\\').len() <= 2 {
            return Err("WPS 安装目录不能是整个盘根".into());
        }
    }
    // 移除课表科目不会解除排除策略；仅验证稳定 ID 格式。
    if settings.excluded_subject_ids.len() > 4096 { return Err("排除科目最多 4096 项".into()); }
    for id in &settings.excluded_subject_ids { identifier(id, "排除科目 ID")?; }
    settings.excluded_subject_ids.sort(); settings.excluded_subject_ids.dedup();
    settings.process_names.sort(); settings.process_names.dedup();
    settings.extensions.sort(); settings.extensions.dedup();
    Ok(settings)
}

pub(crate) fn extension(value: &str) -> Result<String, String> {
    let ext = value.trim().trim_start_matches('.').to_ascii_lowercase();
    if ext.is_empty() || ext.len() > 16 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("扩展名必须是 1–16 个英文或数字".into());
    }
    Ok(ext)
}

fn validate_appearance(value: &crate::api::AppearanceSettings) -> Result<(), String> {
    use crate::api::BACKGROUND_IDS;
    if !["customdark","customlight","system"].contains(&value.theme.as_str()) || value.hue > 360 { return Err("主题或色相无效".into()); }
    if !["off","animated","image"].contains(&value.background.as_str()) || !BACKGROUND_IDS.contains(&value.shader.as_str()) { return Err("背景类型无效".into()); }
    if value.parameters.len()!=BACKGROUND_IDS.len() || BACKGROUND_IDS.iter().any(|id|!value.parameters.contains_key(*id)) { return Err("背景参数必须包含全部已知背景".into()); }
    for p in value.parameters.values() {
        for (name,n,lo,hi) in [("精细度",p.quality,0.25,1.0),("速度",p.speed,0.0,3.0),("强度",p.intensity,0.0,1.0),("尺度",p.scale,0.5,3.0),("细节",p.detail,0.5,3.0),("颗粒",p.grain,0.0,3.0)] {
            if !n.is_finite() || n<lo || n>hi { return Err(format!("背景{name}超出范围")); }
        }
        if ![15,30,60].contains(&p.fps) || !(20..=160).contains(&p.density) { return Err("背景帧率或线路密度无效".into()); }
    }
    if !["cover","contain"].contains(&value.image_fit.as_str()) || !value.image_opacity.is_finite() || !(0.0..=1.0).contains(&value.image_opacity) || !value.image_blur.is_finite() || !(0.0..=30.0).contains(&value.image_blur) { return Err("图片背景参数无效".into()); }
    if let Some(image)=&value.image {
        use base64::Engine;
        let text=image.strip_prefix("data:image/jpeg;base64,").ok_or("背景图片须为应用转换的JPEG显示副本")?;
        if text.len()>440_000 { return Err("背景显示副本过大".into()); }
        let bytes=base64::engine::general_purpose::STANDARD.decode(text).map_err(|_|"背景图片编码无效")?;
        if bytes.len()>320*1024 || !bytes.starts_with(&[0xff,0xd8,0xff]) || !bytes.ends_with(&[0xff,0xd9]) { return Err("背景图片不是有效的有界JPEG副本".into()); }
    }
    if value.background=="image" && value.image.is_none() { return Err("请先选择背景图片".into()); }
    Ok(())
}

pub fn validate_app(app: AppSettings) -> Result<AppSettings, String> {
    if !["horizontal", "vertical"].contains(&app.home_direction.as_str()) {
        return Err("主页方向只能是 horizontal 或 vertical".into());
    }
    if !["grid", "list"].contains(&app.browser_view.as_str()) { return Err("浏览视图只能是 grid 或 list".into()); }
    for (name, ratio) in [("左右比例", app.horizontal_ratio), ("上下比例", app.vertical_ratio),
        ("左右目标区比例", app.horizontal_target_ratio), ("上下目标区比例", app.vertical_target_ratio), ("库比例", app.library_ratio)] {
        if !ratio.is_finite() || !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            return Err(format!("{name}必须在 {MIN_RATIO}–{MAX_RATIO} 之间"));
        }
    }
    validate_appearance(&app.appearance)?;
    if app.notification_seconds > 120 { return Err("通知消失时间必须为 0–120 秒，0 表示手动关闭".into()); }
    if !(75..=200).contains(&app.scale_percent) { return Err("界面缩放必须在 75–200% 之间".into()); }
    if !(1100..=7680).contains(&app.window_width) || !(700..=4320).contains(&app.window_height) {
        return Err("窗口尺寸须在宽 1100–7680、高 700–4320 像素范围内".into());
    }
    Ok(app)
}

pub fn validate_favorites(mut favorites: Vec<FixedFolder>) -> Result<Vec<FixedFolder>, String> {
    if favorites.len() > MAX_FAVORITES { return Err(format!("固定夹最多 {MAX_FAVORITES} 个")); }
    let mut ids = HashSet::new(); let mut locations = HashSet::new();
    for folder in &mut favorites {
        identifier(&folder.id, "固定夹 ID")?;
        if !ids.insert(folder.id.clone()) { return Err("固定夹 ID 重复".into()); }
        text(&folder.device_id, "物理设备身份", 512, false)?;
        folder.volume_id = favorite_volume_id(&folder.volume_id)?;
        text(&folder.alias, "固定夹别名", 128, true)?;
        folder.alias = folder.alias.trim().to_owned();
        if folder.position > MAX_FAVORITE_POSITION { return Err(format!("固定夹顺序不得超过 {MAX_FAVORITE_POSITION}")); }
        if folder.relative_path.len() > 30_000 { return Err("固定夹相对路径过长".into()); }
        let path = folder.relative_path.replace('/', "\\");
        if path.starts_with('\\') || path.ends_with('\\') || path.contains("\\\\") { return Err("固定夹须保存无前后斜线的相对路径；根目录用空字符串".into()); }
        if !path.is_empty() { for part in path.split('\\') { path_component(part)?; } }
        folder.relative_path = path;
        if !locations.insert((folder.device_id.clone(), folder.volume_id.to_ascii_lowercase(), folder.relative_path.to_lowercase())) {
            return Err("同一物理设备/分区中的固定路径重复".into());
        }
    }
    favorites.sort_by(|a, b| a.device_id.cmp(&b.device_id).then(a.position.cmp(&b.position)).then(a.id.cmp(&b.id)));
    Ok(favorites)
}

pub fn validate_config(mut bundle: ConfigBundle) -> Result<ConfigBundle, String> {
    if bundle.version != CONFIG_VERSION { return Err(format!("不支持配置版本 {}，当前版本为 {CONFIG_VERSION}", bundle.version)); }
    bundle.app = validate_app(bundle.app)?;
    bundle.capture = validate_capture(bundle.capture)?;
    bundle.timetable = crate::timetable::validate_timetable(bundle.timetable)?;
    bundle.favorites = validate_favorites(bundle.favorites)?;
    if serde_json::to_vec(&bundle).map_err(|e| e.to_string())?.len() > MAX_CONFIG_BYTES { return Err("完整配置不得超过 1 MiB".into()); }
    Ok(bundle)
}

/// 只产生已验证草稿，不选择数据目录、不写数据库；应用须经一次 save_config。
pub fn parse_config(text: &str) -> Result<ConfigBundle, String> {
    if text.len() > MAX_CONFIG_BYTES { return Err("导入配置不得超过 1 MiB".into()); }
    let bundle = serde_json::from_str(text).map_err(|e| format!("配置 JSON 无效或含未知/缺失字段：{e}"))?;
    validate_config(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excluded_subjects_are_backward_compatible_and_strict() {
        let mut value=serde_json::to_value(ConfigBundle::default()).unwrap();
        value["capture"].as_object_mut().unwrap().remove("excludedSubjectIds");
        assert!(parse_config(&value.to_string()).unwrap().capture.excluded_subject_ids.is_empty());
        let old_capture=value["capture"].clone();
        assert!(serde_json::from_value::<CaptureSettings>(old_capture).unwrap().excluded_subject_ids.is_empty());
        for invalid in [serde_json::json!(null),serde_json::json!("math"),serde_json::json!([1]),serde_json::json!(["中文"])] {
            value["capture"]["excludedSubjectIds"]=invalid;
            assert!(parse_config(&value.to_string()).is_err());
        }
        value["capture"]["excludedSubjectIds"]=serde_json::json!(["removed-subject","a","removed-subject"]);
        assert_eq!(parse_config(&value.to_string()).unwrap().capture.excluded_subject_ids,vec!["a","removed-subject"]);
        value["capture"]["excludedSubjectId"]=serde_json::json!([]);
        assert!(parse_config(&value.to_string()).is_err());
    }
    #[test]
    fn defaults_are_empty_and_valid() {
        let bundle = validate_config(ConfigBundle::default()).unwrap();
        assert!(bundle.timetable.lessons.is_empty());
        assert!(bundle.favorites.is_empty());
    }
    #[test]
    fn notification_time_defaults_for_old_config_and_is_bounded() {
        let mut value=serde_json::to_value(ConfigBundle::default()).unwrap();
        value["app"].as_object_mut().unwrap().remove("notificationSeconds");
        assert_eq!(parse_config(&value.to_string()).unwrap().app.notification_seconds,6);
        for seconds in [0,1,6,120] {
            let mut app=AppSettings::default(); app.notification_seconds=seconds;
            assert!(validate_app(app).is_ok());
        }
        let mut app=AppSettings::default(); app.notification_seconds=121;
        assert!(validate_app(app).is_err());
        value["app"]["notificationSeconds"]=serde_json::json!("6");
        assert!(parse_config(&value.to_string()).is_err());
    }
    #[test]
    fn appearance_defaults_and_parameter_boundaries() {
        let mut value=serde_json::to_value(ConfigBundle::default()).unwrap();
        value["app"].as_object_mut().unwrap().remove("appearance");
        let app=parse_config(&value.to_string()).unwrap().app;
        assert_eq!(app.appearance.theme,"customdark");
        assert_eq!(app.appearance.hue,38);
        assert_eq!(app.appearance.background,"off");
        for theme in ["customdark","customlight","system"] {
            let mut app=AppSettings::default();app.appearance.theme=theme.into();assert!(validate_app(app).is_ok());
        }
        let mut app=AppSettings::default();app.appearance.hue=361;assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.parameters.get_mut("gate").unwrap().density=161;assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.parameters.get_mut("aurora").unwrap().quality=f64::NAN;assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.parameters.remove("ice");assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.background="image".into();assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.image=Some("https://example.invalid/image.jpg".into());assert!(validate_app(app).is_err());
        let mut app=AppSettings::default();app.appearance.image=Some("data:image/jpeg;base64,AAAA".into());assert!(validate_app(app).is_err());
    }
    #[test]
    fn bounds_are_not_silently_clamped() {
        let mut app = AppSettings::default(); app.horizontal_ratio = f64::NAN;
        assert!(validate_app(app).is_err());
        let mut app = AppSettings::default(); app.library_ratio = 0.149;
        assert!(validate_app(app).is_err());
        let mut app = AppSettings::default(); app.scale_percent = 201;
        assert!(validate_app(app).is_err());
        let mut app = AppSettings::default(); app.window_height = 699;
        assert!(validate_app(app).is_err());
        let mut app = AppSettings::default(); app.window_width = 1099;
        assert!(validate_app(app).is_err());
    }
    #[test]
    fn unknown_and_missing_config_are_explicit_errors() {
        assert!(parse_config("{\"version\":1,\"unknown\":true}").is_err());
        assert!(parse_config("{\"version\":99}").is_err());
        let mut value = serde_json::to_value(ConfigBundle::default()).unwrap();
        value["app"]["unknown"] = serde_json::json!(true);
        assert!(parse_config(&value.to_string()).is_err());
    }
    #[test]
    fn favorites_require_identity_safe_paths_and_bounded_order() {
        let folder = FixedFolder { id: "one".into(), device_id: "usb:serial".into(),
            volume_id: "\\\\?\\Volume{12345678-1234-1234-1234-123456789abc}".into(),
            relative_path: "教学资料\\数学".into(), alias: "数学".into(), position: 0 };
        assert!(validate_favorites(vec![folder.clone()]).is_ok());
        let mut bad = folder.clone(); bad.relative_path = "..\\别处".into();
        assert!(validate_favorites(vec![bad]).is_err());
        let mut bad = folder.clone(); bad.volume_id = "D:".into();
        assert!(validate_favorites(vec![bad]).is_err());
        let mut bad = folder.clone(); bad.position = 4096;
        assert!(validate_favorites(vec![bad]).is_err());
        assert!(validate_favorites(vec![folder.clone(), folder]).is_err());
    }
    const FAVORITE_GUID:&str=r"\\?\Volume{12345678-1234-1234-1234-123456789ABC}";
    fn generation_volume(serial:&str,filesystem:&str,index:&str,created:&str)->String {
        format!("win-volume-v1|{FAVORITE_GUID}|{serial}|{filesystem}|{index}:{created}")
    }
    fn generation_folder(volume:String)->FixedFolder {
        FixedFolder {id:"generation-one".into(),device_id:"usb:serial".into(),volume_id:volume,
            relative_path:"教学资料\\数学".into(),alias:"数学".into(),position:0}
    }
    #[test]
    fn favorite_generation_id_is_preserved_through_config_round_trip() {
        let full=generation_volume("DEADBEEF","exFAT","0123456789ABCDEF","01D9ABCD01234567");
        let favorites=validate_favorites(vec![generation_folder(full.clone())]).unwrap();
        assert_eq!(favorites[0].volume_id,full);
        let bundle=ConfigBundle {favorites,..ConfigBundle::default()};
        let imported=parse_config(&serde_json::to_string(&bundle).unwrap()).unwrap();
        assert_eq!(imported.version,1);
        assert_eq!(imported.favorites[0].volume_id,full);
        assert!(volume_id(&full).is_err()); // root/busy 的裸 GUID helper 不接受代际串。
        let legacy=favorite_volume_id(FAVORITE_GUID).unwrap();
        assert!(!legacy.contains('|'));
        assert_ne!(legacy,full); // 旧记录不会被补猜为当前 generation。
    }
    #[test]
    fn windows_volume_root_separator_is_preserved_in_favorites() {
        let full=generation_volume("DEADBEEF","exFAT","0123456789ABCDEF","01D9ABCD01234567")
            .replace(FAVORITE_GUID,&format!("{FAVORITE_GUID}\\"));
        let bundle=ConfigBundle {favorites:vec![generation_folder(full.clone())],..ConfigBundle::default()};
        let validated=validate_config(bundle).unwrap();
        assert_eq!(validated.favorites[0].volume_id,full);
        let imported=parse_config(&serde_json::to_string(&validated).unwrap()).unwrap();
        assert_eq!(imported.favorites[0].volume_id,full);
        assert_eq!(imported.favorites[0].device_id,"usb:serial");
        assert!(full.ends_with("|DEADBEEF|exFAT|0123456789ABCDEF:01D9ABCD01234567"));
    }
    #[test]
    fn favorite_generation_rejects_bad_parts_and_unknown_versions() {
        let index="0123456789abcdef";let created="01d9abcd01234567";
        let valid=generation_volume("12345678","NTFS",index,created);
        let bad=vec![
            valid.replacen("win-volume-v1|","win-volume-v2|",1),
            valid.replace(FAVORITE_GUID,"D:"),
            valid.replacen("12345678-","z2345678-",1),
            valid.replace(FAVORITE_GUID,&format!("{FAVORITE_GUID}\\\\")),
            valid.replace(FAVORITE_GUID,&format!("{FAVORITE_GUID}/")),
            valid.replace(FAVORITE_GUID,&format!("{FAVORITE_GUID}\\child")),
            generation_volume("1234567","NTFS",index,created),
            generation_volume("123456789","NTFS",index,created),
            generation_volume("1234567z","NTFS",index,created),
            generation_volume("12345678","",index,created),
            generation_volume("12345678",&"A".repeat(33),index,created),
            generation_volume("12345678","文件系统",index,created),
            generation_volume("12345678","NT FS",index,created),
            generation_volume("12345678","NTFS|other",index,created),
            generation_volume("12345678","NTFS","0123456789abcde",created),
            generation_volume("12345678","NTFS","0123456789abcdeg",created),
            generation_volume("12345678","NTFS",index,"01d9abcd0123456"),
            generation_volume("12345678","NTFS",index,"01d9abcd0123456g"),
            valid.replace(':',"::"),format!("{valid}|extra"),format!("{valid} "),
        ];
        for volume in bad {assert!(validate_favorites(vec![generation_folder(volume.clone())]).is_err(),"应拒绝 {volume}");}
    }
    #[test]
    fn formatted_generations_with_the_same_guid_do_not_collapse() {
        let index="0000000000000001";let created="01d9abcd01234567";
        let volumes=vec![
            generation_volume("00000001","NTFS",index,created),
            generation_volume("00000002","NTFS",index,created),
            generation_volume("00000001","exFAT",index,created),
            generation_volume("00000001","NTFS","0000000000000002",created),
            generation_volume("00000001","NTFS",index,"01d9abcd01234568"),
        ];
        let folders=volumes.iter().enumerate().map(|(i,volume)| {
            let mut folder=generation_folder(volume.clone());folder.id=format!("generation-{i}");folder.position=i as u32;folder
        }).collect();
        let favorites=validate_favorites(folders).unwrap();
        assert_eq!(favorites.len(),volumes.len());
        for (folder,volume) in favorites.iter().zip(&volumes) {assert_eq!(&folder.volume_id,volume);}
        let bundle=ConfigBundle {favorites,..ConfigBundle::default()};
        let imported=parse_config(&serde_json::to_string(&bundle).unwrap()).unwrap();
        assert_eq!(imported.favorites.iter().map(|f|f.volume_id.clone()).collect::<Vec<_>>(),volumes);
    }
    #[test]
    fn version_one_import_does_not_guess_missing_favorite_fields() {
        let bundle=ConfigBundle {favorites:vec![generation_folder(generation_volume("12345678","NTFS","0123456789abcdef","01d9abcd01234567"))],..ConfigBundle::default()};
        let value=serde_json::to_value(bundle).unwrap();
        for field in ["id","deviceId","volumeId","relativePath","alias","position"] {
            let mut missing=value.clone();missing["favorites"][0].as_object_mut().unwrap().remove(field);
            assert!(parse_config(&missing.to_string()).is_err(),"缺失 {field} 不能补猜");
        }
        let mut missing=value;missing.as_object_mut().unwrap().remove("version");
        assert!(parse_config(&missing.to_string()).is_err());
    }
    #[test]
    fn capture_rules_retain_stage_two_restrictions() {
        let mut capture = CaptureSettings::default(); capture.process_names = vec!["explorer.exe".into()];
        assert!(validate_capture(capture).is_err());
        let mut capture = CaptureSettings::default(); capture.sources = vec![crate::SourceRule {path: "C:\\..\\x".into(), recursive: true}];
        assert!(validate_capture(capture).is_err());
    }
}
