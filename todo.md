# Nabla 实施执行标准

> 基准版本：`7bc64361405a59e4797ea33498b5ec6631c06236`
> 仓库：https://github.com/lainniay/Nabla
> 本文档是 `7bc6436` 之后的唯一实施执行标准。采纳的方案按下文执行；未采纳的方案不执行，也不保留在代码中。

---

## 1. 执行目标

本次整理的目标并不是单纯“把大文件拆小”，而是对 Nabla 当前的模块边界进行重新梳理，重点解决以下问题：

1. 删除同一业务语义的重复实现。
2. 合并职责重叠的 Service / Manager / Broker / Adapter。
3. 建立明确的模块依赖方向。
4. 降低 Rust TUI 与 TypeScript agent-host 两侧的耦合。
5. 让权限、Shell、路径、Plan、Interaction 等核心概念只有一个权威实现。
6. 避免为了“模块化”继续增加无意义的包装层。
7. 为后续功能扩展建立稳定的架构边界。

核心原则：

> **一个概念只能有一个权威实现（Single Source of Truth）。**

可以存在 Adapter、Controller、Presenter、Port，但不能存在多个模块分别维护同一个业务规则。

例如：

- 不能有两套“Shell 是否只读”的判断。
- 不能有两套“Agent Permission Rule”的 evaluator。
- 不能有多个模块同时维护 Plan Mode 当前状态。
- 不能有多套 workspace path canonicalization。
- Permission 层不能同时承担授权和一套实际上没有执行命令的“伪执行系统”。

---

# 2. 总体判断

基于 `7bc6436`，Nabla 当前已经具备较清晰的 Rust TUI + TypeScript agent-host 双运行时结构，整体架构不需要推倒重写。

真正的问题主要集中在三个方面：

## 2.1 历史模块与新模块并存

例如 Permission 相关代码同时分散在：

```text
features/permissions/
permissions/
policy/
workspace.ts
harness.ts
runtime/
```

结果是同一个权限决策可能经过多层转换。

---

## 2.2 “薄 Service”包裹“大 Manager”

典型形式：

```text
Service
  ↓
Manager
  ↓
Domain
```

其中 Service 只做方法转发。

例如：

```text
ContextService
  ↓
ContextBudgetManager
```

以及：

```text
IntegrationService
  ↓
WorktreeManager
```

这种层次并没有形成新的边界，只增加了调用链和命名成本。

---

## 2.3 同一业务语义存在多个实现

目前尤其明显的包括：

```text
Permission policy
Shell safety analysis
Path canonicalization
Plan state restore
Interaction pending lifecycle
Execution bookkeeping
Selection algorithms
```

这类重复比“大文件”更值得优先处理，因为它们会产生行为漂移和安全风险。

---

# 3. 优先处理的重复模块总览

| 优先级 | 当前模块 / 逻辑 | 决定 | 原因 |
|---|---|---|---|
| P0 | `AgentPermissionRule` + `PermissionRule` | 合并 | 当前存在两套权限规则系统 |
| P0 | `policy/tool-policy.ts` + `permissions/shell/planner.ts` | 合并 | 两套 Shell 安全判断 |
| P0 | 多套 path boundary / canonicalization | 合并 | 安全敏感且重复 |
| P0 | `PlanStore` + `PlanModeService` + Tree restore | 合并 | 多个 Plan 状态 owner |
| P0 | `ExecutionBroker` + `DirectRunner` + Rust sandbox | 删除伪执行链 | 实际 Bash 已由 Rust 执行 |
| P1 | `ApprovalQueue` / `QuestionQueue` / `AuthPromptQueue` | 共用 primitive | pending/abort/reply 生命周期重复 |
| P1 | `InteractionBroker` | 删除 | Facade 套 Facade |
| P1 | `PermissionAdapter.proposeGrants/explain` | 删除 | File/Shell Adapter 完全重复 |
| P1 | `ContextService` | 删除或合并 | 几乎全是转发 |
| P1 | `IntegrationService` + `WorktreeManager` | 收敛公开 API | 两层 API 重复 |
| P1 | `session-navigation.ts` | 拆分 | 巨型聚合模块 |
| P1 | `harness.ts` | 最终删除 | 混合多个领域 |
| P1 | `workspace.ts` | 删除 | path security 归 filesystem permission |
| P1 | `runtime/tool-path-normalizer.ts` | 删除 | 与 path subsystem 重复 |
| P2 | Rust `ui/selector.rs` 导航算法 | 删除重复 | `selection.rs` 已有同一算法 |
| P2 | Rust `scene.rs` / `transcript.rs` | 拆分 | 主要是职责过多，不是业务语义重复 |

---

# 4. Permission：当前实际上存在两套权限系统

这是整个 `7bc6436` 中最重要的重构点。

---

## 4.1 第一套权限模型：Agent Profile Permission

当前 `harness.ts` 中存在类似：

```text
AgentPermissions
AgentPermissionRule
agentPermissionEffect()
pathAllowedByGrant()
isCredentialPath()
```

这一层自己负责：

```text
resource pattern
→ allow / ask / deny
→ read-only fallback
```

也就是说 Agent Profile Permission 本身已经是一个完整的 evaluator。

---

## 4.2 第二套权限模型：新的 Permission Kernel

同时又存在：

```text
PermissionIntent
CapabilityAtom
PermissionRule
CapabilityMatcher
evaluatePermission()
GrantBundle
PermissionKernel
ApprovalBroker
```

这是另一套更通用的权限领域模型。

---

## 4.3 当前问题

`PermissionService` 会先执行：

```text
agentPermissionEffect()
```

再把结果翻译成：

```text
PermissionRule
```

最后交给：

```text
PermissionKernel
```

因此实际上是：

```text
旧权限系统判断
      ↓
转换
      ↓
新权限系统再判断
```

这会导致：

- 同一权限语义存在两个 evaluator。
- Agent Profile 与 Permission Kernel 可能产生不同规则解释。
- 新增一种 permission effect 时需要修改两个地方。
- path matching 可能存在不同边界行为。
- 后续很难确认哪个实现才是权威。

---

# 5. Permission 实现方案（已决策）

决定：只保留一种标准规则：

```text
PermissionRule
```

Agent Profile 维持当前对用户友好的配置格式，但加载后立即编译。

例如：

```json
{
  "permission": {
    "read": [
      {
        "resource": "src/**",
        "effect": "allow"
      }
    ],
    "write": [
      {
        "resource": "src/**",
        "effect": "ask"
      }
    ]
  }
}
```

配置加载后：

```ts
compileProfilePermissions(
  profile,
  workspace
): PermissionRule[]
```

转换为标准规则：

```ts
{
  source: "agent-profile",
  effect: "allow",
  matcher: {
    kind: "file",
    operation: "read",
    path: ".../src",
    recursive: true
  }
}
```

之后统一：

```text
Agent Profile
      ↓
compile once
      ↓
PermissionRule[]
      ↓
PermissionKernel
```

---

## 5.1 将删除

将删除：

```text
AgentPermissionEffect
agentPermissionEffect()
pathAllowedByGrant()
AgentPermissionRule 的 runtime evaluator
```

`harness.ts` 不再进行权限决策，只负责读取配置。

---

# 6. Shell：目前存在两套安全分析

这是第二个最高优先级的问题。

---

## 6.1 第一套：`policy/tool-policy.ts`

该模块中存在：

```text
isReadOnlyGitCommand
isReadOnlyWorkspaceCommand
isReadOnlyCdCommand
isReadOnlyFindCommand
isReadOnlyXargsCommand
isWorkspaceCargoCommand
isDangerousExecCommand
isHighRiskCommand
isBenignShellCommand
```

这里实际上已经是一套完整 Shell policy。

---

## 6.2 第二套：`permissions/shell/planner.ts`

另一边 Shell planner 又会分析：

```text
shell AST
exec capability
file capability
network capability
opaque_code
redirect
nested shell
python/node -c
glob
readOnly
requiresShell
```

`ExecutionPlan` 已经包含类似：

```ts
readOnly: boolean;
opaque: boolean;
requiresShell: boolean;
```

---

## 6.3 当前重复

`PermissionService` 并没有完全采用 planner 的结果，而是再次根据 executable 调用：

```text
git → isReadOnlyGitCommand
cd → isReadOnlyCdCommand
find → isReadOnlyFindCommand
xargs → isReadOnlyXargsCommand
...
```

sandbox auto allow 又会调用：

```text
isDangerousExecCommand()
```

所以一个命令可能被两套逻辑分别分类。

---

# 7. Shell 实现方案（已决策）

只保留一个权威入口：

```ts
analyzeShell(command, context): ShellAnalysis
```

返回：

```ts
interface ShellAnalysis {
  plan: ExecutionPlan;
  capabilities: CapabilityAtom[];

  safety: {
    readOnly: boolean;
    mutating: boolean;
    network: boolean;
    opaque: boolean;
    elevated: boolean;
    destructive: boolean;
  };
}
```

classifier 拆为：

```text
permissions/shell/rules/
├── git.ts
├── find.ts
├── xargs.ts
├── cargo.ts
├── filesystem.ts
└── generic.ts
```

Shell planner 使用这些规则。

PermissionService 只消费：

```text
ShellAnalysis
```

而不能重新解析 command。

---

## 7.1 将删除

从 `policy/tool-policy.ts` 删除：

```text
isReadOnlyGitCommand
isReadOnlyCdCommand
isReadOnlyFindCommand
isReadOnlyXargsCommand
isReadOnlyWorkspaceCommand
isWorkspaceCargoCommand
isDangerousExecCommand
isBenignShellCommand
isHighRiskCommand
```

`policy/tool-policy.ts` 无其他职责，整个文件删除。

---

# 8. `policy/` 顶级目录将删除

当前：

```text
policy/
├── path-boundary.ts
└── tool-policy.ts
```

两者其实都属于 Permission domain。

`tool-policy.ts` 合入：

```text
permissions/shell/
```

`path-boundary.ts` 合入：

```text
permissions/filesystem/
```

最终：

```text
policy/
```

整个删除。

---

# 9. Path：目前存在多套重复实现

路径相关代码是另一个需要优先统一的安全基础设施。

目前至少存在以下重复。

---

## 9.1 `policy/path-boundary.ts`

包括：

```text
isPathWithin
nearestExistingRealPath
workspaceRelativePath
assertWorkspaceRelativePath
```

---

## 9.2 `permissions/evaluator.ts`

内部又自己实现：

```text
pathMatches()
resolve()
relative()
isAbsolute()
```

本质上再次判断：

```text
target 是否位于 root 下
```

---

## 9.3 `permissions/adapters/filesystem.ts`

再次实现：

```text
canonicalizePath()
```

通常通过：

```text
existsSync
dirname
realpathSync
suffix
```

逐步定位最近存在路径。

---

## 9.4 `workspace.ts`

再次处理：

```text
resolve
realpath
nearestExistingRealPath
isPathWithin
symlink escape
```

---

## 9.5 `runtime/tool-path-normalizer.ts`

又执行：

```text
absolute
→ isPathWithin(cwd)
→ relative
```

---

# 10. Path 统一方案（已决策）

决定：最终只保留：

```text
permissions/filesystem/path.ts
```

提供：

```ts
resolveCanonicalPath()
resolveCanonicalPathFrom()
isPathWithin()
workspaceRelativePath()
validateWorkspacePath()
```

决定：不建立对象形式（YAGNI），只用函数 API。

---

## 10.1 将删除

```text
workspace.ts
runtime/tool-path-normalizer.ts
filesystem.ts 内重复 canonicalizePath()
evaluator.ts 内重复 path containment 算法
policy/path-boundary.ts
```

逻辑迁移后统一调用一个实现。

---

## 10.2 不能错误删除的安全层

需要注意：

```text
lexical path boundary
```

和：

```text
realpath / symlink boundary
```

是两个不同安全检查。

它们都应该保留。

去掉的是：

```text
算法重复
```

而不是：

```text
安全检查层级
```

---

# 11. Plan：当前存在多个状态 owner

Plan 相关状态目前分散在多个模块。

---

## 11.1 `plan.ts`

负责：

```text
PlanStore
restore()
submit()
restorePlanMode()
PLAN_MODE_ENTRY_TYPE
```

---

## 11.2 `PlanService`

负责：

```text
restore
onSessionActivated
execute
setMode
planState
```

---

## 11.3 `PlanModeService`

自己维护：

```ts
private active = false;
```

并负责：

```text
apply
restore
set
active tool list
```

---

## 11.4 `TreeService`

自身又有类似：

```text
restorePlanState()
```

里面执行：

```text
plans.restore(branch)
restorePlanMode(branch)
planMode.set(...)
send plan_mode_state
send plan_state
```

这意味着 Plan state 的恢复不是由 Plan 模块自己完整拥有。

---

# 12. Plan 实现方案：收敛为一个 Controller（已决策）

目标：

```text
features/plans/
├── model.ts
├── store.ts
├── execution.ts
└── plan-controller.ts
```

唯一公开入口：

```ts
class PlanController {
  current(): PlanSnapshot;

  restore(branch: SessionEntry[]): PlanSnapshot;

  submit(content: string): PlanArtifact;

  setMode(active: boolean): PlanSnapshot;

  execute(mode: PlanExecutionMode): Promise<...>;

  activateSession(
    branch: SessionEntry[]
  ): PlanSnapshot;
}
```

---

## 12.1 最重要的原则

不要再同时拥有：

```text
PlanStore.artifact
```

和：

```text
PlanModeService.active
```

两个独立可变状态 owner。

PlanController 完整拥有：

```text
artifact
mode
```

---

## 12.2 TreeService 不再恢复 Plan

Tree / Session 切换后只需要：

```ts
plans.activateSession(
  sessionManager.getBranch()
);
```

而不能自己：

```text
restorePlanMode
restorePlan
send plan event
```

---

## 12.3 将删除

```text
TreeService.restorePlanState()
runtime/plan-mode-service.ts
PlanService 中纯转发 API
```

Plan Mode 不是 runtime infrastructure，归入：

```text
features/plans/
```

---

# 13. Plan 的双层防御保留（已决策）

这里不能过度去重。

目前 Plan Mode 一方面通过：

```text
session.setActiveToolsByName(...)
```

限制模型可见工具。

另一方面 PermissionService 还会拒绝：

```text
plan-mode mutation
```

这是合理的：

```text
第一层
↓
不暴露 mutating tool

第二层
↓
即使绕过 tool exposure
Permission Kernel 仍拒绝 mutation
```

这属于 defense-in-depth。

真正需要去重的是规则来源。

采用：

```ts
const PLAN_MODE_POLICY = {
  exposedTools: ...,
  permissionRules: ...
};
```

然后两个防御层都消费：

```text
PLAN_MODE_POLICY
```

而不是各自维护一套 Plan 规则。

---

# 14. PermissionService 变薄（已决策）

当前 `PermissionService` 同时负责：

```text
Tool intent
Agent profile policy
Plan mode policy
Workspace policy
Credential path
Shell classifier
Sandbox auto allow
Risk
Approval
Grant
Execution bookkeeping
Pending execution
Workspace rule management
```

职责太多。

完成前述重构后，它将接近：

```ts
class PermissionService {
  authorize(
    request: AuthorizationRequest
  ): Promise<ExecutionPermit>;

  finish(
    permit: ExecutionPermit,
    result: ExecutionResult
  ): void;
}
```

内部流程：

```text
IntentFactory
     ↓
PolicyCompiler
     ↓
PermissionKernel
     ↓
RiskAssessor
     ↓
ApprovalQueue
     ↓
ExecutionPermit
```

目标职责：

```text
不再是一个权限子系统总文件
```

而不是继续成为一个权限子系统总文件。

---

# 15. Execution：当前有一套与真实执行脱节的模型

当前权限模块内有：

```text
PermissionService
  ↓
ExecutionBroker
  ↓
DirectRunner
```

`ExecutionBroker` 设计成真正执行 Shell。

`DirectRunner` 也通过 Node `spawn()` 真正运行进程。

但 `7bc6436` 中 Nabla Bash 的真实执行路径已经是：

```text
createNablaBashTool()
    ↓
permissions.authorizeBash()
    ↓
rustSandboxBackend.operationsFor(profile)
    ↓
Pi Bash Tool
    ↓
Rust __sandbox-exec
```

所以：

```text
真实 Bash executor
```

已经是 Rust Sandbox。

---

# 16. Permission execution broker 改为 ExecutionPermit（已决策）

Permission 只负责：

```text
授权
consume authorization
audit
record result
```

而不是再维护一套执行器。

采用：

```ts
interface ExecutionPermit {
  id: string;
  intentDigest: string;
  sandboxProfile: SandboxExecutionProfile;
}
```

使用方式：

```ts
const permit = await permissions.authorize(...);

try {
  const result = await sandbox.execute(
    permit,
    command
  );

  permissions.complete(
    permit,
    "success"
  );

  return result;
} catch (error) {
  permissions.complete(
    permit,
    "failure"
  );

  throw error;
}
```

---

## 16.1 将删除

```text
ExecutionBroker.beginExternalTool()
ExecutionBroker.finishExternalTool()
toExecutionProfile()
EXTERNAL_TOOL_EXECUTION_PROFILE
pending
pendingBash
```

`ExecutionBroker.executeShell()` 已核实无生产调用者，直接删除：

```text
ExecutionBroker
DirectRunner
ShellFallback
```

最终：

```text
Permission
```

不再假装拥有 execution backend。

真实执行统一：

```text
RustSandboxClient
    ↓
__sandbox-exec
```

---

# 17. Context：当前是“一真一假两个 Service”

目前：

```text
context-manager.ts
features/context/context-service.ts
```

`ContextService` 大量方法只是：

```text
snapshot()
→ budget.snapshot()

onSessionStart()
→ budget.onSessionStart()

onModelResponse()
→ budget.onModelResponse()

filter()
→ budget.filter()
```

因此：

```text
ContextService
→ ContextBudgetManager
```

这层没有形成真正的 application boundary。

---

# 18. Context 实现方案：删除薄 Service，拆真正的大实现（已决策）

最终：

```text
features/context/
├── model.ts
├── policy.ts
├── estimator.ts
├── pruning.ts
├── checkpoint.ts
└── context-engine.ts
```

目前 `context-manager.ts` 中的职责分为：

```text
token estimation
category estimation
tool result pruning
sticky pruning
checkpoint
plan checkpoint
usage tracking
compaction history
snapshot
environment policy parsing
```

这些才是值得拆的真实模块。

最终只保留：

```text
ContextEngine
```

作为公开 API。

event 发布：

```text
context_budget
```

由 Controller / Host Event Adapter 负责。

不要因为发送 event 再包一层 Service。

---

# 19. `harness.ts` 将删除

`harness.ts` 当前混合了多个独立领域：

```text
AgentProfile
HarnessConfig
ResourceSnapshot
default profiles
配置加载
配置 merge
agent markdown
workspace trust
model reference
agent permission
path permission
credential path
```

这并不是一个稳定的 domain。

拆分。

---

## 19.1 Agent Profile

迁移：

```text
features/subagents/
├── profile-model.ts
├── profile-loader.ts
└── default-profiles.ts
```

包括：

```text
AgentProfile
DEFAULT_PROFILES
modelReference
agent profile parsing
```

---

## 19.2 Workspace Config

迁移：

```text
features/workspace/config.ts
```

包括：

```text
HarnessConfig
loadHarnessConfig()
save config
config merge
```

---

## 19.3 Workspace Trust

迁移：

```text
features/workspace/trust.ts
```

包括：

```text
workspaceIsTrusted()
saveWorkspaceTrust()
filterContextFilesByTrust()
```

---

## 19.4 Permission

迁移：

```text
features/permissions/profile-policy-compiler.ts
features/permissions/filesystem/credential.ts
```

其中：

```text
agentPermissionEffect()
```

注意：`agentPermissionEffect` 有**两个**生产调用者（permission-service.ts:263、integration-service.ts:161），两者都迁移后才能删除；替换为：

```text
compileAgentProfileRules()
```

---

## 19.5 最终删除

```text
harness.ts
```

这是整个 TypeScript 架构整理是否完成的一个重要指标。

---

# 20. WorkspaceService 减肥（已决策）

当前 WorkspaceService 同时负责：

```text
config
trust
resources
agents snapshot
model availability
plan-mode refresh
event publishing
```

甚至 Workspace 状态更新时会触发：

```text
planMode.apply()
sendPlanModeState()
```

这说明 Workspace 与 Plan 产生了反向耦合。

---

## 20.1 拆分方案

```text
features/workspace/
├── config.ts
├── trust.ts
├── resources.ts
└── controller.ts
```

具体：

```text
WorkspaceConfig
WorkspaceTrust
ResourceCatalog
WorkspaceController
```

WorkspaceController 协调：

```text
reload()
setTrust()
snapshot()
```

但不应该：

```text
管理 Plan state
管理 Subagent runtime state
管理 Permission evaluator
```

---

# 21. SubagentSupervisor 正在变成第二个 Composition Root

当前 `SubagentSupervisor` 依赖大量 concrete service：

```text
WorkspaceService
IntegrationService
PermissionService
RustSandboxBackend
ModelRuntime
RuntimeSupervisor
PlanModeService
sendEvent
warn
onAgentsChanged
```

同时负责：

```text
agent lifecycle
concurrency
worktree
integration
resolver agent
permission
tool extension
bash construction
model selection
recovery
event serialization
```

这已经超出 Supervisor 的合理范围。

---

# 22. Subagent 拆分（已决策）

将拆为：

```text
features/subagents/
├── profile.ts
├── registry.ts
├── runner.ts
├── supervisor.ts
└── isolation/
```

职责：

### `registry.ts`

负责：

```text
active agents
pending agents
lookup
status
```

### `runner.ts`

负责：

```text
create runtime
create tool extensions
execute subagent
cleanup
```

### `supervisor.ts`

负责：

```text
concurrency
high-level lifecycle
cancel
join
coordination
```

### `isolation/`

负责：

```text
worktree
integration
recovery
```

不要所有实现都做成 class。

---

# 23. 主 Agent 与 Subagent 共享 Permission Tool Guard（已决策）

当前 Subagent 会自己注册：

```text
tool_call
tool_result
```

并处理：

```text
normalizeToolInputPaths
permissions.authorizeTool
permissions.finishTool
```

主 runtime 同样需要这一套行为。

建立：

```text
createPermissionExtension()
```

或者：

```text
ToolAuthorizationHooks
```

统一：

```text
before tool
after tool
path normalization
authorization
execution result recording
```

主 Agent / Subagent 只传不同 context：

```ts
interface AuthorizationSubject {
  agentId?: string;
  profile?: AgentProfile;
  model?: string;
  planReadOnly?: boolean;
}
```

这样可以消除：

```text
主 Agent permission hook
Subagent permission hook
```

两套实现。

---

# 24. IntegrationService 与 WorktreeManager 只保留一个公开 API（已决策）

当前结构：

```text
IntegrationService
      ↓
WorktreeManager
```

而 IntegrationService 很多方法只是：

```text
prepare()
annotate()
capture()
integrate()
keep()
discard()
prepareResolution()
assertResolved()
resolvedBy()
```

直接转发 WorktreeManager。

这是明显的 Service → Manager 重复。

---

# 25. 决定：保留 IntegrationService

对外只暴露：

```text
IntegrationService
```

内部将 WorktreeManager 拆成：

```text
features/subagents/isolation/
├── integration-service.ts
├── worktree.ts
├── git.ts
├── artifact-store.ts
├── recovery.ts
└── model.ts
```

`WorktreeManager` 不再作为一个大型公开 Manager。

内部为：

```text
GitClient
WorktreeArtifactStore
WorktreeIsolation
```

外部调用者无需知道这些实现。

---

# 26. Sessions：目前是一个巨型实现 + 多个薄 Service

`session-navigation.ts` 当前包含：

```text
SessionCatalog
session search
sorting
thread flattening
history projection
turn metrics
tree projection
tree filtering
tree layout
copy text
startup manager
```

然后上层还有：

```text
SessionService
SessionBrowserService
TreeService
```

这说明真正需要拆的是：

```text
session-navigation.ts
```

而不是继续新增 Service。

---

# 27. Sessions 目标目录（已决策）

```text
features/sessions/
├── controller.ts
├── catalog.ts
├── browser-registry.ts
├── history-projection.ts
├── tree-projection.ts
├── tree-controller.ts
├── search.ts
└── turn-metrics.ts
```

最终删除：

```text
session-navigation.ts
```

---

# 28. SessionService 与 RuntimeSupervisor 边界明确（已决策）

当前 SessionService 很薄，核心操作仍然直接转给：

```text
RuntimeSupervisor
```

例如：

```text
newSession
switchSession
current
clearQueue
```

两者都像 application service。

决定：明确为：

```text
RuntimeSupervisor
```

只负责：

```text
runtime instance lifecycle
current runtime
close runtime
require idle
```

而：

```text
SessionController
```

负责：

```text
new session
resume session
switch session
clear queue
activation events
```

因此把当前：

```text
SessionService
```

明确改造成：

```text
SessionController
```

避免 Service/RuntimeSupervisor 语义重叠。

---

# 29. Protocol 成为独立跨语言边界（已决策）

当前 TypeScript `protocol/contracts.ts` 仍引用多个业务模块类型。

协议层不应该依赖：

```text
Plan implementation
Permission implementation
Context manager
Workspace implementation
```

依赖方向：

```text
protocol
   ↑
application
   ↑
domain
```

而不是：

```text
protocol
→ business implementation
```

---

# 30. Protocol 使用 schema-first（已决策）

项目已有 TypeBox，让 schema 成为类型和 validator 的单一来源。

例如：

```ts
export const SandboxStatusSchema = Type.Object({
  mode: Type.Union([
    Type.Literal("enforced"),
    Type.Literal("degraded"),
    Type.Literal("disabled")
  ]),

  backend: Type.Union([
    Type.Literal("bubblewrap"),
    Type.Literal("seatbelt"),
    Type.Literal("none")
  ])
});

export type SandboxStatus =
  Static<typeof SandboxStatusSchema>;
```

这样删除：

```text
type 定义一份
validator 再写一份
```

这种重复。

---

# 31. Protocol 目标目录（已决策）

```text
protocol/
├── bootstrap.ts
├── auth.ts
├── permissions.ts
├── plans.ts
├── sessions.ts
├── subagents.ts
├── sandbox.ts
├── envelope.ts
└── registry.ts
```

`HostEvent` 在 registry 组合，不要让所有协议继续集中在一个巨大 `contracts.ts`。

---

# 32. Rust 协议 DTO 去掉裸 String（已决策）

例如 Sandbox status 的：

```text
mode
backend
filesystem
network
```

如果协议只有固定字符串集合，Rust 使用 enum：

```rust
#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize
)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    Enforced,
    Degraded,
    Disabled,
}
```

而不是：

```rust
String
```

这样：

- UI 不需要 `mode.as_str()`。
- 新增 protocol value 时编译器可以暴露缺失分支。
- Rust 与 TypeScript 的 union / enum 更一致。

---

# 33. Rust / TypeScript DTO 不强制 codegen（已决策）

Rust 与 TS 都定义 wire DTO，看起来有重复，但当前项目规模下不一定需要立即引入复杂 codegen。

优先保留：

```text
protocol-fixtures/
```

并让 Rust / Node 同时解析同一批 fixture。

采用：

```text
protocol-fixtures/
├── bootstrap/
├── events/
├── commands/
└── sandbox/
```

例如：

```text
sandbox/
├── probe-enforced.json
├── probe-disabled.json
├── exec-valid.json
└── exec-invalid-relative-path.json
```

通过 contract tests 控制两端一致性。

---

# 34. Rust：Selection 算法重复（将删除）

Rust 已有：

```text
src/selection.rs
```

包括：

```text
previous_wrapped()
next_wrapped()
page_backward()
page_forward()
centered_visible_start()
```

但：

```text
ui/selector.rs
```

又实现了：

```text
select_previous()
select_next()
visible_range()
```

其中 visible range 与：

```text
centered_visible_start()
```

高度重合。

---

# 35. Rust Selection 统一（已决策）

UI 直接调用：

```rust
self.selected =
    selection::previous_wrapped(
        self.selected,
        self.items.len()
    );
```

以及：

```rust
let start =
    selection::centered_visible_start(
        self.total,
        self.selected,
        self.visible_rows
    );
```

最终：

```text
selection.rs
```

负责：

```text
纯 index math
```

而：

```text
ui/selector.rs
```

只负责：

```text
keyboard mapping
SelectorModel
SelectorPolicy
SelectorAction
```

---

# 36. Rust `file_references.rs` 与 `app/file_references.rs` 不合并（已决策）

这两个模块虽然名字相近，但职责其实合理分离。

根：

```text
src/file_references.rs
```

负责：

```text
@file parser
filesystem index
fuzzy matching
file prepare
canonical path
serialization
```

而：

```text
src/app/file_references.rs
```

负责：

```text
refresh completion
select previous
select next
accept
prepare delivery
produce AppEffect
```

这属于正确的：

```text
App Controller
      ↓
FileReference Domain
```

因此：

```text
不要合并
```

根文件将拆为：

```text
file_references/
├── parser.rs
├── index.rs
├── matcher.rs
├── prepare.rs
└── model.rs
```

---

# 37. Rust `ui/text.rs` 是唯一文本布局 primitive（已决策）

目前 `ui/text.rs` 已经负责：

```text
grapheme
byte offset
display column
unicode width
wrapping
styled wrapping
cursor geometry
```

这条边界继续强化。

要求：

```text
scene.rs
transcript.rs
panel.rs
selector.rs
```

不要重新实现：

```text
unicode width
truncate
wrap
clip
cursor column
```

以后如果发现：

```rust
chars().take(...)
.len()
manual byte truncation
manual display width
```

迁移到：

```text
ui/text.rs
```

而不是继续让 renderer 自己处理。

---

# 38. Rust Scene 拆分（已决策）

当前 `scene.rs` 过大，但不要为了拆文件引入大量微型组件 class。

目标：

```text
ui/scene/
├── mod.rs
├── builder.rs
├── primary.rs
├── alternate.rs
├── status.rs
├── composer.rs
├── panels/
│   ├── completion.rs
│   ├── approval.rs
│   └── selection.rs
├── modals/
│   ├── session.rs
│   ├── tree.rs
│   ├── auth.rs
│   ├── question.rs
│   └── transcript_viewer.rs
├── canvas.rs
└── tests.rs
```

拆分标准：

```text
按 UI surface / 变化原因
```

而不是：

```text
每一个控件一个模块
```

---

# 39. Rust Transcript 拆分（已决策）

目标：

```text
ui/transcript/
├── mod.rs
├── model.rs
├── projection.rs
├── store.rs
├── history.rs
├── cache.rs
└── render/
    ├── mod.rs
    ├── user.rs
    ├── assistant.rs
    ├── tool.rs
    ├── diff.rs
    └── common.rs
```

不要发展成：

```text
UserMessageComponent
UserMessageStore
UserMessageController
UserMessageRenderer
```

这会导致过度模块化。

---

# 40. Renderer 不直接依赖完整 AppState（已决策）

当前 scene/transcript 多处直接读取完整 `AppState`。

目标边界：

```text
AppState
   ↓
Projection / Presenter
   ↓
ViewModel
   ↓
Renderer
```

例如：

```rust
pub struct StatusViewModel {
    pub model: String,
    pub thinking_level: String,
    pub context_percent: Option<f64>,
    pub busy: bool,
    pub connected: bool,
    pub plan_mode: bool,
    pub sandbox: SandboxStatus,
}
```

这样 UI renderer 不知道：

```text
Session
Permission
Plan
Runtime
Host
```

的内部结构。

---

# 41. Rust Host 拆分（已决策）

当前 `host.rs` 混合：

```text
DTO
connection
runtime
client commands
timeout
cleanup
tests
```

将拆为：

```text
host/
├── mod.rs
├── connection.rs
├── client.rs
├── timeout.rs
└── dto/
    ├── mod.rs
    ├── bootstrap.rs
    ├── auth.rs
    ├── permissions.rs
    ├── sessions.rs
    ├── plans.rs
    └── agents.rs
```

不需要为每个 command 创建大量 trait。

只需：

- DTO 分领域。
- Client 通过多个 `impl HostClient` 文件组织。
- 保留通用 `request_data()`。

---

# 42. Rust 进程管理拆分（已决策）

当前 `pi_process.rs` 同时处理：

```text
host path
socket config
Node child process
env injection
Pi JSONL peer
host socket connection
stderr
shutdown
kill
temp directory cleanup
```

将拆为：

```text
process/
├── mod.rs
├── config.rs
├── spawn.rs
├── guard.rs
└── stderr.rs

pi/
├── client.rs
└── events.rs
```

最终：

```text
spawn()
```

只做组合。

---

# 43. TypeScript 目标目录（已决策）

完成主要去重后，整理为：

```text
agent-host/src/
│
├── app/
│   ├── create-host-app.ts
│   ├── host-app.ts
│   └── lifecycle.ts
│
├── protocol/
│   ├── commands/
│   ├── events/
│   ├── schemas/
│   ├── validation.ts
│   └── pending-request.ts
│
├── features/
│   │
│   ├── permissions/
│   │   ├── model.ts
│   │   ├── service.ts
│   │   ├── kernel.ts
│   │   ├── evaluator.ts
│   │   │
│   │   ├── intent/
│   │   │   ├── factory.ts
│   │   │   └── filesystem.ts
│   │   │
│   │   ├── shell/
│   │   │   ├── parser.ts
│   │   │   ├── planner.ts
│   │   │   ├── classifier.ts
│   │   │   ├── digest.ts
│   │   │   └── rules/
│   │   │
│   │   ├── policy/
│   │   │   ├── compiler.ts
│   │   │   ├── builtin.ts
│   │   │   └── profile.ts
│   │   │
│   │   ├── grants/
│   │   │   ├── manager.ts
│   │   │   ├── once.ts
│   │   │   ├── session.ts
│   │   │   └── workspace.ts
│   │   │
│   │   ├── filesystem/
│   │   │   ├── path.ts
│   │   │   └── credential.ts
│   │   │
│   │   └── sandbox/
│   │       ├── client.ts
│   │       ├── capability.ts
│   │       └── profile.ts
│   │
│   ├── interactions/
│   │   ├── request.ts
│   │   ├── approval.ts
│   │   └── questions.ts
│   │
│   ├── plans/
│   │   ├── model.ts
│   │   ├── store.ts
│   │   ├── controller.ts
│   │   └── execution.ts
│   │
│   ├── context/
│   │   ├── model.ts
│   │   ├── engine.ts
│   │   ├── estimator.ts
│   │   ├── pruning.ts
│   │   └── checkpoint.ts
│   │
│   ├── sessions/
│   │   ├── controller.ts
│   │   ├── catalog.ts
│   │   ├── history.ts
│   │   ├── tree.ts
│   │   └── browser-registry.ts
│   │
│   ├── workspace/
│   │   ├── config.ts
│   │   ├── trust.ts
│   │   └── resources.ts
│   │
│   ├── subagents/
│   │   ├── profile.ts
│   │   ├── supervisor.ts
│   │   ├── runner.ts
│   │   └── isolation/
│   │       ├── integration.ts
│   │       ├── worktree.ts
│   │       ├── git.ts
│   │       └── recovery.ts
│   │
│   └── auth/
│
├── runtime/
│   ├── supervisor.ts
│   ├── session-activation.ts
│   └── pi-extension-factory.ts
│
└── infrastructure/
    ├── persistence/
    └── transport/
```

---

# 44. 最重要的一点：只保留一个 Permissions Root（已决策）

当前类似：

```text
features/permissions/
permissions/
policy/
```

三套并存。

这是物理结构上的重复。

最终收敛为 `features/permissions/`（已决策，不再二选一）。

关键不是名字，而是：

> 全项目只有一个 Permission 模块根目录。

不要继续存在：

```text
features/permissions
permissions
policy
workspace.ts
harness.ts
```

五个位置共同拥有权限语义。

---

# 45. Rust 目录目标（已决策）

逐步整理为：

```text
src/
├── app/
│   ├── mod.rs
│   ├── update/
│   └── effects/
│
├── state/
│   ├── agents.rs
│   ├── auth.rs
│   ├── context.rs
│   ├── navigation.rs
│   ├── planning.rs
│   ├── sessions.rs
│   └── transcript.rs
│
├── host/
│   ├── mod.rs
│   ├── connection.rs
│   ├── client.rs
│   ├── timeout.rs
│   └── dto/
│
├── sandbox/
│   ├── mod.rs
│   ├── detect.rs
│   ├── profile.rs
│   ├── request.rs
│   ├── process.rs
│   └── platform/
│
├── process/
│   ├── config.rs
│   ├── spawn.rs
│   ├── guard.rs
│   └── stderr.rs
│
├── file_references/
│   ├── model.rs
│   ├── parser.rs
│   ├── index.rs
│   ├── matcher.rs
│   └── prepare.rs
│
├── selection.rs
│
└── ui/
    ├── text.rs
    ├── selector.rs
    ├── scene/
    └── transcript/
```

注意：

```text
state/
sandbox/
```

目前本身方向已经比较合理，不应该为了统一目录风格重新打乱。

---

# 46. 不执行的重构（非目标）

以下几类虽然表面“重复”，但不粗暴合并。

---

## 46.1 Rust DTO 与 TypeScript DTO

这是跨语言 wire contract。

当前接受双定义。

优先通过：

```text
protocol fixture tests
```

防止漂移。

不要过早引入复杂 code generation。

---

## 46.2 Plan Tool Exposure 与 Permission Denial

这两层是不同安全防线。

共享：

```text
PLAN_MODE_POLICY
```

但两层都保留。

---

## 46.3 Lexical Path Check 与 Realpath Check

这是两个不同攻击面。

必须都保留。

只统一底层 path primitive。

---

## 46.4 Rust State 与 UI ViewModel

它们不属于错误重复。

Domain state 与 presentation state 继续分离。

---

## 46.5 `file_references.rs` 与 `app/file_references.rs`

前者是 domain/helper。

后者是 controller/reducer。

不应合并。

---

### 46.6 Interaction 保持现状

三个 Queue 已共用 `PendingRequestRegistry`；不抽统一原语、不删 `InteractionBroker`、不改名 `ApprovalBroker`。

---

# 47. 分阶段实施（已决策）

不做一个超大 PR。

按语义边界逐步迁移。

---

# Phase 1：统一 Path 基础实现

目标：

```text
整个 TypeScript 只有一个 path security primitive
```

操作：

1. 新建：

```text
features/permissions/filesystem/path.ts
```

2. 迁移：

```text
isPathWithin
nearestExistingRealPath
workspaceRelativePath
canonicalizePath
```

3. 修改调用者。

4. 删除：

```text
workspace.ts
runtime/tool-path-normalizer.ts
policy/path-boundary.ts
```

5. 删除 evaluator 内部重复 containment 算法。

---

## 验收

全仓：

```text
realpath
relative
isPathWithin
```

安全判断只来自一个模块。

---

# Phase 2：统一 Shell Analyzer

目标：

```text
所有 Shell 安全判断只有一套
```

操作：

1. 将 command classifier 迁移至：

```text
features/permissions/shell/rules/
```

2. planner 输出统一：

```text
ShellAnalysis
```

3. PermissionService 不再判断：

```text
git
find
xargs
cargo
```

4. risk assessor 消费 `ShellAnalysis.safety`。

5. 删除：

```text
policy/tool-policy.ts
```

---

## 验收

全仓不能再出现第二套：

```text
isReadOnlyGitCommand
isDangerousExecCommand
isHighRiskCommand(command)
```

---

# Phase 3：统一 Permission Rule

目标：

```text
全项目只存在一个 PermissionRule runtime model
```

操作：

1. 建立：

```text
compileAgentProfileRules()
```

2. Agent profile 加载时编译。

3. PermissionService 直接传标准 rules。

4. 删除：

```text
agentPermissionEffect()
pathAllowedByGrant()
```

注意：`agentPermissionEffect` 有**两个**生产调用者（permission-service.ts:263、integration-service.ts:161），两者都迁移到编译后的规则后再删。

5. `harness.ts` 不再做 permission evaluation。

---

## 验收

权限决策最终只能通过：

```text
PermissionKernel
```

完成。

---

# Phase 4：移除伪 Execution Layer

目标：

```text
Permission 只授权
Rust Sandbox 只执行
```

操作：

1. 引入：

```text
ExecutionPermit
```

2. Authorization 返回 permit。

3. Rust sandbox 消费 profile。

4. Permission complete 只做 audit / grant consumption。

5. 删除：

```text
toExecutionProfile
pending
pendingBash
beginExternalTool
finishExternalTool
```

6. 删除（已核实 `executeShell` 无生产调用者）：

```text
ExecutionBroker
DirectRunner
ShellFallback
```

---

## 验收

全仓只有一个真实 Bash execution backend。

---

# Phase 5：统一 Plan

目标：

```text
Plan 只有一个 state owner
```

操作：

1. 建立：

```text
PlanController
```

2. 持有：

```text
artifact
mode
```

3. Tree / Session 只调用：

```text
activateSession()
```

4. Tool exposure 与 Permission Rule 共用：

```text
PLAN_MODE_POLICY
```

5. 删除：

```text
PlanModeService
TreeService.restorePlanState
```

---

## 验收

全仓不能再有第二个：

```text
planMode.active
restorePlanMode()
```

状态源。

---

# Phase 6：拆掉 `harness.ts`

迁移：

```text
AgentProfile
→ subagents/

Workspace config
→ workspace/

Workspace trust
→ workspace/

Credential path
→ permissions/

Permission profile evaluator
→ 删除，替换为 compiler
```

最终删除：

```text
harness.ts
```

---

# Phase 7：Sessions / Context / Worktree

这一阶段主要解决大型聚合文件。

顺序：

```text
session-navigation.ts
context-manager.ts
worktree.ts
```

分别拆为真实领域组件。

不要增加额外 facade。

---

# Phase 8：Rust UI 与 Host 整理

最后进行：

```text
scene.rs
transcript.rs
host.rs
pi_process.rs
```

拆分。

此时核心业务边界已经稳定，移动 Rust UI 文件不会与权限/协议重构产生过大冲突。

---

# 48. PR 执行顺序（已决策）

按以下 PR 顺序执行。

---

## PR 1 — Path Unification

```text
统一 canonicalization
统一 workspace boundary
删除 workspace.ts
删除 tool-path-normalizer.ts
删除 policy/path-boundary.ts
```

风险：

```text
低
```

收益：

```text
高
```

---

## PR 2 — Shell Analysis Unification

```text
planner 成为唯一 Shell analyzer
迁移 git/find/xargs/cargo rules
删除 tool-policy.ts
```

风险：

```text
中
```

收益：

```text
非常高
```

---

## PR 3 — Profile Permission Compilation

```text
AgentPermissionRule
→ PermissionRule
```

删除双 evaluator。

风险：

```text
中高
```

收益：

```text
最高
```

---

## PR 4 — ExecutionPermit

```text
Permission authorization
与
Sandbox execution
彻底分离
```

风险：

```text
中
```

收益：

```text
非常高
```

---

## PR 5 — Plan Controller

统一：

```text
artifact
mode
restore
activation
```

风险：

```text
中
```

收益：

```text
高
```

---

## PR 6 — Harness Removal

拆：

```text
workspace
subagents
permissions
```

风险：

```text
中
```

收益：

```text
高
```

---

## PR 7 — Sessions / Context / Worktree

大型模块拆分。

风险：

```text
中
```

收益：

```text
长期维护价值高
```

---

## PR 8 — Rust Structure Cleanup

包括：

```text
host
process
scene
transcript
selection
```

风险：

```text
低中
```

收益：

```text
维护性提升明显
```

---

# 49. 重构后的依赖原则（已决策）

决定：整个 agent-host 遵循：

```text
Protocol
   ↑
Application / Controller
   ↓
Domain
   ↓
Ports
   ↓
Infrastructure
```

更具体：

```text
create-host-app
       ↓
Controllers
       ↓
Domain Services
       ↓
Ports
       ↓
Node / Pi / Rust Sandbox / Filesystem
```

禁止：

```text
Domain
→ ControlServer

Domain
→ Pi concrete session

Domain
→ create-host-app

Protocol
→ Business implementation

Workspace
→ Plan implementation

Permission
→ UI interaction implementation details
```

---

# 50. Composition Root 目标（已决策）

当前 `create-host-app.ts` 需要逐步变成真正的 composition root。

理想结构：

```ts
export async function createHostApp(
  options: CreateHostAppOptions
) {
  const core =
    await createCoreServices(options);

  const modules =
    createApplicationModules(core);

  const runtime =
    createRuntime(core, modules);

  const router =
    createRouter(modules, runtime);

  const control =
    createControlServer(
      options.socketPath,
      router,
      core.events
    );

  return new HostAppImpl({
    runtime,
    control,
    modules
  });
}
```

目标：

```text
create-host-app.ts
```

只描述：

```text
谁依赖谁
```

不包含业务规则。

---

# 51. 验收标准的硬性规则（已决策）

完成重构后，使用下面的规则审查代码。

---

## 51.1 Permission

必须满足：

```text
只有一个 PermissionRule
只有一个 evaluator
只有一个 PermissionKernel
```

不存在：

```text
AgentPermissionRule runtime evaluator
```

---

## 51.2 Shell

必须满足：

```text
只有一个 ShellAnalysis
```

任何：

```text
read-only
dangerous
network
opaque
destructive
```

判断都来自它。

---

## 51.3 Path

必须满足：

```text
只有一个 canonical path implementation
只有一个 workspace containment primitive
```

---

## 51.4 Plan

必须满足：

```text
只有一个 mutable Plan state owner
```

Tree / Session / Workspace 不自行 restore Plan。

---

## 51.5 Interaction

必须满足：

```text
Approval / Question / Auth 共用 PendingRequestRegistry
```

保持现状，不新增抽象。

---

## 51.6 Execution

必须满足：

```text
只有一个实际 Bash executor
```

Permission 不再拥有虚假的 executor abstraction。

---

## 51.7 Directory

最终不再存在：

```text
features/permissions/
permissions/
policy/
```

三套 Permission root。

---

## 51.8 Root-level historical modules

最终将删除：

```text
harness.ts
workspace.ts
plan.ts
session-navigation.ts
context-manager.ts
worktree.ts
```

这些不是必须一次删除，但最终不应该继续作为根目录大杂烩。

---

## 51.9 Rust Algorithms

以下算法必须有唯一实现：

```text
selection
text width
wrapping
truncation
path-like display calculations
```

UI component 不重复实现纯算法。

---

# 52. 优先执行的 5 项

按顺序最先执行以下五项：

---

## 1. 合并两套 Permission Policy

这是最大的领域重复。

把：

```text
AgentPermissionRule
```

编译成：

```text
PermissionRule
```

统一使用 PermissionKernel。

---

## 2. 合并 Shell Planner 与 Tool Policy

让：

```text
ShellAnalysis
```

成为唯一命令安全分类来源。

这是最容易发生 security semantic drift 的地方。

---

## 3. 合并全部 Path / Canonicalization

所有：

```text
workspace boundary
realpath
symlink escape
relative path
canonical path
```

使用同一组底层 primitive。

---

## 4. Plan 收敛到一个 PlanController

删除多个 Plan 状态 owner。

Tree / Workspace / Session 不再自行恢复 Plan 状态。

---

## 5. 删除 Permission 层伪 Execution Model

以：

```text
ExecutionPermit
```

代替：

```text
ExecutionBroker + DirectRunner
```

让：

```text
Rust Sandbox
```

成为唯一真实 Bash execution backend。

---

# 53. 重构前后架构对比

当前大量路径接近：

```text
Feature Facade
    ↓
Legacy Service
    ↓
Manager
    ↓
New Domain System
    ↓
Adapter
    ↓
Backend
```

完成后收敛到：

```text
Controller
    ↓
Domain
    ↓
Port
    ↓
Infrastructure
```

例如 Permission：

```text
Before

PermissionService
  ↓
Tool Policy
  ↓
Agent Permission
  ↓
PermissionKernel
  ↓
InteractionBroker
  ↓
ApprovalQueue
  ↓
PermissionApprovalBroker
  ↓
ExecutionBroker
  ↓
DirectRunner / Rust Sandbox
```

变为：

```text
After

PermissionService
  ↓
IntentFactory
  ↓
PolicyCompiler
  ↓
PermissionKernel
  ↓
ApprovalQueue
  ↓
ApprovalBroker
  ↓
ExecutionPermit

RustSandboxClient
  ↓
actual execution
```

---

# 54. 最终目标与模块保留判断

Nabla 的重构目标不应该是：

```text
更多目录
更多 Service
更多 interface
更小的单文件
```

而应该是：

```text
更少的权威实现
更明确的领域 owner
更少的无意义包装层
更稳定的依赖方向
更容易验证的安全语义
```

判断一个模块是否保留，按四个问题：

1. **它是否拥有独立状态？**
2. **它是否拥有独立业务规则？**
3. **它是否隔离一个明确外部系统？**
4. **它是否形成一个真实稳定的公共边界？**

如果四个答案都是“否”，而该模块只是：

```text
A.foo()
  → B.foo()
```

那么删除。

---

# 55. 最终架构原则（已决策）

整个 Nabla 长期遵循以下原则：

1. 一个概念只有一个权威实现。
2. Domain 不依赖 concrete transport。
3. Protocol 不依赖业务实现。
4. Permission 不执行命令。
5. Executor 不判断业务权限。
6. Workspace 不拥有 Plan。
7. Tree / Session 不恢复别的领域状态。
8. Adapter 只负责边界转换。
9. Facade 必须形成真实边界，否则删除。
10. Manager 与 Service 不应同时暴露同一套 API。
11. Rust UI renderer 不依赖完整 AppState。
12. Rust UI 不重复实现 selection/text 等纯算法。
13. 安全相关的双层防御保留，但必须共享规则来源。
14. 模块化的目标是减少认知负担，而不是增加文件数量。

---

# 56. 最终检查清单

完成每一轮重构后，逐项检查：

- [ ] `PermissionRule` 是否只有一种？
- [ ] Shell read-only 判断是否只有一个入口？
- [x] Shell risk 是否直接来自 ShellAnalysis？
- [ ] path canonicalization 是否只有一份？
- [ ] workspace containment 是否只有一份 primitive？
- [ ] Plan mode 是否只有一个 state owner？
- [ ] Tree 是否不再恢复 Plan？
- [ ] Workspace 是否不再控制 Plan？
- [ ] Approval / Question / Auth 是否共享 pending lifecycle？
- [ ] `InteractionBroker` 是否保持薄 facade？
- [ ] Permission 的 Grant Broker 是否保持 `ApprovalBroker` 命名？
- [ ] PermissionAdapter 是否保持现状（不执行接口收敛）？
- [ ] `ExecutionBroker` / `DirectRunner` / `ShellFallback` 是否已删除？
- [ ] Bash 是否只有 Rust Sandbox 一个真实 executor？
- [x] `ContextService` 是否仍只是转发？
- [ ] `harness.ts` 是否已逐步拆空？
- [ ] `session-navigation.ts` 是否已经拆成 session 子模块？
- [x] `WorktreeManager` 是否仍作为大型公开 API？
- [ ] `features/permissions` 是否是唯一 Permission root？
- [ ] `policy/` 是否已经删除？
- [ ] Rust selector 是否复用 `selection.rs`？
- [ ] Rust renderer 是否都复用 `ui/text.rs`？
- [x] `scene.rs` / `transcript.rs` 是否按职责拆分？
- [ ] protocol schema 和 validator 是否来自同一来源？
- [ ] Rust/TS protocol fixture 是否双向测试？
- [ ] `create-host-app.ts` 是否只负责依赖装配？

---

# 57. 结论

针对 Nabla `7bc6436`，最重要的重构方向并不是立即全面拆文件，而是先删除重复的“业务权威”。

最核心的重复依次是：

```text
Permission
Shell
Path
Plan
Execution
Interaction
```

这六类收敛完成以后，再拆：

```text
harness
sessions
context
worktree
Rust UI
Rust host/process
```

会安全得多。

优先目标应从：

```text
“大文件变小”
```

转成：

```text
“一个领域只剩一个真相来源”
```

这是 Nabla 后续继续增加 agent、sandbox、permission、plan、session 和 TUI 功能时，最能降低长期复杂度的重构方向。
