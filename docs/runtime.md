# AAQE 运行机制与环境配置

[English overview](../README.md) · [中文概览](../README.zh-CN.md)

本文保留调度、Docker、MCP、证据和清理的详细约定。安装、公开命令和配置字段总表见 README。

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
tmux attach-session -t aaqe-20261125081514-ab12cd34
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

运行命令时所在目录的可选 `aaqe.yaml` 可以复用已知的操作信息；不会向上搜索仓库根目录，也不会在安装时自动生成。也可以用 `--config PATH` 指定文件。根目录模板见 [aaqe.yaml.example](../aaqe.yaml.example)，最小项目示例见 [examples/minimal/aaqe.yaml](../examples/minimal/aaqe.yaml)。审计意图仍来自提示词。没有配置时，Explorer 尝试从仓库发现环境及测试入口。缺少可执行环境时生成 blocked 报告，不能凭空创建业务约定。

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

项目提供 Compose 文件或由主调度提交 inline Compose，优先使用符合项目版本要求的服务与 Runner 镜像。AAQE 为每轮分配唯一 Compose project。存在安装步骤时先启动容器并安装依赖，再通过 `up --wait` 和真实协议 readiness 检查就绪。依赖安装使用容器内的软件包管理器，版本遵循项目清单、锁文件和构建配置；安装命令应非交互、可重复执行。

Runner 必须是持续运行的容器。AAQE 自动为它增加：

- 本轮 Worktree 的只读挂载 `/workspace`。
- 本轮执行结果的可写挂载 `/aaqe-results`。
- 配置中的 `runner_workdir`，默认 `/workspace`。

编译缓存和产物写入容器内部可写目录或专用卷。例如 Rust 项目设置 `CARGO_TARGET_DIR=/aaqe-results/build-target`。被测程序必须从本轮 Worktree 构建，或能核实使用同一版本的构建产物。

外部现成容器放入 `external_containers`；AAQE 只检查它们是否运行。通过项目定义的 external network 或宿主机端点连接；不会修改它们的网络，也不会在 cleanup 中删除它们。容器自身的业务就绪检查由项目提供。独立启动的服务应使用项目隔离的网络、卷和测试数据名称；不要指定固定 `container_name`。

最小可运行项目见 [examples/minimal/](../examples/minimal/)。它使用 Python 的现成镜像、标准库测试驱动和 JUnit 输出，无需安装项目特有的测试框架。

## 提前准备环境与独立重跑

主调度还可使用 `inspect_environment`、`prepare_environment` 和 `execute_tests` 三个 MCP 工具。它们只在审计运行期间可用；不是新的 CLI 子命令。

`inspect_environment({"probe":{"kind":"tools","tools":["java","mvn"]}})` 默认在本轮 Runner 内检查工具版本；`kind: endpoint` 配合 `url` 从 Runner 内发起 HTTP(S) HEAD 探测，可以检查 Compose 网络内的服务。证据包含 `target` 和 `runner_service`。Runner 尚未准备时返回准备要求；readiness 失败但容器仍在运行时可以继续探测，不回退到宿主机。`target: host` 可显式检查调度端能力；`kind: docker` 和 `kind: ssh_inventory` 在宿主机执行。工具清单只接受内置允许的可执行名称和固定版本参数，不运行任意 shell。缺少宿主机 Maven 不构成测试阻塞。阶段会收到环境、安装配方、就绪标志和探测结果。

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
    "setup": [],
    "readiness": [["python", "--version"]]
  }
}
```

上例仅演示协议；实际镜像、版本与 readiness 应来自目标项目。`compose` 接受对象或编码了对象的 JSON 字符串；模型 Schema 使用字符串表示任意 Compose 对象。AAQE 将其写入运行目录的 `environment/compose.generated.json`，不会写入产品 Worktree。Runner 仍挂载只读源码与可写结果目录。Compose `up --wait` 完成后，harness 在 Runner 内逐项执行 `readiness` argv；可使用项目提供的真实协议探测命令。

`environment.setup` 是安装依赖的 argv 数组列表，在本轮拥有的 Runner 中以用户 `0` 执行；readiness、基线和目标测试继续使用服务配置的用户。编译、缓存及 fixture 输出放在可写容器目录或专用卷，源码挂载仍为只读。安装步骤必须独立于生成的测试，因为基线会临时撤回测试补丁。

主调度发现遗漏依赖时，可在同一轮自动调用 `prepare_environment` 追加安装步骤，例如以下 Python 项目示例：

```json
{"setup":[["python","-m","pip","install","--disable-pip-version-check","pytest==8.3.5"]]}
```

实际包名和版本来自被测项目。重复 argv 去重保存；若之前的安装命令本身有误，传入 `replace_setup: true` 和完整修正配方，包含重建容器所需的全部依赖。配方保存在 `environment/dependency-setup.json`，有效配方最多 32 条，每条最多 64 个参数，每个参数最多 8192 字节且不可含 NUL。

安装过程按 `environment/setup-<index>/<attempt>/` 保存进程记录和日志，失败结果携带证据目录及 stdout/stderr 摘要供主调度诊断。追加或替换配方会清除当前执行和 Validator 结论，历史 attempt 保留；安装、readiness 成功后重跑受影响测试，再独立验证。安装失败也保留配方和容器，允许在预算内纠正后继续。普通依赖缺失触发自动修复；凭据不可用、网络持续不可达、版本无法满足或预算耗尽时保存具体 blocked 原因。

实际重新准备环境时（包括不传入新配方的失败重试）也会清除旧执行及验证结论。每条安装命令执行前保存 `environment/setup-in-progress.json`；失败或超时后停止本轮 Runner，终止容器内安装进程。停止操作使用独立的 10 秒恢复期限，结果保存在 `environment/setup-stop/`；若停止失败，恢复标记保留，拒绝继续探测或测试，下一次准备先恢复停止操作再启动。AAQE 在安装中退出后，也通过该标记恢复。普通测试命令需包含首次执行及重跑所需的 fixture 准备；`reproduction_commands` 仅补充独立复现的准备步骤。

创建所有权快照前可修正环境定义。存在 `environment/ownership.json` 或环境已启动后，Compose 定义及初始设置锁定，即使启动或 readiness 失败也不能替换；容器内依赖修复通过上述顶层 `setup` 参数进行。对相同定义再次调用 `prepare_environment` 可恢复启动、安装和就绪检查；已 ready 且没有提交安装步骤时重复调用不再启动服务。需更换已拥有资源的 Compose 定义时保存本轮并新开审计。

Designer 的 `blocked_reason` 表示缺少可靠 oracle 或无法实现测试。可实现但缺少服务、编译器等运行条件时，使用 `execution_blocked_reason`，Developer 仍可生成测试。独立的单元、集成与 E2E 场景使用不同 test ID。全部计划都因设计原因受阻时，Validator 可在没有 Developer 的情况下独立检查并给出 blocked 结论；可实现计划完成审计仍需要 Developer。

接受补丁后自动执行保持兼容。前置条件恢复后可调用 `execute_tests({"test_ids":["TEST_ID"]})` 只重跑指定测试，省略 `test_ids` 则重跑全部已接受测试，不消耗新 Developer 调用。空列表、重复或未知 ID 在执行前拒绝。实际新一轮会清除当前 Validator 和 verdict；选中轮次仅含选中测试，必须重新独立验证，历史记录保留。

Developer 的每个测试可包含 `baseline` argv 数组，仅检查自身依赖。非空的受信任项目全局 `baseline` 优先，失败阻塞整批；否则使用测试自己的 baseline，未提供或为 null 时退回 Explorer baseline。显式 `[]` 表示不运行 baseline，不能作为健康基线支持确认。每条基线有独立 ID，作用于单个测试的记录包含 `for_test_id`。其失败跳过该目标，继续其他独立目标；补丁恢复失败则停止整轮并保留恢复日志。基线撤回补丁的保护机制保持有效。

SSH 探测仅用于只读容器清单，例如 `inspect_environment({"probe":{"kind":"ssh_inventory","host":"inventory.example.invalid","user":"audit","identity_file":"/private/audit_key"}})`。使用已经验证的 `known_hosts`；SSH 禁用用户配置、转发与本地命令，默认批处理密钥认证。密码认证改为传入 `password_env` 的变量名，不能与 `identity_file` 同时使用；密码通过临时 askpass 环境传递并从已保存的输出中脱敏。通用文档、提示词和测试不保存真实主机或凭据。远端主机不是 Docker 部署目标；环境后端仍为本机 Docker Compose。

每次探测共享最多 60 秒预算（同时受本轮剩余时间与 Docker 超时约束）。工具版本探测每次接受 1–16 个名称。每个探测进程的 stdout/stderr 分别在内存捕获和落盘时限制为 65,536 字节，加截断标记；SSH 密码脱敏在持久化前完成。`result.success` 表示命令健康情况，外层 action `status: completed` 仅表示探测操作已记录；预算不足时 `budget_exhausted` 保留已有证据。Compose 启动、安装步骤和 readiness 共享操作期限；各命令按整秒超时，允许小于一秒的取整偏差。日志收集和补丁恢复保留其证据/恢复职责。

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
run.json                # 兼容输出；当前运行以 harness.json 为准
harness.json            # 带版本的权威快照：状态、有效产物、执行和判定
operation.lock          # 同一运行的操作互斥锁；保留文件以维持锁的身份
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
reproductions/<id>/     # 独立复现的状态、准备步骤及部分结果记录
execution.json          # 当前执行的兼容输出；轮次历史通过路径关联
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

自定义目录使用 `aaqe --state-dir /path/to/runs report RUN_ID` 等形式。报告结束不停止或删除环境；失败和中断也保留已创建的资源。复现先恢复 Runner、重放保存的依赖安装并检查就绪，再执行测试准备和目标测试，不重新调用 AI。每次复现先登记尝试，按步骤保存准备结果、已完成用例和失败原因；环境启动失败也有记录，不覆盖原始审计证据和验证结论。复现前会检查 Worktree 仍处于记录的基准提交，并拒绝正在初始化、运行或等待外层收尾的审计。新运行的 `audit_finalized` 仅在补丁恢复和最终状态保存后置为 true；旧记录缺少该字段时继续兼容。当前运行从快照中的有效 Developer 产物选择测试，兼容别名文件缺失或损坏不影响选择。

当前格式的 `report` 从快照重新生成 JSON/Markdown，报告缺失或损坏时也可重建。读取旧 Rust 快照时兼容导入之前只写入 `run.json` 的复现和清理记录；没有快照的旧 Python 运行继续使用原格式。快照仅保存核心数据，可调用阶段和恢复动作在查询时计算。

安装、readiness、测试和阶段进程的 stdout/stderr 完整流式写入日志文件；日志读写失败会将执行标记为失败，同时继续排空子进程输出。进程记录每个流保留最多 65,536 字节的首尾摘要及截断标记，MCP 和阶段输入中的日志进一步缩减为摘要。需要解析的 Git/Compose 配置和状态输出继续完整捕获。阶段按角色选择上游产物、已完成尝试和最近环境动作，Developer 重试仍收到实际执行反馈；完整产物和日志可通过记录的路径读取。

`cleanup` 清理该运行拥有的 tmux 会话和 Compose 资源，保留镜像缓存、外部容器、报告、阶段日志、测试补丁、分支和 Worktree，并同步报告中的清理状态。即使在创建 Docker 环境前已结束或受阻，也能清理该轮 tmux 会话；重复执行可安全跳过已删除的会话。审计仍在运行时拒绝清理。Docker 清理只使用经过完整性检查的 Compose 快照；快照丢失或改变时拒绝删除 Docker 资源，已经完成的 tmux 清理状态仍会保存。报告生成失败不会阻止 Docker 清理，已完成的清理状态会保存并返回报告错误。需要时由用户另行管理 Worktree 生命周期。同一运行的 MCP、复现、清理和报告操作通过文件锁互斥；遇到正在执行的操作会返回 busy，请在其结束后重试。

运行文件可能包含私有源代码片段、Compose 解析后的环境值和测试日志，保存在本地私有运行目录，不自动提交或上传。


新运行编号使用 `Asia/Shanghai` 的 `YYYYMMDDHHmmss-xxxxxxxx`；时间记录包含 `+08:00` 偏移。主 tmux 会话为 `aaqe-<run-id>`，Git 分支为 `aaqe/<run-id>`，Docker Compose project 为 `aaqe-<run-id>`。历史名称继续按原值读取和清理。每轮 Markdown 报告使用中文，并在开头汇总当前候选判定；JSON 字段和状态枚举、原始日志及命令保持原值。
