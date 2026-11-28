# AAQE

**AI Agent Quality Engineering** — 用自然语言描述产品功能或模块。AAQE 调度 Codex 编写场景文档、操作真实产品、追查异常，最终交付操作步骤、执行证据和独立验证报告。

[English](README.md) · 简体中文

```text
测试需求 → 理解产品 → 设计场景 → 执行操作 → 复现问题 → 独立验证
                              ↓
                    steps.md + 执行证据 + report.md/report.json
```

一次 `aaqe run` 就是一次测试任务。AAQE 围绕指定功能展开正常、异常和边界场景，并按功能需要检查状态变化、权限、数据正确性与跨入口行为。SQL、API、Explorer、CLI 中适用的入口都要分别验证。Explorer 场景实际操作浏览器页面，只有 API 返回不能证明页面可用。源码和文档用于理解预期、定位异常；开始测试不需要先找到疑似代码缺陷。

协调器保留五个阶段：

1. `product-explorer`：理解功能，确认适用入口、参考资料和所需服务。
2. `scenario-designer`：先将场景、操作步骤和预期结果写入 `steps.md`。
3. `product-executor`：执行已保存的操作，检查真实返回结果，按需继续浏览器或交互会话。
4. `issue-reproducer`：追查异常，记录可复现的问题。发现本轮范围内的新线索时，先补写场景文档再执行；范围外线索记录后留待后续处理。
5. `result-reviewer`：逐项核对当前场景和问题的证据、真实服务及业务预期；进程退出码本身不能证明产品行为正确。

场景修订保留历史和来源。修改场景只使该场景的旧证据失效，未变更场景的证据继续保留。AAQE 保护产品代码，保留执行证据和测试环境，方便复现。新任务以文档和验证结果为交付，不设代码覆盖率门槛，也不自动生成测试 MR。

**必须使用真实服务，禁止 mock。** 每个服务优先使用任务提示词明确指定的地址；未指定地址的服务必须通过 Docker 启动真实实例。同一任务可以同时使用外部 TDengine 和 Docker 中的源端、目标端。指定地址不可用时，相关场景受阻，不得静默替换为本地服务。两种部署方式都通过持久 Docker runner 执行操作，保留真实协议检查，以及 Docker 服务的容器、镜像身份。mock、stub、spy、伪造客户端或服务器、替代传输和响应回放均不能替代产品验证。

## 安装

**仅支持 Linux**，预编译版本提供 x86_64：

```bash
curl -fsSL https://raw.githubusercontent.com/qevolg/ai-agent-qe/main/install.sh | bash
aaqe doctor
```

脚本将最新 Release 二进制安装到 `~/.local/bin` 并验证 SHA-256，不安装其他工具；需要仓库已发布包含二进制和校验文件的 Release。通过 `bash -s -- --version VERSION` 或 `--bin-dir DIRECTORY` 指定版本或安装目录。若安装目录不在 PATH 中，脚本会提示添加方式。

宿主机需要 Git、tmux、已登录的 Codex CLI、Docker Engine 和 Compose v2。产品依赖及操作工具在 Docker 内运行，页面验证还需提供浏览器工具。发布历史审计的测试补丁时，另需通过 `glab auth login --hostname YOUR_GITLAB_HOST` 登录，并配置 Git 提交身份。

也可使用 Rust **1.100.0+** 从源码安装：

```bash
git clone https://github.com/qevolg/ai-agent-qe.git
cd ai-agent-qe
cargo install --path . --locked
```

## 使用

在产品仓库目录运行：

```bash
aaqe run "验证导入功能，覆盖正常导入、异常恢复、重复写入和边界输入，分别测试适用的 SQL、API、Explorer 和 CLI 入口"
```

AAQE 自动选择当前分支的远端（否则优先 `origin`，再使用唯一远端），获取该远端的默认分支并创建独立 Worktree；本地未提交的产品改动不参与测试。终端会打印 tmux 连接命令，可随时查看主调度过程。

使用已有服务时，在提示词中写明地址：

```bash
aaqe run "使用 https://db.example:6041 上的 TDengine 验证查询功能，其他所需服务通过 Docker 启动。覆盖合法、非法和边界查询，并测试所有适用入口"
```

结束后输出报告位置。`steps.md` 保存操作步骤文档，`report.md` 汇总全部场景并单列问题结论，`report.json` 提供 schema version 2.0 的结构化结果和原始证据路径。场景结果区分通过、失败、受阻、未执行和不稳定。报告分别记录服务来源、地址、实际观测版本及源码提交。任务中断时保留已有证据，缺失的执行不能计为通过。

## 配置

配置可省略。AAQE 读取运行命令所在目录的 `aaqe.yaml`，也可通过 `--config PATH` 指定。

```yaml
codex:
  model: gpt-6.1-sol
```

默认模型为 `gpt-6.1-sol`；将 `model` 设为 `""` 可沿用 Codex 的模型配置。Git 目标、容器环境和参考资料可按需覆盖，见[配置示例](aaqe.yaml.example)。外部服务地址须在任务提示词中明确指定，保留用户选择该地址的依据。基线命令、测试路径和发布配置继续供历史测试生成审计使用。无法唯一确定 Git 目标时，AAQE 会提示配置 `git.remote` 或 `git.branch`。

执行限制由程序内置：每次阶段 Agent 和操作命令最多两小时，单次环境操作一小时，整轮任务十二小时，环境健康等待默认十分钟。

## 查看与复现

```bash
aaqe report RUN_ID
aaqe reproduce RUN_ID
aaqe cleanup RUN_ID
```

每次任务独立保存在 `~/.aaqe/RUN_ID/`，包含步骤文档、报告、日志、执行证据和历史记录。旧版 `~/.aaqe/runs/RUN_ID/` 中的记录仍可访问。复现重用已保存的环境和操作，不再次调用 AI；追加复现记录并保留原始独立验证结论。清理移除本轮拥有的容器和 tmux 会话，保留报告、文档与 Worktree；外部服务继续由原所有者管理。

历史审计保持原有报告和测试复现行为。`aaqe publish RUN_ID` 继续支持发布历史记录中的测试补丁；产品场景任务不自动创建测试 MR。

[MIT 许可证](LICENSE)
