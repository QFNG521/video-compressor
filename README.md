# 轻压 · 本地视频 / 图片压缩工具

> 一个完全**本地运行**、不联网、不上传文件的视频与图片压缩桌面应用。基于 Tauri v2 + Rust + ffmpeg，体积小、启动快，支持 macOS 与 Windows。

---

## ✨ 特性

- **纯本地处理**：所有压缩在本地完成，文件不会离开你的电脑。
- **视频 / 图片通吃**：一个应用同时压缩视频和图片，支持拖拽与多选。
- **显卡加速（默认）**：自动探测本机 GPU 硬件编码器并优先使用
  - NVIDIA → `hevc_nvenc` / `h264_nvenc`
  - Intel → `hevc_qsv` / `h264_qsv`
  - AMD → `hevc_amf` / `h264_amf`
  - Apple Silicon / macOS → `hevc_videotoolbox` / `h264_videotoolbox`
  - 无可用显卡时自动回退到 CPU 编码（H.265 / H.264）。
- **智能防膨胀**：当压缩结果比源文件还大时（如本已高度压缩的小视频），自动保留原文件，绝不「越压越大」。
- **质量档位**：
  - 视频：视觉无损 / 高画质 / 高压缩
  - 图片：高画质 / 均衡 / 高压缩
- **图片转 WebP**：可将 `bmp` / `tif` / `tiff` 强制转为 WebP 获得更小体积；也支持 JPG/PNG 直接转 WebP。
- **实时进度**：压缩过程中显示百分比与速度，完成后展示体积节省比例。
- **记录持久化**：任务列表与输出文件会保存，重新打开应用仍在。

---

## 🎬 支持的格式

| 类型 | 输入 | 输出 |
| --- | --- | --- |
| 视频 | `mp4` `mov` `mkv` `avi` `flv` `wmv` `ts` `m4v` | `mp4`（H.265/H.264 + AAC） |
| 图片 | `jpg` `jpeg` `png` `webp` `bmp` `tif` `tiff` | 保持原格式，或转 `webp` |

输出文件统一存放在应用数据目录下的 `compressed/` 文件夹，文件名自动去重（如 `video (1).mp4`）。

---

## 🧩 压缩方案说明

| 编码器 | 速度 | 压缩率 | 适用场景 |
| --- | --- | --- | --- |
| 显卡加速（GPU） | 极快（CPU 的数倍～数十倍） | 中等 | 大视频、追求速度 |
| H.265（CPU） | 慢 | 高 | 想压到最小体积 |
| H.264（CPU） | 中 | 中 | 兼容性优先 |

> 提示：GPU 编码速度远胜 CPU，但同等画质下体积通常略大于 CPU 的 H.265。如果更在意体积而非速度，建议选「H.265」。

---

## 🛠 技术栈

- **框架**：[Tauri v2](https://v2.tauri.app/)（Rust 后端 + Web 前端）
- **后端**：Rust，调用 [ffmpeg](https://ffmpeg.org/) 作为 sidecar 资源
- **前端**：原生 HTML / CSS / JavaScript + Vite
- **打包**：macOS → `.app` / `.dmg`；Windows → NSIS 安装器（`.exe`）

---

## 📦 安装与下载

### 方式一：从 Release 下载（推荐普通用户）

前往 GitHub Releases 页面，按系统下载：

- **macOS（Apple Silicon）**：`轻压.app` 或 `轻压_x.x.x_aarch64.dmg`
- **Windows（x64）**：`轻压_x.x.x_x64-setup.exe`

### 方式二：GitHub Actions 自动出包

项目配置了 CI（`push` 打 `v*` 标签时触发）：

- `qingya-macos` —— macOS `.app` 与 `.dmg`
- `qingya-windows` —— Windows NSIS 安装器

打标签即可自动构建：

```bash
git tag v0.1.4
git push origin v0.1.4
```

---

## 🏗 本地开发 / 构建

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 18（建议 22）
- [Rust](https://www.rust-lang.org/) stable 工具链
- 系统已安装 `ffmpeg`（开发用；打包时应用自带 ffmpeg 资源，无需用户安装）

### 安装依赖

```bash
npm install
```

### 开发模式（热重载）

```bash
npm run tauri dev
```

### 打包发布版本

```bash
npm run tauri build
```

> 仓库在 `src-tauri/ffmpeg/` 中签入了 macOS 版 ffmpeg 作为资源。
> Windows 打包由 CI 自动把 ffmpeg 二进制替换为 Windows 版（BtbN 静态构建，含 libx265 + libwebp），无需本机手动处理。

---

## 📁 目录结构

```
video-compressor-tauri/
├── index.html              # 前端入口
├── src/                    # 前端源码（main.js 等）
├── src-tauri/              # Rust 后端
│   ├── src/lib.rs          # 核心逻辑：压缩命令、进度解析、GPU 探测
│   ├── tauri.conf.json     # 应用配置（名称、窗口、打包目标）
│   ├── capabilities/       # 权限配置（文件打开等）
│   ├── ffmpeg/             # 签入的 ffmpeg 资源（macOS 版，CI 替换 Windows 版）
│   └── icons/              # 应用图标
├── .github/workflows/      # GitHub Actions 双平台 CI
└── package.json
```

---

## ❓ 常见问题

**Q：为什么小视频用显卡压缩后反而更大了？**
A：GPU 编码追求速度，压缩率通常低于 CPU 的 H.265；对于本已高度压缩的小视频，结果可能比源文件还大。轻压会检测这种情况并自动保留原文件（记录显示「未压缩（源已足够小）」），不会越压越大。若要压到最小，请改用「H.265」。

**Q：Windows 上选择文件/压缩时会弹黑色命令行窗口吗？**
A：不会。Windows 下已用 `CREATE_NO_WINDOW` 隐藏 ffmpeg 控制台窗口。

**Q：输出文件去哪了？**
A：在应用数据目录的 `compressed/` 下，可在任务完成后的「在文件夹中显示」按钮一键打开。

---

## 📄 许可证

[Apache License 2.0](./LICENSE)
