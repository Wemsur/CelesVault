# CelesVault 原组件接入契约

本闭包从 `C:/Users/iexam/github-project/CelestialLauncher` 当前工作树抽取，已核对 HEAD 为 `3a384015be700fc3eba3351dd4a3339bc3a9d31d`。它是原组件提取，不是重绘。文件来源、原文件/提取文件 SHA-256、修改项目见 `origin-manifest.json`。没有执行本地构建、类型检查或测试；主线程允许的前端浏览器观察记录见项目 `docs/verification.md`。

## 入口与别名

```ts
import {
	NavButton, WindowControls, Card, Checkbox, Toggle,
	ButtonFrame, Button, IconButton,
	Input, InputFrame, InputClearButton,
	DropdownSelect, Combobox, ContextMenu, TeleportOverflowMenu,
	NewModal, TabbedModal, ConfirmModal, LoadingIndicator,
	provideModalBehavior, useModalStack,
} from '@modrinth/ui'
import type { Tab, ButtonMenuOption, ComboboxOption } from '@modrinth/ui'
import {
	CelestialLogo, HomeIcon, LibraryIcon, SettingsIcon, SearchIcon,
	XIcon, FolderIcon, ArchiveIcon, ServerIcon, PlugIcon, UnplugIcon,
	SquareSplitHorizontalIcon, PinIcon, PlusIcon, ChevronDownIcon,
	CheckIcon, InfoIcon,
} from '@modrinth/assets'
import '@/vendor/celestial/styles/index.scss'
```

Vite 别名按精确包名匹配，避免把包名替换成 `index.ts` 后再拼 `/src/...`。

| 匹配 | 目标 |
|---|---|
| `/^@modrinth\/ui$/` | `src/vendor/celestial/ui/index.ts` |
| `/^@modrinth\/assets$/` | `src/vendor/celestial/assets/index.ts` |
| `#ui` | `src/vendor/celestial/ui` |
| `@` | `src` |

**没有额外必需别名。** 内部依赖均为相对叶子导入，仅 SVG 使用精简 assets 入口；WindowControls 只从 `@/platform` 取得宿主能力。不要将上游 packages/ui、assets 或 utils 的完整 barrel 接回来。`#ui/components/base/buttons`、`#ui/components/base/inputs` 是本地精简出口；如要直引文件可使用 `@/vendor/celestial/ui/...`。

SVG 导入均保留 `?component`，由 `vite-svg-loader` 处理。宿主关闭 SVGO（`svgLoader({svgo:false})`），保留原viewBox和主题变量，避免CSS缩小后将原坐标裁掉。宿主类型声明可使用 `/// <reference types="vite-svg-loader" />`，不需要再生成另一套图标。

## 依赖版本

| 依赖 | 原工作树版本/范围 | 本闭包接入 |
|---|---|---|
| Vue | lock `3.5.27`，manifest `^3.5.13` | Vue 3.5；组件使用 defineModel、useId、泛型 SFC |
| Vue Router | manifest `^4.6.0` | Router 4；NavButton 必须位于安装 Router 的应用内 |
| `@vueuse/core` | lock `11.3.0`，manifest `^11.1.0` | `11.3.0`，使用 useElementSize/onClickOutside；不需要 components 包 |
| `floating-vue` | `5.2.2` | `5.2.2`；注册 v-tooltip 并导入其 CSS |
| `@floating-ui/dom` | FloatingVue lock 传递依赖 `1.1.1` | 闭包不直接导入；宿主不添加直接依赖 |
| `@floating-ui/core` | lock `1.7.4` | 仅传递依赖，无需直接依赖 |
| `overlayscrollbars` | `2.15.1` | `2.15.1`；实际 Combobox 内部使用且自行导入其 CSS |
| `marked` | 上游 UI 不直接使用；其他 lock 中有 `7.0.4` | 新安全适配使用宿主已装 `15.0.12`，内置 TS 类型 |
| `dompurify` | lock `3.3.1`，manifest `^3.1.7` | 宿主报告已装 `3.4.16`；不是将该版本冒称为上游版本 |
| Tailwind CSS | manifest `^3.4.4` | Tailwind 3.4，**不是 Tailwind 4** |
| Sass | manifest `^1.74.1` | 用于原 SCSS 和 Vue scoped SCSS |
| `vite-svg-loader` | manifest `^5.1.0` | `5.1.0` |

Vite、Vue 插件、TypeScript、PostCSS/Autoprefixer 由宿主已有轻量工程负责。Tailwind preset 去掉了闭包不使用的 container-query 插件；保留原 token 映射和 `preflight: false`。不需要原 monorepo、Nuxt、业务客户端、编辑器、3D 库或翻译依赖。

## 样式、字体与浮层初始化

**正确样式入口是 `src/vendor/celestial/styles/index.scss`，不是原 assets/omorphia.scss。** 该入口包含 Tailwind directives、原 UI utilities、变量、normalize、classes、defaults（再导入 reset）、accessibility，以及必要 Launcher 全局规则；所有样式导入为本地文件。`tailwind-utilities.scss` 是原 CSS 字节拷贝并改扩展名，以便 Sass 内联，而不是浏览器执行晚到的 CSS @import。

```ts
// tailwind.config.ts
import preset from './src/vendor/celestial/styles/tailwind-preset'

export default {
	presets: [preset],
	content: ['./index.html', './src/**/*.{vue,js,ts,jsx,tsx}'],
}
```

全量 src content glob 必须覆盖 vendor 里的 SFC 和 TS（按钮/菜单的 class 字符串定义在 TS）。不要再重复导入另一份带 Tailwind directives 的全局入口。

宿主在首帧 HTML 上设置：

```html
<html lang="zh-CN" class="customdark-mode" style="--brand-hue: 38">
	<body>
		<div id="app"></div>
		<div id="teleports"></div>
	</body>
</html>
```

`#teleports` 必须在挂载 Vue **之前**存在；Combobox teleport 到这里，Modal 和菜单 teleport 到 body。主题须在 html 而不是仅在 #app 上，以覆盖全部浮层。不要给 #teleports 设置 transform/filter/布局隔离以改变 fixed 定位。

在 `app.mount()` 之前注册原工具提示：

```ts
import 'floating-vue/dist/style.css'
import FloatingVue from 'floating-vue'

app.use(FloatingVue, { themes: { tooltip: { html: false } } })
```

这不是语言 Provider；工具提示使用纯字符串。FloatingVue CSS 在 vendor 入口前导入，保留原 classes.scss 的工具提示覆盖。原菜单本身是原 anchored-teleport，不依赖 FloatingVue dropdown 组件。

原全局样式保留 fixed body/overflow hidden、链接样式、输入外观、滚动条和 `.light-sense` 顶底光线。原 UI utilities 保留浮层展开动画、菜单 tone/trailing-action 规则与 heading utilities。原 App 的背景图片、业务布局、宣传颜色/远程字体规则未迁入。主布局负责内部可滚动容器。

**不含任何联网 Inter 导入。** 宿主维护 `public/fonts/inter/` 和 `src/styles/fonts.css`，导入其离线 Inter 3.19 字体；字体许可由宿主记录。原 defaults 仍采用 `Inter` family 与原默认字重/回退，未改成替代字体。

## 必要平台与可选 modal Provider

WindowControls 要求宿主 `@/platform` 导出的 `usePlatform()` 已可用。平台初始化由宿主原平台模块负责，vendor 不创建 Native/Preview 平台 Provider，也不静默选择浏览器 fallback。

Modal 行为 Provider **可选**，原 NewModal 用 null fallback；未提供时默认有 blur 且没有平台副作用。需要行为控制时，在根组件 setup **同步**调用原叶子 Provider：

```ts
import { ref } from 'vue'
import { provideModalBehavior } from '@modrinth/ui'

provideModalBehavior({
	noblur: ref(false),
	// 可选：onShow / onHide，分别在第一个 modal 打开/最后一个关闭时调用
})
```

`createContext`、`injectModalBehavior`、`ModalBehavior` 亦有精简出口。`useModalStack()` 自身管理堆栈，不需要额外安装。没有必须初始化的业务、账号、通知或翻译 Provider。

## NavButton / WindowControls

NavButton 保留原 RouterLink/函数导航双模式：

```vue
<NavButton to="/"><HomeIcon /></NavButton>
<NavButton :to="() => settingsModal?.show()"><SettingsIcon /></NavButton>
```

`to: string | (() => void)` 必填；`isPrimary`、`isSubpage` 接收当前 RouteLocationNormalizedLoaded 并返回 boolean，`disabled=false`。`highlightOverride` 是上游保留但未被实现使用的 prop。字符串模式的 disabled 仅有原 class 语义，不应将它当安全动作权限边界。

WindowControls 保留原 body Teleport、右上角 48px 高度、三个原 IconButton 及扩展命中区域：

```vue
<WindowControls
	:show-controls="true"
	@width-change="controlsWidth = $event"
	@window-error="handleWindowError"
/>
```

`showControls?: boolean` 默认 true，宿主如启用系统原生窗口装饰可设置 false。宽度会自动写入 html 的 `--window-controls-width`，同时 emit `width-change(number)`，不需要接事件才能留白；卸载时移除变量。顶栏原契约是 `padding-right: var(--window-controls-width, 0px)`。只挂载一个实例。

使用的契约：

```ts
const platform = usePlatform()
platform.window.minimize()
platform.window.toggleMaximize()
platform.window.hide() // 关闭按钮隐藏到托盘，不退出程序
platform.window.getState(): Promise<{ maximized: boolean; visible: boolean }>
platform.window.onStateChanged(fn): Promise<() => void>
```

状态决定 MaximizeIcon/RestoreIcon；监听卸载会释放，异步订阅迟到也会释放。失败发出 window-error，不假称动作成功。原窗口设置/保存/设置 decorations 移到宿主负责。浏览器平台必须显式标明仅模拟窗口状态；这些按钮的浏览器动作不代表 Native 窗口/托盘已验证。

## 基础控件

| 控件 | 原接口 |
|---|---|
| Button | `type='base' \| 'colored' \| 'colored-text' \| 'outlined' \| 'quiet'`；不是 standard/primary。`color='brand' \| 'red' \| 'orange' \| 'green' \| 'blue' \| 'purple' \| 'medal_promotion'`。`size='xs' \| 'sm' \| 'md' \| 'lg' \| 'xl'`，默认 md；disabled/loading/iconOnly/nativeType；loading 禁用按钮并标 aria-busy，不自行画 spinner |
| IconButton | 必需 `label:string`，上述视觉 props，默认 circular=true；尺寸和 SVG 均保留原定义 |
| ButtonFrame | 必需 `as:string \| Component`；原按钮内部框架，一般直接用 Button/IconButton |
| Input | 默认 `v-model`（modelValue/update:modelValue），值为 string/number/undefined；type、placeholder、icon、clearable、clearLabel、disabled、readonly、error、clamp/min/max/step、inputAttrs、inputClass、wrapperClass；appearance='surface'/'button'/'transparent'，size='small'/'standard'/'medium'/'large' |
| Checkbox | 默认 `v-model` boolean，update:modelValue 第二参数可为 MouseEvent；label/description/disabled/indeterminate/labelClass |
| Toggle | 默认 `v-model` boolean，id/disabled/small；必须由宿主提供可访问标签（aria-label 或 aria-labelledby） |
| Card | collapsible/defaultCollapsed，header/default slot；noAutoBody 为原兼容 prop，当前实现没有读取它 |

```vue
<Button type="colored" color="brand" size="lg">保存</Button>
<Button type="outlined" @click="cancel">取消</Button>
<Input v-model="query" :icon="SearchIcon" placeholder="搜索库文件" clearable />
<Input v-model="query">
	<template #prefix>库：</template>
</Input>
<Checkbox v-model="enabled" label="启用监测" />
<Toggle v-model="enabled" aria-label="启用监测" />
```

Input 的 `#leading` 替代 icon，`#prefix` 是输入文字左侧前缀（在图标后），`#trailing`/兼容 `#right` 在右侧，`focus()`/`select()` 可通过 ref 调用。框架 class/style 作用于 wrapper，其他 attrs 传原 input。

## DropdownSelect / Combobox / 菜单

DropdownSelect **没有 display-names 数组/对象 prop**；原接口为 `displayName?: (option) => string`。

```vue
<DropdownSelect
	v-model="direction"
	name="home-direction"
	:options="['horizontal', 'vertical']"
	:display-name="(id) => ({ horizontal: '左右', vertical: '上下' })[id]"
/>
```

原 props：options 和 name 必填，modelValue/defaultValue/placeholder/renderUp/disabled/displayName/maxVisibleOptions。默认 slot 接 `{selected}`，selected 是计算后的显示文字；change emit 为 `{option,index}`。它不是 teleport，原上下展开动画保留；在 overflow 裁切容器内优先使用原 Combobox。原值选择用了 `||`，避免把 0/空串当有效选项；保留该上游行为，未扩张功能。

Combobox 使用泛型 `ComboboxOption<T>`：`{value,label,subLabel?,icon?,disabled?,type?:'button'|'link'|'divider',href?,target?,action?,searchTerms?,class?}`，或独立 `{type:'divider'}`。普通 `v-model` 选 value；searchable/clearable/listbox（默认 true）、searchPlaceholder、syncWithSelection、showSearchIcon、maxHeight、triggerType/Size/Interaction 等保留。`@search-input` 接查询 string，`@select` 接 option。它在 mounted 后使用 #teleports，保留原 OverlayScrollbars/定位/键盘交互。

```ts
const options: ButtonMenuOption[] = [
	{ id: 'open', label: '打开', icon: FolderIcon, action: () => openSelected() },
	{ type: 'divider' },
	{ type: 'heading', label: '显示' },
	{ type: 'submenu', id: 'view', label: '视图', options: [
		{ id: 'list', label: '列表', action: () => setView('list') },
	] },
]
```

`<TeleportOverflowMenu label="更多操作" :options="options"><PlusIcon /></TeleportOverflowMenu>`，原 expose `open(position?:'first'|'last',focus?:boolean)`/`close(restoreFocus?:boolean)`；默认 iconOnly=true/circular=true/placement=bottom-end/distance=8/hoverable=false。

`<ContextMenu ref="menu" label="文件操作" />`，宿主 `@contextmenu.prevent="menu?.open($event, options)"`；原 expose `open(event:MouseEvent, options:ButtonMenuOption[])`/`close()`。长按可由业务层构造带 clientX/clientY 的 MouseEvent 后调用，同一原菜单，不在 vendor 加长按业务。菜单 action 在原 ButtonMenuItem 内执行；select 事件是通知，**不要再重复执行 option.action**。

## NewModal

没有 `v-model:open`，通过模板 ref 的 `show(event?:MouseEvent)`、`hide():boolean` 开关；hide 返回 false 表示 disableClose 或 beforeHide 拦截。另 expose checkScrollState()/scrollToBottom(behavior='smooth')。

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { NewModal, Button } from '@modrinth/ui'
const modal = ref<InstanceType<typeof NewModal> | null>(null)
</script>
<template>
	<Button @click="modal?.show($event)">打开</Button>
	<NewModal ref="modal" header="详情" width="600px" scrollable>
		<p>内容</p>
		<template #actions><Button @click="modal?.hide()">关闭</Button></template>
	</NewModal>
</template>
```

props：noblur（默认用可选 Provider）、closable=true、fade='standard'/'warning'/'danger'（danger 为原兼容 prop）、closeOnEsc=true、closeOnClickOutside=true、header/hideHeader、mergeHeader、scrollable=false、maxContentHeight='70vh'、noPadding=false、width/maxWidth（默认 fit-content/60rem）、disableClose=false、actionsDivider=false。

回调 props `onShow`、`onHide`、`onAfterHide`；**beforeHide 是同步 `() => boolean`，不能返回 Promise**。onHide 在开始退出时触发，onAfterHide 在原 300ms 退出后触发。不要用条件渲染立即卸载来代替 hide，否则原退出动画会被跳过。slots：title/header-actions/default/actions。原 warnOnClose prop 并未实现确认逻辑，业务用 beforeHide + ConfirmModal。

## TabbedModal

```ts
const tabs: Tab[] = [
	{ name: '存储归档', category: '设置', icon: FolderIcon },
	{ name: '界面行为', category: '设置', icon: SettingsIcon },
	{ name: '关于与许可', category: '信息', icon: InfoIcon },
]
const settings = ref<InstanceType<typeof TabbedModal> | null>(null)
```

```vue
<TabbedModal
	ref="settings"
	header="设置"
	width="900px"
	:tabs="tabs"
	:before-hide="tryCloseSettings"
	:before-tab-change="tryChangeSettingsTab"
	:floating-action-bar-shown="dirty"
>
	<template #content="{ index, tab }">
		<!-- 用 index 或稳定业务 ID 选择宿主设置内容；tab.name 是普通中文文字 -->
		<slot :index="index" :tab="tab" />
	</template>
	<template #floating-action-bar>
		<!-- dirty 的显示/消失与业务保存取消，由宿主管理 -->
	</template>
</TabbedModal>
```

`Tab = {name:string;category?:string;icon:Component;content?:Component;href?:string;badge?:string;shown?:boolean}`。category 按相邻相同字符串分组；不是翻译描述符或显示名对象。原 content Component 支持 Suspense fallback。

Expose `show(event?:MouseEvent)`、`hide():boolean`、`selectedTab`（Vue expose ref 会自动解包）、`setTab(index:number)`。索引针对 shown!==false 的 visibleTabs；隐藏项不能按原数组位置 setTab。show 不自动重置页签。header/maxWidth/width/closable/disableClose/onShow/onHide/onAfterHide/beforeHide 透传 NewModal；beforeTabChange(fromIndex,toIndex) 为同步 boolean。hideTabSelection=false/floatingActionBarShown=false。slots：title/sidebar-header/footer/content({tab,index})/floating-action-bar。保留原双列布局、sidebar/content alpha-mask 滚动渐隐与浮动动作条底部留白。

## ConfirmModal

```vue
<ConfirmModal
	ref="confirmation"
	title="放弃未保存的设置？"
	description="当前修改尚未保存。"
	proceed-label="放弃修改"
	:danger="true"
	:markdown="false"
	@proceed="discardSettings"
/>
```

通过 `confirmation?.show()`/`confirmation?.hide()` 开关（没有 open model）。title 必填；description 可选；proceedLabel 默认“继续”、proceedIcon 默认 TrashIcon、danger=true、markdown=true、noblur=false、onHide 回调。hasToType=true 时须输入 confirmationText，不区分大小写；默认 false。proceed 先隐藏原 NewModal、清除已输入确认文本，再 emit proceed。保留原 DOM/按钮/输入布局。

Markdown 接口改为 `marked.parse(..., {async:false,gfm:true,breaks:true})` → DOMPurify 的 HTML profile，禁止脚本/样式、iframe/object/embed、图片/音视频/表单与相关资源属性。不导入原会代理图片的共享 utils。不要二次拼入未清洗的 HTML。仅需普通确认文案时设置 markdown=false。

## 原图标出口与缺失名称

原出口已存在并直接复用：HomeIcon、LibraryIcon、SettingsIcon、SearchIcon、XIcon、FolderIcon、ArchiveIcon、SquareSplitHorizontalIcon、PlugIcon、UnplugIcon、PinIcon、PlusIcon、ChevronDownIcon、CheckIcon、InfoIcon、ServerIcon，以及控件内部用的按钮/菜单/窗口图标。额外保留常用原 FileIcon/FileTextIcon/FileImageIcon/FileArchiveIcon/CopyIcon/RefreshCwIcon/DownloadIcon/UploadIcon/CalendarIcon/UndoIcon/GridIcon/ListIcon。

上游 assets **没有** HardDriveIcon/DriveIcon/UsbIcon/RowsIcon/ColumnsIcon 出口，未编造同名别名：

- `ServerIcon` 的原 `icons/server.svg` 实际为 Lucide hard-drive 图形，适合盘入口。
- 外接设备用原 `PlugIcon`；安全移除占位可用原 `UnplugIcon`。
- 左右方向用原 `SquareSplitHorizontalIcon`；上下可在消费端对**同一原 SVG**加 rotate-90，不重绘图形。
- `CelestialLogo` 是原 `apps/app-frontend/src/assets/celestial.svg` 的精简出口，保留品牌色 CSS 变量，不是 Modrinth 标识。

## 尚未验证

本次只有源阅读、复制、接口适配和文件摘要记录。未启动服务，未执行 build/check/test/lint/tsc/cargo，未验证屏幕布局、WebView2、触摸、Native 窗口或真实设备操作。上游原控件行为不等于这些验收已通过；宿主运行并反馈后继续按同一原组件修正。
