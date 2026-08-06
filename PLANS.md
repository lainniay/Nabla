# Nabla `Plan as Artifact` 最终执行计划

> 面向对象：Coding Agent  
> 目标仓库：`https://github.com/lainniay/Nabla`  
> 目标改动：`refactor: make plan an artifact`  
> 兼容策略：**不兼容旧 Plan schema、旧 session entry 或旧协议字段，直接删除**

---

## 1. 目标

将当前 Plan 从带有运行生命周期的对象：

```text
submitted -> executing -> completed / submitted(error)
```

收敛为一个静态、可版本化、可跨 session 传递的实施 artifact：

```text
Plan mode
  -> submit_plan
  -> PlanArtifact
  -> 用户选择 Execute / Fresh execute / Close
  -> 普通 Agent Turn
```

本次改造后必须满足：

1. Plan 不再拥有 `submitted`、`executing`、`completed` 状态。
2. Agent turn 成功、失败、取消、compaction 均不修改 Plan artifact。
3. `/plan` 只负责进入 Plan mode，以及可选地提交一段规划请求。
4. `submit_plan` 只负责提交最终 Plan artifact，不自动执行，不选择执行上下文。
5. Plan review 只保留三个选项：
   1. `Execute`
   2. `Fresh execute`
   3. `Close`
6. `Execute` 在当前 session 中启动普通实施 turn。
7. `Fresh execute` 创建新 session，并通过完整 Plan artifact 和 handoff 内容启动普通实施 turn。
8. Plan 在低剩余上下文、compaction 后、fresh session 和不同模型 context window 下都必须能够可靠传递。

---

## 2. 非目标

本次不要同时进行以下工作：

- 不实现 Goal 或 WorkflowRun。
- 不实现通用 Artifact framework。
- 不重构子代理 Task Runtime。
- 不拆分整个 `HostBridge`。
- 不重写 TUI modal 系统。
- 不实现 Plan 自动完成判断。
- 不增加 reviewer/verifier 固定流程。
- 不实现旧 Plan schema migration。
- 不保留 deprecated command、event 或字段。

如果 coding agent 在实际 checkout 中发现 Goal 残留，只处理会阻塞本次 Plan 改造的直接编译依赖，不扩大本 PR 范围。

---

## 3. 最终用户交互

### 3.1 `/plan`

唯一入口：

```text
/plan [optional planning prompt]
```

行为：

#### `/plan`

- 当前不在 Plan mode：进入 Plan mode。
- 当前已经在 Plan mode，且存在最新 Plan artifact：重新打开 Plan review。
- 当前已经在 Plan mode，但没有 Plan artifact：显示简短提示，不做 toggle。

#### `/plan <prompt>`

- 当前不在 Plan mode：先切换到 Plan mode，切换成功后再提交 prompt。
- 当前已经在 Plan mode：直接作为普通规划 prompt 提交。

删除以下语法：

```text
/plan exit
/plan status
/plan run
/plan run current
/plan run fresh
```

这些字符串不再特殊处理；输入 `/plan exit`、`/plan status`、`/plan run` 等时，`exit`/`status`/`run` 与其它参数一样作为普通规划 prompt 文本提交，不报错、不提示。

`/plan <prompt>` 始终提交规划 prompt；只有无参数 `/plan` 重新打开已有 Review。

退出 Plan mode 继续使用已有的统一 mode switching 交互，不新增 Plan 专用退出方式。Plan mode 状态应由状态栏和 composer 样式展示，不通过 `/plan status` 查询。

---

### 3.2 `submit_plan`

`submit_plan` 是 Plan mode 的终止工具：

```text
模型调查仓库
  -> 必要时 ask_user
  -> submit_plan
  -> 当前规划 turn 结束
  -> TUI 打开 Plan review
```

工具本身不包含：

- `execute`
- `fresh`
- `executionContext`
- `status`
- `completed`
- `autoRun`

执行方式由用户在 Plan review 中选择。

---

### 3.3 Plan review

固定三个选项，顺序不可变化：

```text
1. Execute
2. Fresh execute
3. Close
```

Plan review 是模态 overlay：打开期间键盘由 review 捕获（`UiModalKind::PlanReview`），只能选择 `Execute` / `Fresh execute` / `Close`；用户先 `Close` 后再提交修订 prompt。本 PR 不改造 modal 系统。

#### Execute

- 关闭 Plan review。
- 退出 Plan mode。
- 在当前 session 中提交普通实施 prompt。
- 不修改 Plan artifact。
- 不创建 Plan execution state。

#### Fresh execute

- 关闭 Plan review。
- 创建一个新 session，并保留与原 session 的 parent/branch 关系。
- 在新 session 中写入同一个 Plan artifact。
- 在新 session 中写入 `planMode.active = false`。
- 切换到新 session。
- 使用 self-contained handoff prompt 启动普通实施 turn。
- 不复制完整 planning transcript。
- 不修改原 Plan artifact。

#### Close

- 仅关闭 Plan review。
- 不执行。
- 不删除 Plan artifact。
- 保持 Plan mode active，使用户可以继续规划或修订。
- 用户再次执行 `/plan` 时可以重新打开 review。

### 3.4 不保留二次确认页

删除当前 `Menu -> Confirm` 两阶段 UI。

用户在 Plan review 中选择 `Execute` 或 `Fresh execute` 后直接 dispatch。为防止重复提交，review state 只需要 `submitting` 标记；提交期间禁用所有选项。

---

## 4. 最终 Plan 数据模型

保留 artifact identity 和 revision，因为它们用于：

- 区分不同 Plan revision。
- 在 current/fresh session 之间引用同一个 artifact。
- 在 context checkpoint 中判断 Plan 是否已经存在。
- 避免 compaction 后重复注入相同 revision。

它们不是运行生命周期。

### 4.1 TypeScript

```ts
export const PLAN_ENTRY_TYPE = "nabla.plan";
export const PLAN_MODE_ENTRY_TYPE = "nabla.plan-mode.v1";

export interface PlanContent {
  title: string;
  summary: string;
  bodyMarkdown: string;
  assumptions: string[];
  testPlan: string[];

  /**
   * Fresh session 所需的最小上下文交接。
   * 必须包含正文中未充分体现、但实施阶段不可丢失的决策、约束、
   * 关键文件和未解决风险。不得复制完整 planning transcript。
   */
  handoffMarkdown: string;
}

export interface PlanArtifact extends PlanContent {
  id: string;
  revision: number;
  sourceSessionId: string;
  createdAt: string;
  updatedAt: string;
}
```

直接删除：

```ts
schemaVersion
PlanStatus
status
lastExecutionError
LEGACY_PLAN_ENTRY_TYPE
PLAN_EXECUTION_MESSAGE_TYPE
RestoreResult.recovered
```

### 4.2 Rust

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub id: String,
    pub revision: u64,
    pub title: String,
    pub summary: String,
    pub body_markdown: String,
    pub assumptions: Vec<String>,
    pub test_plan: Vec<String>,
    pub handoff_markdown: String,
    pub source_session_id: String,
    pub created_at: String,
    pub updated_at: String,
}
```

直接删除：

```rust
PlanStatus
schema_version
status
last_execution_error
```

---

## 5. PlanStore 收敛

`PlanStore` 可以继续作为当前 active session 的轻量 projection，但不再是状态机。

最终只保留：

```ts
class PlanStore {
  latest(): PlanArtifact | undefined;
  clear(): void;
  restore(entries: readonly PlanSessionEntry[]): PlanArtifact | undefined;
  submit(content: PlanContent, sourceSessionId: string): PlanArtifact;
  adopt(artifact: PlanArtifact): void;
}
```

### 5.1 `submit`

规则：

- 没有现有 artifact：创建新 `id`，`revision = 1`。
- 已有 artifact：保留 `id`，`revision += 1`。
- 保留第一次提交的 `createdAt`。
- 更新 `updatedAt`。
- 不检查执行状态。
- `handoffMarkdown` 必须非空（trim 后仍有内容），否则 `submit_plan` 报错。
- 任意时刻只要当前 session idle 且处于 Plan mode，即可提交新 revision。

### 5.2 `restore`

- 只识别 `customType === "nabla.plan"`。
- 只读取当前 branch 最后一个合法 Plan entry。
- 不识别 `nabla.plan.v1`、`nabla.plan.v2`。
- 不做 schema migration。
- 不产生 interrupted warning。
- 不向 session 回写迁移后的 entry。

### 5.3 删除方法

```ts
markExecuting()
markSubmitted()
markCompleted()
normalizeStoredPlan() 的旧 schema 分支
```

`nextTimestamp()` 保留（`submit` 仍用于 revision 时间单调性），改名为 `nextArtifactTimestamp()`。

---

## 6. Context window 与跨 session 传递设计

这是本次改造的必要部分，不是后续增强。

### 6.1 Plan mode system prompt 必须包含动态上下文余量

当前静态 `PLAN_INSTRUCTIONS` 改为函数：

```ts
function buildPlanInstructions(snapshot: ContextSnapshot): string
```

提示中至少包含：

```text
Context window status
- Usage source: actual | estimated | recalculating（不可用时 UI 显示 unknown）
- Context window: N tokens
- Used: N tokens / N%
- Remaining: N tokens / N%
```

计算规则：

```ts
usedTokens = snapshot.actualTokens ?? snapshot.estimatedNextRequestTokens;
usedPercent = snapshot.actualPercent
  ?? (snapshot.contextWindow
      ? usedTokens / snapshot.contextWindow * 100
      : null);
remainingTokens = contextWindow === null
  ? null
  : Math.max(0, contextWindow - usedTokens);
remainingPercent = usedPercent === null
  ? null
  : Math.max(0, 100 - usedPercent);
```

`unknown` 仅是展示层 fallback（usageState / contextWindow 不可用时显示）；`ContextUsageState` 枚举保持 `"actual" | "estimated" | "recalculating"`，不新增值。

提示必须明确告诉模型：

```text
The submitted plan must be self-contained.
Fresh execute receives the Plan artifact and handoff only, not the full planning transcript.
Do not rely on phrases such as "as discussed above" or references that require the original transcript.
Include critical decisions, relevant files, constraints, and unresolved risks in the artifact.
Keep handoffMarkdown concise and implementation-oriented.
```

不要把 `remainingTokens` 或 `remainingPercent` 写入 Plan artifact，因为它们在稍后执行时会失效。

### 6.2 `handoffMarkdown`

`handoffMarkdown` 必须覆盖：

- 用户原始目标的精简复述。
- 规划期间作出的关键技术决定。
- 必须保留的用户约束。
- 关键文件、模块和接口。
- 已确认不做的事项。
- 未解决问题或实施风险。
- 对 fresh session 有用、但未自然写入 `bodyMarkdown` 的信息。

禁止：

- 完整复制 transcript。
- 复制大段工具输出。
- 使用“见上文”“如前所述”等依赖源 session 的表达。
- 写入当前 context 使用百分比。

### 6.3 Current execution 在低剩余上下文下

`Execute` 仍然允许在低余量 session 中启动，因为：

- 用户明确选择了 current context。
- 当前 conversation 可能包含高价值隐式信息。
- 后续 compaction 应由现有 context manager 处理。

必须保证 compaction 后 Plan 不丢失：

- `ContextActiveState.plan` 使用新的 `PlanArtifact`。
- `PLAN_ENTRY_TYPE` entry 不参与首次普通模型上下文投影，仅用于持久化、TUI、restore 与 compaction 数据源；首次 implementation prompt 是唯一完整 Plan 副本。
- 发生 compaction 时 checkpoint 重新注入完整 Plan；`{id, revision}` marker 不视为完整 Plan。
- `containsPlanRevision()` 只识别完整 Plan 文本（`id + revision`），marker 不视为完整 Plan。
- 删除对 `status`、`schemaVersion`、`lastExecutionError` 的任何依赖。

### 6.4 Fresh execution

Fresh session 只接收：

1. 正常 system prompt、资源、skills 和 workspace context。
2. Plan artifact entry。
3. `planMode.active = false` entry。
4. 一条实施 handoff prompt。

不接收完整源 transcript。

Plan artifact entry 用于持久化、TUI、restore 与 compaction 数据源，不投影进首次模型上下文；完整 Plan 只出现在 implementation prompt 中，compaction 后由 checkpoint 重新注入。

实施 prompt 固定格式：

```text
You are implementing an approved Nabla plan in a fresh session.
The planning transcript is not available in this session.
Re-check repository state before editing because files may have changed.
Treat the plan as implementation guidance, not an immutable workflow.
Report material deviations from the plan.

## Source objective and handoff
{artifact.handoffMarkdown}

## Approved plan
# {artifact.title}

{artifact.summary}

{artifact.bodyMarkdown}

## Assumptions
- ...

## Test plan
- ...
```

### 6.5 不允许静默截断

transfer budget 只在 `Fresh execute` 时强制执行，用继承模型的 `contextWindow` 校验（§9 第 3 步确定的模型）。`Execute` 不校验：低余量由现有 context manager / compaction 处理。

```ts
const PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS = 24_000;
const PLAN_TRANSFER_MAX_CONTEXT_FRACTION = 0.25;

allowed = Math.min(
  PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS,
  Math.floor(contextWindow * PLAN_TRANSFER_MAX_CONTEXT_FRACTION),
);
```

contextWindow 为 null 时 `allowed = PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS`（24k 绝对上限兜底），不产生 NaN。

对最终 implementation prompt 的实际完整文本（含 handoff 模板与完整 Plan）使用现有 `estimateTextTokens`（`ceil(length / 4)`）估算。

行为：

- 仅 `Fresh execute` 时校验，超限返回清晰错误，要求返回 Plan mode 缩短 Plan。
- 不裁剪 `bodyMarkdown`、不裁剪 `handoffMarkdown`。
- 不自动省略 test plan 或 assumptions。

`estimateTextTokens` 已存在于 `agent-host/src/context-manager.ts`（本地函数，未导出），且 context-manager 对 `plan.ts` 只是 `import type`，没有运行时环依赖。直接 `export` 现有函数，供 `plan.ts` / `plan-execution.ts` 引用；不新增 `token-estimate.ts`，不复制实现。

---

## 7. Host 执行协议

### 7.1 单一 RPC command

将：

```text
execute_plan_current
execute_plan_fresh
```

合并为：

```text
plan_execute
```

请求：

```json
{
  "context": "current" | "fresh"
}
```

响应：

```json
{
  "sessionId": "...",
  "context": "current" | "fresh"
}
```

响应中不再返回 artifact，因为执行不会修改 artifact。

删除事件：

```text
plan_executing
plan_completed
plan_execution_error
```

保留事件：

```text
plan_ready
plan_state
plan_mode_state
```

### 7.2 普通 Agent Turn

删除：

```text
PLAN_EXECUTION_MESSAGE_TYPE
nabla.plan.execution.v1
completePlanExecution()
executionFailed()
agent_settled -> completePlanExecution
```

执行使用固定依赖 `@earendil-works/pi-coding-agent@0.83.0` 已确认存在的普通用户 prompt API（`AgentSession.prompt(text, options?): Promise<void>`）：

```ts
void session.prompt(planImplementationPrompt(artifact)).catch((error) => {
  // 按普通 turn 错误处理，与 Plan artifact 无关
});
```

必须 fire-and-forget（`void` + `.catch`），不要 `await`：`prompt()` 要等实施 turn 结束才 resolve，await 会让 `plan_execute` RPC 阻塞到整个实施结束，违背 §8 第 11 步。

验收条件是仓库中不再存在：

```text
PLAN_EXECUTION_MESSAGE_TYPE
nabla.plan.execution
```

不要通过换名为 `nabla.plan.execution.v2` 保留特殊 lifecycle transport。

---

## 8. Current execute 具体流程

新增：

```text
agent-host/src/plan-execution.ts
agent-host/src/plan-execution.test.ts
```

接口示意：

```ts
export type PlanExecutionContext = "current" | "fresh";

export interface PlanExecutionResult {
  sessionId: string;
  context: PlanExecutionContext;
}
```

Current 流程：

1. 获取 `plans.latest()`；不存在则失败。
2. 检查当前 session `isIdle`。
3. 构造实施 prompt。
4. 不执行 transfer budget 校验（Current 不强制，低余量由现有 compaction 处理）。
5. 将 Plan mode 切换为 false。
6. 将 `{ active: false }` 追加为 `PLAN_MODE_ENTRY_TYPE`。
7. 通过 `void session.prompt(planImplementationPrompt(artifact)).catch(...)` 在当前 session fire-and-forget 启动实施 turn。
8. 返回 `{ sessionId, context: "current" }`。
9. 不修改 PlanStore。
10. 不 append 新 Plan entry。
11. 不等待整个实施 turn 完成。

失败规则：

- 调用 `session.prompt()` 之前的失败（session busy、无 Plan、prompt 构造失败、fresh 创建失败、budget 超限）：RPC 返回错误，TUI review 保持打开。
- `session.prompt()` 返回的 Promise 异步 reject：RPC 已成功返回，按普通 turn error 展示，不重新打开 review、不修改 Plan artifact。

---

## 9. Fresh execute 具体流程

1. 获取 `plans.latest()`；不存在则失败。
2. 检查当前 session `isIdle`。
3. 确定继承目标：源 session 当前 model 与 thinking level；若该 model 无法在新 session 使用（provider/权限不可用），直接失败，不静默回退其他模型。
4. 构造 fresh handoff prompt。
5. 预算预检：用继承模型的 context window 计算 allowed；contextWindow 未知时退化为 24k 绝对上限。超限直接失败，不创建新 session。
6. 保存源 session 和 parent session 文件信息。
7. 通过 `newSession({ parentSession, setup })` 创建新 session（0.83.0 中 `session_start` 先于 setup 执行）。
8. 在新 session setup 中追加（仅用于持久化与 restore，不投影进首次模型上下文）：
   - `PLAN_ENTRY_TYPE` + 同一个 artifact。
   - `PLAN_MODE_ENTRY_TYPE` + `{ active: false }`。
9. 激活新 session。
10. `newSession()` resolve 后应用继承的 model 与 thinking level（`setModel` + `setThinkingLevel`），然后 `plans.adopt(artifact)` 并 `send({ type: "plan_state", artifact })`（替代旧 `replacementPlan` 桥接）。
11. 通过 `void session.prompt(planImplementationPrompt(artifact)).catch(...)` fire-and-forget 启动实施 turn。
12. 返回 `{ sessionId, context: "fresh" }`。

步骤 1-6 的任何失败都发生在 session 切换之前：不创建新 session、不留半完成状态，无需回滚。`session_start` 的普通 restore 路径会对空 branch 发 `plan_state: null`，随后第 10 步再发 artifact；接受这个顺序，TUI 幂等覆盖，不新增临时标志。

### 9.1 `replacementPlan` 删除方案

`AgentSessionRuntime.newSession()`（0.83.0）的执行顺序是 `session_start` 事件先于 `options.setup()`，所以 setup 写入的 Plan entry 在 `session_start` 时不可见——现有 `replacementPlan` 桥接正是为此存在。

删除：

- `private replacementPlan?: PlanArtifact` 字段。
- `session_start` handler 中 `replacementPlan && !entries.some(...)` 的桥接分支；handler 只保留普通 restore 路径（fresh 新 session 无 Plan entry，返回 null）。

fresh 流程在 `newSession()` resolve 后确定性补上状态（§9 第 10 步），不依赖事件顺序、不写入协议、无临时字段。

---

## 10. Rust TUI 状态模型

### 10.1 执行上下文

保留 current/fresh 选择，但改名以避免生命周期语义：

```rust
pub enum PlanExecutionContext {
    Current,
    Fresh,
}
```

### 10.2 Review state

将当前 enum：

```rust
PlanReviewState::Menu
PlanReviewState::Confirm
```

替换为：

```rust
pub struct PlanReviewState {
    pub selected: usize,
    pub submitting: bool,
}
```

固定索引：

```text
0 = Execute
1 = Fresh execute
2 = Close
```

### 10.3 键盘行为

- `Up` / `BackTab`：上一项。
- `Down` / `Tab`：下一项。
- `Enter`：执行当前选择。
- `Esc`：等价于 Close。
- submitting 时忽略所有键。

删除：

- `Y/N` 二次确认。
- Confirm 页面。
- `target.label()` confirmation 文案。

### 10.4 Close 行为

```rust
self.state.plan_review = None;
```

不要自动退出 Plan mode。

### 10.5 Execute/Fresh execute 行为

- 设置 `submitting = true`。
- 设置 `run_state = RunState::Submitting`。
- 返回 `AppEffect::ExecutePlan(context)`。

RPC 失败：

- `submitting = false`。
- review 保持打开。
- `run_state = Idle`。
- 展示错误。

RPC 成功：

- review 关闭。
- `plan_mode_active = false`。
- `pending_plan_mode = None`。
- fresh 时更新 session id，并重置 session-local UI 缓存。
- Plan artifact 保持原值。

---

## 11. 需要修改的模块和文件

实施前必须在本地 checkout 执行全文检索，以下是当前仓库结构下的预期修改范围。

### 11.1 TypeScript Host

#### `agent-host/src/plan.ts`

- 将 `PLAN_ENTRY_TYPE` 改为 `nabla.plan`。
- 删除 legacy entry type。
- 删除 execution message type。
- 删除 `PlanStatus`。
- 将 `PlanArtifactV2` 改为 `PlanArtifact`。
- 增加 `handoffMarkdown`。
- 删除 schemaVersion、status、lastExecutionError。
- 删除 lifecycle transition methods。
- 简化 restore。
- 更新 validation。
- 只导出 implementation prompt 构建函数，不组装 fresh 完整 prompt。

#### `agent-host/src/main.ts`

- 修改 `submit_plan` TypeBox schema，增加 `handoffMarkdown`。
- `submit_plan` 只 append artifact 并发送 `plan_ready`。
- 动态构造 Plan instructions，注入 context remaining。
- 删除 `agent_settled` Plan completion handler。
- 删除两个旧 execute command 分支。
- 添加统一 `plan_execute` 分支。
- 删除 lifecycle events。
- 删除 `completePlanExecution()`。
- 删除 `executionFailed()`。
- 删除 `PLAN_EXECUTION_MESSAGE_TYPE` 使用。
- 调用新的 plan execution helper。
- 删除 `replacementPlan` 字段与 `session_start` 桥接分支；fresh 成功后 `adopt` + 发送 `plan_state`。

#### `agent-host/src/context-manager.ts`

- 更新 Plan type import。
- 更新 context checkpoint 中的 Plan shape。
- 过滤 `PLAN_ENTRY_TYPE`，使其不进入首次普通模型上下文投影。
- checkpoint 仅在 compaction 后注入完整 Plan；`{id, revision}` marker 不视为完整 Plan。
- 完整 Plan 已出现在 post-compaction 消息中时不重复注入。
- 删除 lifecycle 字段依赖。
- 直接导出 `estimateTextTokens`，供 plan.ts / plan-execution.ts 引用（不新增文件）。

#### `agent-host/src/plan-execution.ts`（新增）

- 封装 current/fresh dispatch。
- 组装 fresh 完整 prompt（含 handoff 模板），调用 plan.ts 导出的构建函数。
- 执行 transfer budget 检查。
- 不保存运行状态。

#### `agent-host/src/protocol/contracts.ts`

- 将 `PlanArtifactV2` 改为 `PlanArtifact`。
- 更新 bootstrap Plan validation。
- 删除 schemaVersion/status/lastExecutionError 验证。
- 增加 handoffMarkdown 验证。

#### `agent-host/src/plan.test.ts`

- 删除 migration/lifecycle tests。
- 重写 artifact/revision/restore tests。

#### `agent-host/src/session.test.ts`

- 重写 fresh-session 测试（当前使用 legacy schema + `PLAN_EXECUTION_MESSAGE_TYPE`，改造后必挂）。
- 断言 fresh setup 含新 PlanArtifact 与 `{ active: false }` mode entry，且不含 execution message。

#### `agent-host/src/plan-execution.test.ts`（新增）

- 测试 current/fresh dispatch 和 context transfer。

#### `agent-host/src/context-manager.test.ts`

- 更新 Plan fixture。
- 测试 compaction checkpoint 保留 Plan。
- 测试不重复注入相同 revision。

#### `agent-host/src/protocol-contract.test.ts`

- 更新共享 fixture 断言。

#### `agent-host/package.json`

通常无需修改；确认新增测试仍被 `node --test src/*.test.ts` 自动发现。

---

### 11.2 Rust Host 协议

#### `src/host.rs`

- 删除 `PlanExecutionData.artifact` 和 `fresh: bool` 旧结构。
- 改为：

```rust
pub struct PlanExecutionData {
    pub session_id: String,
    pub context: PlanExecutionContext,
}
```

- 将两个 command 合并为 `plan_execute`，传 context 参数。
- 更新 serde tests。

#### `src/event.rs`

保留一个 command completion：

```rust
PlanExecutionFinished {
    context: PlanExecutionContext,
    result: Result<Box<PlanExecutionData>, String>,
}
```

不增加 completion/failed Plan event。

#### `src/runtime.rs`

- 修改 `AppEffect::ExecutePlan(context)` dispatch。
- 调用统一 `host.execute_plan(context)`。

#### `src/app.rs`

- 更新 `AppEffect` 的 Plan 类型 import。
- 不添加 Plan lifecycle effect。

---

### 11.3 Rust Domain State

#### `src/state/planning.rs`

- 删除 `PlanStatus`。
- 修改 `PlanArtifact`。
- `PlanExecutionTarget` 改名为 `PlanExecutionContext`。
- 将 `PlanReviewState` 收敛为 struct。

#### `src/state/app_state.rs`

- 检查 `plan`、`plan_review`、`plan_mode_active` 初始化。
- 新增 `pending_plan_prompt: Option<String>` 字段并处理初始化/清空。
- 不新增 Plan runtime 状态。
- review 可以直接读取现有 `ContextSnapshot` 展示余量。

#### `src/state/transcript.rs`

- `TranscriptItem::Plan` 保留。
- 确保类型使用新的 PlanArtifact。

---

### 11.4 Rust Command 与 Reducer

#### `src/command.rs`

- `/plan` description 改为“Enter Plan mode and optionally submit a planning prompt”。
- 保留 `LocalCommand::Plan(Option<String>)`。
- 不解析 Plan 子命令。

#### `src/app/actions.rs`

- 删除 `exit/status/run/run current/run fresh` 分支。
- 删除 `PlanStatus::Submitted` gate。
- 实现 `/plan [prompt]` 行为。
- 新增 `pending_plan_prompt: Option<String>`：未处于 Plan mode 时 `/plan <prompt>` 先 `SetPlanMode(true)` 并缓冲 prompt；`SetPlanModeFinished` 成功后清 `pending_plan_mode` 并调用 `prepare_delivery(prompt, PromptDelivery::Prompt)`。
- 任意 `<prompt>`（含 `exit`/`status`/`run` 等字符串）作为普通规划 prompt 提交，不报错、不提示。
- `receive_plan()` 对任意新 revision 打开 review，不检查 status。

#### `src/app/workflow_input.rs`

- 删除 Menu/Confirm 两阶段逻辑。
- 固定三项选择。
- 删除 Y/N 快捷确认。
- Close 保持 Plan mode。

#### `src/app/command_events.rs`

- 成功响应不覆盖 `state.plan`。
- 删除“Executing plan …”生命周期文案。
- 改为“Started plan rN in current/fresh context”。
- 失败时保持 review。

#### `src/app/host_events.rs`

- `plan_ready` 继续调用 `receive_plan(artifact, true)`。
- 删除 `plan_executing`、`plan_completed`、`plan_execution_error` handlers。
- `plan_state` 仅恢复静态 artifact；fresh 场景接受先 `null` 后 artifact 的顺序，幂等覆盖，不新增标志。

#### `src/app/session_flow.rs`

- 恢复 session 时恢复 Plan artifact。
- 不根据 status 自动打开 Plan review。
- fresh session 激活时正常显示 artifact，但执行由 Host 已提交的普通 turn 驱动。

#### `src/app/tests.rs`

- 删除 lifecycle tests。
- 增加最终交互与 current/fresh 测试。

---

### 11.5 Rust UI

#### `src/ui/scene.rs`

Plan review 只渲染：

```text
Execute
Fresh execute
Close
```

固定描述：

```text
Execute       Continue in this conversation
Fresh execute Start a new session with the Plan and handoff
Close         Keep the Plan without executing
```

Panel 顶部可显示现有 context snapshot：

```text
Current context remaining: 28% (estimated)
```

删除 Confirm panel。

#### `src/ui/transcript.rs`

- Plan renderer 删除 status badge。
- 删除 last execution error。
- 保留 title、summary、revision、body、assumptions、test plan。
- handoffMarkdown 默认不在主 transcript 全量展示；在 expanded view 中展示，避免噪音。

---

### 11.6 Shared Protocol Fixture

#### `protocol-fixtures/bootstrap-state.json`

Plan 改为：

```json
{
  "artifact": {
    "id": "plan-contract",
    "revision": 3,
    "title": "Contract plan",
    "summary": "Exercise every Plan field",
    "bodyMarkdown": "# Contract plan\n\nImplement it.",
    "assumptions": ["The fixture is intentional"],
    "testPlan": ["Round-trip both protocol implementations"],
    "handoffMarkdown": "Preserve the protocol contract and update both Rust and TypeScript.",
    "sourceSessionId": "session-contract",
    "createdAt": "2026-07-31T00:00:00.000Z",
    "updatedAt": "2026-07-31T00:01:00.000Z"
  }
}
```

删除：

```text
schemaVersion
status
lastExecutionError
```

---

## 12. 测试计划

### 12.1 TypeScript：`plan.test.ts`

删除以下旧测试：

- legacy schema migration。
- interrupted execution recovery。
- lifecycle timestamp transition。
- invalid status jump。
- executing 时禁止 revision。
- completed 状态。

新增或保留：

1. 首次 submit 创建 `revision = 1`。
2. 再次 submit 保持 id 并增加 revision。
3. revision 更新不依赖任何 status。
4. `createdAt` 保持，`updatedAt` 更新。
5. `clear()` 后创建新 id 和 revision 1。
6. restore 只读取最后一个合法 `nabla.plan` entry。
7. 旧 `nabla.plan.v1/v2` entry 被忽略。
8. malformed Plan entry 被忽略。
9. Plan mode 从当前 branch 最后一条 mode entry 恢复。
10. `handoffMarkdown` 必须非空。
11. Plan 正文、assumptions 和 test plan 正常规范化。
12. implementation prompt 包含完整 artifact 和 handoff。
13. implementation prompt 不包含 status/completed/executing。

### 12.2 TypeScript：`plan-execution.test.ts`

Current：

1. 不创建新 session。
2. 退出 Plan mode。
3. 追加 mode inactive entry。
4. 只调用一次普通 prompt API。
5. Plan artifact deep-equal，不发生 mutation。
6. 不 append 新 Plan entry。
7. 不发送 lifecycle event。
8. session busy 时在任何 mutation 前失败。
9. 没有 Plan artifact 时在任何 mutation 前失败。
10. 调用 `session.prompt()` 前的失败（busy、无 Plan、构造失败）RPC 返回错误；Promise 异步 reject 按普通 turn error 处理，不重新打开 review。
11. 不等待实施 turn 完成：`prompt()` fire-and-forget，RPC 响应在 dispatch 后立即返回。

Fresh：

1. 创建新 session。
2. 保留 parent session 关系。
3. setup 中写入相同 Plan artifact。
4. setup 中写入 mode inactive。
5. 切换到新 session。
6. fresh prompt 包含 handoff 和完整 Plan。
7. 不复制完整源 transcript。
8. Plan id/revision 不变。
9. 原 session Plan 不变化。
10. 新 session 创建失败时不提交 prompt。
11. 继承源 session 当前 model 与 thinking level：`newSession()` resolve 后 `setModel` + `setThinkingLevel`。
12. 源 model 无法在新 session 使用时，在创建 session 前返回错误，不静默回退其他模型。
13. 不依赖 `replacementPlan`：`newSession()` resolve 后 adopt 并发送 `plan_state`；session_start 先发的 `null` 由 TUI 幂等覆盖。
14. 预算预检发生在 `newSession()` 之前：已知 contextWindow 按继承模型窗口计算，未知按 24k 绝对上限兜底；只有估算超限才失败，失败不创建新 session、源 session 不变。
15. 超限时不静默截断，返回可读错误。
16. transfer budget 在 32k context 下拒绝过大 Plan。
17. transfer budget 在 64k/128k context 下允许合理 Plan。
18. contextWindow 为 null 时按 24k 绝对上限校验：超过 24k 拒绝、以下放行，不产生 NaN。
19. fresh session 包含相同 Plan artifact 与 `{ active: false }` mode entry，且不含 execution message（session.test.ts 场景）。

### 12.3 TypeScript：`context-manager.test.ts`

1. 新 PlanArtifact 可进入 active state。
2. `PLAN_ENTRY_TYPE` entry 不进入首次普通模型上下文投影（仅持久化/TUI/restore/compaction 数据源）。
3. 首次请求不注入完整 Plan（implementation prompt 已含完整 Plan）。
4. compaction 后 checkpoint 注入完整 Plan。
5. 仅存在 `{id, revision}` marker 时仍注入完整 Plan（marker 不视为完整 Plan）。
6. 完整 Plan 已出现在 post-compaction 消息中时不重复注入。
7. 新 revision 会更新 checkpoint key。
8. checkpoint 不包含 status 或 lastExecutionError。
9. context snapshot remaining 计算在 actual usage 下正确。
10. actual usage 不可用时使用 estimatedNextRequestTokens。
11. context window 不可用时 prompt 显示 unknown，不产生 NaN。

### 12.4 TypeScript：protocol contract

1. bootstrap fixture 通过解析。
2. Plan 包含 `handoffMarkdown`。
3. Plan 不包含 schemaVersion/status/lastExecutionError。
4. `plan_execute` current response 解析正确。
5. `plan_execute` fresh response 解析正确。

### 12.5 Rust：`src/host.rs` tests

1. 解析新的 PlanArtifact。
2. 缺 lifecycle 字段正常解析。
3. 共享 bootstrap fixture round-trip。
4. `PlanExecutionContext::Current` 序列化为 `current`。
5. `PlanExecutionContext::Fresh` 序列化为 `fresh`。
6. `plan_execute` 请求参数正确。
7. response 不含 artifact。

### 12.6 Rust：`src/app/tests.rs`

#### `/plan`

1. `/plan` 从 normal 进入 Plan mode。
2. `/plan <prompt>` 先进入 Plan mode，再提交 prompt。
3. 已在 Plan mode 时 `/plan <prompt>` 直接提交。
4. 已在 Plan mode且存在 artifact 时 `/plan` 重新打开 review。
5. 不再识别 `exit/status/run` 为特殊 Plan 子命令；它们与其它参数一样作为普通规划 prompt 提交，不报错、不提示。

#### Plan ready

1. `plan_ready` 保存 Plan artifact。
2. `plan_ready` 打开 review。
3. 更高 revision 替换当前 artifact。
4. 相同或更低 revision 被去重。
5. 不检查 status。

#### Review

1. review 恰好三项。
2. 默认选择 Execute。
3. Execute 返回 current effect。
4. Fresh execute 返回 fresh effect。
5. Close 关闭 review并保持 Plan mode。
6. Esc 等价 Close。
7. submitting 时忽略输入。
8. 不存在 Confirm state。
9. review 打开时键盘输入被 modal 捕获，无法提交新 prompt；Close 后可继续规划。

#### Execution result

1. current 成功关闭 review。
2. current 成功退出 Plan mode。
3. current 成功不修改 artifact。
4. fresh 成功更新 session id。
5. fresh 成功清理必要的 session-local UI 缓存。
6. 失败时 review 保持打开。
7. 失败时 submitting 恢复 false。
8. Agent turn 后续失败不修改 artifact。

#### Session/compaction

1. session restore 恢复 artifact但不自动打开 review。
2. fresh session 包含 artifact。
3. compaction 后 Plan 仍能进入下一次模型 context。
4. 低 context current execute 能正常启动。

### 12.7 Rust UI tests

1. Panel 只显示 Execute、Fresh execute、Close。
2. Panel 不显示 Confirm 子页。
3. Panel 显示当前 context remaining，若未知则显示 unknown。
4. Plan transcript 不显示 status。
5. Plan transcript expanded view 能查看 handoff。

---

## 13. 测试命令

### 13.1 TypeScript Host

```bash
cd agent-host
npm run typecheck
npm test
```

当前 package scripts 预期为：

```text
typecheck = tsc --noEmit
test      = node --test src/*.test.ts
```

针对性运行：

```bash
cd agent-host
node --test src/plan.test.ts
node --test src/plan-execution.test.ts
node --test src/context-manager.test.ts
node --test src/protocol-contract.test.ts
```

### 13.2 Rust

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

针对性运行：

```bash
cargo test plan
cargo test host
cargo test command
cargo test scene
```

---

## 14. 静态清理检查

完成后以下检索必须为空：

```bash
rg -n \
  'PlanStatus|lastExecutionError|last_execution_error|markExecuting|markSubmitted|markCompleted|PLAN_EXECUTION_MESSAGE_TYPE|plan_executing|plan_completed|plan_execution_error|completePlanExecution|executionFailed' \
  agent-host/src src protocol-fixtures
```

```bash
rg -n \
  'execute_plan_current|execute_plan_fresh|nabla\.plan\.execution' \
  agent-host/src src protocol-fixtures
```

```bash
rg -n \
  'nabla\.plan\.v1|nabla\.plan\.v2|LEGACY_PLAN_ENTRY_TYPE|schemaVersion' \
  agent-host/src/plan.ts agent-host/src/plan.test.ts src/state/planning.rs protocol-fixtures/bootstrap-state.json
```

以下结果允许存在：

```text
PlanExecutionContext
AppEffect::ExecutePlan
plan_execute
id
revision
```

因为它们表达用户选择和 artifact identity，而不是 lifecycle。

---

## 15. 推荐提交顺序

### Commit 1

```text
refactor(plan): remove plan lifecycle state
```

修改：

- `agent-host/src/plan.ts`
- `src/state/planning.rs`
- `protocol-fixtures/bootstrap-state.json`
- Plan 基础测试

### Commit 2

```text
refactor(plan): add self-contained handoff and context budget guidance
```

修改：

- `agent-host/src/context-manager.ts`
- `agent-host/src/main.ts` Plan instructions
- `submit_plan` schema
- context tests

### Commit 3

```text
refactor(plan): dispatch current and fresh execution as normal turns
```

修改：

- `agent-host/src/plan-execution.ts`
- `agent-host/src/main.ts`
- Host protocol
- 删除 lifecycle events和 special execution message

### Commit 4

```text
refactor(tui): simplify plan review to execute fresh close
```

修改：

- `src/state/planning.rs`
- `src/app/workflow_input.rs`
- `src/ui/scene.rs`
- `src/app/command_events.rs`

### Commit 5

```text
refactor(command): reduce plan command to a single mode entry
```

修改：

- `src/command.rs`
- `src/app/actions.rs`
- pending Plan prompt delivery

### Commit 6

```text
test(plan): cover artifact transfer across context windows
```

修改所有 Rust/TypeScript tests（含 session.test.ts）和 protocol fixture。

### Commit 7

```text
chore(plan): remove lifecycle symbols and dead code
```

执行全文清理、格式化、clippy 和完整测试。

Commit 1-3 在 TypeScript 侧互相依赖（删除 lifecycle 字段会立即破坏 `main.ts` 的调用点），不强制每个 commit 独立编译，可按需合并；只保证最终 PR 整体可编译、格式化与测试通过，且任何中间状态都不提交 Rust/TypeScript wire shape 不一致的组合。

---

## 16. 手工端到端验收

### 场景 A：普通 Plan

1. 启动 Nabla。
2. 输入 `/plan`。
3. 确认 composer 和状态栏显示 Plan mode。
4. 输入规划请求。
5. 模型调用 `submit_plan`。
6. Plan review 显示恰好三个选项。

### 场景 B：Execute

1. 选择 Execute。
2. Plan mode 退出。
3. 当前 session 启动普通实施 turn。
4. Plan artifact 不出现 executing/completed 状态。
5. turn 失败或取消后 artifact 不变化。
6. 再次 `/plan` 可以打开同一 artifact review。

### 场景 C：Fresh execute

1. 选择 Fresh execute。
2. 创建新 session。
3. 新 session 能找到同一个 Plan id/revision。
4. 新 session 不包含完整 planning transcript。
5. 实施模型能够仅根据 handoff + Plan 理解目标并开始工作。
6. 原 session artifact 不变化。

### 场景 D：低剩余 context

1. 将源 session 推到较低剩余 context。
2. 进入 Plan mode。
3. 确认模型 prompt 中包含正确 context remaining。
4. 提交 Plan。
5. 选择 Execute，确认能启动并在必要时 compaction。
6. compaction 后模型仍能获得 Plan checkpoint。

### 场景 E：较小目标 context window

1. 在源模型 context window 较小（或 Plan 超限）时执行 Fresh execute。
2. 正常大小 Plan 成功传递。
3. 超大 Plan 被明确拒绝。
4. 不发生静默截断。
5. 返回 Plan mode 后缩短 Plan，可以重新执行。

### 场景 F：Close 和修订

1. 提交 Plan。
2. 选择 Close。
3. 保持 Plan mode。
4. 继续输入修订意见。
5. 再次 `submit_plan`，id 保持、revision 增加。
6. 新 revision review 正常打开。

---

## 17. 完成定义

本次改造只有在以下条件全部满足时完成：

```text
Plan = immutable revisioned artifact
Plan mode = read-only planning mode
Execute = normal turn in current session
Fresh execute = normal turn in new session with self-contained handoff
Close = keep artifact and remain in Plan mode
```

并且仓库中不存在：

```text
Plan-specific submitted/executing/completed 状态或逻辑
Plan execution recovery
Plan completion on agent_settled
Plan execution error rollback
Plan-specific execution custom message
/plan status
/plan exit
/plan run current
/plan run fresh
```

最终核心数据流：

```text
/plan [prompt]
  -> Plan mode with context remaining in system prompt
  -> submit_plan(PlanArtifact with handoffMarkdown)
  -> [Execute | Fresh execute | Close]
  -> ordinary Agent Turn or no execution
```
