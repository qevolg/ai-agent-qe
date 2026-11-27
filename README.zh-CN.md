# AAQE

**AI Agent Quality Engineering** — 用自然语言描述产品或模块。AAQE 调度 Codex 分析代码、添加测试，在 Docker 中验证问题，并将测试提交为被审计项目的 GitLab MR。

[English](README.md) · 简体中文

```text
审计需求 → 分析代码 → 寻找问题 → 设计测试 → 编写测试 → 独立验证
                                  ↓
                      Docker 执行证据 + 模块覆盖率 + GitLab MR
```

AAQE 保护生产代码，保留执行证据和测试环境，方便复现。测试沿用项目已有规范，不限定语言、测试框架或报告格式。现有测试与新增测试合计须达到被审计模块生产代码 **80% 的行覆盖率**。

## 安装

**仅支持 Linux**，预编译版本提供 x86_64：

```bash
curl -fsSL https://raw.githubusercontent.com/qevolg/ai-agent-qe/main/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
aaqe doctor
```

脚本下载 Release 二进制并验证 SHA-256，不安装其他工具；需要仓库已发布对应 Release。通过 `bash -s -- --version VERSION` 或 `--bin-dir DIRECTORY` 指定版本或安装目录。

宿主机需要 Git、tmux、已登录的 Codex CLI、Docker Engine 和 Compose v2。被审计项目的依赖、构建和测试在 Docker 内执行。发布 GitLab MR 还需要已登录的 `glab` 和 Git 提交身份。

也可使用 Rust **1.88+** 从源码安装：

```bash
git clone https://github.com/qevolg/ai-agent-qe.git
cd ai-agent-qe
cargo install --path . --locked
```

## 使用

在被审计项目的仓库目录运行：

```bash
aaqe run "检查导入模块，重点关注异常恢复和重复写入"
```

AAQE 获取 `origin/main`，创建独立 Worktree；本地未提交的产品改动不参与审计。终端会打印 tmux 连接命令，可随时查看主调度过程。

结束后输出报告位置和 GitLab MR 链接。揭示产品缺陷的测试失败会作为证据保留。覆盖率不足或无法采集时，结果标记受阻并创建 Draft MR。发布失败会保留本地成果，支持重试。

## 配置

配置可省略。AAQE 读取运行命令所在目录的 `aaqe.yaml`，也可通过 `--config PATH` 指定。

```yaml
git:
  remote: origin
  branch: main

codex:
  model: gpt-6.1-sol

timeouts:
  agent: 7200
  test: 7200
  docker: 3600

loop:
  timeout: 43200
```

以上均为默认值：每次阶段 Agent 和测试命令最多两小时，单次环境操作一小时，整轮审计十二小时，环境健康等待默认十分钟。项目专用的构建、环境和测试入口由主调度发现。

更多选项见[带注释的配置示例](aaqe.yaml.example)和[配置参考](docs/configuration.md)，包括容器、路径、证据和 GitLab 认证。

## 查看与复现

```bash
aaqe report RUN_ID
aaqe reproduce RUN_ID
aaqe publish RUN_ID
aaqe cleanup RUN_ID
```

报告、日志、覆盖率和发布状态保存在 `~/.aaqe/runs/RUN_ID/`。复现重用已保存的环境和命令，不再次调用 AI。清理移除本轮拥有的容器和 tmux 会话，保留报告与测试 Worktree。

[开发与运行参考](docs/configuration.md#development) · [MIT 许可证](LICENSE)

## 继续审计与运行期间升级

当外层命令输出 `blocked` 后，可以连接打印出的 tmux 会话，解决具体阻塞，
然后在原 Codex 对话中输入“继续，从 test-validator 开始”。协调器会调用
harness 的 `resume` 工具，在原会话中继续执行。

也可以从另一终端运行：

```bash
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID --from-stage test-validator
```

阶段值为 `code-explorer`、`bug-hunter`、`test-designer`、`test-developer`、
`test-validator`，不提供阶段时选择下一可执行角色。原协调器仍存活时向原对话
发送继续指令；原协调器已退出时才启动替代 TUI。恢复使用保存的配置、原提交和
Worktree，不重新拉取源码。旧检查点归档到 `resumes/`，所有 attempt、原始证据
继续保留，新一轮单独计算预算，不计入人工等待时间。重新分析或修改测试会使相关
下游结论失效；Validator 可以复用仍匹配的执行证据，证据缺失时重新执行测试。
安全策略拒绝仍是终止条件。
原命令返回后，恢复中的 TUI 若超时、崩溃或关闭，独立监控进程会终结该轮次并更新报告。
修改已接受测试后再次发布，会在原发布分支追加提交并更新同一个 MR。
异常终结会先保存本地结果，不等待远程发布；超时后可用 `aaqe publish RUN_ID` 重试交付。

新 Run 会把 AAQE 复制为 `RUN_DIR/runtime/aaqe`，保存版本、SHA-256 和路径，
所有内部进程都使用这份固定执行器。安装器先校验完整的临时文件、刷盘，再原子替换
安装入口。新安装影响后续新 Run，正在运行的 Run 继续使用原版本。
如需让已有 Run 明确采用新安装的修复版本：

```bash
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID --from-stage test-validator --takeover
```

接管会检查状态格式和执行器协议兼容性，新增一份执行器，保留旧副本。新版 MCP relay
会将后续请求交给新执行器，TUI 无需重启。旧版协调器没有 relay，且终端处于只读沙箱，
无法在原 TUI 中采用新执行器。检测到仍存活的旧协调器时，恢复命令会在修改 Run 前拒绝，
请保留原对话历史、退出旧协调器 TUI，再次执行恢复命令，在同一 Run 上启动替代 TUI。
旧版 Run 会在这次恢复时建立第一份快照和 relay。仅替换安装文件不会修复已启动的旧 MCP。

“仅修改测试代码”约束的是被审计项目。测试可以位于任意路径，也可以内嵌于生产源文件。
Developer 和独立 Validator 逐项审查测试语义，AAQE 检查路径安全、基线提交及已接受补丁
的一致性。设计阶段根据实际行为考虑正常、边界、异常输入、生命周期、并发、依赖故障、
精度、转义和标识冲突等情况，并说明不适用的维度。
