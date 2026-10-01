# 离线文件工具箱（Offline File Toolbox）

一个基于 **Tauri v2** 的 Windows 桌面工具集：批量重命名、图片压缩/格式转换（含 HEIC）、PDF 合并/拆分、
图片水印、音频格式转换。**纯本地处理，不含任何网络代码** —— 不联网、不上传、无统计，所有文件读写都在你自己的电脑上完成。

- 免安装绿色单文件（约 10 MB），双击即用
- 界面朴素轻量，**5 种语言**：简体中文 / 繁體中文 / English / 日本語 / 한국어
- **浅色 / 深色主题**，首次运行跟随系统，切换后记住你的选择
- 输出文件**永不覆盖**原文件，重名自动改名

## 功能

| 模块 | 能力 |
| --- | --- |
| 批量重命名 | 模板化命名：`{name}` `{upper}` `{lower}` `{num:N}` `{date:格式}`，配合查找替换、大小写转换、空格处理；右侧实时预览新旧文件名与冲突标记，一键自动解决重名 |
| 图片压缩 / 转换 | 批量转 JPEG / PNG / WebP 或保持原格式重压缩，质量可调；**支持苹果 HEIC/HEIF 照片输入**（libheif 本地解码）；等比缩放（百分比或限制最长边）；缩略图预览 + 处理前后体积对比 |
| PDF 合并 / 拆分 | 合并多个 PDF（可上移/下移调整顺序）；按页码范围（如 `1-3, 5, 7-10`）或每 N 页拆分，实时预览拆分计划；加密 PDF 会明确提示 |
| 图片水印 | 文字水印（系统字体渲染，支持中文）与图片水印（PNG 徽标）可**多层叠加**；每层可设不透明度、旋转、九宫格位置或**网格铺满**（可设数量）；右侧实时预览 |
| 音频格式转换 | 12 种目标格式（见下表），转换前用 ffprobe 读取真实参数并给出计划（含被钳制的参数与原因）；队列 + 并发转换（默认 4，最高 16）、可取消、可重试失败项；标签与封面随转码迁移；**源文件删除/回收默认关闭，且只在校验输出时长通过后才执行** |

音频目标格式：MP3、WAV、FLAC、AAC(M4A)、OGG Vorbis、ALAC(M4A)、Opus、WMA、AMR-NB、AC3、WavPack 共 11 种可用；
APE 为「仅可解码」（FFmpeg 无 APE 编码器），界面中已停用。

## 技术栈

- **前端**：原生 HTML / CSS / JS，零构建步骤、零 npm 依赖；三栏布局：左侧导航 / 中间操作区 / 右侧预览区
- **后端**：Rust（Tauri v2）
  - 重命名：标准库 `std::fs`
  - 图片：[`image`](https://crates.io/crates/image) + [`webp`](https://crates.io/crates/webp)（原生 libwebp）+ [`libheif-rs`](https://crates.io/crates/libheif-rs)（HEIC 解码）
  - 水印：[`ab_glyph`](https://crates.io/crates/ab_glyph)（系统字体栅格化）
  - PDF：[`lopdf`](https://crates.io/crates/lopdf)
  - 音频：**外部 ffmpeg / ffprobe 可执行程序**（不链接进主程序，以命令行方式调用）
  - 对话框：[`tauri-plugin-dialog`](https://crates.io/crates/tauri-plugin-dialog)
- 前端资源在构建时被压缩内嵌进 exe，因此发布物是**单个可执行文件**

## 环境要求

- Windows 10 / 11（自带 WebView2 运行时）
- [Rust](https://rustup.rs)（MSVC 工具链，即默认安装方式）；Node.js 仅在需要重新生成图标时用到
- **vcpkg + libheif**（HEIC 解码所需的 C++ 库，一次性安装）：
  ```powershell
  git clone https://github.com/microsoft/vcpkg.git C:\vcpkg
  cd C:\vcpkg; .\bootstrap-vcpkg.bat
  .\vcpkg install "libheif[core]:x64-windows-static-md"
  setx VCPKG_ROOT "C:\vcpkg"
  ```
- 首次构建需要联网下载 crates 依赖

### 音频组件（ffmpeg）是可选的

音频格式转换依赖外部的 `ffmpeg.exe` 与 `ffprobe.exe`。**其余四个模块不需要它。** 两种获取方式：

1. **下载完整版发布包**（推荐给普通用户）：`OfflineFileTool-v1.5.1-full.zip`，内已含组件，解压即用。
2. **自行下载**：从 <https://www.gyan.dev/ffmpeg/builds/> 获取 FFmpeg Windows 构建，把 `ffmpeg.exe`、`ffprobe.exe`
   放到主程序同级的 `ffmpeg\bin\` 目录，或加入系统 `PATH`。

程序按以下顺序查找组件（找到即用）：

```
<exe 所在目录>\ffmpeg.exe
<exe 所在目录>\bin\ffmpeg.exe
<exe 所在目录>\ffmpeg\bin\ffmpeg.exe     ← 完整版发布包使用的布局
<exe 所在目录>\..\ffmpeg\bin\ffmpeg.exe  （再向上 2 级）
<工作目录>\ffmpeg\bin\ffmpeg.exe         （再向上 2 级）
PATH 中的 ffmpeg.exe
```

**许可证提示**：常用的 Windows 构建（gyan.dev 的 full / essentials）启用了 `--enable-gpl`，属于 **GPL v3**。
它作为独立程序被调用、未链接进本程序（聚合分发），因此本程序仍为 MIT；但**请勿将 ffmpeg 静态链接进主程序**，
否则整个软件必须按 GPL 发布。详见 `packaging/FFmpeg-Notice.txt`。

## 构建与运行

```powershell
# 方式一：一键脚本（推荐）
.\build.ps1        # 编译 release 版并询问是否启动

# 方式二：手动
cargo build --release --manifest-path src-tauri\Cargo.toml
.\src-tauri\target\release\file-toolbox.exe
```

产物是**绿色单文件** `file-toolbox.exe`（约 10 MB），复制到任何 Windows 10/11 电脑都能直接运行。

## 开发调试

```powershell
.\dev.ps1          # 安装 tauri-cli（一次性）并进入开发模式，修改 ui/ 后自动重载
```

> 修改 `ui/` 下的文件后，需要重新 `cargo build` 才会生效 —— 前端资源是在编译时内嵌进 exe 的
> （`src-tauri/build.rs` 已声明对 `../ui` 的依赖，改动会触发重新内嵌）。

## 自动化检查

```powershell
cargo test --release --manifest-path src-tauri/Cargo.toml   # 49 项单元测试

node scripts\check-i18n.js           # 5 种语言键值一致性（380 键）
node scripts\check-error-codes.js    # 后端错误码是否都有本地化文案
node scripts\check-theme.js          # 深浅色主题接线（防闪屏脚本、按钮、过渡、对比度）
node scripts\check-theme-vars.js     # CSS 变量完整性（深色下是否有未覆盖的颜色）
node scripts\check-perf.js           # 性能优化是否被回退（事件委托/帧合并/长列表）
node scripts\check-delegation.js     # 列表事件委托是否覆盖所有可点击控件（带自检）
node scripts\check-version.js        # 版本号一致性（配置 / Cargo.toml / 关于页 / 文档）
node scripts\audit-exe.js            # 隐私审计：检查 exe 中是否残留构建机用户名/路径
```

## 项目结构

```
file-toolbox/
├── ui/                        # 前端（静态文件，编译时内嵌进 exe）
│   ├── index.html             # 三栏布局骨架 + 首帧防闪屏脚本
│   ├── styles.css             # 浅色/深色两套调色板（CSS 变量）
│   ├── app.js                 # 界面逻辑、事件委托、主题与音频队列
│   ├── i18n.js                # 5 种语言文案（380 键）
│   └── assets/logo.png
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs             # 命令注册与应用入口
│   │   ├── commands.rs        # 文件对话框 + 批量重命名
│   │   ├── images.rs          # 图片压缩/格式转换（含 HEIC 解码）
│   │   ├── pdf.rs             # PDF 合并/拆分
│   │   ├── watermark.rs       # 图片水印（多层/网格）
│   │   ├── audio.rs           # 音频格式转换（ffmpeg 调用、队列、并发、校验）
│   │   └── parallel.rs        # 无第三方依赖的保序并行映射（导入阶段加速）
│   ├── icons/                 # 应用图标（scripts/make-icons.mjs 生成）
│   ├── capabilities/          # 权限声明
│   ├── build.rs               # 声明对 ../ui 的依赖（UI 改动触发重新内嵌）
│   └── tauri.conf.json
├── packaging/                 # 随包分发的说明与许可证
│   ├── README-zh.txt          # 完整版包内的中文使用说明
│   ├── FFmpeg-Notice.txt      # FFmpeg 版本/构建参数/源码地址/替换说明
│   └── GPL-3.0.txt            # GPL v3 许可证全文
├── scripts/                   # 检查、图标、打包与审计脚本（见上）
├── build.ps1 / dev.ps1        # 构建与开发脚本
└── LICENSE                    # MIT
```

## 隐私说明

- 应用**不包含任何网络请求代码**：前端无 CDN、无统计脚本，后端无 HTTP 客户端
- 处理过程全部在本地内存与磁盘之间完成，输出写入你指定的本地目录
- 不收集、不缓存、不传输任何文件内容
- 音频功能调用的是本地 ffmpeg 可执行程序，同样不产生任何网络流量

## 分发打包

三种发布物，按需选择：

| 发布物 | 体积 | 说明 |
| --- | --- | --- |
| `file-toolbox.exe` | 约 10 MB | 绿色单文件；音频功能需用户自备 ffmpeg |
| `OfflineFileTool-v1.5.1.exe` | 约 10 MB | 同上，带版本号的发布名（老用户升级只需换这个文件） |
| `OfflineFileTool-v1.5.1-full.zip` | 约 169 MB | **完整版**：主程序 + ffmpeg/ffprobe + 许可证 + 中文说明 + SHA256SUMS，解压即用 |

重新生成完整版发布包：

```powershell
pwsh -File scripts\package-release.ps1
```

脚本会复制主程序与组件、写入带 BOM 的 UTF-8 说明文档、校验 GPLv3 正文抬头与结尾、生成 `SHA256SUMS.txt`、
打 ZIP，并解压后实跑 `ffmpeg -version` / `ffprobe -version` 自检。

> **上传到 GitHub 时注意**：仓库单文件上限 100 MB，完整版 ZIP 与 ffmpeg 二进制必须作为
> **Release 资产**上传，不要提交进仓库。`.gitignore` 已排除 `/dist`、`/ffmpeg`、`/target`、
> `/.cargo-neutral`、`/.build-tmp`，避免把数 GB 的构建产物写进 Git 历史。

**个人隐私审计**：发布前运行 `node scripts\audit-exe.js` 检查二进制中是否含有用户名/路径等痕迹。
项目采用「中性构建」机制 —— `build.ps1` 会自动使用项目内的 `.cargo-neutral` 缓存目录编译，
产出的 exe 不含构建机用户名。

**未签名提示**：接收方首次运行可能看到 SmartScreen「Windows 已保护你的电脑」，属正常现象，
点「更多信息 → 仍要运行」即可。如需消除，需购买代码签名证书（个人分发可忽略）。

## 许可证

- 本软件：**MIT License**（作者 WillYin），见 [LICENSE](LICENSE)
- 随完整版分发的 FFmpeg：**GPL v3**（独立程序，聚合分发），见 `packaging/FFmpeg-Notice.txt` 与 `packaging/GPL-3.0.txt`

## 常见问题

- **音频页提示「未找到 ffmpeg」**：下载完整版发布包，或把 `ffmpeg.exe` / `ffprobe.exe` 放到主程序同级的
  `ffmpeg\bin\` 目录（或加入 PATH）。其余模块不受影响。
- **APE 为什么不能转**：FFmpeg 只能解码 APE、没有 APE 编码器，界面中已停用该目标格式。
- **会不会误删我的源文件**：不会。源文件处理默认「保留」；选择删除或回收时，程序会先校验输出文件时长与
  源文件一致，通过后才动手，且删除前会二次确认。
- **下载依赖时反复报 `spurious network error ... HTTP/2 framing layer`**：网络中间设备干扰了 HTTP/2
  多路复用。`build.ps1` 已自动禁用（`CARGO_HTTP_MULTIPLEXING=false`）；手动构建时先执行
  `$env:CARGO_HTTP_MULTIPLEXING="false"`。国内网络可配置镜像加速（写入 `%USERPROFILE%\.cargo\config.toml`）：
  ```toml
  [source.crates-io]
  replace-with = 'rsproxy-sparse'
  [source.rsproxy-sparse]
  registry = "sparse+https://rsproxy.cn/index/"
  ```
- **构建报错 `link.exe not found`**：缺少 MSVC 构建工具，安装
  [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/downloads/) 时勾选「使用 C++ 的桌面开发」。
- **加密 PDF**：合并/拆分暂不支持带密码的 PDF，会明确提示跳过。
- **HEIC 苹果照片**：支持 HEIC/HEIF **输入**（解码后转 JPEG/WebP/PNG）；不支持**输出** HEIC
  （HEVC 编码器 x265 为 GPL 许可证且体积巨大，个人工具不建议引入）。HEIC 选「保持原格式」时会自动转 JPEG。
- **PDF 书签**：合并后不保留原书签（lopdf 限制），页面内容完整保留。
- **超长路径**：暂不支持 `\\?\` 前缀的超长路径（Windows 默认 260 字符限制内正常）。
