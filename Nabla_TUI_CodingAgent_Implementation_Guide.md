# Nabla TUI 底层重构：Coding Agent 实现指南

> **用途**：本文件是 Nabla 终端 UI 重构的实现规范，可直接交给 Coding Agent 作为任务背景、架构约束、开发顺序和验收依据。  
> **语言与技术栈**：Rust、Ratatui/Crossterm 或等价终端后端。  
> **总体目标**：保留 terminal native scrollback，同时实现全高度主屏托管、组件化 transcript、通用输入与选择组件，以及原子化 UI 更新。

---

## 1. 最终产品形态

日常主界面使用 terminal **primary/main screen**，占用整个当前可见区域：

```text
Terminal native scrollback
├── 启动前 shell 输出
├── 已提交用户消息
├── 已提交 assistant 内容
├── 已完成工具调用
├── 已完成 diff
└── 其他不可变历史快照

════════════ 当前可见终端区域 ════════════

Nabla-managed primary surface
┌────────────────────────────────────────┐
│ Recent transcript / streaming content  │
│                                        │
│ Optional upward-expanding panel        │
├────────────────────────────────────────┤
│ Composer                               │
├────────────────────────────────────────┤
│ Status line                            │
└────────────────────────────────────────┘
```

大型界面使用 alternate screen：

- `/resume`
- `/tree`
- 完整 transcript viewer
- 大型 diff/patch viewer
- context/compaction manager
- agent manager
- 复杂 approval
- 大型文件浏览器
- 多步骤登录流程

用户体验目标：

- 启动前 shell 内容仍在 native scrollback；
- 当前可见主屏全部由 Nabla 掌管；
- 聊天记录持续进入 terminal native scrollback；
- 用户可直接使用终端原生滚动条；
- 输入框与状态栏固定在底部；
- `/`、`@`、tool picker、简单 approval 面板可向上展开；
- 面板关闭后准确恢复被遮挡内容；
- resize、resume、工具更新和流式输出不产生覆盖、空洞、残影或局部冻结。

---

## 2. 核心架构决策

### 2.1 日常主界面使用 primary screen

主界面 MUST 使用 primary/main screen，不得长期驻留 alternate screen。

原因：

- 保留 native scrollback；
- 保留标准 CLI 体验；
- 退出后输出自然留在终端；
- 用户可使用终端原生复制与滚动。

日常主界面禁止执行：

```text
CSI 3 J
Clear(Purge)
任何等价的“清除 scrollback”操作
```

### 2.2 当前 viewport 始终等于 terminal size

新架构不再维护 4、12、16 行之间反复变化的 inline viewport。

```rust
viewport.width = terminal.columns;
viewport.height = terminal.rows;
```

终端 resize 后更新为新的完整尺寸。动态变化的是内部布局：

```rust
struct MainLayout {
    transcript: Rect,
    panel: Option<Rect>,
    composer: Rect,
    status: Rect,
}
```

### 2.3 Native scrollback 是 append-only sink

已经提交到 native scrollback 的内容必须被视为不可变快照。

应用不能假设可以在其中：

- 按 component ID 定位；
- 原地修改；
- 删除；
- 折叠或展开；
- 修改旧 Markdown 换行；
- 更新工具状态。

正确抽象：

```rust
trait HistorySink {
    fn append(&mut self, blocks: &[CommittedHistoryBlock]) -> io::Result<()>;
}
```

禁止提供随机更新接口。

### 2.4 Canonical transcript 是唯一权威历史

终端当前画面和 native scrollback 都不是状态源。

```rust
struct TranscriptStore {
    order: Vec<ComponentId>,
    components: HashMap<ComponentId, Arc<TranscriptBlock>>,
    revision: u64,
}
```

Canonical transcript 用于当前重绘、resume、tree、viewer、搜索、复制、导出、resize 和 terminal recovery。

### 2.5 大型交互使用 alternate screen

复杂界面 SHOULD 使用 alternate screen。进入时由终端保存 Nabla 主屏，退出时恢复原画面。

---

## 3. 强制不变量

### Terminal ownership

1. 只有 `TerminalDriver` 可以写 stdout。
2. 应用只能清除自己拥有的 primary/alternate surface。
3. 不得清除或覆盖启动前 shell-owned 内容。
4. 不得以“看起来是空白”判断某行属于应用。

### State/frame consistency

5. scene、layout、visual rows、hit map、cursor、terminal plan 必须来自同一 `revision`。
6. 禁止同一帧混用旧 presenter 与新 projection。
7. viewport 是 terminal size 的派生结果，不是业务状态。
8. 只有 terminal commit 成功后，才能推进 `committed_revision` 与 `previous_frame`。

### Transcript/history

9. Canonical transcript 始终保存完整语义数据。
10. Native History 只能追加。
11. 已 committed 的终端快照不可更新。
12. 仍可能变化或重排的内容不得提交 History。
13. resize、resume、recovery 必须从 canonical transcript 重建。

### Geometry

14. 所有视觉坐标基于统一 `VisualRow`。
15. mouse hit test、selection、scroll、search、cursor 使用同一几何模型。
16. 必须区分 UTF-8 byte offset、grapheme index、terminal display column。

### Async

17. 异步任务不得直接修改 UI。
18. 异步结果必须携带 request ID 或 entity revision。
19. 过期结果必须丢弃。
20. 工具 partial update 必须按 tool ID 合并，不依赖完整收到每个事件。

---

## 4. 分层数据流

```text
Domain / Session State
        ↓ UiEvent
UiStore + Reducer
        ↓ immutable snapshot
Scene Builder
        ↓ semantic component tree
Layout Engine
        ↓ VisualFrame
Surface Planner
        ↓ TerminalCommitPlan
Terminal Driver
```

任何新功能必须接入该数据流，不得绕过。

---

## 5. 推荐模块结构

```text
src/ui/
├── store/
│   ├── state.rs
│   ├── event.rs
│   ├── reducer.rs
│   └── effect.rs
├── input/
│   ├── editor_core.rs
│   ├── input_session.rs
│   ├── router.rs
│   ├── keymap.rs
│   └── text_geometry.rs
├── component/
│   ├── traits.rs
│   ├── transcript/
│   │   ├── user.rs
│   │   ├── assistant.rs
│   │   ├── markdown.rs
│   │   ├── tool.rs
│   │   ├── diff.rs
│   │   ├── plan.rs
│   │   ├── goal.rs
│   │   ├── agent.rs
│   │   ├── approval.rs
│   │   └── notice.rs
│   ├── selector/
│   │   ├── model.rs
│   │   ├── view.rs
│   │   ├── policy.rs
│   │   └── virtual_list.rs
│   ├── panel.rs
│   ├── composer.rs
│   └── status.rs
├── scene/
│   ├── builder.rs
│   ├── overlay.rs
│   └── focus.rs
├── layout/
│   ├── engine.rs
│   ├── visual_row.rs
│   ├── visual_frame.rs
│   ├── hit_map.rs
│   └── geometry.rs
├── transcript/
│   ├── store.rs
│   ├── lifecycle.rs
│   ├── projection.rs
│   ├── stability.rs
│   └── history_sink.rs
├── surface/
│   ├── manager.rs
│   ├── primary.rs
│   └── alternate.rs
├── terminal/
│   ├── driver.rs
│   ├── capabilities.rs
│   ├── commit_plan.rs
│   ├── screen_buffer.rs
│   └── recovery.rs
└── test_support/
    ├── virtual_terminal.rs
    ├── fixture.rs
    └── assertions.rs
```

---

## 6. UiStore 与 Reducer

```rust
#[derive(Clone)]
pub struct UiState {
    pub revision: u64,
    pub transcript: Arc<TranscriptStore>,
    pub surface: SurfaceState,
    pub overlays: OverlayStack,
    pub focus: FocusTarget,
    pub inputs: InputSessions,
    pub session_ui: SessionUiState,
    pub terminal: TerminalUiState,
}
```

`UiState` 不得保存 stdout、ANSI 截图、旧宽度 wrapped rows 或永久物理坐标。

事件示例：

```rust
pub enum UiEvent {
    Key(KeyEvent),
    Paste(String),
    Resize(TerminalSize),
    Tick(Instant),

    AgentDelta {
        message_id: MessageId,
        delta: AgentDelta,
    },

    ToolSnapshotUpdated {
        tool_id: ToolId,
        revision: u64,
    },

    ToolFinished {
        tool_id: ToolId,
        revision: u64,
    },

    OpenOverlay(OverlayRequest),
    CloseOverlay(OverlayId),

    AsyncResult {
        request_id: RequestId,
        result: AsyncPayload,
    },

    EnterAlternate(AlternateRoute),
    LeaveAlternate,
}
```

Reducer：

```rust
pub struct ReduceResult {
    pub next_state: UiState,
    pub effects: Vec<UiEffect>,
    pub invalidation: Invalidation,
}

pub fn reduce(state: &UiState, event: UiEvent) -> ReduceResult;
```

规则：

- Reducer 不写 terminal；
- Renderer 不修改 `UiState`；
- Async effect 只能通过 `UiEvent` 返回；
- 可见状态改变必须增加 revision；
- Tick 只有在动画或超时确实变化时才增加 revision。

---

## 7. 原子帧流水线

每次绘制：

```text
UiState[N]
→ StateSnapshot[N]
→ Scene[N]
→ Layout[N]
→ VisualFrame[N]
→ TerminalCommitPlan[N]
→ terminal commit
→ committed_revision = N
```

```rust
pub struct FrameCoordinator {
    committed_revision: u64,
    previous_frame: Option<VisualFrame>,
    terminal_invalid: bool,
}
```

提交成功才更新 previous frame：

```rust
match terminal_driver.commit(&plan) {
    Ok(()) => {
        coordinator.committed_revision = frame.revision;
        coordinator.previous_frame = Some(frame);
        coordinator.terminal_invalid = false;
    }
    Err(error) => {
        coordinator.terminal_invalid = true;
        // previous_frame 不推进
        return Err(error);
    }
}
```

布局允许最多 2–3 次同帧收敛，不得等待下一次用户输入修正布局。

---

## 8. VisualFrame：唯一视觉坐标

```rust
pub struct VisualRow {
    pub component_id: ComponentId,
    pub logical_line: usize,
    pub wrap_index: usize,
    pub cells: Vec<StyledCell>,
}
```

```rust
pub struct VisualFrame {
    pub revision: u64,
    pub terminal_size: TerminalSize,
    pub rows: Vec<VisualRow>,
    pub component_bounds: HashMap<ComponentId, RowRange>,
    pub hit_regions: Vec<HitRegion>,
    pub cursor: Option<CursorPosition>,
    pub main_layout: MainLayout,
}
```

组件在 layout 阶段生成最终视觉行。禁止手工测高后再交给 Ratatui 二次 wrap。

---

## 9. Primary Surface

### 启动

```text
1. 进入 raw mode
2. 查询 terminal size
3. 查询 cursor position
4. 用普通换行或全屏滚动将旧 shell 画面送入 scrollback
5. owned_rect = full terminal
6. 绘制首个完整 VisualFrame
```

禁止用 `Clear(All)` 替代滚动。

```rust
pub struct PrimarySurfaceState {
    pub owned_rect: Rect,
    pub terminal_size: TerminalSize,
}
```

必须保持：

```text
owned_rect == Rect(0, 0, terminal.width, terminal.height)
```

### Resize

```text
resize event
→ 更新 terminal size
→ previous frame invalid
→ 从 canonical state 重新 layout
→ 完整重画
```

不得复用旧宽度 wrapped rows。

---

## 10. Transcript 生命周期与 History 提交

```rust
pub enum ComponentPhase {
    Streaming,
    Stable,
    Sealed,
    Committed,
}
```

- `Streaming`：仍在变化；
- `Stable`：局部稳定但可能仍受后续 block 影响；
- `Sealed`：语义完成，可生成不可变 History 快照；
- `Committed`：快照已进入 native scrollback。

```rust
pub struct CommittedHistoryBlock {
    pub component_id: ComponentId,
    pub source_revision: u64,
    pub rows: Vec<VisualRow>,
}
```

```rust
pub struct TranscriptProjection {
    pub committed_cursor: TranscriptCursor,
    pub active_components: Vec<ComponentId>,
    pub pending_history: Vec<CommittedHistoryBlock>,
}
```

只有顶部连续、已 sealed、不会再重排的组件可以提交。

### 全高度主屏提交方式

因为 viewport 占满主屏，不能直接复制小 viewport 的 `insert_before`。

推荐：

```text
1. 计算提交的顶部视觉行数 n
2. BeginSynchronizedUpdate
3. 使用可进入 native scrollback 的全屏滚动方式滚动 n 行
4. 从 canonical state 绘制新的完整可见 frame
5. 恢复 panel/composer/status/cursor
6. EndSynchronizedUpdate
7. 成功后推进 history cursor
```

兼容策略：

- 不依赖局部 scrolling margin 滚出的内容一定进入全局 scrollback；
- 优先使用全屏滚动或在底部发出换行；
- native scrollback 是辅助显示层，canonical transcript 始终是权威历史。

```rust
pub struct TerminalCommitPlan {
    pub revision: u64,
    pub surface: SurfaceKind,
    pub history_scroll_rows: u16,
    pub history_blocks: Vec<CommittedHistoryBlock>,
    pub frame_update: FrameUpdate,
    pub cursor: Option<CursorPosition>,
    pub full_redraw: bool,
}
```

---

## 11. Transcript 组件系统

```rust
pub enum TranscriptBlock {
    UserMessage(UserMessageModel),
    AssistantMessage(AssistantMessageModel),
    ToolCall(ToolCallModel),
    Diff(DiffModel),
    Plan(PlanModel),
    Goal(GoalModel),
    Approval(ApprovalModel),
    Agent(AgentModel),
    Notice(NoticeModel),
    Compaction(CompactionModel),
}
```

```rust
pub trait TranscriptComponent {
    fn id(&self) -> ComponentId;
    fn phase(&self) -> ComponentPhase;
    fn measure(&self, ctx: &RenderContext, width: u16) -> BlockMetrics;
    fn render(&self, ctx: &RenderContext, width: u16) -> VisualBlock;
}
```

组件不得写 stdout、resize viewport、清 terminal、修改全局 focus 或自行提交 History。

```rust
pub struct VisualBlock {
    pub component_id: ComponentId,
    pub rows: Vec<VisualRow>,
    pub hit_regions: Vec<LocalHitRegion>,
    pub cursor: Option<LocalCursor>,
}
```

---

## 12. Markdown 流式稳定性

禁止用 `contains('|')` 等单字符启发式判断稳定性。

```rust
pub enum MarkdownBlockKind {
    Paragraph,
    List,
    Quote,
    Fence,
    Table,
    Html,
    Heading,
}
```

至少实现 block scanner，判断 paragraph、fence、table、list/quote continuation、HTML block 是否结束。

保持：

```text
stable prefix | mutable tail
```

只有完整 block 可 seal；已 committed prefix 不得因后续 token 重新解释。

---

## 13. 通用输入系统

共享编辑引擎，不共享 buffer。

```rust
pub struct EditorCore {
    pub text: Rope,
    pub cursor: GraphemeIndex,
    pub selection: Option<TextRange>,
    pub undo: UndoStack,
}
```

```rust
pub enum InputRole {
    Prompt,
    Search,
    InlineCompletion,
    Secret,
    ConfirmationReason,
}

pub struct InputSession {
    pub id: InputSessionId,
    pub role: InputRole,
    pub editor: EditorCore,
    pub submit_policy: SubmitPolicy,
    pub history_policy: HistoryPolicy,
    pub completion_policy: CompletionPolicy,
}
```

`/` 与 `@` 通常从 prompt 当前 token 派生 query，不创建第二输入框。

Secret 输入必须：

- 不进入 history；
- 不进入 transcript；
- 不写日志；
- 不持久化 undo；
- 关闭后清零。

---

## 14. Overlay、Panel 与 Selector

```rust
pub struct OverlayStack {
    pub entries: Vec<OverlayInstance>,
    pub focused: Option<OverlayId>,
}
```

```rust
pub enum OverlayPlacement {
    AboveComposer,
    Centered,
    FullHeight,
    Anchored(ComponentId),
}
```

```rust
pub enum HeightPolicy {
    Content { min: u16, max: u16 },
    Fixed(u16),
    Fraction {
        numerator: u16,
        denominator: u16,
        min: u16,
        max: u16,
    },
    Available,
}
```

面板最大高度：

```text
terminal_height
- composer_height
- status_height
- minimum_transcript_height
```

内容超出时内部滚动。Panel 只返回 `SizeRequest`，不得直接 resize terminal。

Selector 可共享 chrome、list、filter、loading、error、preview、navigation 和 virtual scrolling；不可共享安全语义。

```rust
pub trait SelectorPolicy<T> {
    fn actions(&self, item: &T) -> Vec<Action>;
    fn default_action(&self) -> Option<Action>;
    fn on_escape(&self) -> SelectorResult;
    fn filter_mode(&self) -> FilterMode;
}
```

高危 approval 不得继承 command completion 的默认接受行为。

---

## 15. Focus 与输入路由

```rust
pub enum FocusTarget {
    Prompt,
    Overlay(OverlayId),
    Transcript,
    Modal(ModalId),
}
```

```text
Terminal event
→ normalize
→ InputRouter
→ focused controller
→ UiAction
→ reducer
```

禁止在主循环堆叠大量 `if xxx_open` 分支。

---

## 16. Surface 路由

| 功能 | Surface |
|---|---|
| `/` command completion | Primary panel |
| `@` file completion | Primary panel |
| 简单 tool picker | Primary panel |
| 简单 Allow/Deny | Primary panel |
| `/resume` | Alternate |
| `/tree` | Alternate |
| 完整 transcript | Alternate |
| context/compaction manager | Alternate |
| 大型 diff | Alternate |
| agent manager | Alternate |
| 复杂 approval | Alternate |
| 多步骤 login | Alternate |

如果 inline panel 的首选高度超过 hard max，应自动升级为 alternate screen。

---

## 17. 工具更新与背压

不得依赖收到每一个 partial event。

```rust
pub struct ToolSnapshotStore {
    latest: HashMap<ToolId, Arc<ToolSnapshot>>,
}
```

收到更新后覆盖最新快照并发送 wakeup。UI 渲染时读取最新权威状态。

最终事件必须携带完整输出，或触发一次强制同步，避免“状态 succeeded 但输出仍是旧 partial”。

---

## 18. Unicode 与文本几何

必须明确：

```text
协议/持久化 offset：UTF-8 byte offset
编辑 cursor：grapheme index
终端坐标：display column
```

不得用 `chars()` 代替视觉字符。

测试覆盖 CJK、combining mark、ZWJ emoji、skin tone、variation selector、flag emoji 和宽窄字符混排。

---

## 19. Resume 与 Compaction

模型上下文和显示历史必须分离：

```ts
{
  contextEntries: manager.buildContextEntries(),
  displayEntries: manager.getBranch()
}
```

- `contextEntries`：模型继续推理；
- `displayEntries`：UI 恢复完整历史。

恢复流程：

```text
读取 session entries
→ schema migration
→ 构建 branch
→ 构建 canonical transcript
→ 恢复组件状态
→ 按当前终端宽度重新布局
→ 生成 VisualFrame
→ 提交主屏
```

不得把 ANSI、物理 cursor、旧宽度 wrapped rows 或 framebuffer 截图作为恢复权威数据。

---

## 20. Terminal Driver

```rust
pub struct TerminalDriver<W: Write> {
    output: W,
    capabilities: TerminalCapabilities,
    surface: SurfaceKind,
}
```

所有 crossterm/ANSI 操作集中在此模块。

支持 CSI 2026 时使用 synchronized output；不支持时也必须保证最终正确，只允许出现短暂闪烁。

任何写入失败：

```text
terminal_invalid = true
previous_frame 不推进
下一次完整重画
```

退出 primary-screen 主界面：

```text
关闭 overlay
→ 提交允许提交的最终内容
→ cursor 移到底部
→ 输出换行
→ 显示 cursor
→ 恢复 cooked mode
→ 退出
```

---

## 21. 内存与性能

全高度 framebuffer 相对小 viewport 会增加屏幕缓冲区内存，但通常只有约 0.3–5 MB，取决于 terminal size 和 buffer 数量。

只应长期保留：

```text
Canonical semantic transcript
当前 VisualFrame
previous/current terminal buffers
有限 LRU 渲染缓存
```

禁止永久缓存整个历史的多份 `Line`、`VisualRow`、Cell buffer 或多个宽度下的完整渲染副本。

长工具输出和 diff 应截断主屏摘要，在 viewer 中分页或按需加载。

---

## 22. 测试策略

### 单元测试

- reducer；
- component lifecycle；
- Markdown stability；
- panel height；
- selector policy；
- input session 隔离；
- grapheme/column 转换；
- visual wrapping；
- revision consistency。

### Frame golden tests

固定 state 与 terminal size，验证 `VisualFrame.rows`、bounds、hit regions、cursor、layout 和 history cursor。

### PTY/VT 多帧测试

必须覆盖：

1. 启动前预填 shell history，启动后仍在 native scrollback；
2. streaming → finish → History commit，无空洞、覆盖、重复；
3. 打开/关闭向上 panel，被遮挡 transcript 完整恢复；
4. `120×40 → 80×24 → 200×60` resize；
5. busy → idle 同帧收敛；
6. terminal write failure 后完整恢复；
7. alternate screen 进入/退出恢复主屏；
8. Unicode cursor、selection、wrap、hit test。

### 终端兼容矩阵

至少验证 macOS Terminal、iTerm2、WezTerm、Alacritty、Kitty、Ghostty、Windows Terminal、tmux、SSH 和不支持 CSI 2026 的终端。

重点检查全屏主屏滚动是否进入 native scrollback。

---

## 23. 分阶段迁移计划

### Phase 1：终端基座

实现 `TerminalDriver`、full-height `PrimarySurface`、`ScreenBuffer`、`VisualFrame`、同步提交、resize 完整重画与 recovery。

验收：不覆盖 shell、全屏主屏可控、resize 无残影、退出位置正确。

### Phase 2：UiStore 与原子帧

实现 reducer、revision、snapshot、`FrameCoordinator` 和 commit success/failure 语义。

验收：同一帧 revision 一致，失败不推进 previous frame，不依赖下一次输入收敛。

### Phase 3：Transcript 组件

迁移 user、assistant、Markdown、tool、diff、plan/goal、notice、approval、agent；实现生命周期。

### Phase 4：Native History

实现 stable prefix、History block、全屏滚动、frame rebuild、history cursor 和 PTY scrollback 测试。

### Phase 5：输入与 Panel

实现 `EditorCore`、`InputSession`、`InputRouter`、`OverlayStack`、upward panel、virtual list、selector policy。

### Phase 6：Alternate-screen 功能

迁移 `/resume`、`/tree`、viewer、context manager、large diff、agent manager、complex approval。

### Phase 7：Resume/Compaction

分离 `contextEntries` 和 `displayEntries`，完成 branch/schema/expansion restore。

### Phase 8：性能与兼容性

实现 tool coalescing、LRU、分页、allocation reuse、profiling 和 capability detection。

---

## 24. PR 检查表

每个 UI PR 必须回答：

1. 是否新增直接 stdout 写入？
2. 是否新增独立视觉高度计算而未使用 `VisualRow`？
3. 是否可能修改 committed History？
4. 是否混用不同 revision？
5. terminal write 失败时 previous frame 是否不变？
6. resize 后是否从 canonical state 重建？
7. panel 是否只覆盖 managed surface？
8. prompt/search/secret 是否错误共享 buffer？
9. 异步结果是否带 request ID/revision？
10. Unicode 是否按 grapheme/display column 处理？
11. 是否添加多帧 PTY 测试？
12. 不支持 synchronized output 时是否仍正确？

---

## 25. 禁止迁入新基座的旧模式

- 动态销毁并重建不同高度的 Inline Terminal；
- 为 panel 向上清除 native History；
- `recent_history_background` 式补偿恢复；
- layout 使用新 projection、busy 使用旧 presenter；
- 组件直接操作 stdout；
- 每个 modal 单独写输入分支；
- `lines.len()` 代替视觉高度；
- hit map 假设逻辑行只占一行；
- Ratatui 二次隐式 wrap；
- 用模型 context entries 恢复完整 UI；
- 将 ANSI/物理坐标持久化为 resume 数据；
- 工具 partial 满队列后直接丢弃且无最终同步；
- 用 `contains('|')` 判断 Markdown 稳定性；
- 每帧深拷贝完整 transcript；
- 为多个 width 永久缓存完整历史 Cell。

---

## 26. 完成定义

新 UI 基座完成时必须同时满足：

### 架构

- 单一 reducer；
- 单一 VisualFrame；
- 单一 TerminalDriver；
- revision 一致；
- full-height primary surface；
- alternate-screen router；
- append-only History sink。

### 行为

- 启动不覆盖 shell；
- History 自然进入 native scrollback；
- 输入框固定底部；
- panel 可向上扩展并准确恢复；
- resize 无残影；
- busy → idle 无空洞；
- resume 完整；
- committed 内容不被错误更新。

### 测试

- 单元测试；
- frame golden tests；
- PTY/VT 多帧测试；
- terminal compatibility tests；
- failure injection；
- Unicode 测试；
- memory/profile 基线。

---

## 27. 最终定义

新的 Nabla UI 是：

> 一个运行于 terminal primary screen 的全高度、组件化 framebuffer。它以 canonical transcript 作为唯一语义来源，将当前可变内容、输入框、状态栏和小型 panel 保留在应用管理的可见 surface 中；将 sealed 内容以不可变快照持续提交到 native scrollback；并使用 alternate screen 承载复杂浏览界面。所有可见更新通过带 revision 的 `VisualFrame` 与 `TerminalCommitPlan` 原子提交。

最终数据流：

```text
Domain Events
    ↓
UiStore / Reducer
    ↓
Immutable State Snapshot
    ↓
Semantic Component Scene
    ↓
VisualFrame
    ↓
TerminalCommitPlan
    ↓
Primary Screen + Native Scrollback
```
