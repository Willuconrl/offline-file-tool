# 离线文件工具箱（Offline File Toolbox）

一个 **Tauri 桌面应用**：批量重命名、图片压缩/格式转换（含 HEIC）、PDF 合并/拆分、图片与 PDF 水印。
**纯本地处理，绝不联网、绝不上传** —— 所有文件读写都在你自己的电脑上完成。

## 功能

| 模块 | 能力 |
| --- | --- |
| ✏️ 批量重命名 | 查找替换（支持忽略大小写）、添加前缀/后缀、大小写转换、自动序号（起始值/步长/补零/位置/分隔符），右侧实时预览新旧文件名，冲突检测 + 一键自动解决重名，支持链条重命名（A→B、B→C） |
| 🖼️ 图片压缩/转换 | 批量转 JPEG / WebP / PNG 或保持原格式重新压缩，质量可调（1-100），WebP 为原生 libwebp 有损压缩；**支持苹果 HEIC/HEIF 照片输入**（libheif 本地解码，自动转 JPG/WebP/PNG，HEIC 选「保持原格式」时自动转 JPEG）；等比缩放（百分比或限制最长边），输出目录可选，自动改名不覆盖源文件，处理前后体积对比 |
| 📄 PDF 合并/拆分 | 合并多个 PDF（可调整顺序）；按页码范围（如 `1-3, 5, 7-10`）或每 N 页拆分，实时预览拆分计划 |
| 💧 图片/PDF 水印 | 批量添加**文字水印**（系统字体渲染，支持中文、颜色/不透明度/旋转/九宫格位置/平铺）或**图片水印**（PNG 徽标），右侧实时预览水印效果；PDF 用标准 XObject+透明遮罩嵌入，原内容无损，输出自动改名不覆盖 |

## 技术栈

- **前端**：原生 HTML/CSS/JS（零构建步骤、零 npm 依赖），三栏布局：左侧导航 / 中间操作区 / 右侧预览区
- **后端**：Rust（Tauri v2）
  - 重命名：标准库 `std::fs`
  - 图片：[`image`](https://crates.io/crates/image)（JPEG/PNG 编码）+ [`webp`](https://crates.io/crates/webp)（原生 libwebp 有损压缩）+ [`libheif-rs`](https://crates.io/crates/libheif-rs)（HEIC/HEIF 解码）
  - 水印：[`ab_glyph`](https://crates.io/crates/ab_glyph)（系统字体文字栅格化，纯本地）
  - PDF：[`lopdf`](https://crates.io/crates/lopdf)
  - 文件对话框：[`tauri-plugin-dialog`](https://crates.io/crates/tauri-plugin-dialog)
- 全部文件内容只在本地内存与磁盘之间流转
- 内置 7 项自动化测试覆盖重命名规则、图片编码往返、PDF 合并/拆分（`cargo test`）

## 环境要求

- Windows 10/11（自带 WebView2）
- [Rust](https://rustup.rs)（MSVC 工具链，即默认安装方式）——Node.js 不是必需
- **vcpkg + libheif**（HEIC 解码的 C++ 库，一次性安装）：
  ```powershell
  git clone https://github.com/microsoft/vcpkg.git C:\vcpkg
  cd C:\vcpkg; .\bootstrap-vcpkg.bat
  .\vcpkg install "libheif[core]:x64-windows-static-md"
  setx VCPKG_ROOT "C:\vcpkg"
  ```
- 首次构建需要联网下载 crates 依赖

## 构建与运行

```powershell
# 方式一：一键脚本（推荐）
.\build.ps1        # 编译 release 版并询问是否启动

# 方式二：手动
cargo build --release --manifest-path src-tauri\Cargo.toml
.\src-tauri\target\release\file-toolbox.exe   # 单文件即可运行，无需安装
```

输出的是一个**绿色单文件** `file-toolbox.exe`，复制到任何 Windows 10/11 电脑都能直接运行。

## 开发调试

```powershell
.\dev.ps1          # 安装 tauri-cli（一次性）并进入开发模式，修改 ui/ 后自动重载
```

## 项目结构

```
file-toolbox/
├── ui/                  # 前端（静态文件，构建时内嵌进 exe）
│   ├── index.html       # 三栏布局骨架
│   ├── styles.css       # 深色主题样式
│   └── app.js           # 界面逻辑与后端调用
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs       # 命令注册
│   │   ├── commands.rs  # 对话框 + 批量重命名
│   │   ├── images.rs    # 图片压缩/格式转换
│   │   └── pdf.rs       # PDF 合并/拆分
│   ├── icons/           # 应用图标（scripts/make-icons.mjs 生成）
│   ├── capabilities/    # 权限声明
│   └── tauri.conf.json
├── scripts/make-icons.mjs  # 离线图标生成器（纯 Node，无依赖）
├── build.ps1 / dev.ps1     # 构建与开发脚本
```

## 隐私说明

- 应用**不包含任何网络请求代码**：前端无 CDN、无统计，后端无 HTTP 客户端
- 处理过程全部在本地内存中完成，输出文件直接写入你指定的本地目录
- 不收集、不缓存、不传输任何文件内容

## 分发打包（给他人使用）

只需发送**一个文件**：`src-tauri\target\release\file-toolbox.exe`（约 8 MB，绿色单文件）。

- 接收方要求：Windows 10/11（系统自带 WebView2 运行时）
- **个人隐私审计**：发布前建议运行 `node scripts\audit-exe.js` 检查二进制中是否含有
  用户名/路径等痕迹。项目已采用「中性构建」机制——`build.ps1` 会自动使用项目内的
  `.cargo-neutral` 缓存目录编译，产出的 exe 不包含构建机的用户名。
  （注意：请勿删除 `.cargo-neutral` 目录后执行 `cargo clean` 再重建，否则会退回默认
  缓存路径、重新引入构建机用户名。）
- 未签名程序提示：接收方首次运行可能看到 SmartScreen「未知发布者」警告，属正常现象，
  点「更多信息 → 仍要运行」即可。如需消除，需购买代码签名证书（个人分发可忽略）。

## 常见问题

- **下载依赖时反复报 `spurious network error ... HTTP/2 framing layer`**：网络中间设备干扰了
  HTTP/2 多路复用。`build.ps1` 已自动禁用（`CARGO_HTTP_MULTIPLEXING=false`）；手动构建时先执行
  `$env:CARGO_HTTP_MULTIPLEXING="false"`。若网络仍不稳，可直接重跑脚本——已下载的 crate 会续传，
  不会从头开始。国内网络还可以配置镜像加速（把下面内容写入 `%USERPROFILE%\.cargo\config.toml`）：
  ```toml
  [source.crates-io]
  replace-with = 'rsproxy-sparse'
  [source.rsproxy-sparse]
  registry = "sparse+https://rsproxy.cn/index/"
  ```
- **构建报错 `link.exe not found`**：缺少 MSVC 构建工具，安装
  [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/downloads/)
  时勾选「使用 C++ 的桌面开发」即可。
- **加密 PDF**：合并/拆分暂不支持带密码的 PDF，会明确提示跳过。
- **HEIC 苹果照片**：支持 HEIC/HEIF **输入**（解码后转 JPEG/WebP/PNG）；不支持**输出** HEIC
  （HEVC 编码器 x265 为 GPL 许可证且编译体积巨大，个人工具不建议引入）。HEIC 选
  「保持原格式」时会自动转 JPEG。
- **PDF 书签**：合并后不保留原书签（lopdf 限制），页面内容完整保留。
- **重命名冲突**：右侧预览会标出冲突项，勾选「自动解决重名冲突」后自动加序号。
