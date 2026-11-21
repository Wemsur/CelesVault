//! 用户明确提交的回收意图：先记录状态，再按同一自有 GUID 句柄移动/删除，最后提交元数据。
//! 重启只重放已知意图；未知文件、外部替代文件和原始资料永不删除。
use crate::{archive, engine::{lock, Shared}, secure_fs::{self, DirectoryGuard, Opened}, storage::{Entity, FileIntent, Store}, thumbnail};
use std::path::Path;
use uuid::Uuid;

fn relative_kind(rel:&str,prefix:&str) -> Result<(),String> {
    if !rel.replace('\\',"/").starts_with(prefix) {return Err("回收意图不在规定的自有 archive/recovery 区内".into());}
    Ok(())
}
fn validate_intent(store:&Store,intent:&FileIntent) -> Result<(),String> {
    store.validate()?;let entity=store.entity(&intent.entity_id)?;
    if entity.digest!=intent.digest || entity.size!=intent.size || entity.owned_identity.as_ref()!=Some(&intent.identity) {
        return Err("回收意图与实体的内容/自有身份不一致，未接纳替代文件".into());
    }
    let valid=match intent.kind.as_str() {
        "trash" => entity.state=="deleting" && intent.from_rel==entity.rel && intent.to_rel==entity.trash_rel,
        "restore" => entity.state=="restoring" && Some(&intent.from_rel)==entity.trash_rel.as_ref() && intent.to_rel.as_deref()==Some(entity.rel.as_str()),
        "purge" => entity.state=="purging" && Some(&intent.from_rel)==entity.trash_rel.as_ref() && intent.to_rel.is_none(),
        _ => false,
    };
    if !valid {return Err("回收意图路径/生命周期状态不一致，拒绝旧操作覆盖当前状态".into());}
    relative_kind(&entity.rel,"archive/")?;
    if let Some(rel)=&entity.trash_rel {relative_kind(rel,"recovery/")?;}
    secure_fs::relative(&store.root,&intent.from_rel)?;
    if let Some(to)=&intent.to_rel {secure_fs::relative(&store.root,to)?;}
    Ok(())
}
fn verified_owned(root:&Path,rel:&str,entity:&Entity) -> Result<Opened,String> {
    let path=secure_fs::relative(root,rel)?;
    let mut opened=Opened::owned(&path,false)?;
    let before=secure_fs::snapshot(&opened.file)?;
    if entity.owned_identity.as_ref().is_some_and(|id|id!=&before.identity) {return Err("资料路径已被外部文件替换，禁止移动/删除它".into());}
    if entity.owned_identity.is_none() && before.attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY==0 {
        return Err("旧资料尚无身份基线且只读保护已移除；不能猜测它仍是自有副本".into());
    }
    archive::verify_handle(&mut opened.file,&entity.digest,entity.size,None)?;
    Ok(opened)
}
fn verify_intent_file(opened:&mut Opened,intent:&FileIntent) -> Result<(),String> {
    let snapshot=secure_fs::snapshot(&opened.file)?;
    if snapshot.identity!=intent.identity {return Err("意图文件身份已被外部换代，不删除/移动同名替代文件".into());}
    archive::verify_handle(&mut opened.file,&intent.digest,intent.size,None)?;
    Ok(())
}

/// 返回移动后仍独占的原句柄，调用者必须在 available/deleted 元数据提交后才释放。
fn run_io(root:&Path,intent:&FileIntent,prepared:Option<Opened>) -> Result<Option<Opened>,String> {
    let from=secure_fs::relative(root,&intent.from_rel)?;
    if intent.kind=="purge" {
        relative_kind(&intent.from_rel,"recovery/")?;
        let mut opened=match prepared {
            Some(opened)=>opened,
            None=>{if !from.try_exists().map_err(|e|format!("检查已清理副本失败：{e}"))? {return Ok(None);} Opened::owned(&from,false)?},
        };
        verify_intent_file(&mut opened,intent)?;
        // readonly 只在已固定、identity+SHA 校验成功的 OWN 句柄上移除。
        secure_fs::remove_handle(&opened.file)?;
        drop(opened); // 删除挂起必须实际关闭后，才允许提交 purged。
        return Ok(None);
    }
    let to=secure_fs::relative(root,intent.to_rel.as_deref().ok_or("移动意图缺少目标")?)?;
    let from_exists=from.try_exists().map_err(|e|format!("检查回收源失败：{e}"))?;
    let to_exists=to.try_exists().map_err(|e|format!("检查回收目标失败：{e}"))?;
    if from_exists && to_exists {return Err("回收源和目标同时存在，拒绝覆盖或猜测删除其中任一文件".into());}
    if !from_exists && !to_exists {return Err("回收源与目标均缺失，保留意图/历史，不模拟操作成功".into());}
    let mut opened=match prepared {
        Some(opened)=>{if !from_exists {return Err("已固定的回收源路径意外消失".into());}opened},
        None=>Opened::owned(if from_exists {&from} else {&to},false)?,
    };
    verify_intent_file(&mut opened,intent)?;
    if from_exists {
        let target=DirectoryGuard::for_publish(to.parent().ok_or("回收目标没有父目录")?)?;
        let name=to.file_name().and_then(|n|n.to_str()).ok_or("回收目标名无效")?;
        secure_fs::publish_handle(&opened.file,&target,name)?;
        opened._parents=target;
        verify_intent_file(&mut opened,intent)?;
    }
    // 归档始终维持只读；恢复到 available 之前再用同一句柄确保属性。
    secure_fs::readonly_handle(&opened.file)?;
    opened.file.sync_all().map_err(|e|format!("同步回收移动失败：{e}"))?;
    Ok(Some(opened))
}

pub(crate) fn action(shared:&Shared,id:&str,kind:&str) -> Result<(),String> {
    let result=action_inner(shared,id,kind);
    if let Err(error)=result {
        let record=(|| {
            let inner=lock(&shared.inner)?;
            let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;
            if store.intent(id)?.is_some() {store.intent_error(id,&error)?;}
            Ok::<_,String>(())
        })();
        return match record {Ok(())=>Err(error),Err(metadata)=>Err(format!("{error}；保存回收失败状态也失败：{metadata}"))};
    }
    Ok(())
}
fn action_inner(shared:&Shared,id:&str,kind:&str) -> Result<(),String> {
    let (root,entity,existing)={
        let inner=lock(&shared.inner)?;let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;
        if inner.shutdown || inner.switching {return Err("引擎处于关闭/换根状态，拒绝回收操作".into());}
        if store.has_active_entity(id)? {return Err("资料有活动归档任务，暂不能删除/恢复/清理".into());}
        (store.root.clone(),store.entity(id)?,store.intent(id)?)
    };
    let (intent,prepared)=if let Some(intent)=existing {
        if intent.kind!=kind {return Err(format!("该资料尚有 {} 意图，请先完成该动作，不覆盖为 {kind}",intent.kind));}
        {let inner=lock(&shared.inner)?;validate_intent(inner.store.as_ref().ok_or("数据目录尚未配置")?,&intent)?;}
        (intent,None)
    } else {
        let (from,to)=match kind {
            "trash"=>{
                if !matches!(entity.state.as_str(),"available"|"failed") {return Err("只能将现有归档副本移入应用回收站".into());}
                relative_kind(&entity.rel,"archive/")?;
                let uuid=Uuid::parse_str(&entity.id).map_err(|_|"资料 ID 不是自有 UUID，拒绝构造回收路径")?;
                (entity.rel.clone(),Some(format!("recovery/{uuid}/{}.cvtrash",Uuid::new_v4())))
            }
            "restore"=>{
                if entity.state!="deleted" {return Err("只有已回收资料可以恢复".into());}
                (entity.trash_rel.clone().ok_or("回收资料缺少自有恢复路径")?,Some(entity.rel.clone()))
            }
            "purge"=>{
                if entity.state!="deleted" {return Err("永久清理只允许对应用回收站资料明确执行，不能直接删除活动归档".into());}
                (entity.trash_rel.clone().ok_or("回收资料缺少自有恢复路径")?,None)
            }
            _=>return Err("未知回收动作".into()),
        };
        if kind!="trash" {relative_kind(&from,"recovery/")?;}
        let opened=if kind=="purge" && !secure_fs::relative(&root,&from)?.try_exists().map_err(|e|format!("检查已缺失回收副本失败：{e}"))? {
            None // 用户明确清理已缺失副本：只提交 tombstone/清理其已知缓存，不删除任何替代路径。
        } else {Some(verified_owned(&root,&from,&entity)?)};
        let identity=match &opened {Some(opened)=>secure_fs::snapshot(&opened.file)?.identity,
            None=>entity.owned_identity.clone().ok_or("缺失副本没有自有身份基线，不能猜测永久清理状态")?};
        let intent={let mut inner=lock(&shared.inner)?;inner.store.as_mut().ok_or("数据目录尚未配置")?.begin_intent(&entity,kind,&from,to.as_deref(),&identity)?};
        (intent,opened)
    };
    let opened=run_io(&root,&intent,prepared)?;
    if intent.kind=="purge" {thumbnail::remove_cached(shared,id)?;}
    let mut inner=lock(&shared.inner)?;
    let result=inner.store.as_mut().ok_or("数据目录尚未配置")?.complete_intent(&intent);
    drop(opened); // 状态锁释放前释放私有 writer，避免查询把自己的锁误判为篡改。
    inner.verified_archives.remove(id);
    result
}

/// Store 尚未交给后台线程时重放，不需要递归获取 Shared/生命周期锁。
pub(crate) fn resume_all(store:&mut Store) -> Vec<String> {
    let intents=match store.intents() {Ok(intents)=>intents,Err(error)=>return vec![format!("读取回收中断意图失败：{error}")]};
    let mut errors=Vec::new();
    for intent in intents {
        let result=(|| {
            validate_intent(store,&intent)?;
            let opened=run_io(&store.root,&intent,None)?;
            if intent.kind=="purge" {
                if let Some(record)=store.thumbnail_record(&intent.entity_id)? {thumbnail::remove_record_file(&store.root,&record)?;store.drop_thumbnail_record(&record)?;}
            }
            let result=store.complete_intent(&intent);drop(opened);result
        })();
        if let Err(error)=result {
            let message=match store.intent_error(&intent.entity_id,&error) {Ok(())=>error,Err(metadata)=>format!("{error}；记录失败状态失败：{metadata}")};
            errors.push(format!("回收意图 {} 未完成：{message}",intent.entity_id));
        }
    }
    errors
}
