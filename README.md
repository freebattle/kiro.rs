# kiro-rs

一个用 Rust 编写的 **Anthropic Messages API** / **OpenAI Responses API** 兼容代理服务，将客户端请求转换为 Kiro Runtime API。

支持 Claude Code、Codex 等客户端，覆盖 Claude（Sonnet / Opus / Haiku）与 GPT-5.6-luna 上游模型。


## 免责声明

本项目仅供研究使用, Use at your own risk, 使用本项目所导致的任何后果由使用人承担, 与本项目无关。
本项目与 AWS/KIRO/Anthropic/Claude/OpenAI 等官方无关, 本项目不代表官方立场。

## 注意！

因 TLS 默认从 native-tls 切换至 rustls，你可能需要专门安装证书后才能配置 HTTP 代理。可通过 `config.json` 的 `tlsBackend` 切回 `native-tls`。
如果遇到请求报错, 尤其是无法刷新 token, 或者是直接返回 error request, 请尝试切换 tls 后端为 `native-tls`, 一般即可解决。

**Write Failed/会话卡死**: 如果遇到持续的 Write File / Write Failed 并导致会话不可用，参考 Issue [#22](https://github.com/hank9999/kiro.rs/issues/22) 和 [#49](https://github.com/hank9999/kiro.rs/issues/49) 的说明与临时解决方案（通常与输出过长被截断有关，可尝试调低输出相关 token 上限）

## 功能特性

- **Anthropic API 兼容**: 完整支持 `/v1/messages`、流式 SSE、thinking、tool use
- **OpenAI Responses 兼容**: 支持 `/v1/responses`（Codex / `wire_api=responses`），含 `previous_response_id`、自定义工具、`additional_tools`
- **GPT-5.6 上游**: 支持 `gpt-5.6-luna` / `gpt-5.6-sol` / `gpt-5.6-terra`（`gpt-5.6` 默认 luna），自动注入 `reasoning.effort`
- **Claude 1.0.138 协议**: IDE Runtime 端点（`x-amz-target` + `application/x-amz-json-1.0`）、`agentMode=vibe`、`output_config.effort`（Sonnet/Opus）
- **模型能力区分**: Haiku 不发送 `additionalModelRequestFields`（上游不支持思考强度）
- **流式响应**: SSE 流式输出，含 ping 保活
- **Token 自动刷新**: 自动管理和刷新 OAuth Token
- **Kiro API Key 认证**: 支持 `kiroApiKey` / `KIRO_API_KEY` headless 凭据
- **多凭据 / 负载均衡**: `priority` / `balanced`，故障转移与 Token 回写
- **多客户端 API Key**: `apiKeys` 区分调用者，日志按 `caller` 统计
- **动态模型列表**: 远程 `ListAvailableModels` + 缓存 + 内置兜底
- **请求日志与用量统计**: 耗时、TTFT、Token、Credits
- **Admin 管理界面**: 凭据、请求日志、用量统计（嵌入二进制）
- **跨平台**: Windows / macOS / Linux（x64 & arm64，含 musl 静态包）

---

- [开始](#开始)
  - [1. 编译](#1-编译)
  - [2. 最小配置](#2-最小配置)
  - [3. 启动](#3-启动)
  - [4. 验证](#4-验证)
  - [Docker](#docker)
  - [Linux 服务器部署](#linux-服务器部署)
- [配置详解](#配置详解)
  - [config.json](#configjson)
  - [credentials.json](#credentialsjson)
  - [Region 配置](#region-配置)
  - [代理配置](#代理配置)
  - [认证方式](#认证方式)
  - [环境变量](#环境变量)
- [API 端点](#api-端点)
  - [标准端点 (/v1)](#标准端点-v1)
  - [Claude Code 兼容端点 (/cc/v1)](#claude-code-兼容端点-ccv1)
  - [OpenAI Responses 端点](#openai-responses-端点)
  - [Thinking 模式](#thinking-模式)
  - [工具调用](#工具调用)
- [模型映射](#模型映射)
- [Admin（可选）](#admin可选)
- [上传 GitHub / 发布注意](#上传-github--发布注意)
- [注意事项](#注意事项)
- [项目结构](#项目结构)
- [技术栈](#技术栈)
- [License](#license)
- [致谢](#致谢)

## 开始

### 1. 编译

> PS: 如果不想编译可直接前往 Release 下载对应平台二进制

**前置依赖**

| 组件 | 版本建议 |
|------|----------|
| Rust | 1.85+（edition 2024，推荐较新 stable） |
| Node.js | 20+（仅构建 Admin UI 需要） |
| pnpm / npm | 任一 |

> **必须先构建前端**，再编译 Rust（`rust_embed` 会嵌入 `admin-ui/dist`）：

```bash
cd admin-ui && pnpm install && pnpm build && cd ..
cargo build --release
```

产物：

- Linux/macOS: `./target/release/kiro-rs`
- Windows: `.\target\release\kiro-rs.exe`

### 2. 最小配置

创建 `config.json`：

```json
{
  "host": "127.0.0.1",
  "port": 8990,
  "apiKey": "sk-kiro-rs-qazWSXedcRFV123456",
  "region": "us-east-1",
  "kiroVersion": "1.0.138"
}
```

> 公网 / Docker 监听请设 `"host": "0.0.0.0"`。  
> 需要 Web 管理面板时请配置 `adminApiKey`。

创建 `credentials.json`（从 Kiro IDE 等获取；也可在 Admin 面板导入）：

Social：

```json
{
  "refreshToken": "你的刷新token",
  "expiresAt": "2025-12-31T02:32:45.144Z",
  "authMethod": "social"
}
```

IdC：

```json
{
  "refreshToken": "你的刷新token",
  "expiresAt": "2025-12-31T02:32:45.144Z",
  "authMethod": "idc",
  "clientId": "你的clientId",
  "clientSecret": "你的clientSecret"
}
```

Kiro API Key（Headless）：

```json
{
  "kiroApiKey": "ksk_your_api_key_here",
  "authMethod": "api_key"
}
```

也可参考仓库内：

- `config.example.json`
- `credentials.example.social.json` / `credentials.example.idc.json` / `credentials.example.apikey.json` / `credentials.example.multiple.json`

### 3. 启动

```bash
./target/release/kiro-rs
```

或指定路径：

```bash
./target/release/kiro-rs -c /path/to/config.json --credentials /path/to/credentials.json
```

### 4. 验证

Anthropic Messages：

```bash
curl http://127.0.0.1:8990/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: sk-kiro-rs-qazWSXedcRFV123456" \
  -d '{
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "stream": true,
    "messages": [
      {"role": "user", "content": "Hello, Claude!"}
    ]
  }'
```

OpenAI Responses（Codex）：

```bash
curl http://127.0.0.1:8990/v1/responses \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-kiro-rs-qazWSXedcRFV123456" \
  -d '{
    "model": "gpt-5.6-luna",
    "input": "Say only: pong",
    "stream": false
  }'
```

### Docker

```bash
# 准备配置目录
mkdir -p config data
cp config.example.json config/config.json
# 编辑 config/config.json：
#   - host 建议 0.0.0.0
#   - 填写 apiKey / adminApiKey
# 放入 credentials
cp credentials.example.social.json config/credentials.json

docker compose up -d
```

说明：

- 镜像默认：`ghcr.io/hank9999/kiro-rs:latest`
- 挂载 `./config` → `/app/config`，`./data` → `/app/data`
- **容器内 `host` 必须是 `0.0.0.0`**，否则映射端口无法访问
- 本地源码构建：在 `docker-compose.yml` 中启用 `build: .`

### Linux 服务器部署

项目是标准 Rust + 相对路径数据目录，**无需为 Linux 改业务代码**。注意下面几点即可。

#### A. 二进制 / Docker 二选一

**方式 1：Release 二进制（推荐省事）**

1. 从 GitHub Actions / Release 下载 `Linux-x64` 或 `Linux-musl-x64` / arm64
2. 放到例如 `/opt/kiro-rs/`
3. 同目录放 `config.json`、`credentials.json`
4. 用 systemd 托管（示例见 `scripts/kiro-rs.service`）

```bash
sudo useradd -r -s /usr/sbin/nologin kiro || true
sudo mkdir -p /opt/kiro-rs/data
sudo cp kiro-rs config.json credentials.json /opt/kiro-rs/
sudo chown -R kiro:kiro /opt/kiro-rs
sudo chmod 600 /opt/kiro-rs/credentials.json /opt/kiro-rs/config.json
sudo cp scripts/kiro-rs.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now kiro-rs
sudo systemctl status kiro-rs
```

**方式 2：Docker Compose**

见上一节。适合不想装运行时、方便升级的场景。

**方式 3：源码编译**

```bash
# Ubuntu/Debian 示例
sudo apt update
sudo apt install -y build-essential pkg-config curl
# 安装 Rust：https://rustup.rs
# 安装 Node.js 20+
git clone <your-fork-or-repo> kiro-rs && cd kiro-rs
cd admin-ui && npm i -g pnpm && pnpm install && pnpm build && cd ..
cargo build --release
```

#### B. Linux 上需要注意的配置

| 项 | 建议 |
|----|------|
| `host` | 本机反代后仅本地访问用 `127.0.0.1`；直接对外或 Docker 用 `0.0.0.0` |
| 工作目录 | 进程 cwd 决定 `./data/usage_stats`、`data/responses` 位置，systemd 请设 `WorkingDirectory` |
| `systemVersion` | 可填 `linux#6.x.x`，不填则随机 `darwin`/`win32` 标识（仅 UA，一般无妨） |
| `kiroVersion` | 默认 `1.0.138`，建议保持与当前协议一致 |
| TLS / 代理 | 出问题优先试 `tlsBackend: "native-tls"`，并确保系统 CA 正常（`ca-certificates`） |
| 防火墙 | 放行业务端口，或只走 Nginx/Caddy 反代 |
| 数据持久化 | 备份 `credentials.json`、`data/usage_stats`、`data/responses` |
| 调试 | 生产默认不要开 `debugLogDir`；排查时可临时设为 `data/debug_logs` |

#### C. 反代示例（可选）

```nginx
server {
  listen 443 ssl http2;
  server_name kiro.example.com;

  location / {
    proxy_pass http://127.0.0.1:8990;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header Connection "";
    proxy_buffering off;          # SSE 必须
    proxy_read_timeout 3600s;
  }
}
```

## 配置详解

### config.json

| 字段 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `host` | string | `127.0.0.1` | 服务监听地址；公网/Docker 用 `0.0.0.0` |
| `port` | number | `8080` | 服务监听端口（示例常用 `8990`） |
| `apiKey` | string | - | 客户端认证 Key（必配） |
| `apiKeys` | array | `[]` | 多 Key：`{ "key", "name" }`，日志按 caller 区分 |
| `region` | string | `us-east-1` | AWS 区域 |
| `authRegion` | string | - | Token 刷新区域，未配置回退 region |
| `apiRegion` | string | - | API 请求区域，未配置回退 region |
| `kiroVersion` | string | `1.0.138` | Kiro IDE 版本号（UA / 协议对齐） |
| `machineId` | string | - | 自定义机器码（64 位 hex），不填自动生成 |
| `systemVersion` | string | 随机 | 系统版本标识（如 `darwin#24.6.0` / `win32#10.0.22631`） |
| `nodeVersion` | string | `22.22.0` | Node.js 版本标识 |
| `tlsBackend` | string | `rustls` | `rustls` 或 `native-tls` |
| `countTokensApiUrl` | string | - | 外部 count_tokens API |
| `countTokensApiKey` | string | - | 外部 count_tokens 密钥 |
| `countTokensAuthType` | string | `x-api-key` | `x-api-key` 或 `bearer` |
| `proxyUrl` | string | - | HTTP/SOCKS5 代理 |
| `proxyUsername` | string | - | 代理用户名 |
| `proxyPassword` | string | - | 代理密码 |
| `adminApiKey` | string | - | 配置后启用 Admin API + Web UI |
| `loadBalancingMode` | string | `priority` | `priority` 或 `balanced` |
| `extractThinking` | boolean | `true` | 非流式响应解析 `<thinking>` 块 |
| `debugLogDir` | string | - | 调试日志目录；不设则关闭 |
| `defaultEndpoint` | string | `ide` | 默认 Kiro 端点 |
| `endpoints` | object | `{}` | 端点特定配置 |
| `includeOpenSourceModels` | boolean | `false` | `/v1/models` 是否包含开源模型 |

完整配置示例：

```json
{
  "host": "0.0.0.0",
  "port": 8990,
  "apiKey": "sk-kiro-rs-qazWSXedcRFV123456",
  "apiKeys": [
    {"key": "sk-user-alice-abc123", "name": "alice"},
    {"key": "sk-user-bob-def456", "name": "bob"}
  ],
  "region": "us-east-1",
  "tlsBackend": "rustls",
  "kiroVersion": "1.0.138",
  "machineId": "64位十六进制机器码",
  "systemVersion": "linux#6.8.0",
  "nodeVersion": "22.22.0",
  "authRegion": "us-east-1",
  "apiRegion": "us-east-1",
  "proxyUrl": "http://127.0.0.1:7890",
  "adminApiKey": "sk-admin-your-secret-key",
  "loadBalancingMode": "priority",
  "extractThinking": true,
  "debugLogDir": "data/debug_logs",
  "defaultEndpoint": "ide",
  "includeOpenSourceModels": false
}
```

### credentials.json

支持单对象格式（向后兼容）或数组格式（多凭据）。

#### 字段说明

| 字段 | 类型 | 描述 |
|------|------|------|
| `id` | number | 凭据唯一 ID（可选，Admin 管理用） |
| `accessToken` | string | OAuth 访问令牌（可选，可自动刷新） |
| `refreshToken` | string | OAuth 刷新令牌 |
| `profileArn` | string | AWS Profile ARN（可选） |
| `expiresAt` | string | Token 过期时间 (RFC3339) |
| `authMethod` | string | `social` / `idc` / `api_key` |
| `clientId` | string | IdC 客户端 ID |
| `clientSecret` | string | IdC 客户端密钥 |
| `priority` | number | 优先级，越小越优先，默认 0 |
| `region` | string | 凭据级 Auth Region（兼容字段） |
| `authRegion` | string | 凭据级 Auth Region |
| `apiRegion` | string | 凭据级 API Region |
| `machineId` | string | 凭据级机器码 |
| `email` | string | 用户邮箱（可选） |
| `subscriptionTitle` | string | 订阅等级（可选） |
| `proxyUrl` | string | 凭据级代理；`direct` 表示强制直连 |
| `proxyUsername` | string | 凭据级代理用户名 |
| `proxyPassword` | string | 凭据级代理密码 |
| `disabled` | boolean | 是否禁用，默认 false |
| `kiroApiKey` | string | 上游 Kiro API Key（`ksk_...`） |
| `endpoint` | string | 凭据级端点，未配置用 `defaultEndpoint` |

说明：

- IdC / Builder-ID / IAM 统一配置为 `authMethod: "idc"`
- 旧值 `builder-id` / `iam` 仍识别为 `idc`
- API Key 凭据：`authMethod: "api_key"`（兼容 `apikey`）

#### 多凭据格式

```json
[
  {
    "refreshToken": "第一个凭据",
    "expiresAt": "2025-12-31T02:32:45.144Z",
    "authMethod": "social",
    "priority": 0
  },
  {
    "refreshToken": "第二个凭据",
    "expiresAt": "2025-12-31T02:32:45.144Z",
    "authMethod": "idc",
    "clientId": "xxxxxxxxx",
    "clientSecret": "xxxxxxxxx",
    "priority": 1,
    "proxyUrl": "socks5://proxy.example.com:1080"
  },
  {
    "kiroApiKey": "ksk_xxx",
    "authMethod": "api_key",
    "priority": 2
  }
]
```

多凭据特性：

- 按 `priority` 排序
- 可混用 social / idc / api_key
- 单凭据最多重试 3 次，单请求最多 9 次
- 自动故障转移；多凭据格式下 Token 刷新后回写源文件

### Region 配置

**Auth Region**：`凭据.authRegion` > `凭据.region` > `config.authRegion` > `config.region`  
**API Region**：`凭据.apiRegion` > `config.apiRegion` > `config.region`

### 代理配置

**优先级**：`凭据.proxyUrl` > `config.proxyUrl` > 无代理

| 凭据 `proxyUrl` | 行为 |
|-----------------|------|
| 具体 URL | 使用该代理 |
| `direct` | 强制直连（忽略全局代理） |
| 未配置 | 回退全局代理 |

### 认证方式

客户端访问本服务：

1. `x-api-key: sk-your-api-key`
2. `Authorization: Bearer sk-your-api-key`

`apiKeys` 中的 `name` 会作为日志/统计的 `caller`。

### 环境变量

```bash
RUST_LOG=debug ./kiro-rs

# 注入最高优先级 Kiro API Key 凭据（不是客户端 apiKey）
KIRO_API_KEY=ksk_your_kiro_api_key ./kiro-rs
```

## API 端点

### 标准端点 (/v1)

| 端点 | 方法 | 描述 |
|------|------|------|
| `/v1/models` | GET | 可用模型列表 |
| `/v1/messages` | POST | Anthropic Messages（Claude Code 等） |
| `/v1/messages/count_tokens` | POST | Token 估算 |
| `/v1/responses` | POST | OpenAI Responses（Codex 等） |

### Claude Code 兼容端点 (/cc/v1)

| 端点 | 方法 | 描述 |
|------|------|------|
| `/cc/v1/messages` | POST | 缓冲模式，校正准确 `input_tokens` |
| `/cc/v1/messages/count_tokens` | POST | 同 `/v1` |

> `/v1/messages` 实时流式，`input_tokens` 可能为估算值；  
> `/cc/v1/messages` 等上游结束后用 `contextUsageEvent` 校正，期间每 25s ping 保活。

### OpenAI Responses 端点

`POST /v1/responses` 面向 Codex（`wire_api = responses`）。

支持要点：

- `input`：string / array（message、function_call、function_call_output、custom_tool_call…）
- `tools`：function / custom / tool_search / namespace；`web_search` 等 server 工具会被安全丢弃
- Codex Lite `additional_tools` 输入项会合并进工具列表
- `previous_response_id` 多轮历史（磁盘存储，默认目录 `data/responses`，约 30 天 TTL）
- 流式事件：`response.created` / `response.output_text.delta` / `response.function_call_arguments.delta` / `response.custom_tool_call_input.*` / `response.completed` 等

Codex 配置示例（`~/.codex/config.toml`）：

```toml
model_provider = "kiro-rs"
model = "gpt-5.6-luna"

[model_providers.kiro-rs]
name = "kiro-rs"
base_url = "http://127.0.0.1:8990/v1"
wire_api = "responses"
# env_key 或在客户端里配置 api key
```

### Thinking 模式

Claude extended thinking：

```json
{
  "model": "claude-opus-4-8",
  "max_tokens": 16000,
  "thinking": {
    "type": "enabled",
    "budget_tokens": 10000
  },
  "messages": []
}
```

adaptive + effort（**Sonnet / Opus / GPT**；**Haiku 不支持 effort，不会发送该字段**）：

```json
{
  "model": "claude-opus-4-8",
  "max_tokens": 16000,
  "thinking": { "type": "adaptive" },
  "output_config": { "effort": "high" },
  "messages": []
}
```

上游字段映射：

| 模型 | additionalModelRequestFields |
|------|------------------------------|
| GPT-5.6-luna | `{ "reasoning": { "effort": "high" } }` |
| Claude Sonnet / Opus | `{ "output_config": { "effort": "medium\|high\|..." } }` |
| Claude Haiku | **不发送** |

### 工具调用

完整支持 Anthropic tool use；Responses 路径支持 function / custom tool 及 Codex `additional_tools`。

```json
{
  "model": "claude-sonnet-4-6",
  "max_tokens": 1024,
  "tools": [
    {
      "name": "get_weather",
      "description": "获取指定城市的天气",
      "input_schema": {
        "type": "object",
        "properties": {
          "city": {"type": "string"}
        },
        "required": ["city"]
      }
    }
  ],
  "messages": []
}
```

## 模型映射

`/v1/models` 优先调用 Kiro `ListAvailableModels`（缓存约 30 分钟），失败回退内置列表。  
Claude 客户端 ID 会做新旧格式兼容（`claude-opus-4-8` ↔ `claude-opus-4.8`），**不写死具体版本号列表**。

| 客户端模型 | 上游 modelId | 说明 |
|------------|--------------|------|
| `claude-*-x-y` / `claude-*-x.y` | 对应 `claude-*-x.y` | 版本号格式兼容 |
| `*haiku*` | `claude-haiku-4.5` | 无 thinking effort |
| `gpt-5.6` / `gpt-5-6` | `gpt-5.6-luna` | GPT 默认别名 |
| `gpt-5.6-luna` / `gpt-5.6-sol` / `gpt-5.6-terra` | 同名上游 ID | GPT 5.6 变体 |
| 其他非 Claude/GPT | 不支持 | 返回模型错误 |

上下文窗口：Opus 4.6+ / Sonnet 4.6+ 按 1M 估算，其余约 200K。

## Admin（可选）

当配置了非空 `adminApiKey` 时启用：

- **Admin API**（`adminApiKey` 认证）
  - 凭据 CRUD / 禁用 / 优先级 / 强制刷新 / 余额
  - 负载均衡模式读写
  - 当日请求日志、统计、月度用量
- **Admin UI**：`GET /admin`（需编译前构建 `admin-ui`）

## 上传 GitHub / 发布注意

### 可以上传吗？

**可以**，但**不要**把本地密钥和抓包数据推上去。仓库已 ignore：

- `config.json` / `credentials.json` / `credentials.*`
- `/data`（含 debug_logs、HAR 导出、usage_stats、responses 等）
- `admin-ui/node_modules`、`admin-ui/dist`、`target/`

### 推送前检查清单

```bash
# 1. 确认敏感文件不会被提交
git status
git check-ignore -v config.json credentials.json data/

# 2. 不要提交真实密钥、HAR、debug_logs
# 3. 仅提交源码与 example 配置

git add -A
git status   # 再看一眼
```

### 关于当前 remote

本仓库默认 remote 指向上游 `https://github.com/hank9999/kiro.rs.git`。  
如果你要发布**自己的 fork / 私有仓库**：

```bash
# 推荐：先 fork，或新建自己的仓库后改 remote
git remote rename origin upstream   # 可选：保留上游
git remote add origin git@github.com:<you>/<your-repo>.git
git push -u origin master
```

> 当前分支若显示 `ahead/behind` 上游，**不要盲目 force push 到别人的仓库**。  
> 私有部署建议推自己的 repo；给上游贡献请用 PR。

### CI / Release

`.github/workflows` 已包含多平台构建（macOS / Windows / Linux x64&arm64 / musl）。  
推送到自己的 GitHub 并启用 Actions 后，可自动产出二进制产物。

## 注意事项

1. **凭证安全**：`credentials.json` / `apiKey` / `adminApiKey` 不要提交版本库，生产环境权限尽量 `600`
2. **Token 刷新**：服务会自动刷新过期 Token
3. **相对路径数据**：`data/usage_stats`、`data/responses`、`debugLogDir` 相对进程工作目录
4. **Haiku**：不能设置思考强度；若客户端仍带 `output_config.effort`，本服务会自动忽略
5. **GPT / Opus 4.8**：依赖较新的 Kiro Runtime 协议字段（`agentMode`、effort 等），请保持 `kiroVersion` 较新
6. **WebSearch 工具**：Anthropic 路径下仅单个 `web_search` 工具时走内置转换逻辑；Responses 路径会丢弃 server-side web_search 定义以免上游校验失败

## 项目结构

```
kiro-rs/
├── src/
│   ├── main.rs                 # 入口
│   ├── model/                  # 应用配置 / CLI
│   ├── anthropic/              # Anthropic Messages 兼容层
│   ├── openai/                 # OpenAI Responses 兼容层
│   ├── kiro/                   # Kiro Runtime 客户端 / 协议 / UA
│   ├── admin/                  # Admin API
│   ├── admin_ui/               # 嵌入静态资源路由
│   └── ...
├── admin-ui/                   # 管理前端（构建后嵌入二进制）
├── scripts/
│   └── kiro-rs.service         # systemd 示例
├── config.example.json
├── credentials.example.*.json
├── docker-compose.yml
├── Dockerfile
└── Cargo.toml
```

## 技术栈

- **Web 框架**: [Axum](https://github.com/tokio-rs/axum) 0.8
- **异步运行时**: [Tokio](https://tokio.rs/)
- **HTTP 客户端**: [Reqwest](https://github.com/seanmonstar/reqwest)
- **序列化**: [Serde](https://serde.rs/)
- **日志**: [tracing](https://github.com/tokio-rs/tracing)
- **命令行**: [Clap](https://github.com/clap-rs/clap)
- **静态资源**: [rust-embed](https://github.com/pyros2097/rust-embed)

## License

MIT

## 致谢

本项目的实现离不开前辈的努力:

- [kiro2api](https://github.com/caidaoli/kiro2api)
- [proxycast](https://github.com/aiclientproxy/proxycast)

本项目部分逻辑参考了以上项目，再次由衷感谢！
