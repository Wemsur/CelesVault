#![cfg(windows)]
mod etw;
mod filter;
mod model;
mod report;
mod windows;

use filter::{DEFAULT_EXTENSIONS, Scope};
use model::{LogRow, Marker};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    io::{self, BufRead},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    duration_seconds: Option<u64>,
    extra_roots: Vec<String>,
    only_explicit_roots: bool,
    extensions: Option<String>,
}
struct Options {
    duration: u64,
    roots: Vec<String>,
    only_roots: bool,
    extensions: String,
    output: PathBuf,
    smoke: bool,
}

fn help() {
    println!(
        "CelesVault WPS 文件访问诊断工具 {}\n\n只记录本地元数据，不复制资料、不上传、不安装驱动。\n学校：双击 Start-Probe.cmd，通过 UAC，按数字 + Enter 标记，再去操作样本。\n\n参数：\n  --duration-seconds 600     自动停止时限（1–3600 秒）\n  --root <绝对目录>           增加监听目录，可重复\n  --only-roots              仅监听 --root 或配置中指定的目录\n  --extensions doc,docx,png  覆盖扩展名列表\n  --output <目录>            输出父目录（每次新建子目录）\n  --smoke-test               8 秒本机 ETW 检查，只创建/读取自己的临时样本\n  --stop-session <完整会话名> 仅清理日志中指定的本工具遗留会话\n  --help                    不需要管理员权限\n\n默认读取 EXE 旁的 probe-config.json（存在时）。\n日志含文件名、路径和程序路径，请使用非敏感样本并检查后再分享。",
        env!("CARGO_PKG_VERSION")
    );
}
fn menu() {
    println!(
        "\n操作标记：输入编号，可附样本文件名，然后按 Enter，再切到 WPS/资源管理器操作。\n0 空闲基线   1 预览关闭/仅选中   2 预览开启/仅选中\n3 双击正式打开   4 WPS 已运行/打开另一文件   5 WPS 内部打开\n6 图片缩略图浏览   7 WPS 图片查看器正式打开\nm 自定义标签   d 显示盘符/根目录   q 正常结束   ? 显示菜单\n例如：3 D:\\测试课件.pptx\n"
    );
}
fn label(line: &str) -> Option<String> {
    let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
    let base = match cmd {
        "0" => "空闲基线",
        "1" => "预览关闭_仅选中",
        "2" => "预览开启_仅选中",
        "3" => "双击正式打开",
        "4" => "WPS已运行_另一文件",
        "5" => "WPS内部打开",
        "6" => "图片缩略图浏览",
        "7" => "WPS图片正式打开",
        "m" if !rest.trim().is_empty() => "自定义",
        _ => return None,
    };
    Some(
        format!("{base} {}", rest.trim())
            .trim()
            .chars()
            .take(512)
            .collect(),
    )
}
fn options() -> Result<Option<Options>, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        help();
        return Ok(None);
    }
    if args.first().is_some_and(|a| a == "--stop-session") {
        if args.len() != 2 {
            return Err("--stop-session 必须单独使用并提供完整会话名".into());
        }
        if !windows::elevated() {
            return Err("清理会话需要管理员权限".into());
        }
        etw::stop_named(&args[1])?;
        println!("指定诊断会话已停止。");
        return Ok(None);
    }
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("无法获取程序目录")?
        .to_owned();
    let config = exe_dir.join("probe-config.json");
    let settings: Settings = if config.exists() {
        serde_json::from_slice(&fs::read(&config).map_err(|e| e.to_string())?)
            .map_err(|e| format!("配置文件错误：{e}"))?
    } else {
        Settings::default()
    };
    let mut result = Options {
        duration: settings.duration_seconds.unwrap_or(600),
        roots: settings.extra_roots,
        only_roots: settings.only_explicit_roots,
        extensions: settings
            .extensions
            .unwrap_or_else(|| DEFAULT_EXTENSIONS.into()),
        output: exe_dir.join("logs"),
        smoke: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--only-roots" => result.only_roots = true,
            "--smoke-test" => {
                result.smoke = true;
                result.duration = 8;
                result.only_roots = true;
                result.roots.clear();
            }
            "--root" | "--output" | "--duration-seconds" | "--extensions" => {
                let flag = &args[i];
                i += 1;
                let value = args.get(i).ok_or_else(|| format!("{flag} 缺少参数"))?;
                match flag.as_str() {
                    "--root" => result.roots.push(value.clone()),
                    "--output" => result.output = PathBuf::from(value),
                    "--duration-seconds" => {
                        result.duration = value.parse().map_err(|_| "时限必须是整数秒")?
                    }
                    "--extensions" => result.extensions = value.clone(),
                    _ => unreachable!(),
                }
            }
            other => return Err(format!("未知参数：{other}，使用 --help 查看说明")),
        }
        i += 1;
    }
    if !(1..=3600).contains(&result.duration) {
        return Err("时限必须在 1–3600 秒之间".into());
    }
    for root in &result.roots {
        if !PathBuf::from(root).is_absolute() {
            return Err(format!("监听目录必须是绝对路径：{root}"));
        }
    }
    if !result.output.is_absolute() {
        result.output = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(result.output);
    }
    Ok(Some(result))
}
fn display_scope(scope: &Scope, drives: &[model::DriveInfo]) {
    println!("有效根目录：");
    for root in &scope.roots {
        println!("  {root}\\");
    }
    println!("排除目录：");
    for root in &scope.excluded {
        println!("  {root}");
    }
    println!("盘符：");
    for d in drives {
        println!(
            "  {} 类型={} 总线={:?} 自动移动盘={} 设备查询错误={:?}",
            d.root, d.drive_type, d.bus_type, d.selected_automatically, d.device_query_error
        );
    }
    println!(
        "若实际桌面/下载目录不正确，正常退出后在 probe-config.json 的 extra_roots 中添加。\n插入新 U 盘后等 3 秒，再开始测试；拔出盘的历史映射保留用于延迟事件。\n"
    );
}

fn capture(mut opts: Options) -> Result<bool, String> {
    if !windows::elevated() {
        return Err("当前没有管理员权限，ETW 未启动。学校请双击 Start-Probe.cmd；或以管理员权限运行此 EXE。".into());
    }
    let mut smoke_base = None;
    if opts.smoke {
        let base = std::env::temp_dir().join(format!(
            "CelesVault-Probe-Smoke-{}-{}",
            std::process::id(),
            windows::unix_ms()
        ));
        fs::create_dir(&base).map_err(|e| e.to_string())?;
        let source = base.join("source");
        fs::create_dir(&source).map_err(|e| e.to_string())?;
        opts.roots = vec![source.to_string_lossy().into_owned()];
        opts.only_roots = true;
        smoke_base = Some(base);
    }
    let run_dir = opts
        .output
        .join(format!("run-{}-{}", windows::unix_ms(), std::process::id()));
    let mut roots = if opts.only_roots {
        Vec::new()
    } else {
        windows::default_roots()
    };
    roots.extend(opts.roots);
    let mut scope = Scope::new(
        roots,
        vec![opts.output.to_string_lossy().into_owned()],
        &opts.extensions,
    );
    let mut drive_info = windows::drives();
    windows::update_scope(&mut scope, &drive_info, !opts.only_roots);
    if scope.roots.is_empty() {
        return Err("没有有效根目录，请用 --root 或配置指定目录".into());
    }
    if scope.extensions.is_empty() {
        return Err("扩展名列表为空".into());
    }
    let session = json!({"tool":"CelesVault WPS Probe","tool_version":env!("CARGO_PKG_VERSION"),"os":windows::os_version(),"started_unix_ms":windows::unix_ms(),"capture_started":false,"duration_seconds":opts.duration,"initial_roots":scope.roots,"excluded":scope.excluded,"extensions":scope.extension_list(),"initial_drives":drive_info,"scope_policy":"all_processes_but_only_selected_paths_and_extensions","content_collected":false,"network_used":false,"raw_etl_saved":false,"smoke_test":opts.smoke,"notes":["file I/O is not proof of a user opening a document","metadata name events are not opening evidence","unknown path statistics include out-of-scope system events","historical device mappings are retained during this short recording","process command lines and window titles are not collected"]});
    let mut report = report::Report::new(run_dir.clone(), session)
        .map_err(|e| format!("无法创建独立日志目录：{e}"))?;
    println!(
        "CelesVault WPS Probe {}\n{}\n日志目录：{}\n自动停止：{} 秒",
        env!("CARGO_PKG_VERSION"),
        windows::os_version(),
        run_dir.display(),
        opts.duration
    );
    display_scope(&scope, &drive_info);
    let scope = Arc::new(RwLock::new(scope));
    let (tx, rx) = mpsc::sync_channel(8192);
    let collector = Arc::new(Mutex::new(etw::Collector::new(scope.clone(), tx)));
    let mut trace = match etw::TraceSession::start(collector.clone()) {
        Ok(trace) => trace,
        Err(error) => {
            let summary = report
                .finish(Default::default(), Default::default(), &error, opts.smoke)
                .map_err(|e| e.to_string())?;
            eprintln!("{}\n失败报告：{}", summary["status"], run_dir.display());
            return Err(error);
        }
    };
    report.started(&trace.name).map_err(|e| e.to_string())?;
    println!(
        "ETW 已启动：{}\n开始前请等待 15 秒，确认基线；停止请用 q + Enter 或 Ctrl+C，不要直接关闭窗口。",
        trace.name
    );
    menu();
    let (input_tx, input_rx) = mpsc::channel();
    if !opts.smoke {
        thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                match line {
                    Ok(line) => {
                        let marker_time = windows::unix_ms();
                        if input_tx.send((marker_time, line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
    let start = Instant::now();
    let mut last_flush = Instant::now();
    let mut last_drive = Instant::now();
    let mut last_progress = Instant::now();
    let mut previous_foreground = 0;
    let mut displayed = HashSet::new();
    let mut reason = "自动时限到达".to_owned();
    let mut io_failed = false;
    if let Some(base) = &smoke_base {
        thread::sleep(Duration::from_millis(750));
        report
            .marker(Marker {
                unix_ms: windows::unix_ms(),
                label: "冒烟_本工具创建并读取临时样本".into(),
            })
            .map_err(|e| e.to_string())?;
        let file = base.join("source/probe-smoke.docx");
        // Synthetic bytes for OS I/O only; this is intentionally not a valid Word document.
        fs::write(&file, vec![0u8; 1024 * 1024]).map_err(|e| e.to_string())?;
        let bytes = fs::read(&file).map_err(|e| e.to_string())?;
        std::hint::black_box(bytes);
    }
    while start.elapsed() < Duration::from_secs(opts.duration)
        && !windows::STOP_REQUESTED.load(Ordering::Relaxed)
    {
        for _ in 0..2048 {
            match rx.try_recv() {
                Ok(event) => {
                    if displayed.len() < 20
                        && (event.operation == "open_create_handle" || event.operation == "read")
                        && displayed.insert((event.path.clone(), event.attributed_pid))
                    {
                        println!(
                            "观察到 {}：{}，进程={}，PID={:?}",
                            event.operation,
                            event.path,
                            event
                                .process
                                .as_ref()
                                .map(|p| p.name.as_str())
                                .unwrap_or("未知"),
                            event.attributed_pid
                        );
                    }
                    if let Err(e) = report.file(event) {
                        reason = format!("日志写入失败：{e}");
                        io_failed = true;
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    reason = "采集事件队列已断开".into();
                    io_failed = true;
                    break;
                }
            }
        }
        if io_failed {
            break;
        }
        while let Ok((time, line)) = input_rx.try_recv() {
            let line = line.trim();
            if line == "q" {
                reason = "用户正常结束".into();
                windows::STOP_REQUESTED.store(true, Ordering::Relaxed);
                break;
            }
            if line == "?" {
                menu();
                continue;
            }
            if line == "d" {
                display_scope(
                    &scope.read().unwrap_or_else(|p| p.into_inner()),
                    &drive_info,
                );
                continue;
            }
            if let Some(label) = label(line) {
                println!("已标记 [{time}] {label} —— 现在切到目标程序操作，动作之间留 5–10 秒。");
                if let Err(e) = report.marker(Marker {
                    unix_ms: time,
                    label,
                }) {
                    reason = format!("标记写入失败：{e}");
                    io_failed = true;
                    break;
                }
            } else {
                println!("未识别输入，输入 ? 查看菜单。");
            }
        }
        if io_failed {
            break;
        }
        let foreground = windows::foreground_pid();
        if foreground != previous_foreground && foreground != 0 {
            previous_foreground = foreground;
            let info = collector
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get_process(foreground);
            if let Some(info) = info
                && windows::relevant_name(&info.name)
                && let Err(e) = report.row(&LogRow::Foreground {
                    unix_ms: windows::unix_ms(),
                    process: info,
                })
            {
                reason = format!("前台辅助日志写入失败：{e}");
                io_failed = true;
                break;
            }
        }
        if last_drive.elapsed() >= Duration::from_secs(2) {
            let next = windows::drives();
            let old = serde_json::to_string(&drive_info).unwrap_or_default();
            let new = serde_json::to_string(&next).unwrap_or_default();
            if old != new {
                drive_info = next;
                let mut s = scope.write().unwrap_or_else(|p| p.into_inner());
                windows::update_scope(&mut s, &drive_info, !opts.only_roots);
                println!("存储设备发生变化。");
                display_scope(&s, &drive_info);
                if let Err(e) = report.row(&LogRow::Drives {
                    unix_ms: windows::unix_ms(),
                    drives: drive_info.clone(),
                    effective_roots: s.roots.clone(),
                }) {
                    reason = format!("设备日志写入失败：{e}");
                    io_failed = true;
                    break;
                }
            }
            last_drive = Instant::now();
        }
        if last_flush.elapsed() >= Duration::from_secs(1) {
            if let Err(e) = report.flush() {
                reason = format!("刷新日志失败：{e}");
                io_failed = true;
                break;
            }
            last_flush = Instant::now();
        }
        if last_progress.elapsed() >= Duration::from_secs(5) {
            let count = &collector.lock().unwrap_or_else(|p| p.into_inner()).counters;
            println!(
                "已运行 {} 秒；系统文件事件={}，范围匹配={}，写盘={}，队列丢弃={}",
                start.elapsed().as_secs(),
                count.file_events,
                count.scope_matching_events,
                report.file_rows,
                count.queue_dropped
            );
            last_progress = Instant::now();
        }
        if report.limit_hit {
            reason = "达到 64 MiB 或 250000 条日志上限，自动结束（不表示完整采集）".into();
            break;
        }
        if trace.consumer_finished() {
            reason = "ETW 消费者提前退出，采集异常".into();
            io_failed = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    if windows::STOP_REQUESTED.load(Ordering::Relaxed) && reason == "自动时限到达" {
        reason = "用户 Ctrl+C / Ctrl+Break 结束".into();
    }
    println!("正在停止本工具会话并生成摘要，请稍候……");
    let health = trace.stop();
    while let Ok(event) = rx.try_recv() {
        if !io_failed && let Err(e) = report.file(event) {
            eprintln!("尾部日志写入失败：{e}");
            io_failed = true;
        }
    }
    let counters = collector
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .counters
        .clone();
    let summary = report
        .finish(counters, health, &reason, opts.smoke)
        .map_err(|e| format!("生成摘要失败：{e}；已有日志位于 {}", run_dir.display()))?;
    println!(
        "\n{}\n报告：{}\n检查 summary.md 后，只压缩本次 run 文件夹。无需提供原始资料。",
        summary["status"],
        run_dir.display()
    );
    let okay = summary["trace_runtime_ok"] == true
        && summary["no_observed_transport_losses"] == true
        && !io_failed
        && (!opts.smoke || summary["smoke_test"]["passed"] == true);
    if opts.smoke {
        println!(
            "普通文件 I/O 冒烟测试 passed={}；WPS 本身尚未验证。",
            summary["smoke_test"]["passed"]
        );
    }
    if let Some(base) = smoke_base {
        let _ = fs::remove_file(base.join("source/probe-smoke.docx"));
        let _ = fs::remove_dir(base.join("source"));
        let _ = fs::remove_dir(base);
    }
    Ok(okay)
}
fn main() {
    windows::prepare_console();
    let result = options().and_then(|opts| match opts {
        Some(opts) => capture(opts),
        None => Ok(true),
    });
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("错误：{e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_labels_keep_context() {
        assert_eq!(
            label("3 D:\\a.pptx").as_deref(),
            Some("双击正式打开 D:\\a.pptx")
        );
        assert!(label("q").is_none());
    }
    #[test]
    fn marker_bound() {
        assert!(
            label(&format!("m {}", "字".repeat(2000)))
                .unwrap()
                .chars()
                .count()
                <= 512
        );
    }
}
