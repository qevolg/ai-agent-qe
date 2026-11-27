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
