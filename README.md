# AI Agent QE · AAQE

用自然语言描述要检查的产品或模块。AAQE 从远程主分支创建独立 Git Worktree，由一个常驻的主 Codex TUI 调度五种阶段角色，每次阶段调用都启动新的独立后台 Codex 进程，分析代码、寻找缺陷、编写测试，并在真实 Docker 环境中验证。完成后保存报告和环境，主 TUI 继续运行，供人工追问和复现。

首版是可以执行的通用工具。产品术语、接口约定、业务预期、镜像及测试环境由被审计项目提供；工具及五个阶段 Skill 不内置具体产品知识。

## 安装

AAQE 现在使用 Rust 实现，运行时需要 Git、tmux、已登录的 Codex CLI、Docker Engine / Docker Desktop Linux containers，以及 Docker Compose v2。Codex CLI 需支持交互入口的 `--no-alt-screen --no-daemon`，以及 `exec --ephemeral --sandbox read-only --output-schema --output-last-message`。在 Debian/Ubuntu 上可用 `sudo apt-get install tmux` 安装 tmux。

在本仓库根目录执行一次：

```bash
cargo install --path . --locked
aaqe doctor
```

安装完成后，可切换到任意需要检查的项目目录运行 `aaqe`。

从源码安装需要 Rust 1.88+ 和 Cargo；确保 `~/.cargo/bin` 在 `PATH` 中。也可以执行 `cargo build --release --locked`，将 `target/release/aaqe` 复制到 `PATH` 中的目录。可执行文件内嵌五个阶段的角色、Skill 和 Schema，可以单独复制运行，工具本身无需 Python 或虚拟环境。示例产品及审计目标的测试仍使用各自的语言和依赖。

旧 Python 版用户可卸载原先的包 `python3 -m pip uninstall ai-agent-qe`，再安装 Rust 版，并用 `command -v aaqe` 检查实际入口。现有 `aaqe.yaml`、运行目录及 JSON/Markdown 报告格式继续沿用，已有运行记录可通过 `report`、`reproduce` 和 `cleanup` 使用。

请在 Linux 或 WSL2 + Docker 的开发环境运行。Windows 用户需在 WSL2 内安装并运行 AAQE、tmux 和 Codex；原生 Windows 不支持此 tmux 启动方式。

## 输入提示词

在要审计的 Git 仓库中执行：

```bash
aaqe
```

输入一段需求，例如“检查当前产品的数据导入模块，重点验证错误恢复和重复写入，补充可复现的测试”。也可以直接运行：

```bash
aaqe run "审计整个产品，优先检查任务生命周期及各模块之间的异常传播"
```

无需 `--target`，无需预定义产品名称。Explorer 根据提示词和真实仓库结构确定范围，报告列出实际检查及未覆盖区域。

默认获取 `origin/main`，创建 `aaqe/<run-id>` 分支和独立 Worktree。当前分支与未提交修改不会带入审计，也不会被更改。每轮记录本次获取的提交，后续远程更新不影响复现。网络失败不会静默退回旧代码。

## 主调度与阶段执行

| Stage | 执行者 | 产物 |
|---|---|---|
| 0 初始化 | Rust | 本轮记录、远程基准提交、新分支和 Worktree |
| 1 Code Explorer | 独立 Codex 调用 | 范围、调用关系、行为证据、现有测试、环境需求 |
| 2 Bug Hunter | 独立 Codex 调用 | 有来源依据的缺陷候选及覆盖限制 |
| 3 Test Designer | 独立 Codex 调用 | 场景、预期、目标断言、数据准备和复现条件 |
| 4 Test Developer | 独立 Codex 调用 + Rust Patch Gate | 测试补丁、测试清单、补丁检查结果 |
| 5 Test Validator | 独立 Codex 调用 | 根据真实执行证据形成独立验证结论 |
| 6 报告 | Rust | Markdown、JSON 和完整运行记录；保留环境 |
| 7 清理 | 用户显式调用 | 清理本轮拥有的容器、网络、卷和 tmux 会话；保留报告与 Worktree |

以上数字表示阶段角色，不是固定执行顺序。初始化完成后，一个主 Codex TUI 在本轮唯一的 tmux 会话内持续运行，通过本地 MCP harness 的 `status`、`run_stage`、`inspect_environment`、`prepare_environment`、`execute_tests`、`finish` 工具取得结果并决定下一步。阶段使用后台 `codex exec`，不创建 tmux 会话。Rust 不代替模型选择阶段，负责检查依赖、预算、补丁和证据；每次接受 Developer 补丁后执行真实基线与目标测试，再将执行结果反馈给主窗口。

主窗口可以重试阶段、回到上游重新分析、没有候选时提前结束，或说明阻塞原因。重试必须重新开启独立 Codex，不能复用之前阶段的对话。每次调用带上主窗口指令、当前有效产物、执行证据和历史尝试记录。五个阶段的 Skill、角色和 Schema 继续内嵌在二进制中。

重新接受上游产物后，旧的下游有效产物、执行和验证结论失效，历史 attempt 文件仍保留。Developer 重试提交相对于本轮原始基准的完整替换补丁；补丁被拒绝时恢复上一份测试，接受后重新跑基线和测试。切换前保存恢复日志，中断后以已保存的 harness 状态恢复补丁和有效产物。Explorer 的源码引用必须在本轮基准提交中存在，生成的测试通过历史 attempt 传递。产品失败应作为证据保留；只有证据表明测试或 fixture 有误时才修正测试，不能为了通过而弱化断言或修复生产代码。

结束必须显式调用 `finish`。`completed` 要求当前 Explorer、Hunter 产物；有候选时还要求独立 Validator 和前置产物。模型只输出“已完成”不会使审计完成。缺少行为依据或执行条件时保存 blocked 原因；候选也可单独为 blocked。

## 查看 Codex 会话

每轮只有主调度在 tmux 中启动真实的交互式 `codex` TUI。AAQE 在终端打印主会话的连接命令，例如：

```bash
tmux attach-session -t aaqe-0123456789ab
```

在同一主机、同一用户的另一个终端执行该命令即可直接查看 Codex TUI、输入消息和使用界面操作。标准输入、输出和错误均直接连接 tmux 的终端。已在 tmux 内时可用 `tmux switch-client -t SESSION` 切换。使用 tmux 的 prefix 后按 `d` 分离，Codex 会继续运行；默认 prefix 为 `Ctrl+b`，以自己的 tmux 配置为准。

Codex CLI 需要支持 stdio MCP 及服务工具授权配置；本次验证版本为 `0.172.0`。

首次在未获 Codex 信任的仓库运行时，TUI 可能显示目录信任页。请 attach 主会话，核对目录后按 Codex 界面操作继续；AAQE 不代替用户保存目录信任。已信任的仓库可直接进入审计。登录或其他 Codex 初始化提示也可在同一 TUI 内处理，等待时间计入本轮审计超时。

主窗口记录在 `stages/coordinator/`，其中 `tmux.json` 保存会话名和完整 attach 命令，`terminal.log` 实时保存终端输出（含界面控制序列），`tools.jsonl` 保存 harness 调用记录。主 TUI 退出后才写入其 `process.json` 和 `finished`；它们与审计完成状态分开。

阶段 Codex 是独立的后台非交互进程，每次重试都启动新进程。首次阶段调用保留 `stages/<role>/` 路径，第二次开始使用 `stages/<role>/attempts/0002/` 等目录。阶段不生成 `tmux.json`；`stdout.log`、`stderr.log` 在运行中持续写入，`process.json` 保存最终退出码与超时状态，结构化输出和历史尝试继续保留。

主窗口与阶段进程的权限策略均为 `read-only` 和 `approval_policy="never"`。主窗口通过经过依赖、补丁和证据检查的 MCP `finish` 完成审计后，外层 AAQE 生成报告并返回，主 TUI 继续等待输入，且不再受本轮 `loop.timeout` 限制。可以追问保存的结果；已结束的审计不能继续调用阶段或再次结束。

审计期间主窗口超时或在 AAQE 终端按 `Ctrl+C` 时，会停止主调度和仍在运行的后台阶段进程，并保存部分报告。单个阶段超时则将失败结果反馈给主窗口，并保留日志。直接关闭主 TUI 或删除其 tmux 会话也会取消正在运行的阶段。tmux 会话属于当前主机，主机或 tmux 服务重启后请查看已保存的日志。

第七阶段执行 `aaqe cleanup RUN_ID` 会关闭主 TUI 并删除本轮记录的 tmux 会话及其窗口，保留已写入磁盘的日志；旧版运行记录中的阶段会话也兼容清理。清理会核对会话的目录归属，不影响其他运行或用户自己的 tmux 会话；已手动删除的会话会跳过。也可通过 `tmux kill-session -t SESSION` 提前删除指定会话。

## 项目配置是可选的

根目录的 `aaqe.yaml` 可以复用已知的操作信息；审计意图仍来自提示词。没有配置时，Explorer 尝试从仓库发现环境及测试入口。缺少可执行环境时生成 blocked 报告，不能凭空创建业务约定。

```yaml
git:
  remote: origin
  branch: main

references:
  - docs/interfaces.md

environment:
  compose_files:
    - tests/compose.yaml
  services:
    - runner
  runner_service: runner
  runner_workdir: /workspace
  readiness_timeout: 180
  external_containers: []
  # env_file: /private/project-test.env

baseline:
  - [python, run_tests.py, tests/test_baseline.py, "{result_dir}/baseline.xml"]

test_paths:
  - tests/**
  - "**/tests/**"
  - .aaqe/tests/**

timeouts:
  agent: 1800
  test: 1200
  docker: 1200

loop:
  max_stage_calls: 20
  max_stage_attempts: 4
  timeout: 14400
```

源码、参考资料和 Compose 路径默认相对新 Worktree；外部 `env_file` 相对所读取的配置文件解析为绝对路径。配置在创建 Worktree 前从调用目录读取，因此可使用尚未提交的操作配置。私有连接信息放入环境变量或独立 env 文件，不要写入通用 Skill。

`loop.timeout` 是审计从主调度启动到成功调用 `finish` 的秒数上限，不限制审计结束后的 TUI 交互时间；`timeouts.agent` 限制每次阶段调用。次数预算耗尽后只能结束并说明原因，不能继续启动阶段进程。

其他设置：`codex.command` 是可执行命令数组，默认 `[codex]`；`codex.model` 可覆盖模型；`state_dir` 可覆盖运行目录。相对 `state_dir` 以配置文件所在目录解析。

## Docker 环境约定

项目提供 Compose 文件或由主调度提交 inline Compose，使用已有服务镜像。AAQE 为每轮分配唯一 Compose project，通过 `up --wait` 启动选定服务。配置健康检查和必要的真实协议就绪检查，确保服务不仅启动而且可用。

Runner 必须是持续运行的容器。AAQE 自动为它增加：

- 本轮 Worktree 的只读挂载 `/workspace`。
- 本轮执行结果的可写挂载 `/aaqe-results`。
- 配置中的 `runner_workdir`，默认 `/workspace`。

编译缓存和产物写入容器内部可写目录或专用卷。例如 Rust 项目设置 `CARGO_TARGET_DIR=/aaqe-results/build-target`。被测程序必须从本轮 Worktree 构建，或能核实使用同一版本的构建产物。

外部现成容器放入 `external_containers`；AAQE 只检查它们是否运行。通过项目定义的 external network 或宿主机端点连接；不会修改它们的网络，也不会在 cleanup 中删除它们。容器自身的业务就绪检查由项目提供。独立启动的服务应使用项目隔离的网络、卷和测试数据名称；不要指定固定 `container_name`。

最小可运行项目见 `examples/minimal/`。它使用 Python 的现成镜像、标准库测试驱动和 JUnit 输出，无需安装项目特有的测试框架。

## 提前准备环境与独立重跑

主调度还可使用 `inspect_environment`、`prepare_environment` 和 `execute_tests` 三个 MCP 工具。它们只在审计运行期间可用；不是新的 CLI 子命令。

`inspect_environment({"probe":{"kind":"tools","tools":["java","mvn"]}})` 保存宿主机工具版本证据；`kind: docker` 检查本地 Docker/Compose；`kind: endpoint` 配合 `url` 发起 HTTP(S) HEAD 探测。工具清单只接受内置允许的可执行名称和固定版本参数，不运行任意 shell。缺少宿主机 Maven 不代表 Runner 里没有 Maven，也不代表无法设计测试。阶段会收到已记录的环境、就绪标志和探测结果。

`prepare_environment` 可在 Explorer 或 Developer 之前调用。可从项目文件发现环境，也可显式提交以下定义；项目 `aaqe.yaml` 中的环境字段优先于显式参数，显式参数优先于 Explorer 推断。Compose 来源替换时清除继承的另一种来源，同一份定义不能同时提供非空 `compose_files` 和 `compose`。

```json
{
  "environment": {
    "compose": {
      "services": {
        "runner": {
          "image": "python:3.12-slim",
          "command": ["sleep", "infinity"],
          "environment": {"PYTHONDONTWRITEBYTECODE": "1"}
        }
      }
    },
    "services": ["runner"],
    "runner_service": "runner",
    "runner_workdir": "/workspace",
    "readiness_timeout": 90,
    "readiness": [["python", "--version"]]
  }
}
```

上例仅演示协议；实际镜像、版本与 readiness 应来自目标项目。`compose` 接受对象或编码了对象的 JSON 字符串；模型 Schema 使用字符串表示任意 Compose 对象。AAQE 将其写入运行目录的 `environment/compose.generated.json`，不会写入产品 Worktree。Runner 仍挂载只读源码与可写结果目录。Compose `up --wait` 完成后，harness 在 Runner 内逐项执行 `readiness` argv；可使用项目提供的真实协议探测命令。

创建所有权快照前可修正环境定义。存在 `environment/ownership.json` 或环境已启动后，定义锁定，即使启动或 readiness 失败也不能替换。对相同定义再次调用 `prepare_environment` 可恢复启动/就绪检查；已 ready 时重复调用不再启动服务。需更换已拥有资源的定义时保存本轮并新开审计。

Designer 的 `blocked_reason` 表示缺少可靠 oracle 或无法实现测试。可实现但缺少服务、编译器等运行条件时，使用 `execution_blocked_reason`，Developer 仍可生成测试。独立的单元、集成与 E2E 场景使用不同 test ID。全部计划都因设计原因受阻时，Validator 可在没有 Developer 的情况下独立检查并给出 blocked 结论；可实现计划完成审计仍需要 Developer。

接受补丁后自动执行保持兼容。前置条件恢复后可调用 `execute_tests({"test_ids":["TEST_ID"]})` 只重跑指定测试，省略 `test_ids` 则重跑全部已接受测试，不消耗新 Developer 调用。空列表、重复或未知 ID 在执行前拒绝。实际新一轮会清除当前 Validator 和 verdict；选中轮次仅含选中测试，必须重新独立验证，历史记录保留。

Developer 的每个测试可包含 `baseline` argv 数组，仅检查自身依赖。非空的受信任项目全局 `baseline` 优先，失败阻塞整批；否则使用测试自己的 baseline，未提供或为 null 时退回 Explorer baseline。显式 `[]` 表示不运行 baseline，不能作为健康基线支持确认。每条基线有独立 ID，作用于单个测试的记录包含 `for_test_id`。其失败跳过该目标，继续其他独立目标；补丁恢复失败则停止整轮并保留恢复日志。基线撤回补丁的保护机制保持有效。

SSH 探测仅用于只读容器清单，例如 `inspect_environment({"probe":{"kind":"ssh_inventory","host":"inventory.example.invalid","user":"audit","identity_file":"/private/audit_key"}})`。使用已经验证的 `known_hosts`；SSH 禁用用户配置、转发与本地命令，默认批处理密钥认证。密码认证改为传入 `password_env` 的变量名，不能与 `identity_file` 同时使用；密码通过临时 askpass 环境传递并从已保存的输出中脱敏。通用文档、提示词和测试不保存真实主机或凭据。远端主机不是 Docker 部署目标；环境后端仍为本机 Docker Compose。

每次探测共享最多 60 秒预算（同时受本轮剩余时间与 Docker 超时约束）。工具版本探测每次接受 1–16 个名称。每个进程的 stdout/stderr 分别在内存捕获和落盘时限制为 65,536 字节，加截断标记；SSH 密码脱敏在持久化前完成。`result.success` 表示命令健康情况，外层 action `status: completed` 仅表示探测操作已记录；预算不足时 `budget_exhausted` 保留已有证据。Compose 启动链和 readiness 共享操作期限；各命令按整秒超时，允许小于一秒的取整偏差。日志收集和补丁恢复保留其证据/恢复职责。

阶段调用预算与环境操作预算分别使用 `loop.max_stage_calls` 上限；环境操作预算统计 `environment_actions` 加 `execution_attempts`，自动执行也消耗一次准备操作和一次执行尝试。`status.allowed_stages` 只列当前可调度角色，`stages` 列全部角色，`stage_availability` 说明具体门槛，`recovery` 给出可用动作。遭拒调度记在 `dispatch_rejections`，不计作已启动 worker；先修复前置条件或让上游修订计划，再重试。

## 生产代码保护和证据

Agent 使用 Codex read-only sandbox，只返回结构化结果。Test Developer 返回统一 diff，由 Rust 检查后应用。默认允许测试目录；新增测试辅助路径可通过受信任的 `test_paths` 声明。请不要把整个源码目录声明为纯测试目录。

Rust 混合源码文件支持语法树检查：仅可修改顶层 `#[cfg(test)] mod` 测试区，区外语法必须保持一致。其他语言的混合文件先使用独立测试文件。删除、重命名、二进制、符号链接和文件模式修改被拒绝。Validator 独立检查弱化断言、吞掉异常及只验证 mock 的测试。

基线执行时临时撤回生成的测试补丁，执行完恢复。Compose 配置在启动时保存为解析后的快照，因此复现/清理不依赖之后被修改的项目定义。同一轮保持该环境定义；启动后需要更换环境时新建审计，避免遗留旧环境资源。

每个测试必须写出本次运行的 JUnit XML，声明需要匹配的精确 `name` 或 `classname::name`。命令中的 `{result_dir}` 替换为独立的容器输出目录，同时设置 `AAQE_RESULT_DIR`。跳过、未发现、重复身份、报告缺失、错误报告和进程结果不一致均不能支持确认缺陷。完整失败证据仍需 Validator 判断断言和行为依据是否有效。

`confirmed` 表示目标失败证据与验证判断一致，不表示工具提供形式化正确性保证。执行配置、构建脚本和项目扩展工具属于受信任本地代码；Patch Gate 及只读 Agent 不应作为运行恶意仓库的安全边界。

## 报告、复现和清理

报告按候选列出计划的设计/执行阻塞原因、已生成测试及当前轮次实际执行证据，同时保留环境动作、执行轮次和遭拒调度；这些记录与已启动 worker 次数分开。旧运行缺少新字段时按空历史读取。

默认运行目录为 `~/.aaqe/runs/<run-id>/`：

```text
run.json
harness.json            # 持久化当前状态、有效产物和证据
worktree/
stages/coordinator/     # 主 TUI 输入、提示词、tmux.json、terminal.log、tools.jsonl
stages/<stage>/         # 首次调用的 input、prompt、Skill、Schema、输出与进程记录
stages/<stage>/attempts/0002/  # 第二次及后续独立调用，含 result 和执行证据
artifacts/              # 当前有效产物；accepted.json 留在各 attempt 中
tests.diff
patch-check.json
environment/            # Compose 定义/快照、所有权、readiness 日志
environment/probes/     # 有界探测的 process 与 probe.json
executions/<attempt>/   # 原始 XML、stdout/stderr、执行记录
executions/rounds/<id>/  # 显式重跑的轮次 execution.json
execution.json
report.md
report.json
```

```bash
aaqe report RUN_ID
aaqe env status RUN_ID
aaqe env shell RUN_ID
aaqe reproduce RUN_ID --case TEST_ID
aaqe cleanup RUN_ID
```

自定义目录使用 `aaqe --state-dir /path/to/runs report RUN_ID` 等形式。报告结束不停止或删除环境；失败和中断也保留已创建的资源。复现直接使用保存的测试与准备步骤，不重新调用 AI。每次复现使用新目录，不覆盖首次失败证据；报告同步追加复现记录，保留首次验证结论。复现前会检查 Worktree 仍处于记录的基准提交。

`cleanup` 清理该运行拥有的 tmux 会话和 Compose 资源，保留镜像缓存、外部容器、报告、阶段日志、测试补丁、分支和 Worktree，并同步报告中的清理状态。即使在创建 Docker 环境前已结束或受阻，也能清理该轮 tmux 会话；重复执行可安全跳过已删除的会话。审计仍在运行时拒绝清理。Docker 清理只使用经过完整性检查的 Compose 快照；快照丢失或改变时拒绝删除 Docker 资源，已经完成的 tmux 清理状态仍会保存。需要时由用户另行管理 Worktree 生命周期。同一个运行的复现和清理请顺序执行。

运行文件可能包含私有源代码片段、Compose 解析后的环境值和测试日志，保存在本地私有运行目录，不自动提交或上传。

## 开发与验证

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

普通 Rust 测试使用真实临时 Git 仓库、真实 tmux 会话，以及明确标记的脚本化 Codex / Docker 边界替身。测试需要 tmux。真实 Docker 集成测试需显式启用，使用现成的 `python:3.12-slim` 镜像；该测试的确定性模型夹具需要宿主机上的 `python3`：

```bash
cargo test --locked --test workflow real_docker_workflow_reproduction_and_owned_cleanup -- --ignored --nocapture
```

这项测试分别验证现有文件 Compose 和提前 inline Compose 两种流程，包括真实容器、readiness、只读源码/可写结果挂载、补丁、基线、选中重跑、JUnit、保留、复现和清理；模型输出是确定性测试夹具，不代表真实 Codex 已审计某个业务产品。测试创建的临时环境会在验证后显式清理。真实产品的 AI 审计还需在对应仓库配置环境后运行。
