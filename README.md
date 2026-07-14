# MPrism

Windows 本地 AI 聊天桌面应用。对接 OpenAI 兼容及多种模型协议服务商，数据保存在本机，前端不直连模型 API。

当前版本：`0.1.0`  
技术栈：Tauri 2 · React · TypeScript · Rust（`mprism-protocol`）

---

## 功能

- **多服务商配置**  
  添加 / 编辑 / 删除模型服务商，填写名称、协议、Base URL、API Key（可留空以对接本地无鉴权服务）。

- **多协议**  
  - OpenAI-compatible Chat Completions  
  - OpenAI Responses  
  - Anthropic Messages  
  - Gemini `generateContent`

- **模型管理**  
  从服务商发现模型、手工添加、保留列表、设默认模型；可编辑显示名、`temperature`、`max_tokens`。

- **会话聊天**  
  多会话列表（新建 / 重命名 / 删除）、流式生成与停止、系统提示词、Markdown 与代码高亮、思考过程展示。

- **本机存储**  
  配置与会话写入用户目录 `%USERPROFILE%\.mprism`，前端不直接访问该目录。

- **界面**  
  亮色 / 暗色 / 跟随系统；侧栏会话与模型服务导航；中文界面。

- **打包**  
  Windows NSIS 安装包；WebView2 引导安装；单实例运行。

---

## 环境要求

| 依赖 | 说明 |
|------|------|
| Windows 10/11 | 目标平台 |
| [Node.js](https://nodejs.org/) 18+ | 前端构建 |
| [pnpm](https://pnpm.io/) 10.x | 仓库指定 `packageManager` 为 `pnpm@10.20.0` |
| [Rust](https://rustup.rs/) stable | Tauri / 协议 SDK |
| WebView2 | 运行桌面壳；打包时可引导下载 |
| Visual Studio C++ 构建工具 | Windows 上编译 Rust/Tauri 常用 |

---

## 获取代码与安装依赖

```powershell
git clone <你的仓库地址>
cd MPrism
pnpm install
```

---

## 开发

启动带窗口的桌面开发环境（Vite + Tauri）：

```powershell
pnpm tauri:dev
```

仅前端（无 Tauri 壳，部分 IPC 不可用）：

```powershell
pnpm dev
```

常用检查：

```powershell
pnpm typecheck
pnpm test
pnpm lint
```

生产配置体检（CSP、capabilities、敏感信息扫描等，见 `scripts/audit-production.ps1`）：

```powershell
pnpm audit:production
```

---

## 构建安装包

生成 Windows 安装产物：

```powershell
pnpm tauri:build
```

成功后安装包一般在：

```text
apps/desktop/src-tauri/target/release/bundle/nsis/
```

仅构建前端静态资源：

```powershell
pnpm build
```

---

## 使用说明

### 1. 配置模型服务

1. 打开应用，进入左侧 **模型服务**。  
2. **新建服务商**，填写名称、协议、Base URL。  
3. 如需鉴权则填写 API Key；本地 Ollama 等可留空。  
4. **保存配置** 后点击 **获取模型**，在发现结果中勾选要保留的模型；也可 **手工添加**。  
5. 在已保留模型中可改 **模型名**、temperature、max_tokens，并设为默认。

### 2. 开始聊天

1. 回到 **聊天**。  
2. **新建会话**，在输入栏选择服务商与模型。  
3. 输入消息，`Enter` 发送，`Shift+Enter` 换行。  
4. 生成中可 **停止**；顶栏可编辑 **系统提示词**。  
5. 会话列表支持重命名、删除；侧栏可折叠与调宽。

### 3. 主题

左侧导轨底部可切换 **跟随系统 / 亮色 / 暗色**。

### 4. 数据位置

本地数据目录（请勿把含密钥的目录提交到 Git）：

```text
%USERPROFILE%\.mprism
```

---

## 仓库结构（简要）

```text
MPrism/
  apps/desktop/          # React 前端 + Tauri 应用
  crates/mprism-protocol # 模型协议 SDK（无 UI / 无 Tauri 依赖）
  package.json           # pnpm scripts
  Cargo.toml             # Rust workspace
```

架构约定：UI → Tauri 命令 → 应用层存储 / 生成管理 → `mprism-protocol` → 远端或本地模型 HTTP API。

---

## 版本号

对外桌面版本以 `apps/desktop/src-tauri/tauri.conf.json` 的 `version` 为准。  
Rust crate 版本见根目录 `Cargo.toml` 的 `[workspace.package]`；前端包版本见 `apps/desktop/package.json`。发版时建议三者对齐。

---

## 许可

见仓库中的 license 声明（workspace 元数据为 MIT）。
