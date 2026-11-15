//! 任务意图 -> 自有 staging（流式摘要、sync）-> 持久 staged 意图 ->
//! 同一文件句柄不覆盖发布 -> 校验/只读 -> SQLite available 事务。
use crate::{engine::{Shared, lock}, secure_fs::{self, DirectoryGuard, Opened, Snapshot}, storage::{self, Entity, Task}};
use sha2::{Digest, Sha256};
use std::{fs::File,io::{Read, Seek, SeekFrom, Write},path::{Path,PathBuf},sync::{Arc,atomic::{AtomicBool,Ordering}}};

/// 宿主必须保留租约，直到复制任务已完成全批源文件的打开/固定。
/// 同一只读句柄完成摘要校验，deny write/delete，父目录句柄也保持存活。
pub struct ArchiveLease {pub path:PathBuf,_opened:Opened,pub(crate) fingerprint:Snapshot}
impl ArchiveLease {
    pub(crate) fn validated_display_path(&self)->Result<PathBuf,String> {
        self._opened._parents.validate()?;
        let directory=self._opened._parents.display_path()?;
        if directory.to_string_lossy().starts_with("\\\\?\\Volume{") {return Err("原归档卷没有当前可用的盘符路径，不能调用系统打开".into());}
        let path=directory.join(self.path.file_name().ok_or("归档文件名为空")?);
        let opened=Opened::source(&path,true)?;
        if secure_fs::snapshot(&opened.file)?.identity!=self.fingerprint.identity {return Err("归档盘符映射已变化，禁止打开另一个盘的同名文件".into());}
        Ok(path)
    }
}
#[derive(Clone)]
pub(crate) struct VerifiedArchive {pub rel:String,pub digest:String,pub size:u64,pub fingerprint:Snapshot}
impl VerifiedArchive {
    pub fn new(entity:&Entity,fingerprint:Snapshot)->Self {
        Self {rel:entity.rel.clone(),digest:entity.digest.clone(),size:entity.size,fingerprint}
    }
}

#[derive(Debug)]
pub struct Failure {pub message:String,pub terminal:bool}
impl From<String> for Failure {fn from(message:String)->Self {Self {message,terminal:false}}}
fn terminal(message:impl Into<String>)->Failure {Failure {message:message.into(),terminal:true}}
fn io_error(e:std::io::Error)->Failure {Failure {message:format!("复制归档失败：{e}"),terminal:false}}

pub fn digest_handle(file:&mut File,stop:Option<&AtomicBool>)->Result<(String,u64),String> {
    secure_fs::ensure_single_link(file)?;
    let before=secure_fs::snapshot(file)?;
    file.seek(SeekFrom::Start(0)).map_err(|e|format!("定位归档句柄失败：{e}"))?;
    let mut hasher=Sha256::new();
    let mut buffer=[0u8;64*1024];
    let mut size=0u64;
    loop {
        if stop.is_some_and(|s|s.load(Ordering::Acquire)) {return Err("应用退出，校验已中断".into());}
        let n=file.read(&mut buffer).map_err(|e|format!("读取归档句柄失败：{e}"))?;
        if n==0 {break;}
        hasher.update(&buffer[..n]);
        size=size.checked_add(n as u64).ok_or("归档大小溢出")?;
    }
    if size!=before.size || secure_fs::snapshot(file)?!=before {return Err("归档在同句柄校验期间变化".into());}
    file.seek(SeekFrom::Start(0)).map_err(|e|format!("复位归档句柄失败：{e}"))?;
    Ok((format!("{:x}",hasher.finalize()),size))
}
fn verify_handle(file:&mut File,digest:&str,size:u64,stop:Option<&AtomicBool>)->Result<Snapshot,String> {
    let (actual,actual_size)=digest_handle(file,stop)?;
    if actual!=digest || actual_size!=size {return Err("归档副本缺失、大小或 SHA-256 不匹配；禁止打开和复用".into());}
    secure_fs::snapshot(file)
}
fn require_available(entity:&Entity)->Result<(),String> {
    if entity.state!="available" {return Err("归档实体不是可用状态".into());} Ok(())
}
fn metadata_check(file:&File,entity:&Entity)->Result<Snapshot,String> {
    secure_fs::ensure_single_link(file)?;
    let snapshot=secure_fs::snapshot(file)?;
    if snapshot.size!=entity.size {return Err("归档副本大小发生变化".into());}
    if snapshot.attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY==0 {
        return Err("归档副本的只读保护已被移除".into());
    }
    Ok(snapshot)
}
pub fn lease(root:&Path,entity:&Entity,stop:Option<&AtomicBool>)->Result<ArchiveLease,String> {
    require_available(entity)?;
    let path=secure_fs::relative(root,&entity.rel)?;
    let mut opened=Opened::archive(&path)?;
    metadata_check(&opened.file,entity)?;
    let fingerprint=verify_handle(&mut opened.file,&entity.digest,entity.size,stop)?;
    Ok(ArchiveLease {path,_opened:opened,fingerprint})
}
pub fn check_cached(root:&Path,entity:&Entity,cached:Option<&VerifiedArchive>)->Result<Snapshot,String> {
    require_available(entity)?;
    let path=secure_fs::relative(root,&entity.rel)?;
    let mut opened=Opened::archive(&path)?;
    let current=metadata_check(&opened.file,entity)?;
    // 每次检查存在性/大小/卷及文件ID/mtime/ChangeTime/属性；只有匹配先前真实
    // 摘要验证的完整指纹才免 hash。初次、重启或任何指纹改变均在同一 handle 重验。
    if cached.is_some_and(|c|c.rel==entity.rel && c.digest==entity.digest && c.size==entity.size && c.fingerprint==current) {
        return Ok(current);
    }
    verify_handle(&mut opened.file,&entity.digest,entity.size,None)
}

fn root(shared:&Shared)->Result<PathBuf,String> {
    let inner=lock(&shared.inner)?;
    let store=inner.store.as_ref().ok_or("持久目录尚未配置")?;
    store.validate()?;Ok(store.root.clone())
}
fn reuse(shared:&Shared,root:&Path,task:&Task,entity:&Entity,stop:&AtomicBool)->Result<ArchiveLease,Failure> {
    match lease(root,entity,Some(stop)) {
        Ok(lease)=>Ok(lease),
        Err(error)=>{
            if stop.load(Ordering::Acquire) {return Err(error.into());}
            lock(&shared.inner)?.store.as_mut().ok_or("持久目录未配置")?.invalidate(&entity.id,&error)?;
            Err(terminal(format!("同内容实体校验失败，拒绝静默复用（任务 {}）：{error}",task.id)))
        }
    }
}

pub fn execute(shared:&Arc<Shared>,mut task:Task,stop:&AtomicBool)->Result<(),Failure> {
    let root=root(shared)?;
    let stage=secure_fs::relative(&root,&task.stage_rel)?;
    let final_path=secure_fs::relative(&root,&task.final_rel)?;
    let stage_guard=DirectoryGuard::acquire(stage.parent().ok_or("staging 没有父目录")?,false)?;
    let final_guard=DirectoryGuard::for_publish(final_path.parent().ok_or("归档没有父目录")?)?;
    let mut artifact:Option<Opened>=None;
    let mut recovered_final=false;
    if let Some(expected)=&task.digest {
        if final_path.try_exists().map_err(io_error)? {
            let mut opened=Opened::owned(&final_path,false)?;
            verify_handle(&mut opened.file,expected,task.size,Some(stop))
                .map_err(|e|if stop.load(Ordering::Acquire) {e.into()} else {terminal(format!("任务已知最终文件无法恢复，不覆盖：{e}"))})?;
            artifact=Some(opened);recovered_final=true;
        } else if stage.try_exists().map_err(io_error)? {
            let mut opened=Opened::owned(&stage,true)?;
            verify_handle(&mut opened.file,expected,task.size,Some(stop))
                .map_err(|e|if stop.load(Ordering::Acquire) {e.into()} else {terminal(format!("恢复区任务副本无法验证，保留异常文件：{e}"))})?;
            artifact=Some(opened);
        } else {
            let existing={let i=lock(&shared.inner)?;i.store.as_ref().ok_or("持久目录未配置")?.by_content(expected,task.size)?};
            if let Some(entity)=existing {
                let validated=reuse(shared,&root,&task,&entity,stop)?;
                lock(&shared.inner)?.store.as_mut().ok_or("持久目录未配置")?.finish(&task,&entity,false)?;
                drop(validated);
                return Ok(());
            }
            task.digest=None;
        }
    }
    if task.digest.is_none() {
        if final_path.try_exists().map_err(io_error)? {return Err(terminal("没有完整 staged 意图的最终路径已存在，不覆盖"));}
        secure_fs::remove_owned(&stage)?;
        let source=secure_fs::local_path(&task.source)?;
        if crate::capture::probe::filter::under(&task.source,&root.to_string_lossy()) {return Err(terminal("拒绝捕获持久目录自身"));}
        let mut input=Opened::source(&source,false)?;
        let before=secure_fs::snapshot(&input.file)?;
        if crate::capture::probe::filter::under(&secure_fs::guid_path(&input.file)?.to_string_lossy(),&root.to_string_lossy()) {return Err(terminal("拒绝捕获原数据卷归档自身"));}
        if before.identity!=task.identity {return Err(terminal("源卷或文件身份已更换，拒绝按同盘符/同路径误读"));}
        if storage::now_ms()>=task.deadline {return Err(terminal("已超过本轮 40 分钟重试截止时间"));}
        let mut output=Opened {file:secure_fs::create_owned(&stage)?,_parents:stage_guard.clone()};
        let mut hasher=Sha256::new();let mut size=0u64;let mut buffer=[0u8;64*1024];
        loop {
            if stop.load(Ordering::Acquire) {return Err("退出时保留捕获意图，稍后恢复".to_owned().into());}
            if storage::now_ms()>=task.deadline {return Err(terminal("复制超过 40 分钟截止时间"));}
            let n=input.file.read(&mut buffer).map_err(io_error)?;
            if n==0 {break;}
            output.file.write_all(&buffer[..n]).map_err(io_error)?;
            hasher.update(&buffer[..n]);size=size.checked_add(n as u64).ok_or_else(||terminal("文件大小溢出"))?;
        }
        if size!=before.size || secure_fs::snapshot(&input.file)?!=before {return Err("源文件在复制期间变化，未将副本标为可用".to_owned().into());}
        output.file.flush().map_err(io_error)?;output.file.sync_all().map_err(io_error)?;
        drop(input);
        let digest=format!("{:x}",hasher.finalize());
        // 保持 DELETE/RW 自有句柄不关闭，并从此同一 handle 回读验证真实保存内容。
        verify_handle(&mut output.file,&digest,size,Some(stop))?;
        lock(&shared.inner)?.store.as_ref().ok_or("持久目录未配置")?.staged(&task,&digest,size)?;
        task.digest=Some(digest);task.size=size;artifact=Some(output);
    }
    let digest=task.digest.clone().ok_or("未产生保存副本摘要")?;
    let existing={let i=lock(&shared.inner)?;i.store.as_ref().ok_or("持久目录未配置")?.by_content(&digest,task.size)?};
    if let Some(entity)=existing {
        if recovered_final && entity.id==task.entity_id && entity.rel==task.final_rel {
            require_available(&entity)?;
            secure_fs::readonly_handle(&artifact.as_ref().ok_or("恢复最终副本未锁定")?.file)?;
            let mut inner=lock(&shared.inner)?;
            inner.store.as_mut().ok_or("持久目录未配置")?.finish(&task,&entity,false)?;
            drop(artifact); // available 对外可查询前释放私有 writer/delete 句柄。
            return Ok(());
        }
        let validated=reuse(shared,&root,&task,&entity,stop)?;
        if let Some(owned)=&artifact {secure_fs::remove_handle(&owned.file)?;}
        drop(artifact.take());
        lock(&shared.inner)?.store.as_mut().ok_or("持久目录未配置")?.finish(&task,&entity,false)?;
        drop(validated); // 复用实体全程由校验过的只读租约固定到提交完成。
        return Ok(());
    }
    let owned=artifact.as_mut().ok_or("任务没有已锁定的保存副本")?;
    if !recovered_final {
        let name=final_path.file_name().ok_or("归档文件名无效")?.to_str().ok_or("归档文件名非 Unicode")?;
        secure_fs::publish_handle(&owned.file,&final_guard,name)?;
    }
    secure_fs::readonly_handle(&owned.file)?;
    if !recovered_final {owned.file.sync_all().map_err(io_error)?;}
    let entity=Entity {id:task.entity_id.clone(),rel:task.final_rel.clone(),digest,size:task.size,state:"available".into()};
    // 内容、发布与只读均由同一 handle 完成；SQLite 提交后，在状态锁释放之前
    // 关闭私有 RW/DELETE 句柄，防止首次查询把我们自己的短暂 share lock 当异常。
    let mut inner=lock(&shared.inner)?;
    let result=inner.store.as_mut().ok_or("持久目录未配置")?.finish(&task,&entity,true);
    drop(artifact);
    result?;
    Ok(())
}
