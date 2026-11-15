use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveItem {
    pub id: String,
    pub display_name: String,
    pub extension: String,
    pub size: u64,
    pub detected_unix_ms: u64,
    pub state: String,
    pub task_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRule {
    pub path: String,
    pub recursive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureSettings {
    /// loose: 主程序或明确 WPS 安装目录内的辅助程序；strict: 仅主程序。
    pub mode: String,
    pub process_names: Vec<String>,
    pub process_paths: Vec<String>,
    pub wps_install_paths: Vec<String>,
    pub sources: Vec<SourceRule>,
    pub include_default_sources: bool,
    pub include_usb: bool,
    pub extensions: Vec<String>,
    pub excluded_paths: Vec<String>,
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
        }
    }
}
