//! 与状态/SQLite 锁分离的 RAII 读写登记；租约跨宿主异步复制的整个源固定阶段存活。
use std::{collections::HashMap, sync::{Arc, Mutex}};

#[derive(Default)]
struct Access { readers:usize, writer:bool }
#[derive(Default)]
struct State { entities:HashMap<String,Access>, sources:HashMap<String,usize> }
#[derive(Default)]
pub(crate) struct ArchiveActivity { state:Mutex<State> }
impl ArchiveActivity {
    pub fn read(self:&Arc<Self>,id:&str) -> Result<ActivityToken,String> {
        let mut state=self.state.lock().map_err(|_|"资料活动锁已损坏")?;
        let access=state.entities.entry(id.into()).or_default();
        if access.writer {return Err("资料正在被其他读写操作占用：存在活动写入/回收操作".into());}
        access.readers=access.readers.checked_add(1).ok_or("资料读租约计数溢出")?;
        Ok(ActivityToken {activity:self.clone(),id:id.into(),writer:false})
    }
    pub fn write(self:&Arc<Self>,id:&str) -> Result<ActivityToken,String> {
        let mut state=self.state.lock().map_err(|_|"资料活动锁已损坏")?;
        let access=state.entities.entry(id.into()).or_default();
        if access.writer || access.readers>0 {return Err("资料正在被其他读写操作占用：请等待归档/打开/导出租约释放".into());}
        access.writer=true;
        Ok(ActivityToken {activity:self.clone(),id:id.into(),writer:true})
    }
    pub fn busy(&self) -> Result<bool,String> {
        let state=self.state.lock().map_err(|_|"资料活动锁已损坏")?;
        Ok(state.entities.values().any(|a|a.writer || a.readers>0) || !state.sources.is_empty())
    }
    pub fn source(self:&Arc<Self>,volume:&str) -> Result<SourceToken,String> {
        let volume=crate::config::volume_id(volume)?;
        let mut state=self.state.lock().map_err(|_|"源活动锁已损坏")?;
        let count=state.sources.entry(volume.clone()).or_default();
        *count=count.checked_add(1).ok_or("源活动计数溢出")?;
        Ok(SourceToken {activity:self.clone(),volume})
    }
    pub fn source_volumes(&self) -> Result<Vec<String>,String> {
        Ok(self.state.lock().map_err(|_|"源活动锁已损坏")?.sources.keys().cloned().collect())
    }
}

pub(crate) struct ActivityToken { activity:Arc<ArchiveActivity>, id:String, writer:bool }
impl Drop for ActivityToken {
    fn drop(&mut self) {
        let mut state=self.activity.state.lock().unwrap_or_else(|poison|poison.into_inner());
        if let Some(access)=state.entities.get_mut(&self.id) {
            if self.writer {access.writer=false;} else {access.readers=access.readers.saturating_sub(1);}
            if !access.writer && access.readers==0 {state.entities.remove(&self.id);}
        }
    }
}
pub(crate) struct SourceToken { activity:Arc<ArchiveActivity>, volume:String }
impl Drop for SourceToken {
    fn drop(&mut self) {
        let mut state=self.activity.state.lock().unwrap_or_else(|poison|poison.into_inner());
        if let Some(count)=state.sources.get_mut(&self.volume) {
            *count=count.saturating_sub(1);if *count==0 {state.sources.remove(&self.volume);}
        }
    }
}
