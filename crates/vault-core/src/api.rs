use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 1;

/// 仅供原生宿主读取的进程内活动快照；不序列化，不包含源路径或墙钟时间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureActivityKind { Detected, Archived }

#[derive(Clone, Copy, Debug)]
pub struct CaptureActivity {
    pub sequence: u64,
    pub kind: CaptureActivityKind,
    pub occurred_at: std::time::Instant,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub data_root: Option<String>,
    pub monitor_state: String,
    pub monitor_detail: String,
    pub pending_count: u64,
    pub failed_count: u64,
    pub has_errors: bool,
    pub health: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveItem {
    pub id: String,
    pub display_name: String,
    pub extension: String,
    pub size: u64,
    pub detected_unix_ms: u64,
    pub state: String,
    pub task_id: Option<String>,
    pub error: Option<String>,
    pub course_id: Option<String>,
    pub subject_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRule {
    pub path: String,
    pub recursive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureSettings {
    /// loose：主程序或明确 WPS 安装目录内的辅助程序；strict：仅主程序。
    pub mode: String,
    pub process_names: Vec<String>,
    pub process_paths: Vec<String>,
    pub wps_install_paths: Vec<String>,
    pub sources: Vec<SourceRule>,
    pub include_default_sources: bool,
    pub include_usb: bool,
    pub extensions: Vec<String>,
    pub excluded_paths: Vec<String>,
    /// 稳定科目 ID；保留课表中已经移除的 ID，不按名称匹配。
    #[serde(default)]
    pub excluded_subject_ids: Vec<String>,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            mode: "loose".into(),
            process_names: ["wps.exe", "wpp.exe", "et.exe", "wpspdf.exe", "photolaunch.exe"]
                .into_iter().map(str::to_owned).collect(),
            process_paths: Vec::new(),
            wps_install_paths: Vec::new(),
            sources: Vec::new(),
            include_default_sources: true,
            include_usb: true,
            extensions: "doc,docx,docm,dot,dotx,dotm,wps,wpt,pdf,ppt,pptx,pptm,pps,ppsx,ppsm,pot,potx,potm,dps,dpt,xls,xlsx,xlsm,xlsb,xlt,xltx,xltm,et,ett,jpg,jpeg,png,bmp,gif,tif,tiff,webp,ico,heic,heif"
                .split(',').map(str::to_owned).collect(),
            excluded_paths: Vec::new(),
            excluded_subject_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Subject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimeSlot {
    pub id: String,
    pub label: String,
    pub start: String,
    pub end: String,
    pub is_class: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeekLesson {
    pub weekday: u8,
    pub slot_id: String,
    pub subject_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Timetable {
    pub subjects: Vec<Subject>,
    pub slots: Vec<TimeSlot>,
    pub lessons: Vec<WeekLesson>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSettings {
    pub home_direction: String,
    pub horizontal_ratio: f64,
    pub vertical_ratio: f64,
    pub horizontal_target_ratio: f64,
    pub vertical_target_ratio: f64,
    pub library_ratio: f64,
    pub scale_percent: u32,
    pub show_hidden: bool,
    pub browser_view: String,
    pub window_width: u32,
    pub window_height: u32,
    pub window_maximized: bool,
    pub close_noticed: bool,
    #[serde(default = "default_notification_seconds")]
    pub notification_seconds: u32,
    #[serde(default)]
    pub appearance: AppearanceSettings,
}

pub const BACKGROUND_IDS: [&str; 13] = ["aurora","silk","mesh","grain","ice","veil","smoke","neuro","flow","rays","blend","flux","gate"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundParameters {
    pub quality: f64, pub speed: f64, pub intensity: f64, pub fps: u32,
    pub scale: f64, pub detail: f64, pub grain: f64, pub density: u32, pub shockwave: bool,
}
impl Default for BackgroundParameters {
    fn default() -> Self { Self { quality: 0.5, speed: 1.0, intensity: 0.6, fps: 30, scale: 1.0, detail: 1.0, grain: 1.0, density: 80, shockwave: true } }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearanceSettings {
    pub theme: String, pub hue: u32, pub background: String, pub shader: String,
    pub parameters: std::collections::BTreeMap<String, BackgroundParameters>,
    pub image: Option<String>, pub image_fit: String, pub image_opacity: f64, pub image_blur: f64,
}
impl Default for AppearanceSettings {
    fn default() -> Self { Self {
        theme: "customdark".into(), hue: 38, background: "off".into(), shader: "aurora".into(),
        parameters: BACKGROUND_IDS.into_iter().map(|id|(id.into(),BackgroundParameters::default())).collect(),
        image: None, image_fit: "cover".into(), image_opacity: 0.6, image_blur: 0.0,
    } }
}

fn default_notification_seconds() -> u32 { 6 }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            home_direction: "horizontal".into(), horizontal_ratio: 0.5, vertical_ratio: 0.5,
            horizontal_target_ratio: 0.5, vertical_target_ratio: 0.5, library_ratio: 0.5,
            scale_percent: 100, show_hidden: false, browser_view: "grid".into(),
            window_width: 1280, window_height: 800, window_maximized: false, close_noticed: false, notification_seconds: default_notification_seconds(), appearance: AppearanceSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixedFolder {
    pub id: String,
    pub device_id: String,
    pub volume_id: String,
    pub relative_path: String,
    pub alias: String,
    pub position: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigBundle {
    pub version: u32,
    pub app: AppSettings,
    pub capture: CaptureSettings,
    pub timetable: Timetable,
    pub favorites: Vec<FixedFolder>,
}

impl Default for ConfigBundle {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION, app: AppSettings::default(), capture: CaptureSettings::default(),
            timetable: Timetable::default(), favorites: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CourseView {
    pub id: String,
    pub date: String,
    pub slot_id: Option<String>,
    pub subject_id: Option<String>,
    pub subject_name: String,
    pub label: String,
    pub start_unix_ms: u64,
    pub end_unix_ms: u64,
    pub enabled: bool,
    pub items: Vec<ArchiveItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryQuery {
    pub name: String,
    pub regex: bool,
    pub subjects: Vec<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub extensions: Vec<String>,
    pub group_by: String,
    pub sort_by: String,
    pub descending: bool,
    pub offset: u64,
    pub limit: u32,
}

impl Default for LibraryQuery {
    fn default() -> Self {
        Self {
            name: String::new(), regex: false, subjects: Vec::new(), date_from: None, date_to: None,
            extensions: Vec::new(), group_by: "none".into(), sort_by: "archived".into(),
            descending: true, offset: 0, limit: 100,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryRow { pub group_key: String, pub group_label: String, pub item: ArchiveItem }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryPage { pub rows: Vec<LibraryRow>, pub total: u64 }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageDetail {
    pub date: String,
    pub course_id: String,
    pub subject_name: String,
    pub detected_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveDetail {
    pub item: ArchiveItem,
    pub archive_path: String,
    pub source_paths: Vec<String>,
    pub uses: Vec<UsageDetail>,
    pub deleted: bool,
}
