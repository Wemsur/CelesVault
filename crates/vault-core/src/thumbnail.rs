//! 同步 CPU/IO API，由宿主 spawn_blocking 异步调用；只解码已校验归档句柄，不读原文件。
use crate::{archive::{self, ArchiveLease}, engine::{lock, Shared}, secure_fs::{self, DirectoryGuard, Opened}, storage::{self, Entity, ThumbnailRecord}};
use base64::Engine as _;
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};
use std::{fs, io::{BufReader, Cursor, Read, Seek, SeekFrom, Write}, os::windows::fs::MetadataExt, path::Path};
use uuid::Uuid;

const MAX_PIXELS:u64=32_000_000;
pub(crate) const MAX_SOURCE_BYTES:u64=64*1024*1024;
const MAX_PNG_BYTES:u64=1024*1024;
const CACHE_BYTES:u64=32*1024*1024;
const CACHE_FILES:usize=256;
const SIDE:u32=256;

pub(crate) fn supported(extension:&str) -> Option<ImageFormat> {
    match extension.to_ascii_lowercase().as_str() {
        "png"=>Some(ImageFormat::Png),"jpg"|"jpeg"=>Some(ImageFormat::Jpeg),"webp"=>Some(ImageFormat::WebP),
        "gif"=>Some(ImageFormat::Gif),"bmp"=>Some(ImageFormat::Bmp),"tif"|"tiff"=>Some(ImageFormat::Tiff),_=>None,
    }
}
fn uri(bytes:&[u8])->String {format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}
fn cache_path(root:&Path,record:&ThumbnailRecord)->Result<std::path::PathBuf,String> {
    if !record.rel.replace('\\',"/").starts_with("cache/thumbnails/") || !record.rel.ends_with(".png") {return Err("缓存索引路径不在自有缩略图区".into());}
    if record.size>MAX_PNG_BYTES {return Err("缓存 PNG 超过 1 MiB 上限".into());}
    secure_fs::relative(root,&record.rel)
}
fn verify_cache(opened:&mut Opened,record:&ThumbnailRecord)->Result<(),String> {
    if secure_fs::snapshot(&opened.file)?.identity!=record.identity {return Err("缩略图缓存已被外部替换；未读取/删除同名替代文件".into());}
    archive::verify_handle(&mut opened.file,&record.png_digest,record.size,None)?;
    Ok(())
}
fn load(root:&Path,record:&ThumbnailRecord)->Result<Vec<u8>,String> {
    let path=cache_path(root,record)?;let mut opened=Opened::archive(&path)?;
    verify_cache(&mut opened,record)?;
    let mut bytes=Vec::with_capacity(record.size as usize);
    (&mut opened.file).take(MAX_PNG_BYTES+1).read_to_end(&mut bytes).map_err(|e|format!("读取缩略图缓存失败：{e}"))?;
    if bytes.len() as u64!=record.size || bytes.len() as u64>MAX_PNG_BYTES {return Err("缩略图缓存大小不一致".into());}
    Ok(bytes)
}
pub(crate) fn remove_record_file(root:&Path,record:&ThumbnailRecord)->Result<(),String> {
    let path=cache_path(root,record)?;
    if !path.try_exists().map_err(|e|format!("检查缩略图缓存失败：{e}"))? {return Ok(());}
    let mut opened=Opened::owned(&path,false)?;
    verify_cache(&mut opened,record)?;
    secure_fs::remove_handle(&opened.file)?;drop(opened);Ok(())
}
pub(crate) fn remove_cached(shared:&Shared,id:&str)->Result<(),String> {
    let (root,record)={let inner=lock(&shared.inner)?;let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;(store.root.clone(),store.thumbnail_record(id)?)};
    if let Some(record)=record {
        remove_record_file(&root,&record)?;
        lock(&shared.inner)?.store.as_ref().ok_or("数据目录尚未配置")?.drop_thumbnail_record(&record)?;
    }
    Ok(())
}
fn reader(lease:&ArchiveLease,format:ImageFormat)->Result<ImageReader<BufReader<std::fs::File>>,String> {
    let mut file=lease.reader()?;file.seek(SeekFrom::Start(0)).map_err(|e|format!("定位图片归档失败：{e}"))?;
    let mut reader=ImageReader::with_format(BufReader::new(file),format);
    let mut limits=Limits::default();limits.max_image_width=Some(32768);limits.max_image_height=Some(32768);limits.max_alloc=Some(128*1024*1024);
    reader.limits(limits);Ok(reader)
}
fn generate(lease:&ArchiveLease,entity:&Entity,format:ImageFormat)->Result<Vec<u8>,String> {
    if entity.size>MAX_SOURCE_BYTES {return Err("图片超过 64 MiB 缩略图输入上限，归档副本仍保留可导出".into());}
    let (width,height)=reader(lease,format)?.into_dimensions().map_err(|e|format!("读取图片尺寸失败：{e}"))?;
    if width==0 || height==0 || u64::from(width)*u64::from(height)>MAX_PIXELS {return Err("图片超过 32 MP 或尺寸无效，不解码超限图片".into());}
    let decoded=reader(lease,format)?.decode().map_err(|e|format!("图片解码失败或超出内存限制：{e}"))?;
    if u64::from(decoded.width())*u64::from(decoded.height())>MAX_PIXELS {return Err("实际图片尺寸超限".into());}
    // GIF 仅首帧；不运行动画解码，也不提供 Office/PDF 伪首页图。
    let thumbnail=decoded.thumbnail(SIDE,SIDE);drop(decoded);
    let mut out=Cursor::new(Vec::new());thumbnail.write_to(&mut out,ImageFormat::Png).map_err(|e|format!("编码 PNG 缩略图失败：{e}"))?;
    let bytes=out.into_inner();if bytes.len() as u64>MAX_PNG_BYTES {return Err("生成 PNG 缩略图超过 1 MiB 上限".into());}Ok(bytes)
}

/// 所有未知缓存文件也计入预算；它们不因 LRU 被删除。扫描有界，无目录递归。
fn budget(root:&Path,additional:u64)->Result<bool,String> {
    let guard=DirectoryGuard::acquire(&root.join("cache/thumbnails"),false)?;guard.validate()?;
    let entries=fs::read_dir(&guard.path).map_err(|e|format!("读取自有缩略图缓存目录失败：{e}"))?;
    let mut count=0usize;let mut bytes=0u64;
    for entry in entries {
        count+=1;if count>1024 {return Err("缓存目录含超过 1024 个条目，停止扫描；未知文件不会被清理".into());}
        let entry=entry.map_err(|e|format!("读取缓存条目失败：{e}"))?;
        let info=fs::symlink_metadata(entry.path()).map_err(|e|format!("检查缓存条目失败：{e}"))?;
        if info.is_dir() || info.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT!=0 {return Err("缓存目录出现目录/重解析点，停止缓存写入，不沿外部路径清理".into());}
        bytes=bytes.checked_add(info.len()).ok_or("缓存大小溢出")?;
    }
    guard.validate()?;Ok(count<CACHE_FILES && bytes.checked_add(additional).is_some_and(|b|b<=CACHE_BYTES))
}
fn reserve_budget(shared:&Shared,root:&Path,bytes:u64)->Result<bool,String> {
    if budget(root,bytes)? {return Ok(true);}
    let records={let inner=lock(&shared.inner)?;inner.store.as_ref().ok_or("数据目录尚未配置")?.thumbnails()?};
    for record in records {
        remove_record_file(root,&record)?;
        lock(&shared.inner)?.store.as_ref().ok_or("数据目录尚未配置")?.drop_thumbnail_record(&record)?;
        if budget(root,bytes)? {return Ok(true);}
    }
    Ok(false)
}

pub(crate) fn get(shared:&Shared,root:&Path,entity:&Entity,lease:&ArchiveLease)->Result<Option<String>,String> {
    let Some(format)=supported(&entity.extension) else {return Ok(None);};
    let cached={let inner=lock(&shared.inner)?;inner.store.as_ref().ok_or("数据目录尚未配置")?.thumbnail_record(&entity.id)?};
    if let Some(record)=cached {
        if record.source_digest!=entity.digest {return Err("缓存来源摘要与归档不一致，未静默使用旧图".into());}
        let bytes=load(root,&record)?;
        lock(&shared.inner)?.store.as_ref().ok_or("数据目录尚未配置")?.touch_thumbnail(&entity.id)?;
        return Ok(Some(uri(&bytes)));
    }
    let bytes=generate(lease,entity,format)?;
    if !reserve_budget(shared,root,bytes.len() as u64)? {
        shared.note_capture_error("缩略图已真实生成，但自有缓存预算被未知文件占满；未扩大缓存、换目录或删除未知文件".into());
        return Ok(Some(uri(&bytes)));
    }
    let rel=format!("cache/thumbnails/{}--{}.png",entity.id,Uuid::new_v4());
    let path=secure_fs::relative(root,&rel)?;
    let parents=DirectoryGuard::acquire(path.parent().ok_or("缓存没有父目录")?,false)?;
    let mut owned=Some(Opened {file:secure_fs::create_owned(&path)?,_parents:parents});
    let written=(|| {
        let file=&mut owned.as_mut().ok_or("缓存写句柄缺失")?.file;
        file.write_all(&bytes).map_err(|e|format!("写缩略图缓存失败：{e}"))?;
        file.flush().map_err(|e|e.to_string())?;file.sync_all().map_err(|e|e.to_string())?;
        let digest=format!("{:x}",Sha256::digest(&bytes));
        archive::verify_handle(file,&digest,bytes.len() as u64,None)?;
        secure_fs::readonly_handle(file)?;file.sync_all().map_err(|e|e.to_string())?;
        let identity=secure_fs::snapshot(file)?.identity;let now=storage::now_ms();
        let record=ThumbnailRecord {entity_id:entity.id.clone(),rel,source_digest:entity.digest.clone(),png_digest:digest,identity,size:bytes.len() as u64,created:now,used:now};
        let mut inner=lock(&shared.inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.save_thumbnail(&record)?;
        drop(owned.take()); // 对外可查前关闭私有 writer。
        Ok::<_,String>(())
    })();
    if let Err(error)=written {
        let cleanup=owned.as_ref().map(|o|secure_fs::remove_handle(&o.file)).transpose();drop(owned);
        return match cleanup {Ok(_)=>Err(error),Err(cleanup)=>Err(format!("{error}；移除未提交的自有缓存失败：{cleanup}"))};
    }
    Ok(Some(uri(&bytes)))
}
