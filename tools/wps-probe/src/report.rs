use crate::{
    model::{Counters, FileEvent, LogRow, Marker, TraceHealth},
    windows,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

pub struct Report {
    pub dir: PathBuf,
    events: BufWriter<File>,
    markers: BufWriter<File>,
    pub file_rows: u64,
    pub log_write_failures: u64,
    pub limit_hit: bool,
    bytes: u64,
    rows: u64,
    marks: Vec<Marker>,
    session: Value,
}
fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut f, value)?;
    f.write_all(b"\n")?;
    f.flush()
}
impl Report {
    pub fn new(dir: PathBuf, session: Value) -> io::Result<Self> {
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&dir)?; // Never replace an earlier run's logs.
        write_json(&dir.join("session.json"), &session)?;
        Ok(Self {
            events: BufWriter::new(File::create(dir.join("events.jsonl"))?),
            markers: BufWriter::new(File::create(dir.join("markers.jsonl"))?),
            dir,
            session,
            file_rows: 0,
            log_write_failures: 0,
            limit_hit: false,
            bytes: 0,
            rows: 0,
            marks: Vec::new(),
        })
    }
    pub fn started(&mut self, name: &str) -> io::Result<()> {
        self.session["capture_started"] = json!(true);
        self.session["trace_session_name"] = json!(name);
        write_json(&self.dir.join("session.json"), &self.session)
    }
    pub fn row(&mut self, row: &LogRow) -> io::Result<()> {
        let data = serde_json::to_vec(row)?;
        if self.rows >= 250_000 || self.bytes + data.len() as u64 > 64 * 1024 * 1024 {
            self.limit_hit = true;
            return Ok(());
        }
        if let Err(e) = self
            .events
            .write_all(&data)
            .and_then(|()| self.events.write_all(b"\n"))
        {
            self.log_write_failures += 1;
            return Err(e);
        }
        self.bytes += data.len() as u64 + 1;
        self.rows += 1;
        if matches!(row, LogRow::File(_)) {
            self.file_rows += 1;
        }
        Ok(())
    }
    pub fn file(&mut self, event: FileEvent) -> io::Result<()> {
        self.row(&LogRow::File(event))
    }
    pub fn marker(&mut self, marker: Marker) -> io::Result<()> {
        if self.marks.len() >= 1000 {
            return Err(io::Error::other("操作标记达到 1000 条上限"));
        }
        let data = serde_json::to_vec(&marker)?;
        if let Err(e) = self
            .markers
            .write_all(&data)
            .and_then(|()| self.markers.write_all(b"\n"))
            .and_then(|()| self.markers.flush())
        {
            self.log_write_failures += 1;
            return Err(e);
        }
        self.marks.push(marker);
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        let result = self.events.flush().and_then(|()| self.markers.flush());
        if result.is_err() {
            self.log_write_failures += 1;
        }
        result
    }
    pub fn finish(
        mut self,
        counters: Counters,
        health: TraceHealth,
        reason: &str,
        smoke: bool,
    ) -> io::Result<Value> {
        self.flush()?;
        self.marks.sort_by_key(|m| m.unix_ms);
        let mut groups = BTreeMap::<(usize, String, Option<u32>, String, String), Group>::new();
        let mut summary_groups_dropped = 0;
        let mut smoke_open = false;
        let mut smoke_read = false;
        let mut malformed_rows = 0;
        for line in BufReader::new(File::open(self.dir.join("events.jsonl"))?).lines() {
            let line = line?;
            let row = match serde_json::from_str::<LogRow>(&line) {
                Ok(row) => row,
                Err(_) => {
                    malformed_rows += 1;
                    continue;
                }
            };
            if let LogRow::File(e) = row {
                let interval = interval_for(e.event_unix_ms, &self.marks);
                let process = e
                    .process
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未知进程".into());
                let key = (
                    interval,
                    e.path.clone(),
                    e.attributed_pid,
                    process.clone(),
                    e.operation.clone(),
                );
                if groups.len() >= 20_000 && !groups.contains_key(&key) {
                    summary_groups_dropped += 1;
                    continue;
                }
                let group = groups.entry(key).or_insert_with(|| Group {
                    interval,
                    label: if interval == 0 {
                        "未标记".into()
                    } else {
                        self.marks[interval - 1].label.clone()
                    },
                    path: e.path.clone(),
                    pid: e.attributed_pid,
                    process,
                    operation: e.operation.clone(),
                    count: 0,
                    first_unix_ms: e.event_unix_ms,
                    last_unix_ms: e.event_unix_ms,
                    total_io_bytes: 0,
                });
                group.count += 1;
                group.first_unix_ms = group.first_unix_ms.min(e.event_unix_ms);
                group.last_unix_ms = group.last_unix_ms.max(e.event_unix_ms);
                group.total_io_bytes += e.io_size.unwrap_or(0);
                if e.attributed_pid == Some(std::process::id())
                    && e.path.ends_with("probe-smoke.docx")
                {
                    smoke_open |= e.operation == "open_create_handle";
                    smoke_read |= e.operation == "read";
                }
            }
        }
        let capture_started = self.session["capture_started"].as_bool().unwrap_or(false);
        let trace_ok = capture_started
            && health.stop_code == 0
            && health.consumer_joined
            && health.consumer_code == Some(0)
            && counters.file_events > 0;
        let no_losses = health.events_lost == 0
            && health.realtime_buffers_lost == 0
            && counters.queue_dropped == 0
            && counters.callback_panics == 0
            && !self.limit_hit
            && self.log_write_failures == 0
            && malformed_rows == 0;
        let status = if !trace_ok {
            "采集未成功或未正常结束"
        } else if !no_losses {
            "采集完成，但存在丢失或日志异常"
        } else {
            "采集完成；覆盖率仍需对照人工操作"
        };
        let groups: Vec<_> = groups.into_values().collect();
        let summary = json!({"tool_version":env!("CARGO_PKG_VERSION"),"status":status,"capture_started":capture_started,"trace_runtime_ok":trace_ok,"no_observed_transport_losses":no_losses,"stop_reason":reason,"finished_unix_ms":windows::unix_ms(),"counters":counters,"trace_health":health,"file_rows_written":self.file_rows,"log_bytes":self.bytes,"log_write_failures":self.log_write_failures,"record_limit_hit":self.limit_hit,"malformed_rows":malformed_rows,"summary_groups_dropped":summary_groups_dropped,"markers":self.marks,"groups":groups,"smoke_test":if smoke {json!({"observed_own_open":smoke_open,"observed_own_read":smoke_read,"passed":trace_ok&&no_losses&&smoke_open&&smoke_read})}else{Value::Null}});
        self.session["finished_unix_ms"] = summary["finished_unix_ms"].clone();
        self.session["stop_reason"] = json!(reason);
        self.session["status"] = json!(status);
        self.session["trace_health"] = serde_json::to_value(&health)?;
        write_json(&self.dir.join("session.json"), &self.session)?;
        write_json(&self.dir.join("summary.json"), &summary)?;
        let mut md = String::from("# CelesVault 文件访问诊断报告\n\n");
        md.push_str(&format!(
            "**状态：{status}**\n\n停止原因：{}\n\n文件记录：{} 条；操作标记：{} 条。\n\n",
            escape(reason),
            self.file_rows,
            self.marks.len()
        ));
        md.push_str("## 如何解读\n\n- `open_create_handle` 是系统打开文件句柄，不等同于老师正式打开。以手动标记区间作对照。\n- `name_*_metadata` 仅是文件名称映射信息，不能算作打开证据。\n- 路径未关联统计包含全系统范围外的事件，不可直接当作教学文件漏收数。\n- PID/TID 归属来源记录在 events.jsonl；未知归属不会被猜成 WPS。\n- 进程启动/线程生命周期用于关联；实时进程路径查询不是历史 PID 身份的绝对证明。\n- 事件不采样、不聚合写盘；下面按区间/文件/进程/操作汇总。时间是 UTC Unix 毫秒。\n- 首次写入后每秒刷新，但不保证强制杀进程或突然断电时缓冲区完整。\n\n");
        md.push_str(&format!("## 采集健康\n\n- ETW 丢事件：{}；丢实时缓冲区：{}\n- 应用队列丢弃：{}；日志写入失败：{}；达到日志上限：{}\n- TDH 模式解析失败：{}；属性读取失败：{}\n- 全局未关联路径事件：{}；匹配事件中未知进程归属：{}\n- 名称缓存重置：{}；回调异常：{}\n- 会话停止返回码：{}；消费者返回码：{:?}；消费者已退出：{}\n\n",health.events_lost,health.realtime_buffers_lost,counters.queue_dropped,self.log_write_failures,self.limit_hit,counters.schema_failures,counters.property_failures,counters.path_unresolved_events,counters.unknown_attribution_matching_events,counters.name_cache_resets,counters.callback_panics,health.stop_code,health.consumer_code,health.consumer_joined));
        if smoke {
            md.push_str(&format!("## 本机冒烟测试\n\n观察到本工具自己的打开：{smoke_open}；读取：{smoke_read}。仅检查普通文件 I/O，不代表 WPS 已验证。\n\n"));
        }
        md.push_str("## 操作标记\n\n| UTC Unix 毫秒 | 标签 |\n|---|---|\n");
        for marker in &self.marks {
            md.push_str(&format!(
                "| {} | {} |\n",
                marker.unix_ms,
                escape(&marker.label)
            ));
        }
        md.push_str("\n## 文件与进程汇总（最多显示 200 组，完整数据在 summary.json）\n\n| 标记 | 文件 | 进程 / PID | 操作 | 次数 |\n|---|---|---|---|---|\n");
        for group in groups.iter().take(200) {
            md.push_str(&format!(
                "| {} | {} | {} / {} | {} | {} |\n",
                escape(&group.label),
                escape(&group.path),
                escape(&group.process),
                group
                    .pid
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "未知".into()),
                group.operation,
                group.count
            ));
        }
        fs::write(self.dir.join("summary.md"), md)?;
        Ok(summary)
    }
}

#[derive(Serialize)]
struct Group {
    interval: usize,
    label: String,
    path: String,
    pid: Option<u32>,
    process: String,
    operation: String,
    count: u64,
    first_unix_ms: u64,
    last_unix_ms: u64,
    total_io_bytes: u64,
}
fn escape(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
        .replace('|', "\\|")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub fn interval_for(time: u64, markers: &[Marker]) -> usize {
    markers.partition_point(|m| m.unix_ms <= time)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_etw_uses_event_time_not_arrival() {
        let m = vec![
            Marker {
                unix_ms: 100,
                label: "a".into(),
            },
            Marker {
                unix_ms: 200,
                label: "b".into(),
            },
        ];
        assert_eq!(interval_for(50, &m), 0);
        assert_eq!(interval_for(199, &m), 1);
        assert_eq!(interval_for(200, &m), 2);
    }
    #[test]
    fn metadata_error_counters_bounded() {
        let mut c = Counters::default();
        for n in 0..100 {
            c.error("schema", n, 64, 3);
        }
        assert_eq!(c.metadata_errors.len(), 32);
        c.error("schema", 0, 64, 3);
        assert_eq!(c.metadata_errors["schema:win32=0:opcode=64:version=3"], 2);
    }
    #[test]
    fn report_keeps_failed_capture_visible() {
        let dir = std::env::temp_dir().join(format!(
            "CelesVault-Probe-Unit-{}-{}",
            std::process::id(),
            windows::unix_ms()
        ));
        let report = Report::new(dir.clone(), json!({"capture_started":false})).unwrap();
        let summary = report
            .finish(
                Counters::default(),
                TraceHealth::default(),
                "测试启动失败",
                false,
            )
            .unwrap();
        assert_eq!(summary["trace_runtime_ok"], false);
        assert!(dir.join("summary.md").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
