# AI Agent QE · AAQE

用自然语言描述要检查的产品或模块。AAQE 从远程主分支创建独立 Git Worktree，通过五次独立的 Codex 调用分析代码、寻找缺陷、编写测试，并在真实 Docker 环境中验证。完成后保存报告和环境，供人工复现。

首版是可以执行的通用工具。产品术语、接口约定、业务预期、镜像及测试环境由被审计项目提供；工具及五个阶段 Skill 不内置具体产品知识。

## 安装

需要 Python 3.11+、Git、已登录的 Codex CLI、Docker Engine / Docker Desktop Linux containers，以及 Docker Compose v2。Codex CLI 需支持 `exec --ephemeral --sandbox read-only --output-schema --output-last-message`。

在本仓库根目录执行一次：

```bash
python -m pip install .
aaqe doctor
```

安装完成后，可切换到任意需要检查的项目目录运行 `aaqe`。这两条命令适用于 Bash 和 PowerShell。

虚拟环境（venv）仅用于隔离 Python 依赖，不是运行 AAQE 的必要条件；已有环境可以直接使用。如果系统 Python 拒绝直接安装，且已安装 pipx，可改用 `pipx install .`，由它自动管理隔离环境。开发工具本身时才需要可编辑安装 `python -m pip install -e .`。

建议先在 Linux 或 WSL2 + Docker 的开发环境使用；原生 Windows 路径与进程终止逻辑尚需单独验收。

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

## 工作流

| Stage | 执行者 | 产物 |
|---|---|---|
| 0 初始化 | Python | 本轮记录、远程基准提交、新分支和 Worktree |
| 1 Code Explorer | 独立 Codex 调用 | 范围、调用关系、行为证据、现有测试、环境需求 |
| 2 Bug Hunter | 独立 Codex 调用 | 有来源依据的缺陷候选及覆盖限制 |
| 3 Test Designer | 独立 Codex 调用 | 场景、预期、目标断言、数据准备和复现条件 |
| 4 Test Developer | 独立 Codex 调用 + Python Patch Gate | 测试补丁、测试清单、补丁检查结果 |
| 5 Test Validator | Python Docker 执行 + 独立 Codex 调用 | 基线、真实用例结果、独立验证结论 |
| 6 报告 | Python | Markdown、JSON 和完整运行记录；保留环境 |
| 7 清理 | 用户显式调用 | 清理本轮拥有的容器、网络和卷；保留报告与 Worktree |

没有候选时在 Stage 2 后生成报告；缺少行为依据或环境条件时保存 blocked 原因。每个到达的 AI 阶段只调用 Codex 一次，不自动进行模型重试。五个 Skill 随 Python 包发布，定义输入、步骤、输出与完成条件。

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
```

源码、参考资料和 Compose 路径默认相对新 Worktree；外部 `env_file` 相对所读取的配置文件解析为绝对路径。配置在创建 Worktree 前从调用目录读取，因此可使用尚未提交的操作配置。私有连接信息放入环境变量或独立 env 文件，不要写入通用 Skill。

其他设置：`codex.command` 是可执行命令数组，默认 `[codex]`；`codex.model` 可覆盖模型；`state_dir` 可覆盖运行目录。相对 `state_dir` 以配置文件所在目录解析。

## Docker 环境约定

项目提供 Compose 文件，使用已有服务镜像。AAQE 为每轮分配唯一 Compose project，通过 `up --wait` 启动选定服务。配置健康检查和必要的真实协议就绪检查，确保服务不仅启动而且可用。

Runner 必须是持续运行的容器。AAQE 自动为它增加：

- 本轮 Worktree 的只读挂载 `/workspace`。
- 本轮执行结果的可写挂载 `/aaqe-results`。
- 配置中的 `runner_workdir`，默认 `/workspace`。

编译缓存和产物写入容器内部可写目录或专用卷。例如 Rust 项目设置 `CARGO_TARGET_DIR=/aaqe-results/build-target`。被测程序必须从本轮 Worktree 构建，或能核实使用同一版本的构建产物。

外部现成容器放入 `external_containers`；AAQE 只检查它们是否运行。通过项目定义的 external network 或宿主机端点连接；不会修改它们的网络，也不会在 cleanup 中删除它们。容器自身的业务就绪检查由项目提供。独立启动的服务应使用项目隔离的网络、卷和测试数据名称；不要指定固定 `container_name`。

最小可运行项目见 `examples/minimal/`。它使用 Python 的现成镜像、标准库测试驱动和 JUnit 输出，无需安装项目特有的测试框架。

## 生产代码保护和证据

Agent 使用 Codex read-only sandbox，只返回结构化结果。Test Developer 返回统一 diff，由 Python 检查后应用。默认允许测试目录；新增测试辅助路径可通过受信任的 `test_paths` 声明。请不要把整个源码目录声明为纯测试目录。

Rust 混合源码文件支持语法树检查：仅可修改顶层 `#[cfg(test)] mod` 测试区，区外语法必须保持一致。其他语言的混合文件先使用独立测试文件。删除、重命名、二进制、符号链接和文件模式修改被拒绝。Validator 独立检查弱化断言、吞掉异常及只验证 mock 的测试。

基线执行时临时撤回生成的测试补丁，执行完恢复。Compose 配置在启动时保存为解析后的快照，因此复现/清理不依赖之后被修改的项目定义。

每个测试必须写出本次运行的 JUnit XML，声明需要匹配的精确 `name` 或 `classname::name`。命令中的 `{result_dir}` 替换为独立的容器输出目录，同时设置 `AAQE_RESULT_DIR`。跳过、未发现、重复身份、报告缺失、错误报告和进程结果不一致均不能支持确认缺陷。完整失败证据仍需 Validator 判断断言和行为依据是否有效。

`confirmed` 表示目标失败证据与验证判断一致，不表示工具提供形式化正确性保证。执行配置、构建脚本和项目扩展工具属于受信任本地代码；Patch Gate 及只读 Agent 不应作为运行恶意仓库的安全边界。

## 报告、复现和清理

默认运行目录为 `~/.aaqe/runs/<run-id>/`：

```text
run.json
worktree/
stages/<stage>/          # prompt、Skill、Schema、输出与进程记录
artifacts/              # 经校验的阶段产物
tests.diff
patch-check.json
environment/            # Compose 快照、所有权、日志
executions/<attempt>/   # 原始 XML、stdout/stderr、执行记录
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

`cleanup` 仅清理该运行拥有的 Compose 资源，保留镜像缓存、外部容器、报告、测试补丁、分支和 Worktree，并同步报告中的清理状态。清理只使用经过完整性检查的 Compose 快照；快照丢失或改变时拒绝执行。需要时由用户另行管理 Worktree 生命周期。同一个运行的复现和清理请顺序执行。

运行文件可能包含私有源代码片段、Compose 解析后的环境值和测试日志，保存在本地私有运行目录，不自动提交或上传。

## 开发与验证

```bash
python -m pip install -e '.[dev]'
python -m pytest -q
python -m ruff check .
python -m build
```

普通测试使用真实临时 Git 仓库，以及明确标记的脚本化 Codex / Docker 边界替身。真实 Docker 集成测试需显式启用，使用现成的 `python:3.12-slim` 镜像：

```bash
AAQE_DOCKER_TESTS=1 python -m pytest -q tests/test_docker_smoke.py
```

这项测试验证真实容器、补丁、JUnit、保留、复现和清理；模型输出是确定性测试夹具，不代表真实 Codex 已审计某个业务产品。测试创建的临时环境会在验证后显式清理。真实产品的 AI 审计还需在对应仓库配置环境后运行。
