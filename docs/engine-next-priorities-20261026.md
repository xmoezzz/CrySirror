> 2026-10-26 更正：本文保留部分历史审查记录；RFVP/BGI/RealLive 托管代码已删除，旧接入完成结论撤回。当前规则以 AGENTS.md 和 NATIVE_ADAPTERS.md 为准。

# CrySirror：下一阶段的具体优先级（2026-10-26）

这份审查以当前源码及最近的实际验证记录为依据。结论是：**共享图片、物理、音频、排版的基础已经相当多；主要缺口转向原生调用接线、完整状态恢复、内容生产流程和持续验证。** 不能再把“还没有物理／音频／文字系统”作为下一步计划，也不能把一个原生 framebuffer 上传成功当作共享服务提取完成。

最初的只读审查没有重跑整套测试；下文路线表是接线前的审查快照。随后授权的音频／视频实现和本 agent 实际验证，单列在下面。RealLive 的适配器、存档、输入和绘图桥由其他并行任务推进，应以各自最新集成结果为准，不应把本快照里的缺口继续当作尚未实现的事实。

## 随后完成的音频／视频接线

RealLive 的 `Sound` 已移除 Kira／设备管理，原生 PCM、通道、淡化、等待和循环策略通过公共 `AudioRequest` 进入共享 mixer。新增 `SetLooped` 可以在当前位置切换循环并显式选择循环原点；原生启用循环传 `Some(0)`，公共调用传 `None` 可保留自定义原点。`Sound::new(false)` 仍产生音频命令，只有适配器的显式 silent 配置丢弃它们。

电影增量解码已放入 `crysirror-video::mpeg`：只接受 reader、回调和取消信号，不持 VM／文件路径。RealLive 保留播放时钟、3000ms 音频等待和循环策略，四帧队列可取消。后台读／解码／panic 错误通过原生错误日志进入 strict host check；不会直接当 EOF。原生电影音轨仍按原策略累积为有上限的完整 PCM，超过 32Mi 个 i16 sample 会报错，不能据此称为长音频流式播放。

**编译：** `crysirror-video` 和 `reallive-original` 已实际 check 通过。**行为：** 原生音频 6 项、共享 channel/mixer 15 项、原生 PCM→公共 mixer 1 项；共享 MPEG 普通测试 4 项，电影 worker 2 项，均通过。显式执行的只读本地影片前缀对照另有 1 项通过：12 帧 RGBA／PTS 和 2 个 PCM／PTS 块与固定上游一致。普通测试里保留的 ignored 外部用例没有混入普通通过数。

这仅证明这些服务链和对照样本，不证明完整影片、音频设备或整部游戏。原 MPEG 解码器内部重同步和已记录的最后一帧局部解码问题仍存在。逐项文件、命令、日志和限制见 [当前约束](NATIVE_ADAPTERS.md)。

## 先纠正旧路线图的几项状态

| ADR 0041 的旧表述 | 当前应如何理解 | 可核对的证据 |
| --- | --- | --- |
| triggers/contact events 缺失，joints 缺失 | 已有持久 Rapier 0.22 后端、sensor/contact/force/sleep 事件及 fixed/spherical/revolute/prismatic 关节；脚本可通过 `Physics.events()` 取事件。布料、流体、软体及物理编辑工具仍缺。 | `crates/crysirror-physics/src/`、`crates/crysirror-engine/src/physics_api.rs`；[13 项集成回归](physics-integration-validation.json)、[25 项后端测试及基准](physics-benchmark-results.json) |
| VFS 下一步是 streaming reads | `AssetReader::read_at` 和磁盘随机访问已经存在；早先拿到的 client storage 也能看到后来挂载的 provider。真正缺的是异步调度、取消/背压、压缩条目的分块解码及全进程内存预算。 | `crates/crysirror-core/src/service.rs`、`crates/crysirror-runtime/src/vfs.rs`、`crates/crysirror-resource/src/`；[资源结果](resource-physics-validation.json) |
| 图片能力只到 PNG/HDR/BC | 96 个上游图片 ID 已有具体注册实现／适配，其中 80 ported、16 partial；5 个归档 ID 已接。不能把注册数当作所有变体、动画元数据或游戏均兼容。 | [图片总结果](arc-image-validation.json)、[逐格式限制](arc-image-support.json) |
| 音频尚需 occlusion | 已有 voice pool、优先级抢占、bus/ducking、音乐淡化、循环点、流式标准音频解码、pan、空间衰减及射线估计遮挡低通。仍缺硬实时线程保证、HRTF／房间混响、完整会话快照及原生专有音频接线。 | `crates/crysirror-audio/src/{mixer,channels,decoder}.rs`、`crates/crysirror-engine/src/audio_api.rs`、[ADR 0030](adr/0030-audio-mixer-and-music.md) |
| 文字只是一般字形输出 | 已有 rustybuzz、bidi、换行/CJK tailoring、ruby/竖排、富文本、逐字事件、fallback、CPU SDF 数据及可复用 glyph coverage raster。GPU SDF/3D 文字仍未接；原生消息规则不是 generic rich layout。 | `crates/crysirror-text/src/{layout,glyph,sdf}.rs`、`crates/crysirror-engine/src/text_api.rs`、[ADR 0031](adr/0031-rich-text-shaping-and-sdf.md) |
| 早期记录称 JS 仅离线 authoring | 当前 `JsHost` 实现 init/start/fixed/update/late-update 等 lifecycle hook，App 每帧驱动它们。离线 authoring 的旧检查点不能覆盖现在的实现；调试器／热重载仍没有相应成熟工作流证据。 | `crates/crysirror-script-js/src/lib.rs:229`、`crates/crysirror-engine/src/lifecycle.rs:276` |

最近的图片检查点记录了 workspace **464 passed / 0 failed / 42 ignored**、Python **162 passed**、177 个外部图片输出比较中的 174 个逐字节相等及 3 个明确记录的有损舍入差异。Metal 实测只证明本机 Apple M5 上，自造 SAR→TEYL/PGX/NVSG→共享 Layer 路径的 49,152 像素与 PNG 参考相等；没有据此推导 Windows/Linux、硬件光追或完整商业游戏结果。[原始记录](arc-image-validation.json)

## 接下来按这个顺序做

### 1. 补齐 CrySirror 公共 API，禁止旧引擎托管

RFVP/BGI/RealLive 托管实现已经删除。之前开场画面验证只证明被否决的托管路径能运行，不能作为能力提取验收。后续从其他引擎提取通用算法进入 CrySirror，并通过独立 API/共享状态验证；VM 兼容层只保留原生语义和系统调用映射。缺能力必须补引擎，不能再上传原引擎合成整帧。

### 2. 把 Siglus 的完整消息链真正接上，而不是继续扩壳 API

当前注册的 `siglus` 仍是 `crysirror-siglus-vm` 子集；`CD_TEXT/CD_NAME` 明确拒绝。现代 Text API 已存在，但它不能自动还原 MWND 配置、姓名窗口、page/reveal/wait、ruby、read flags、历史和原始存档里的文字状态。BGI 已有“原生布局／draw plan → 共享 glyph/image 服务”的实例，适合借用机制。

**验收：** 切换到完整 SceneVm 前，用真实 native opcode 自造 fixture 检验姓名、两页文本、ruby、等待边沿、回看与原始存档恢复；将 glyph/draw trace 和必要的像素差异与固定上游对照。尚未实现的窗口／效果不可静默降级。证据入口：`docs/NATIVE_ADAPTERS.md`、`docs/siglus-service-coverage.json`、`crates/crysirror-bgi/src/frame_bridge.rs`。

### 3. 收拢“游戏能保存”的完整边界和 schema 迁移

Runtime 已有 staged restore 和 schema/identity 校验；物理逻辑状态可重建。但 App 级 pooled 音频、文字 reveal 配置、binding/localization profile 不是全部自动进入 Runtime snapshot，audio 会重启，Rapier warm-start 不保存；BGI 明确拒绝 Runtime snapshot，RealLive 原生存档只在自己的 savepoint 恢复。`crysirror-core/src/save.rs` 当前 schema 不同就拒绝，没有通用迁移注册表。

**验收：** 明确一份项目级 checkpoint 所有权清单；为一个已发布 schema 写显式 migration 和失败回滚用例；同时测 paused impulse、文字 reveal 未重复发事件、音频恢复政策、provider 重新挂载、内容版本变化及原生存档与 Runtime snapshot 的不同承诺。不要把原生 `save::restore` 直接当成任意 long-op 时刻的完整 VM snapshot。

### 4. 建立资产身份、依赖和 cook 流程，再做异步按需加载

当前能导入 glTF、原生图片和归档，并不等于有内容数据库。源码中未找到通用 GUID/依赖图/增量重导入/cook manifest 系统；现在仍主要按路径和内存缓存工作。96 个图片 decoder 解决的是解析能力，不能解决重命名引用、依赖失效、打包确定性和运行时加载尖峰。

**验收：** 从一个 glTF+native texture+font alias 项目生成可复现 cook manifest；改一张贴图只重建其依赖；重命名不破坏逻辑引用；压缩包里的大资源读取有取消、并发和 resident-byte 限制；打包器拒绝缺依赖和未声明的外部文件。先用当前 `AssetReader`/VFS/provider 机制，不另建一套绕过 client root 的加载器。

### 5. 补可定位问题的 profiler 和编辑闭环

已有 reflective API、MCP 离线 capture 和统计值；但 MCP capture 会启动新进程，不是 live VM 检查。当前关键 wgpu pass 的 `timestamp_writes` 仍为 `None`，没有证据支持完整 CPU/GPU frame profiler。缺少 inspector/gizmo/undo/hot reload 的编辑工作流，会直接拖慢后续内容接入。

**验收：** 一帧能关联 VM instructions、resource decode、shared text/composition、physics step、GPU pass、上传字节及 resident memory；报告 p50/p95/p99 和失帧原因。随后用同一 validated API 做最小 scene inspector/undo 和 MCP live transport，保留 client/generation/expected-revision 约束；不得用任意内存写作为编辑接口。

### 6. 完成角色运动与内容绑定的 3D 闭环

已有 skinned clips、blend spaces/montages、Rapier character sweep、碰撞事件和 grid A*；下一步不是再写物理求解器。当前 `animgraph` 输出加权 clip layers，未找到通用 root-motion extraction/application、IK/retargeting/ragdoll authoring；导航仍是 2.5D grid。GoW 原生角色的骨骼/权重/ANM 则属于特定 importer 的另一个缺口，不能由一个 Fox 替身走路示例证明完成。

**验收：** 一个合法小场景里，clip root delta 只应用一次并通过角色碰撞解算，斜坡脚步/转向/攻击 notify/受击物理一致；存档恢复不重复位移或事件。先对 synthetic glTF clip 做数值反例，再接 GoW native 数据；原生实体/rest-space 变换和完整关卡物件分别列覆盖。

### 7. 把已跑通的本机切片变成桌面发布回归

Windows/Linux/macOS 的 CI matrix 已写在 `.github/workflows/ci.yml`，但配置存在不是三平台通过的证据。当前公开行为记录以本机 macOS/Metal 为主，声卡／手柄、长时间运行、device loss、打包后相对路径和资源恢复仍需实际结果。硬件 RT 仍是明确未支持能力；已有 compute raytrace prototype 不等同硬件 RT。

**验收：** 在三个实际 runner 上各跑同一自造 2D 原生服务切片与一个 3D playable slice，核对图像/输入/存档、声卡与注入 PCM 分别验证、无设备 headless 路径、renderer/device 重建和独立安装包。保留 ignored 与显式运行结果的区别，不把物理微基准或单张图片 capture 当作整机帧率。

## RealLive 接入里必须继续分开的两类东西

共享机制包括：受限文件与随机读流、图片 entropy/像素解码、glyph raster、已对照的 bitmap 数学、PCM resample/mix、logical surface publication、engine clock/input delivery。原生政策包括：NLS 字节扫描、FOLDNAME/扩展名优先级与 case-insensitive 查找、G00 pattern 原点、消息窗口固定网格与禁则、DC/transition/manual-refresh、long-op/wait/read/savepoint，以及 voice archive 的 sample 和 loop 定义。

BGI 的共享桥证明前一类可以在 CrySirror 内完成，并不证明后一类可以跨 VM 复用。RealLive 首个 profile 完成后，应把实际保留的原生模块、已共享的调用点和拒绝的功能逐项列出，而不是只写“RealLive 已接入”。
