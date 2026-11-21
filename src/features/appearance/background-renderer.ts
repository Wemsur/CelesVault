import { ShaderProgram, setupQuad } from '@/vendor/margin-background/program'
import { GateFlow } from '@/vendor/margin-background/gate'
import type { BackgroundId, BackgroundParameters } from '@/platform/appearance-types'

/** One current renderer. Hidden windows and reduced motion do not advance time. */
export class BackgroundRenderer {
	private gl: WebGLRenderingContext | null = null
	private program: ShaderProgram | null = null
	private buffer: WebGLBuffer | null = null
	private gate: GateFlow | null = null
	private observer: ResizeObserver
	private motion = matchMedia('(prefers-reduced-motion: reduce)')
	private frame = 0
	private last = 0
	private elapsed = 12
	private width = 1
	private height = 1
	private dead = false
	private soft = false
	private dark = true
	private id: BackgroundId = 'aurora'
	private params: BackgroundParameters | null = null
	constructor(private canvas: HTMLCanvasElement, private host: HTMLElement, private report: (message: string) => void) {
		this.observer = new ResizeObserver(([entry]) => { this.width=entry.contentRect.width; this.height=entry.contentRect.height; this.resize() })
		this.observer.observe(host)
		document.addEventListener('visibilitychange',this.visibility)
		this.motion.addEventListener('change',this.visibility)
		canvas.addEventListener('webglcontextlost',this.lost)
		canvas.addEventListener('webglcontextrestored',this.restored)
		window.addEventListener('pointerdown',this.pointer,{passive:true})
	}
	private pointer = (event: PointerEvent) => {
		if (!event.isPrimary || !this.params?.shockwave || this.motion.matches || document.hidden) return
		const bounds=this.host.getBoundingClientRect(); this.gate?.explode(event.clientX-bounds.left,event.clientY-bounds.top)
	}
	private lost = (event: Event) => { event.preventDefault(); this.stop(); this.report('背景渲染已中断，等待图形上下文恢复') }
	private restored = () => { this.gl=null;this.program=null;this.buffer=null;const p=this.params;if(p)this.apply(this.id,p,this.dark) }
	private visibility = () => { this.stop(); if (!document.hidden) { this.draw(0);this.loop() } }
	apply(id: BackgroundId, params: BackgroundParameters, dark: boolean) {
		if(this.dead)return
		this.stop()
		const rebuild=this.id!==id || !this.program || this.gl?.isContextLost()
		const gateChanged=this.id!==id || this.params?.density!==params.density || this.params?.quality!==params.quality
		this.id=id;this.params={...params};this.dark=dark
		try {
			if(!this.gl){
				this.gl=this.canvas.getContext('webgl',{antialias:false,alpha:false,powerPreference:'low-power'})
				if(!this.gl)throw new Error('当前环境不支持WebGL，请选择图片或关闭背景')
				this.buffer=setupQuad(this.gl)
				const extension=this.gl.getExtension('WEBGL_debug_renderer_info')
				this.soft=!!extension && /swiftshader|llvmpipe|software/i.test(this.gl.getParameter(extension.UNMASKED_RENDERER_WEBGL))
			}
			if(rebuild){if(this.program)this.gl.deleteProgram(this.program.prog);this.program=null;this.program=new ShaderProgram(this.gl,id)}
			if(gateChanged){this.gate?.canvas.remove();this.gate=id==='gate'?new GateFlow(Math.min(1.5,params.quality*2),params.density):null;if(this.gate){this.gate.canvas.style.cssText='position:absolute;inset:0;width:100%;height:100%;pointer-events:none';this.host.append(this.gate.canvas)}}
			this.report(this.soft?'软件渲染：背景显示静态画面':'')
			this.resize();this.loop()
		}catch(error){this.report(String(error));this.stop()}
	}
	private resize(){
		if(!this.gl||!this.params)return
		const scale=Math.min(this.params.quality,1280/Math.max(1,this.width),Math.sqrt(1_000_000/Math.max(1,this.width*this.height)))
		const w=Math.max(1,Math.round(this.width*scale)),h=Math.max(1,Math.round(this.height*scale))
		if(this.canvas.width!==w)this.canvas.width=w;if(this.canvas.height!==h)this.canvas.height=h
		this.gl.viewport(0,0,w,h);this.gate?.resize(this.width,this.height);this.draw(0)
	}
	private draw(step:number){
		if(!this.gl||!this.program||!this.params||this.gl.isContextLost())return
		this.program.bind();this.program.draw(this.canvas.width,this.canvas.height,this.elapsed*this.params.speed,this.dark?'dark':'light',this.params)
		this.gate?.render(step*this.params.speed,this.dark?1:0,step>0)
	}
	private loop(){
		if(this.frame||this.dead||document.hidden||this.motion.matches||this.soft||!this.params||this.params.speed===0)return
		this.frame=requestAnimationFrame(this.tick)
	}
	private tick=(time:number)=>{
		this.frame=0;if(this.dead||document.hidden||!this.params)return
		if(!this.last){this.last=time;this.loop();return}
		const ms=time-this.last
		if(ms>=1000/this.params.fps-1){this.last=time;const delta=Math.min(ms,100)/1000;this.elapsed+=delta;this.draw(delta*60)}
		this.loop()
	}
	private stop(){cancelAnimationFrame(this.frame);this.frame=0;this.last=0}
	dispose(){
		this.dead=true;this.stop();this.observer.disconnect();this.motion.removeEventListener('change',this.visibility);document.removeEventListener('visibilitychange',this.visibility)
		window.removeEventListener('pointerdown',this.pointer);this.canvas.removeEventListener('webglcontextlost',this.lost);this.canvas.removeEventListener('webglcontextrestored',this.restored)
		this.gate?.canvas.remove();if(this.program)this.gl?.deleteProgram(this.program.prog);if(this.buffer)this.gl?.deleteBuffer(this.buffer)
		this.program=null;this.gl=null
	}
}
