import { emptyConfig, localDate, type ArchiveItem, type ArchiveDetail, type ConfigBundle, type CourseView, type DirectoryEntry, type DirectoryView, type DriveInfo, type LibraryQuery, type LibraryPage, type CopyPlan, type CopyJob } from './vault-types'

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value))
export const DEMO_ROOT = '演示内存（刷新重置，不是磁盘目录）'
export const DEMO_PATH = 'P:\\'
const volumeId = 'demo-volume-v1'
const deviceId = 'demo-device-v1'

/** DEV-only memory fixtures. No fetch, disk bridge, database or browser storage. */
export function createPreviewFixtures() {
	let config = emptyConfig()
	config.timetable.subjects = [
		{id:'demo-chinese',name:'语文',color:'#cfa864'}, {id:'demo-math',name:'数学',color:'#699bca'},
		{id:'demo-english',name:'英语',color:'#86ad8a'}, {id:'demo-physics',name:'物理',color:'#b88bbd'},
	]
	const now = Date.now(), day = localDate()
	const start = new Date(); start.setHours(8,0,0,0)
	const counts = [0,1,24,6,0,40,3,12,0]
	const courses: CourseView[] = counts.map((count,index) => {
		const subject = config.timetable.subjects[index % 4]
		const startUnixMs = start.getTime() + index * 50 * 60_000
		const course: CourseView = {id:`demo-course-${index}`,date:day,slotId:`demo-slot-${index}`,subjectId:subject.id,subjectName:subject.name,label:`第${index+1}节 · 演示`,startUnixMs,endUnixMs:startUnixMs+40*60_000,enabled:true,items:[]}
		config.timetable.slots.push({id:course.slotId!,label:`第${index+1}节`,start:new Date(startUnixMs).toTimeString().slice(0,5),end:new Date(course.endUnixMs).toTimeString().slice(0,5),isClass:true})
		config.timetable.lessons.push({weekday:((new Date().getDay()+6)%7)+1,slotId:course.slotId!,subjectId:subject.id,enabled:true})
		course.items=Array.from({length:count},(_,fileIndex)=>{
			const extension=['pdf','pptx','png','docx'][fileIndex%4]
			const failed=index===2&&fileIndex===3, pending=index===5&&fileIndex===2
			return {id:`demo-file-${index}-${fileIndex}`,displayName:`【演示】${subject.name}${fileIndex%3===0?'很长的课程复习资料名称用于检查两行省略和触屏滚动':'课堂讲义'}-${fileIndex+1}.${extension}`,extension,size:1024*(fileIndex+1)*37,detectedUnixMs:startUnixMs+fileIndex*1000,state:failed?'failed':pending?'pending':'available',taskId:`demo-task-${index}-${fileIndex}`,error:failed?'演示错误：这是用于检查失败覆盖层的虚构资料，没有访问任何原文件。':null,courseId:course.id,subjectName:subject.name} satisfies ArchiveItem
		})
		return course
	})
	config.favorites=Array.from({length:18},(_,i)=>({id:`demo-pin-${i}`,deviceId,volumeId,relativePath:`演示文件夹${i+1}`,alias:`演示固定夹 ${i+1}`,position:i}))
	const drives: DriveInfo[]=[{root:DEMO_PATH,name:'演示磁盘 P:（内存）',identity:'demo-drive',deviceId,volumeId,identityReliable:true,isUsb:false,canEject:false}]
	const directories=new Map<string,DirectoryEntry[]>()
	const key=(path:string)=>path.replace(/\\+$/,'').toLowerCase()
	const identity=(path:string)=>`demo-directory:${key(path)}`
	const rootEntries:DirectoryEntry[]=Array.from({length:55},(_,i)=>({name:`演示文件夹${i+1}`,path:`${DEMO_PATH}演示文件夹${i+1}`,identity:identity(`${DEMO_PATH}演示文件夹${i+1}`),extension:'',isDir:true,size:0,modifiedUnixMs:now-i*60_000}))
	rootEntries.push(...Array.from({length:35},(_,i)=>({name:`【演示】目录文件-${i+1}.pdf`,path:`${DEMO_PATH}演示目录文件-${i+1}.pdf`,identity:`demo-entry-${i}`,extension:'pdf',isDir:false,size:2048*(i+1),modifiedUnixMs:now})))
	directories.set(key(DEMO_PATH),rootEntries)
	for(const folder of rootEntries.filter(entry=>entry.isDir))directories.set(key(folder.path),Array.from({length:28},(_,i)=>({name:`【演示】${folder.name}-文档${i+1}.pdf`,path:`${folder.path}\\演示文档${i+1}.pdf`,identity:`demo-entry:${folder.path}:${i}`,extension:'pdf',isDir:false,size:4096*(i+1),modifiedUnixMs:now})))
	const deleted=new Set<string>(),dismissed=new Set<string>()
	const all=()=>courses.flatMap(course=>course.items).filter(item=>!deleted.has(item.id)&&!dismissed.has(item.id))
	const find=(id:string)=>{const item=courses.flatMap(course=>course.items).find(item=>item.id===id);if(!item)throw new Error('没有这份演示资料');return item}
	function browse(path:string):DirectoryView{const entries=directories.get(key(path));if(!entries)throw new Error('仅能浏览预置的 P: 演示目录，不能访问电脑或手机文件');return clone({path:key(path)===key(DEMO_PATH)?DEMO_PATH:path.replace(/\\+$/,''),identity:identity(path),volumeId,writable:true,entries})}
	function detail(id:string):ArchiveDetail{const item=find(id);const course=courses.find(course=>course.id===item.courseId)!;return clone({item,archivePath:`演示内存/归档/${item.displayName}`,sourcePaths:['演示内存/来源（不存在真实文件）'],uses:[{date:day,courseId:course.id,subjectName:course.subjectName,detectedUnixMs:item.detectedUnixMs}],deleted:deleted.has(id)})}
	function library(query:LibraryQuery):LibraryPage{
		if(query.regex)throw new Error('演示模式不运行正则引擎，请关闭正则测试关键词搜索；受限Rust正则须在Native验证')
		let items=all().filter(item=>item.displayName.toLowerCase().includes(query.name.toLowerCase())&&(!query.subjects.length||query.subjects.includes(item.courseId?courses.find(course=>course.id===item.courseId)?.subjectId??'':''))&&(!query.extensions.length||query.extensions.includes(item.extension)))
		if((query.dateFrom&&query.dateFrom>day)||(query.dateTo&&query.dateTo<day))items=[]
		items.sort((a,b)=>{const order=query.sortBy==='name'?a.displayName.localeCompare(b.displayName,'zh-CN'):query.sortBy==='size'?a.size-b.size:a.detectedUnixMs-b.detectedUnixMs;return query.descending?-order:order})
		const rows=items.map(item=>{const group=query.groupBy==='none'?'':query.groupBy==='subject'?item.subjectName??'未归类':query.groupBy==='course'?courses.find(course=>course.id===item.courseId)?.label??'未归类':query.groupBy==='type'?item.extension:query.groupBy==='month'?day.slice(0,7):day;return{item,groupKey:group,groupLabel:group}})
		return clone({rows:rows.slice(query.offset,query.offset+query.limit),total:rows.length})
	}
	let sequence=0
	const plans=new Map<string,{plan:CopyPlan;path:string;ids:string[]}>(),jobs:CopyJob[]=[]
	function previewCopy(ids:string[],path:string,targetIdentity:string):CopyPlan{
		const directory=browse(path);if(directory.identity!==targetIdentity)throw new Error('演示目标已变化')
		const selected=[...new Set(ids)].map(find);if(selected.some(item=>item.state!=='available'||deleted.has(item.id)))throw new Error('只能拖放可用的演示资料')
		const plan:CopyPlan={id:`demo-plan-${++sequence}`,items:selected.map(item=>({sourceId:item.id,sourceName:item.displayName,targetName:item.displayName,exists:directory.entries.some(entry=>entry.name===item.displayName),existingSize:null}))}
		if(plans.size>=8)plans.delete(plans.keys().next().value!)
		plans.set(plan.id,{plan,path,ids:selected.map(item=>item.id)});return clone(plan)
	}
	function startPlan(id:string,resolutions:{sourceId:string;policy:string}[]):CopyJob{
		const plan=plans.get(id);if(!plan)throw new Error('演示拖放计划已过期');plans.delete(id)
		const entries=directories.get(key(plan.path))!;let skipped=0
		for(const itemId of plan.ids){const item=find(itemId);const policy=resolutions.find(value=>value.sourceId===itemId)?.policy??'keepBoth';let name=item.displayName;const existing=entries.findIndex(entry=>entry.name===name)
			if(existing>=0&&policy==='skip'){skipped++;continue}
			if(existing>=0&&policy==='replace')entries.splice(existing,1)
			if(existing>=0&&policy==='keepBoth'){let n=2;while(entries.some(entry=>entry.name===name))name=item.displayName.replace(/(\.[^.]+)$/,` (${n++})$1`)}
			entries.push({name,path:`${plan.path.replace(/\\+$/,'')}\\${name}`,identity:`demo-export-${++sequence}`,extension:item.extension,isDir:false,size:item.size,modifiedUnixMs:Date.now()})
		}
		const job:CopyJob={id:`demo-job-${++sequence}`,state:'completed',total:plan.ids.length,processed:plan.ids.length,copied:plan.ids.length-skipped,skipped,error:null};jobs.unshift(job);jobs.splice(16);return clone(job)
	}
	return {
		config:()=>clone(config),
		saveConfig:(value:ConfigBundle)=>{config=clone(value);return clone(config)},
		today:()=>clone(all()), courses:()=>clone(courses.filter(course=>course.enabled).map(course=>({...course,items:course.items.filter(item=>!deleted.has(item.id)&&!dismissed.has(item.id))}))),
		drives:()=>clone(drives),browse,detail,library,
		favoriteSave:(folder:ConfigBundle['favorites'][number])=>{if(folder.deviceId!==deviceId||folder.volumeId!==volumeId)throw new Error('只能固定演示设备目录');browse(DEMO_PATH+folder.relativePath.replaceAll('/','\\'));config.favorites=config.favorites.filter(item=>item.id!==folder.id);config.favorites.push(clone(folder))},
		favoriteRemove:(id:string)=>{config.favorites=config.favorites.filter(item=>item.id!==id)},
		changeCourse:(id:string,subjectId:string)=>{const course=courses.find(item=>item.id===id);if(!course)throw new Error('演示课次不存在');if(!subjectId){course.enabled=false;return}const subject=config.timetable.subjects.find(item=>item.id===subjectId);if(!subject)throw new Error('请选择演示科目');course.subjectId=subjectId;course.subjectName=subject.name;for(const item of course.items)item.subjectName=subject.name},
		retry:(task:string)=>{const item=all().find(item=>item.taskId===task);if(item){item.state='available';item.error=null}},
		dismiss:(task:string)=>{const item=all().find(item=>item.taskId===task);if(item)dismissed.add(item.id)},
		trash:(id:string)=>{find(id);deleted.add(id)},restore:(id:string)=>{deleted.delete(id)},purge:(id:string)=>{deleted.delete(id);dismissed.add(id)},trashList:()=>[...deleted].map(detail),
		previewCopy,startPlan,jobs:()=>clone(jobs),
	}
}
