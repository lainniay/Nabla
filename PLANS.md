# Nabla Goal 硬删除实施计划

> 目标对象：Coding Agent
>
> 基线：`https://github.com/lainniay/Nabla` 当前 `main` 分支
>
> 变更性质：破坏性删除；不保留兼容层
>
> 唯一目标：从 Nabla Core 中移除所有 Goal 相关实现、协议、状态、UI、持久化和测试，并同步修复测试体系

---

## 1. 任务定义

本任务只做一件事：**彻底移除 Nabla 中所有 Goal runtime 能力和 Goal 专用抽象。**

删除完成后，Nabla Core 不得再理解或持有以下概念：

- Goal；
- Goals 列表；
- GoalSpec；
- GoalTask；
- GoalStore；
- Goal 生命周期；
- Goal 审批；
- Goal 自动调度；
- Goal planner、worker、verifier、reviewer 固定流水线；
- Goal repair loop；
- Goal capability lease；
- Goal 专用 permission 上下文；
- Goal 专用 worktree 恢复关联；
- Goal RPC command；
- Goal Host event；
- Goal Rust state；
- Goal TUI modal、卡片和命令；
- Goal 持久化与迁移；
- Goal protocol fixture；
- Goal 测试。

将来如需提供类似 Goal 的行为，只允许通过 Skill 或 Prompt 表达。**本次变更不创建任何 Goal Skill、Prompt 或替代实现。**

---

## 2. 强制范围约束

### 2.1 本次必须完成

1. 删除 TypeScript Host 中的全部 Goal 类型、存储和编排逻辑。
2. 删除 Goal 对普通子代理的字段和行为污染。
3. 删除 Goal 对 permission 的 lease、grant、approval 上下文和判定分支。
4. 删除 Goal 对 worktree metadata、恢复、集成和冲突流程的关联。
5. 删除 Host 协议中的 Goal commands、events 和 payload 字段。
6. 删除 Rust 中的 Goal model、AppState、AppEffect、CommandEvent、HostEvent 和 UI。
7. 删除 `/goal`、`/goals` 本地命令。
8. 删除 Goal transcript item 和渲染代码。
9. 删除 Goal 持久化、迁移和旧格式恢复代码。
10. 同步删除 Goal-only 测试并更新所有受影响的非 Goal 测试。
11. 保证现有 coding-agent 核心路径继续通过测试：普通会话、Plan、子代理、permission、worktree、集成和恢复。
12. 最终执行全仓 Goal 符号扫描，确认 Core 中归零。

### 2.2 本次明确禁止

Coding Agent 不得在本任务中做以下工作：

- 不实现 Goal-lite；
- 不实现 session completion condition；
- 不新增 `/goal` Skill；
- 不新增 Goal Prompt；
- 不把 Goal 改名为 `WorkflowRun`、`ManagedRun`、`Automation`、`Campaign` 或其他名称；
- 不引入统一 Task Runtime；
- 不持久化 child session；
- 不实现 `yield_task`；
- 不重构 Plan；
- 不修改 Plan 生命周期；
- 不重构 HostBridge 的整体架构；
- 不重写 TUI；
- 不统一 modal 系统；
- 不实现 event sourcing；
- 不实现新的 sandbox；
- 不改变 permission 的一般行为；
- 不改变 worktree 的一般行为；
- 不添加新的 agent profile；
- 不删除仍可独立使用的通用 agent profile；
- 不做与 Goal 删除无关的格式化、重命名、文件移动或代码清理；
- 不增加 Goal 兼容字段；
- 不保留 `goal: null`；
- 不保留 deprecated Goal command；
- 不增加 Goal 数据迁移器；
- 不在启动时自动删除用户旧状态文件。

### 2.3 允许的最小伴随修改

仅允许为完成 Goal 删除而进行以下伴随修改：

- 将被错误放在 Goal 文件中的非 Goal 类型移动到正确模块；
- 删除 Goal 字段后修正构造函数、序列化结构和测试 fixture；
- 删除 Goal 分支后清理 unused imports、unused helper 和 dead code；
- 为消除 Goal 命名而对仍需保留的通用 helper 做最小重命名；
- 更新测试断言，使其验证删除后的协议与行为；
- 修复编译器、Clippy 和 TypeScript 报告的直接关联错误。

---

## 3. 完成后的目标边界

Goal 删除后，Core 只保留已有通用能力：

```text
Pi Session
├── Plan
├── Tool Calls
├── Generic Subagents
├── Permission Approval
└── Worktree Integration
```

Core 不应出现如下关系：

```text
Plan → Goal → GoalTask → Subagent
```

普通子代理不得再包含以下 Goal 来源字段：

```text
goalId
goal_id
taskId      # 当前实现中用于 GoalTask 关联的字段
task_id     # 当前实现中用于 GoalTask 关联的字段
```

注意：这里删除的是当前 Goal 专用 `taskId/task_id`。本次不得将其重命名为新的通用运行 ID。未来如需通用 Task ID，应由独立设计任务重新引入。

---

## 4. 总体执行原则

### 4.1 采用纵向切除

Goal 已穿透 TypeScript、Rust、协议、权限、worktree、UI 和测试。不得只删除 `GoalStore` 后依靠注释、空对象或兼容分支维持编译。

正确顺序：

1. 记录基线；
2. 解除 Goal 对共享模型的耦合；
3. 删除 TypeScript Goal Core；
4. 删除 Host 调度和权限/worktree Goal 分支；
5. 删除协议面；
6. 删除 Rust 状态和 TUI；
7. 同步测试；
8. 全仓归零扫描；
9. 执行完整回归。

### 4.2 不保留双协议期

Rust TUI 与 TypeScript Host 在同一仓库同步修改，因此本任务不需要：

- 同时接受新旧 bootstrap；
- 同时接受带 Goal 和不带 Goal 的 agent snapshot；
- 返回 deprecated warning；
- 返回空 Goal payload；
- 协议版本分支。

更新后，旧 Goal command 应自然落入现有 unknown/unsupported command 处理。

### 4.3 测试与实现同一提交链完成

禁止通过以下方式临时过关：

- `skip`；
- `ignore`；
- 注释失败测试；
- 删除整个混合测试文件；
- 降低 assertion；
- 只运行局部测试而不运行全量测试。

Goal-only 测试应删除。非 Goal 测试因类型或 fixture 变化受到影响时，应同步更新并继续验证原有行为。

---

## 5. 阶段 0：建立基线和删除清单

### 5.1 更新工作树

执行：

```bash
git status --short
git fetch origin
git switch main
git pull --ff-only
git switch -c refactor/remove-goal
```

若工作树已有用户修改，不得覆盖、stash、reset 或清理。停止并报告冲突文件。

### 5.2 记录基线测试

在修改前执行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets

cd agent-host
npm run typecheck
npm test
cd ..
```

记录：

- 哪些测试在基线已失败；
- Node、npm、Rust、Cargo 版本；
- 当前分支与 commit；
- 测试命令退出码。

不要为修复与 Goal 无关的基线失败扩大本任务范围。

### 5.3 生成 Goal 引用清单

执行以下扫描，将结果保存到临时文件供删除时核对：

```bash
rg -n --hidden \
  -g '!target/**' \
  -g '!agent-host/node_modules/**' \
  -g '!.git/**' \
  'GoalStore|GoalRecord|GoalSnapshot|GoalsSnapshot|GoalTask|GoalSpec|GoalApproval|goalId|goal_id|goal_state|goals_state|goal_start|goal_action|goal_approve|goal_spec_ready|goal_review|goal_error|goal_lease|goalWorkerPermissions' \
  . > /tmp/nabla-goal-symbols-before.txt || true

rg -ni --hidden \
  -g '!target/**' \
  -g '!agent-host/node_modules/**' \
  -g '!.git/**' \
  '\bgoals?\b' \
  agent-host/src src protocol-fixtures AGENTS.md SUBAGENTS.md TRANSCRIPT_SURFACE.md test.md \
  > /tmp/nabla-goal-words-before.txt || true
```

再执行：

```bash
wc -l /tmp/nabla-goal-symbols-before.txt
wc -l /tmp/nabla-goal-words-before.txt
```

这两个文件只用于本地核对，不提交到仓库。

### 5.4 阶段完成条件

- 基线测试结果已记录；
- Goal 引用清单已生成；
- 未修改任何源文件；
- 已确认没有用户未处理的工作树冲突。

---

## 6. 阶段 1：解除共享模型中的 Goal 耦合

本阶段只删除 Goal 专用字段和语义，不删除整个 GoalStore。目的在于先把普通子代理、permission 和 worktree 从 Goal 生命周期中分离，避免后续误删通用能力。

### 6.1 `agent-host/src/main.ts`：清理普通子代理结构

定位 `ActiveSubagent`、公开 agent snapshot、subagent options 和 `delegate_task` schema。

删除：

```ts
taskId?: string;
goalId?: string;
```

从以下路径同步删除：

- `ActiveSubagent`；
- `PublicSubagent` 或 agents snapshot payload；
- `SubagentOptions`；
- `delegate_task` tool parameter schema；
- tool 参数解析；
- subagent 创建；
- subagent state/event payload；
- completed subagent record；
- integration prompt payload；
- recovery record；
-日志和 diagnostics。

删除逻辑：

```text
当 delegate_task 接收到 taskId 时查 GoalTask
由 taskId 推导 goalId
根据 GoalTask 覆盖 profile 或 prompt
```

不得用 `runId`、`workItemId` 或其他字段替换。

### 6.2 删除 `goal_spec` 输出分支

在子代理运行与最终结果解析中删除：

```text
outputKind: "goal_spec"
goalSpecFromToolParams(...)
normalizeGoalSpec(...)
Goal planner 专用 system instruction
Goal planner 最终 JSON 解析
```

处理 `outputKind`：

1. 使用 `rg` 查找所有非 Goal 调用点；
2. 若删除 `goal_spec` 后 `outputKind` 仍有非 Goal 用途，保留该字段及非 Goal 分支；
3. 若 `outputKind` 仅剩固定单值且无行为价值，删除字段；
4. 若 `review` 输出分支只被 Goal reviewer 使用，则随 Goal 一并删除；
5. 不得为了保留结构而制造新的通用 output kind。

### 6.3 `agent-host/src/harness.ts`：中和通用 profile 文案

保留仍可通过普通 `/agent` 或 delegation 使用的通用 profiles，例如：

- planner；
- worker；
- verifier；
- reviewer。

只删除 Goal 专用文案：

- “goal”；
- “goal task”；
- “capability lease”；
- “goal plan”；
- 固定 Goal 阶段说明。

例如：

```text
旧：Work within its capability lease...
新：Work only within the configured tools and permissions...
```

```text
旧：Review independently against the goal, plan...
新：Review the assigned task, supplied evidence, and resulting changes...
```

若 helper 名为：

```ts
goalWorkerPermissions()
```

且 worker profile 仍使用该 helper，则做最小重命名：

```ts
writeAgentPermissions()
```

不得改变权限内容。

### 6.4 `agent-host/src/approval.ts`：删除 Goal approval 上下文

搜索并删除：

```ts
goalId?: string;
```

同步更新：

- approval request 构造；
- approval queue；
- approval 序列化；
- approval diagnostics；
- approval tests/fixtures；
- Rust 对应 payload。

保留一般字段：

```text
approvalId
toolCallId
sessionId
workspaceId
intent
proposedScope
```

### 6.5 Permission：删除 Goal lease 和 Goal task grants

在 `agent-host/src/permissions/**`、`agent-host/src/main.ts` 和相关配置解析中删除所有 Goal 专用权限路径：

- Goal capability lease；
- Goal task grants；
- `goalId` 匹配；
- `taskId` 匹配；
- active Goal lookup；
- Goal path boundary；
- Goal repair grants；
- Goal-specific deny/ask reason；
- `goal_lease` 配置值或 proposal scope；
- 子代理根据 GoalTask grants 授权。

普通子代理授权应只依赖现有一般机制：

```text
profile policy
normalized permission intent
session/workspace grant
user approval
```

删除 Goal 后不得改变一般 permission evaluator、digest binding、once/session/workspace grant、audit 或 TOCTOU 检查。

### 6.6 `agent-host/src/worktree.ts`：删除 Goal 恢复关联

在 `WorktreeRecoveryState` 删除：

```ts
taskId?: string;
goalId?: string;
```

同步删除：

- recovery metadata 写入；
- recovery metadata 读取后的 Goal 恢复行为；
- patch capture 中的 Goal 关联；
- integration/discard 中的 Goal task 更新；
- conflict 中的 Goal 状态更新；
- resolver 中传播 Goal ID；
- Goal path grant validation。

保留 worktree 的所有一般能力：

- prepare；
- shared workspace fallback；
- dirty baseline；
- capture；
- changed paths；
- patch hash；
- apply；
- discard；
- conflict；
- recovery；
- prune；
- credential-like path protection。

旧 JSON metadata 中可能残留 `goalId`、`taskId`。不得新增 Goal 迁移器。只依赖现有宽容 JSON/结构解析忽略未知字段；若当前 parser 因未知字段失败，只允许做一般性的未知字段容忍，不得写 Goal 专用 fallback。

### 6.7 阶段测试

执行：

```bash
cd agent-host
npm run typecheck
npm test -- --test-name-pattern='permission|approval|worktree|subagent|agent'
cd ..
```

若 npm script 不支持附加参数，直接执行：

```bash
cd agent-host
npm run typecheck
npm test
cd ..
```

### 6.8 阶段完成条件

- 普通 subagent model 不再含 `goalId/taskId`；
- approval payload 不再含 `goalId`；
- worktree recovery state 不再含 `goalId/taskId`；
- permission 不再读取 GoalStore 或 GoalTask grants；
- `goal_spec` 输出分支已删除；
- 普通子代理、permission、worktree 测试仍可运行；
- 未引入替代运行抽象。

---

## 7. 阶段 2：删除 TypeScript Goal 类型、存储与持久化

### 7.1 `agent-host/src/harness.ts`：删除 Goal 数据模型

删除 Goal 专用类型。包括但不限于：

```text
GoalStage
TaskStatus                 # 仅 GoalTask 使用时
ReviewVerdict              # 仅 Goal reviewer 使用时
GoalTask
GoalSpec
GoalSpecInput
GoalRecord
GoalSnapshot
GoalsSnapshot
GoalSourcePlan
GoalReview
GoalVerification
CapabilityLease            # 仅 GoalStore 使用时
```

对下列类型执行引用确认：

```text
TaskResult
VerificationEvidence
ReviewFinding
CapabilityGrantSet
```

规则：

1. 使用 `rg` 确认是否有非 Goal 调用方；
2. 只有 Goal 使用时直接删除；
3. 有普通子代理、permission 或 profile 使用时保留在原位置或最小移动到已有合适模块；
4. 不创建新的 domain layer；
5. 不为了保存 Goal 结构而重命名成 Workflow 类型。

### 7.2 删除 Goal 状态迁移

删除：

```text
GOAL_TRANSITIONS
TASK_TRANSITIONS
Goal stage validation
Goal task transition validation
Goal dependency graph validation
Goal completion aggregation
repair cycle counting
review verdict handling
```

### 7.3 删除 `GoalStore`

完整删除：

```text
GoalStoreOptions
GoalStore
```

连同所有方法：

- create；
- create from Plan；
- attach session；
- restore；
- persist；
- migrate；
- list；
- snapshot；
- transition Goal；
- transition task；
- approve；
- pause；
- resume；
- cancel；
- block；
- complete；
- set verification；
- set review；
- append repair cycle；
- legacy normalization。

删除 Goal 状态文件路径构造、workspace hash、session state lookup 等仅服务 GoalStore 的 helper。

### 7.4 删除 Goal persistence 与 migration

删除所有对 Goal state 路径的：

- read；
- write；
- atomic replace；
- directory listing；
- legacy migration；
- attach/re-key；
- restart recovery。

不得：

- 删除磁盘上的旧文件；
- 在启动时扫描旧文件；
- 提示用户迁移；
- 写 tombstone；
- 保留 deprecated reader。

旧文件在新版本中应完全无人读取。

### 7.5 清理 exports/imports

删除 `harness.ts` 对外暴露的 Goal exports，并根据 TypeScript compiler 清理：

- unused imports；
- unused JSON helper；
- unused timestamp helper；
- unused path helper；
- Goal-only migration constants；
- Goal-only error classes。

只删除因 Goal 消失而无引用的代码。

### 7.6 阶段测试

执行：

```bash
cd agent-host
npm run typecheck
npm test
cd ..
```

此时 `main.ts` 可能仍有 GoalStore 引用而无法编译。如果阶段 2 和阶段 3 必须连续完成，可将二者作为一个本地提交前工作段；不得提交无法编译的中间状态。

### 7.7 阶段完成条件

- `harness.ts` 不再导出任何 Goal 类型或 GoalStore；
- Goal persistence 与 migration 完全删除；
- 无空壳 Goal class；
- 无 Goal alias；
- 无兼容 reader。

---

## 8. 阶段 3：删除 TypeScript Host 中的 Goal 编排

主要文件：`agent-host/src/main.ts`。

### 8.1 删除 imports

删除所有 Goal imports，例如：

```text
GoalStore
GoalRecord
GoalSnapshot
GoalsSnapshot
GoalTask
GoalSpec
GoalReview
GoalVerification
goalSpecFromToolParams
normalizeGoalSpec
```

### 8.2 删除 `HostBridge` Goal 字段

删除：

```text
goals
goalOperationGeneration
goalPreparationRunning
goalAutomationRunning
```

以及任何：

- Goal timer；
- Goal waiter；
- Goal queue；
- Goal generation counter；
- Goal automation lock；
- Goal cancellation token。

更新 constructor 参数和所有创建调用。

### 8.3 删除 GoalStore bootstrap

删除：

```text
new GoalStore(...)
GoalStore options
Goal state directory setup
Goal restoration during startup
Goal attachment during session activation
```

不得用空 implementation 替换。

### 8.4 删除 Goal host commands

从 command dispatcher 删除：

```text
goal_state
goals_state
goal_start
goal_action
goal_approve
```

删除对应 request payload 类型、validation、response payload 和 error mapping。

删除 command lane 中 Goal 专用 lane，例如：

```text
"goal"
```

若 `command-lanes.test.ts` 只是把字符串 `goal` 当作任意示例 lane，改成无业务含义的 `mutation`，只为保证最终 Goal 词扫描归零；不得改变 command lane 行为。

### 8.5 删除 Goal query/action handlers

删除所有 Goal handler，包括但不限于：

```text
goalSnapshot()
hasMutableGoalTask()
sendGoalState()
startGoal()
goalAction()
approveGoal()
prepareGoal()
validateGoalSpecProfiles()
cancelGoalSubagents()
runGoalExecution()
```

以及所有命名或语义上的：

```text
pumpGoal*
runGoal*
scheduleGoal*
verifyGoal*
reviewGoal*
repairGoal*
completeGoal*
failGoal*
```

不得留下 TODO、空函数或 always-null handler。

### 8.6 删除 Goal fixed pipeline

完整删除：

```text
preparing
awaiting_approval
executing
verifying
reviewing
repairing/completing
```

对应的：

- planner 子代理启动；
- GoalSpec 生成；
- 用户 GoalSpec 审批；
- dependency task scheduling；
- worker 并行调度；
- verifier 自动启动；
- reviewer 自动启动；
- repair cycle；
- targeted repair；
- review verdict aggregation；
- Goal completion determination。

保留普通用户或主代理显式启动的 subagent 能力。

### 8.7 删除 Goal host events

停止发送并删除 payload 构造：

```text
goal_state
goal_spec_ready
goal_review
goal_error
```

删除 Goal revision、Goal generation 和 stale event 处理所需数据。

### 8.8 清理 session 生命周期

从以下流程删除 Goal 行为：

- host bootstrap；
- session new；
- session resume；
- session switch；
- session close；
- socket disconnect；
- host shutdown；
- context compaction；
- catalog refresh。

不得在这些流程中返回 `goal: null`。

### 8.9 清理 subagent completion

从子代理：

- start；
- running；
- completion；
- failure；
- cancellation；
- recovery；
- integration waiting；
- integration applied；
- integration discarded；
- conflict；
- resolver completion

删除所有 GoalTask 更新、Goal stage 更新和 Goal pump 触发。

普通 subagent 完成后仍按现有非 Goal 路径返回结果、更新 agents snapshot 和处理 worktree integration。

### 8.10 清理 permission authorization call site

删除：

```text
this.goals.active(...)
lookup GoalTask grants
Goal-specific approval context
Goal-specific permission reason
```

确认 permission 调用仍能接收普通 session、workspace、tool call 和 profile 上下文。

### 8.11 清理 integration path

删除 integration apply/discard/keep/conflict 中：

- Goal task transition；
- Goal blocked/paused/failed；
- Goal completion pump；
- Goal repair trigger；
- Goal review trigger。

保留一般 integration status、patch apply、discard 和 conflict handling。

### 8.12 阶段测试

执行：

```bash
cd agent-host
npm run typecheck
npm test
cd ..
```

### 8.13 阶段完成条件

- HostBridge 不再拥有 GoalStore；
- Host 不再接受或发送 Goal protocol message；
- 无 Goal automation；
- 无 Goal fixed pipeline；
- 普通 subagent、permission、worktree 仍可通过 TypeScript 测试；
- TypeScript 编译通过。

---

## 9. 阶段 4：删除共享协议和 fixture 中的 Goal

### 9.1 `protocol-fixtures/bootstrap-state.json`

删除顶层完整字段：

```json
"goal": { ... }
```

不是改为：

```json
"goal": null
```

从所有 agent、pending agent、completed agent、pending integration agent 或 recovery object 删除：

```json
"goalId": "..."
"taskId": "..."
```

保留所有非 Goal fixture 数据。

### 9.2 TypeScript protocol type

在 Host bootstrap/session activation 类型中删除：

```text
goal
goals
```

从 agents snapshot type 删除 Goal 专用 IDs。

从 approval、integration、worktree recovery payload 删除 Goal 字段。

### 9.3 Rust protocol type

在 `src/host.rs` 删除：

```rust
BootstrapStateData.goal
SessionActivationData.goal
```

同步删除 imports：

```rust
GoalSnapshot
GoalsSnapshot
```

删除 Host client methods：

```rust
get_goal()
get_goals()
start_goal(...)
goal_action(...)
approve_goal(...)
```

### 9.4 不增加兼容解析

Rust `serde` 和 TypeScript parser 对旧 JSON 的未知字段若已经宽容，则保持现状。不得增加：

- `legacyGoal`；
- optional deprecated Goal field；
- custom Goal migration；
- protocol version branch。

### 9.5 更新协议契约测试

修改 `agent-host/src/protocol-contract.test.ts`：

- 删除所有 `parsed.goal...` 断言；
- 保留对 bootstrap 其他字段的断言；
- 增加明确的负面断言：fixture 顶层没有 `goal`；
- 增加 agent entries 不含 `goalId` 和当前 Goal 专用 `taskId` 的断言；
- 保证 fixture 可由当前 Host parser 读取。

修改 Rust `src/host.rs` 内的 fixture/serde 测试：

- 删除测试 JSON 中的 Goal block；
- 删除 Goal assertion；
- 保留对 session、plan、resources、agents、context 和 integrations 的断言；
- 增加序列化/反序列化结果无 Goal 字段的断言（若现有测试框架适合）。

### 9.6 阶段测试

执行：

```bash
cd agent-host
npm run typecheck
npm test
cd ..

cargo test --all-targets host
```

若 Cargo 不支持按上述方式筛选，执行：

```bash
cargo test --all-targets
```

### 9.7 阶段完成条件

- bootstrap JSON 不含 Goal；
- session activation 不含 Goal；
- agents snapshot 不含 Goal ID/GoalTask ID；
- approval/worktree/integration payload 不含 Goal；
- TypeScript 与 Rust 契约测试同步通过；
- 无新旧协议兼容层。

---

## 10. 阶段 5：删除 Rust Goal model 与 App 状态

### 10.1 处理 `src/state/goals.rs`

该文件除 Goal 类型外还包含非 Goal 的 `AgentProfileSnapshot`。执行顺序：

1. 将 `AgentProfileSnapshot` 原样移动到 `src/state/agents.rs`；
2. 更新 imports；
3. 不改变该类型字段、serde 行为或测试；
4. 删除 `src/state/goals.rs` 整个文件。

不得将 Goal 类型一起移动。

### 10.2 `src/state.rs`

删除：

```rust
mod goals;
pub use goals::*;
```

确认 `AgentProfileSnapshot` 通过 agents 模块继续导出。

### 10.3 `src/state/agents.rs`

从 `ActiveAgentSnapshot` 删除：

```rust
task_id: Option<String>,
goal_id: Option<String>,
```

同步更新 Default、serde fixture、构造和渲染引用。

不得新增替代字段。

### 10.4 `src/state/app_state.rs`

删除字段：

```rust
goal: Option<GoalSnapshot>,
goal_approval: Option<GoalApprovalState>,
```

删除初始化、reset、session activation、bootstrap assignment 和 modal priority 分支。

删除：

```rust
UiModalKind::GoalApproval
```

对应的 active modal 判断。

### 10.5 `src/state/planning.rs`

删除：

```rust
GoalApprovalState
```

只删除 Goal 类型，不修改 Plan review 状态。

### 10.6 `src/state/transcript.rs`

从 `TranscriptItem` 删除：

```rust
Goal(Box<GoalSnapshot>),
Goals(GoalsSnapshot),
```

删除所有 match arms、stable key、measurement、serialization 或 equality 辅助。

### 10.7 阶段测试

执行：

```bash
cargo fmt --check
cargo check --all-targets
```

此时 Rust app 层可能仍引用已删除类型。阶段 5 与阶段 6 可以连续完成后再提交；不得提交无法构建的中间状态。

### 10.8 阶段完成条件

- `src/state/goals.rs` 已删除；
- 非 Goal 的 `AgentProfileSnapshot` 已保留；
- AppState 不含 Goal；
- TranscriptItem 不含 Goal；
- Rust agent snapshot 不含 Goal IDs。

---

## 11. 阶段 6：删除 Rust command、effect、event 和 TUI

### 11.1 `src/command.rs`

删除本地命令定义：

```text
/goal
/goals
```

删除：

```rust
LocalCommandKind::Goal
LocalCommandKind::Goals
LocalCommand::Goal(...)
LocalCommand::Goals
```

删除 parser arms、usage、description、completion 和 routing。

不增加 deprecated 提示。未来动态 Skill/Prompt 是否使用 `/goal` 不属于本任务。

### 11.2 `src/app.rs`

从 `AppEffect` 删除：

```rust
GetGoal,
GetGoals,
StartGoal { ... },
GoalAction(...),
ApproveGoal,
```

从 `LocalCommandCompletion` 删除所有 Goal variants。

删除 bootstrap/session activation 对 `state.goal` 的赋值。

### 11.3 `src/event.rs`

删除 Goal imports。

从 `CommandEvent` 删除：

```text
GoalStateFinished
GoalsFinished
GoalStarted
GoalActionFinished
GoalApproved
```

删除对应 result payload 类型。

### 11.4 `src/runtime.rs`

删除 `AppEffect` dispatch arms：

```text
GetGoal
GetGoals
StartGoal
GoalAction
ApproveGoal
```

删除 Host client 调用和异步 completion 映射。

### 11.5 `src/app/actions.rs`

删除：

- `LocalCommand::Goal` route；
- `LocalCommand::Goals` route；
- `receive_goal()`；
- Goal revision/stale filtering；
- Goal spec approval modal 创建；
- Goal transcript item 插入；
- Goal AppEffect 构造；
- Goal local command completion mapping。

不得修改 Plan 命令路径。

### 11.6 `src/app/command_events.rs`

删除所有 Goal command completion handlers：

```text
GoalStateFinished
GoalsFinished
GoalStarted
GoalActionFinished
GoalApproved
```

删除成功/失败 notice 和 state update。

### 11.7 `src/app/host_events.rs`

删除 HostEvent 处理：

```text
goal_state
goal_spec_ready
goal_review
goal_error
```

删除 Goal stale revision、approval、transcript 和 notice 分支。

不得将 Goal event 映射成 generic Notice。

### 11.8 Goal approval 输入路径

使用：

```bash
rg -n 'GoalApproval|goal_approval|update_goal_approval' src
```

删除所有：

- modal enum variant；
- key handler；
- input route；
- accept/reject effect；
- scene builder branch；
- focus target；
- test fixture。

只删除 Goal modal，不重构其他 modal。

### 11.9 `src/ui/scene.rs`

删除 Goal approval modal 渲染。

保留 Plan review、permission approval、question、integration 等其他 UI。

### 11.10 `src/ui/transcript.rs`

删除：

```rust
TranscriptItem::Goal(...)
TranscriptItem::Goals(...)
```

对应的：

- card renderer；
- list renderer；
- summary；
- status style；
- action hint；
- tests/snapshots。

### 11.11 其他 Rust 文件

执行：

```bash
rg -n 'Goal|goal_|goalId|goal_id|/goal|/goals' src
```

逐条判断并删除所有 runtime/UI/test Goal 引用。

不允许把 Goal 名字改成无意义名称以绕过扫描。

### 11.12 阶段测试

执行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

### 11.13 阶段完成条件

- Rust 无 Goal model；
- Rust 无 Goal command；
- Rust 无 Goal effect/event；
- Rust 无 Goal Host client；
- Rust 无 Goal modal/transcript renderer；
- Cargo 全量构建和测试通过。

---

## 12. 阶段 7：同步删除和更新测试

本阶段不是“测试清理”，而是 Goal 删除的组成部分。测试改动必须与生产代码同步。

### 12.1 TypeScript：`agent-host/src/harness.test.ts`

该文件混合了 GoalStore 与非 Goal harness 测试，不得删除整个文件。

删除所有 Goal-only 测试块，包括：

- GoalStore create；
- Goal transition；
- GoalTask transition；
- Goal dependency graph；
- Plan source to Goal；
- Goal approval；
- Goal review；
- Goal verification；
- Goal repair cycle；
- Goal persistence；
- Goal restart recovery；
- Goal session attach；
- Goal listing；
- legacy Goal migration；
- invalid Goal spec；
- Goal profile validation；
- Goal path grants。

删除对应 fixture builders、imports、temporary state directories 和 helper。

保留并修复非 Goal 测试，例如：

- harness config；
- profile parsing；
- resources；
- trust/path helpers；
- permission policy；
- generic subagent-related helpers。

### 12.2 TypeScript：`agent-host/src/protocol-contract.test.ts`

更新为删除后的 bootstrap contract：

必须继续验证：

- scope/session；
- plan；
- resources；
- agents；
- context；
- integrations；
- warnings。

新增负面断言：

```ts
assert.equal("goal" in parsed, false);
```

对 fixture 中所有 agent entries 验证：

```ts
assert.equal("goalId" in agent, false);
assert.equal("taskId" in agent, false);
```

这里只检查当前被删除的 GoalTask 关联字段。

### 12.3 TypeScript：`agent-host/src/command-lanes.test.ts`

若测试只是用字符串 `goal` 作为任意 lane 名称，将其改为：

```text
mutation
```

不改变测试行为或 lane implementation。

### 12.4 TypeScript：permission/approval tests

更新所有含 `goalId` 的 approval fixture。

删除 Goal lease/grant tests。

保留并确认：

- deny > ask > allow；
- once grant；
- session grant；
- workspace grant；
- digest binding；
- tool/session/workspace binding；
- subagent cannot escalate；
- audit；
- shell planner；
- file adapter。

不得减少一般 permission 覆盖。

### 12.5 TypeScript：worktree tests

只在类型变化需要时更新 fixture：

- 删除 `goalId`；
- 删除 Goal 专用 `taskId`；
- 删除 Goal state update assertion。

必须保留并通过：

- normal repo prepare；
- dirty repo baseline；
- unborn repo；
- staged/untracked；
- parallel patches；
- binary patch；
- apply；
- idempotent integration；
- discard；
- conflict；
- crash recovery；
- corrupt metadata；
- non-Git fallback；
- credential-like path exclusion。

### 12.6 TypeScript：subagent/host tests

删除 Goal orchestration tests。

保留或补齐因 Goal 删除受影响的普通 coding-agent 路径：

1. 直接启动普通 subagent；
2. 主代理调用 `delegate_task`；
3. subagent 正常完成；
4. subagent 失败；
5. subagent 取消；
6. 并发上限；
7. worktree subagent 完成后等待 integration；
8. apply/discard；
9. conflict/recovery；
10. permission request/approve/deny。

只补充因删除 Goal 分支而失去覆盖的现有行为，不设计新 Task Runtime。

### 12.7 Rust：`src/app/tests.rs`

删除所有 Goal-only 测试，包括：

- Goal lifecycle event；
- stale Goal revision；
- cross-Goal event；
- Goal spec approval；
- Goal action；
- Goal bootstrap；
- Goal session activation；
- Goal review；
- Goal error；
- Goal transcript；
- Goal modal/input；
- `/goal` 与 `/goals` command。

同步更新测试 helpers 和 imports。

保留并验证：

-普通输入和 streaming；
- tool call/result；
- session bootstrap；
- session resume/switch；
- Plan mode/review；
- permission modal；
- question modal；
- integration prompt；
- agent picker；
- transcript；
- Unicode/paste/file references。

### 12.8 Rust：`src/host.rs` tests

删除 JSON fixture 中 Goal block和 Goal assertion。

测试必须继续覆盖删除后的 bootstrap/session activation 反序列化。

建议增加精确的序列化负面检查：

```rust
assert!(json.get("goal").is_none());
```

若现有测试只做反序列化，不要为了该断言大规模重写测试；可在已有 serialization 测试中加入。

### 12.9 Rust：UI tests

删除 Goal card、Goal list、Goal approval modal 的 snapshot/scene tests。

不要重录与 Goal 无关的全部快照。只更新受到布局 item 删除直接影响的测试。

### 12.10 测试删除规则

对每个被删除测试，必须能回答：

```text
它验证的行为是否完全属于 Goal？
```

- 是：删除。
- 否：保留非 Goal 部分并改写 fixture。
- 不确定：通过调用链确认，不得直接删除。

### 12.11 阶段完成条件

- 无 skip/ignore；
- 无空测试文件；
- 无 Goal fixture；
- 非 Goal coding-agent 回归覆盖未被削弱；
- TypeScript 和 Rust 全量测试均通过。

---

## 13. 阶段 8：文档和注释清理

### 13.1 扫描范围

扫描：

```text
AGENTS.md
SUBAGENTS.md
TRANSCRIPT_SURFACE.md
test.md
README.md
docs/**
agent-host/src/**
src/**
protocol-fixtures/**
```

### 13.2 删除内容

删除所有关于以下实现的说明：

- `/goal`；
- `/goals`；
- GoalStore；
- GoalSpec；
- GoalTask；
- Goal lifecycle；
- Goal approval；
- Goal automation；
- Goal planner/verifier/reviewer；
- Goal repair；
- Goal permission lease；
- Goal worktree recovery；
- Goal protocol events。

删除过期架构图、命令表、测试说明和注释。

### 13.3 不新增替代文档

本次不得添加：

- Goal Skill 文档；
- Prompt 模板；
- WorkflowRun 设计；
- Task Runtime 设计；
- 新架构提案。

如果现有文档必须解释删除后的事实，只做最小删除或一句事实修正，例如：

```text
Goal workflows are not part of the core runtime.
```

但不要扩写未来方案。

### 13.4 阶段完成条件

- Core 文档不再描述 Goal runtime；
- 命令文档不再列出 `/goal`、`/goals`；
- 不包含替代设计。

---

## 14. 最终符号归零检查

### 14.1 精确符号扫描

执行：

```bash
rg -n --hidden \
  -g '!target/**' \
  -g '!agent-host/node_modules/**' \
  -g '!.git/**' \
  'GoalStore|GoalRecord|GoalSnapshot|GoalsSnapshot|GoalTask|GoalSpec|GoalApproval|goalId|goal_id|goal_state|goals_state|goal_start|goal_action|goal_approve|goal_spec_ready|goal_review|goal_error|goal_lease|goalWorkerPermissions' \
  .
```

期望：**无输出，退出码 1。**

### 14.2 词汇扫描

执行：

```bash
rg -ni --hidden \
  -g '!target/**' \
  -g '!agent-host/node_modules/**' \
  -g '!.git/**' \
  '\bgoals?\b' \
  agent-host/src src protocol-fixtures AGENTS.md SUBAGENTS.md TRANSCRIPT_SURFACE.md test.md README.md docs
```

期望：Core、协议、测试和运行文档无 Goal runtime 引用。

若仓库本身不存在某些路径，删除相应参数后重跑。

对于命中项：

- runtime/test/protocol/docs 中的 Goal：必须删除；
- 第三方 license、不可修改 vendored 内容：记录但不修改；
- `skills/**` 或 `prompts/**`：本次不新增，已有内容也不得因本任务扩展；
- 普通英语“goal”若与 Goal runtime 无关：优先改写为 `objective`、`target` 或删除，以满足“尽可能清空 Goal”；不得改变行为。

### 14.3 文件名扫描

执行：

```bash
find . \
  -path './.git' -prune -o \
  -path './target' -prune -o \
  -path './agent-host/node_modules' -prune -o \
  -iname '*goal*' -print
```

期望：无 Core、测试或 protocol 文件名包含 Goal。

### 14.4 JSON key 扫描

执行：

```bash
rg -n '"goal"\s*:|"goals"\s*:|"goalId"\s*:|"taskId"\s*:' \
  protocol-fixtures agent-host/src src
```

对 `taskId` 命中必须判断：

- 当前 GoalTask 关联：删除；
- 与 Goal 无关的第三方/通用协议字段：不得仅为清零盲删。若存在这种情况，记录其非 Goal 用途。

### 14.5 阶段完成条件

- Goal 核心符号扫描为零；
- Goal runtime 词汇扫描为零；
- 无 Goal 文件名；
- 无 Goal JSON keys；
- 无隐藏的兼容层或 deprecated alias。

---

## 15. 完整验证矩阵

### 15.1 TypeScript 静态验证

```bash
cd agent-host
npm run typecheck
cd ..
```

通过标准：退出码 0，无 TypeScript error。

### 15.2 TypeScript 全量测试

```bash
cd agent-host
npm test
cd ..
```

通过标准：所有现有非 Goal 测试与更新后的契约测试通过，无 skip/only。

检查：

```bash
rg -n '\.(skip|only)\(|describe\.skip|it\.skip|test\.skip' agent-host/src
```

不得因本任务新增 skip/only。

### 15.3 Rust 格式和静态检查

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

通过标准：退出码 0。

### 15.4 Rust 全量测试

```bash
cargo test --all-targets
```

通过标准：所有测试通过，无 ignored 数量因本任务增加。

### 15.5 协议契约

必须验证：

- TypeScript 可解析更新后的 `bootstrap-state.json`；
- Rust 可反序列化同一 fixture；
- bootstrap 不含 Goal；
- session activation 不含 Goal；
- agent snapshot 不含 Goal IDs；
- approval/integration/worktree payload 不含 Goal。

### 15.6 Coding-agent 核心回归

至少运行或通过已有自动化测试覆盖以下路径：

#### 会话

- 新建 session；
- 发送普通 prompt；
- streaming assistant output；
- tool call/tool result；
- resume session；
- switch session；
- bootstrap/reconnect。

#### Plan

- 进入 Plan mode；
- submit Plan；
- Plan review/批准或现有执行路径；
- 退出 Plan mode。

本任务不得改变 Plan 行为。

#### Subagent

- `/agent` 或等价普通入口；
- 主代理 `delegate_task`；
- running/completed/failed/cancelled；
- agents snapshot；
- 并发控制；
-普通结果回传。

#### Permission

- allow；
- deny；
- ask；
- once；
- session；
- workspace；
- approval digest；
- tool/session/workspace binding；
- audit。

#### Worktree

- prepare；
- capture；
- apply；
- discard；
- conflict；
- recovery；
- non-Git fallback；
- dirty workspace；
- credential-like file exclusion。

#### TUI

- bootstrap；
-普通 transcript；
- tool rendering；
- permission modal；
- question modal；
- integration prompt；
- agent view/picker；
- resize/scroll/streaming。

### 15.7 手工 smoke test

若项目现有开发流程支持本地运行，执行一轮最小 smoke：

1. 启动 Nabla；
2. 确认命令列表无 `/goal`、`/goals`；
3. 发送普通代码调查请求；
4. 触发一次只读工具；
5. 触发一次需要审批的操作；
6. 启动普通子代理；
7. 查看子代理状态；
8. 在 Git 仓库中启动 worktree 写代理；
9. 捕获并应用或丢弃 patch；
10. 退出并恢复 session。

不得为 smoke test 实现任何新功能。

---

## 16. 文件级修改清单

以下是当前已知的主要修改范围。Coding Agent 必须在实际分支上通过 `rg` 补齐遗漏，不得只依赖本表。

### 16.1 删除文件

- [ ] `src/state/goals.rs`
  - [ ] 先移动 `AgentProfileSnapshot` 到 `src/state/agents.rs`
  - [ ] 确认文件剩余内容全部为 Goal
  - [ ] 删除文件

### 16.2 TypeScript 生产代码

- [ ] `agent-host/src/harness.ts`
  - [ ] 删除 Goal 类型
  - [ ] 删除 Goal transitions
  - [ ] 删除 GoalStore
  - [ ] 删除 Goal persistence/migration
  - [ ] 清理 profile Goal 文案
  - [ ] 最小重命名 `goalWorkerPermissions`
- [ ] `agent-host/src/main.ts`
  - [ ] 删除 Goal imports/fields/bootstrap
  - [ ] 删除 Goal commands/events
  - [ ] 删除 Goal fixed pipeline
  - [ ] 删除 Goal 与 subagent/permission/worktree/integration 耦合
  - [ ] 删除 `taskId/goalId`
  - [ ] 删除 `goal_spec`
- [ ] `agent-host/src/approval.ts`
  - [ ] 删除 Goal approval context
- [ ] `agent-host/src/worktree.ts`
  - [ ] 删除 recovery `taskId/goalId`
  - [ ] 删除 Goal integration/recovery behavior
- [ ] `agent-host/src/permissions/**`
  - [ ] 删除 Goal lease/grants/scopes
  - [ ] 保留一般 permission 行为
- [ ] `agent-host/src/protocol/**`（若存在相关类型）
  - [ ] 删除 Goal commands/events/payload

### 16.3 TypeScript 测试

- [ ] `agent-host/src/harness.test.ts`
  - [ ] 删除 Goal-only tests/helpers
  - [ ] 保留非 Goal harness tests
- [ ] `agent-host/src/protocol-contract.test.ts`
  - [ ] 删除 Goal assertion
  - [ ] 增加无 Goal 断言
- [ ] `agent-host/src/command-lanes.test.ts`
  - [ ] 如仅为示例，替换 `goal` lane 名
- [ ] `agent-host/src/approval.test.ts`（若受影响）
  - [ ] 删除 `goalId` fixture
- [ ] `agent-host/src/permissions.test.ts`（若受影响）
  - [ ] 删除 Goal lease tests
  - [ ] 保留一般 permission tests
- [ ] `agent-host/src/worktree.test.ts`
  - [ ] 删除 Goal recovery fields/assertions
  - [ ] 保留完整 worktree coverage
- [ ] 其他 host/subagent tests
  - [ ] 删除 Goal orchestration tests
  - [ ] 保留普通 subagent regression

### 16.4 协议 fixture

- [ ] `protocol-fixtures/bootstrap-state.json`
  - [ ] 删除顶层 `goal`
  - [ ] 删除所有 `goalId`
  - [ ] 删除 Goal 专用 `taskId`
  - [ ] 保留其他 fixture 数据

### 16.5 Rust state/protocol

- [ ] `src/state.rs`
  - [ ] 删除 goals module/export
- [ ] `src/state/agents.rs`
  - [ ] 接收 `AgentProfileSnapshot`
  - [ ] 删除 `task_id/goal_id`
- [ ] `src/state/app_state.rs`
  - [ ] 删除 `goal`
  - [ ] 删除 `goal_approval`
  - [ ] 删除 Goal modal priority
- [ ] `src/state/planning.rs`
  - [ ] 删除 `GoalApprovalState`
- [ ] `src/state/transcript.rs`
  - [ ] 删除 Goal/Goals variants
- [ ] `src/host.rs`
  - [ ] 删除 bootstrap/session Goal fields
  - [ ] 删除 Goal host methods
  - [ ] 更新 serde tests
- [ ] `src/event.rs`
  - [ ] 删除 Goal CommandEvent variants/imports
- [ ] `src/runtime.rs`
  - [ ] 删除 Goal effect dispatch

### 16.6 Rust app/TUI

- [ ] `src/command.rs`
  - [ ] 删除 `/goal`、`/goals`
- [ ] `src/app.rs`
  - [ ] 删除 Goal AppEffects/Completion
  - [ ] 删除 bootstrap Goal assignment
- [ ] `src/app/actions.rs`
  - [ ] 删除 Goal command/action/receive logic
- [ ] `src/app/command_events.rs`
  - [ ] 删除 Goal command completion
- [ ] `src/app/host_events.rs`
  - [ ] 删除 Goal host events
- [ ] Goal modal key/input 文件（通过 `rg` 定位）
  - [ ] 删除 Goal approval input route
- [ ] UI modal enum 定义文件（通过 `rg` 定位）
  - [ ] 删除 `GoalApproval`
- [ ] `src/ui/scene.rs`
  - [ ] 删除 Goal approval modal
- [ ] `src/ui/transcript.rs`
  - [ ] 删除 Goal/Goals rendering
- [ ] `src/app/tests.rs`
  - [ ] 删除 Goal tests
  - [ ] 更新 bootstrap/session/non-Goal fixtures
- [ ] 其他 Rust tests/snapshots
  - [ ] 删除 Goal-only snapshots
  - [ ] 保留非 Goal coverage

### 16.7 文档

- [ ] `AGENTS.md`
- [ ] `SUBAGENTS.md`
- [ ] `TRANSCRIPT_SURFACE.md`
- [ ] `test.md`
- [ ] `README.md`
- [ ] `docs/**`

只删除 Goal runtime 说明，不添加替代设计。

---

## 17. 推荐提交顺序

这些提交用于降低审查难度。最终 PR 必须整体可构建、可测试。

### Commit 1

```text
refactor: detach shared runtime state from goal
```

包含：

- subagent `taskId/goalId`；
- approval `goalId`；
- worktree recovery `taskId/goalId`；
- Goal lease/grants；
- profile Goal 文案；
- `goal_spec` output branch。

### Commit 2

```text
refactor: remove goal store and host orchestration
```

包含：

- Goal types；
- GoalStore；
- persistence/migration；
- Host commands/events；
- planner/worker/verifier/reviewer pipeline；
- repair loop；
- Goal integration hooks。

### Commit 3

```text
refactor: remove goal protocol and tui surface
```

包含：

- bootstrap/session activation；
- fixture；
- Rust state；
- commands/effects/events；
- modal；
- transcript renderer。

### Commit 4

```text
test: synchronize coverage after goal removal
```

包含：

- 删除 Goal-only tests；
- 更新 fixtures/contracts；
- 保留并修复 coding-agent 回归测试。

### Commit 5

```text
docs: remove goal runtime references
```

包含：

- 文档和注释清理；
- 不包含任何未来设计。

如果某个中间 commit 无法独立构建，可在本地保留该顺序，但在提交前 squash 成逻辑完整的可构建提交。不得为了保持 commit 独立而添加临时兼容层。

---

## 18. Coding Agent 操作规约

Coding Agent 执行本计划时必须遵守：

1. 每次修改前先 `rg` 查调用链；
2. 删除 Goal 代码，不用新抽象替换；
3. 保持 Plan、subagent、permission、worktree 的非 Goal 行为不变；
4. 不修改用户未要求的文件；
5. 不批量格式化无关文件；
6. 不升级依赖；
7. 不修改 package manager lockfile，除非 Goal 删除确实改变依赖且编译器证明依赖已完全无用；
8. 不改协议版本；
9. 不添加兼容 decoder；
10. 不自动删除旧用户数据；
11. 每个阶段后运行对应测试；
12. 最终运行全量测试和全仓扫描；
13. 报告任何残留 Goal 命中及其原因；
14. 若某个 `goal` 单词来自非 Goal 业务，优先改写词汇但不改变行为；
15. 遇到模糊代码时以“最小删除、保留非 Goal 行为”为准。

---

## 19. 完成定义（Definition of Done）

本任务只有在以下条件全部满足时完成。

### 19.1 代码

- [ ] `GoalStore` 不存在；
- [ ] `GoalRecord/GoalSnapshot/GoalTask/GoalSpec` 不存在；
- [ ] Goal state transitions 不存在；
- [ ] Goal persistence/migration 不存在；
- [ ] Host 无 Goal fields/commands/events；
- [ ] 普通 subagent 无 Goal IDs；
- [ ] permission 无 Goal lease/grants/context；
- [ ] worktree 无 Goal metadata/behavior；
- [ ] Rust state 无 Goal；
- [ ] Rust TUI 无 Goal command/modal/transcript；
- [ ] protocol 无 Goal payload；
- [ ] fixture 无 Goal key；
- [ ] Core 文档无 Goal runtime 描述。

### 19.2 禁止项

- [ ] 未添加 Goal-lite；
- [ ] 未添加 WorkflowRun 等替代层；
- [ ] 未添加 Goal Skill/Prompt；
- [ ] 未保留 `goal: null`；
- [ ] 未保留 deprecated command；
- [ ] 未添加 migration/compatibility；
- [ ] 未重构 Plan；
- [ ] 未重构 Task Runtime；
- [ ] 未改变一般 permission/worktree 行为；
- [ ] 未包含无关重构。

### 19.3 测试

- [ ] `npm run typecheck` 通过；
- [ ] `npm test` 通过；
- [ ] `cargo fmt --check` 通过；
- [ ] `cargo clippy --all-targets -- -D warnings` 通过；
- [ ] `cargo test --all-targets` 通过；
- [ ] 协议 fixture 在 TS/Rust 两侧通过；
- [ ] 无新增 skip/ignore/only；
- [ ] coding-agent 普通会话回归通过；
- [ ] Plan 回归通过；
- [ ] subagent 回归通过；
- [ ] permission 回归通过；
- [ ] worktree/integration/recovery 回归通过；
- [ ] TUI 非 Goal 交互回归通过。

### 19.4 归零

- [ ] 精确 Goal 符号扫描无结果；
- [ ] Core Goal 词汇扫描无结果；
- [ ] Goal 文件名扫描无结果；
- [ ] Goal JSON key 扫描无结果；
- [ ] 不存在隐藏空壳或兼容 alias。

---

## 20. 最终交付报告格式

Coding Agent 完成后必须提交一份简洁报告，格式如下：

```markdown
## Goal removal result

### Removed
- 删除的核心类型和模块
- 删除的命令和协议
- 删除的 UI 和持久化
- 删除的测试数量或测试组

### Preserved
- Plan
- Generic subagents
- Permission
- Worktree/integration
- Session/TUI core

### Validation
- `npm run typecheck`: pass/fail
- `npm test`: pass/fail
- `cargo fmt --check`: pass/fail
- `cargo clippy --all-targets -- -D warnings`: pass/fail
- `cargo test --all-targets`: pass/fail
- Goal symbol scan: zero/non-zero

### Residual matches
- 无；或列出每个无法删除的命中及原因

### Out-of-scope changes
- 无
```

如果存在任何非 Goal 改动，必须明确列出并说明其为何是 Goal 删除所必需；否则任务不视为完成。

---

## 21. 一句话执行准则

> 删除 Goal，而不是重建 Goal；同步删除它在 Host、协议、Rust、权限、worktree、UI、持久化和测试中的全部痕迹，同时保持现有 coding-agent 非 Goal 能力不变。
