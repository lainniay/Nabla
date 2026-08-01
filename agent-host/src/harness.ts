import { createHash, randomUUID } from "node:crypto";
import {
  existsSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";

import { parseFrontmatter } from "@earendil-works/pi-coding-agent";

import type { PlanArtifactV2 } from "./plan.ts";
import type { AgentIsolationPolicy } from "./worktree.ts";
import {
  isHighRiskCommand,
  hasShellControlSyntax,
  isSafeReadOnlyCommand,
  READ_ONLY_TOOL_NAMES,
  SAFE_READ_ONLY_COMMAND_PREFIXES,
  THINKING_LEVELS,
  type ThinkingLevel,
} from "./policy/tool-policy.ts";
import { writeAtomicJsonSync } from "./persistence/atomic-json.ts";
import {
  isPathWithin,
  workspaceRelativePath,
} from "./policy/path-boundary.ts";
import {
  isJsonObject as isRecord,
  stringArray,
} from "./protocol/validation.ts";

export type GoalStage =
  | "preparing"
  | "awaiting_approval"
  | "executing"
  | "verifying"
  | "reviewing"
  | "completed"
  | "paused"
  | "cancelled"
  | "blocked";

export type TaskStatus =
  | "pending"
  | "running"
  | "completed"
  | "awaiting_integration"
  | "failed"
  | "blocked"
  | "interrupted";

export type ReviewVerdict = "pass" | "changes_required" | "blocked";

export interface GoalTask {
  id: string;
  title: string;
  description: string;
  profile: string;
  dependsOn: string[];
  allowedPaths: string[];
  acceptanceCriteria: string[];
  status: TaskStatus;
  result?: TaskResult;
}

export interface TaskResult {
  status: Exclude<
    TaskStatus,
    "pending" | "running" | "interrupted" | "awaiting_integration"
  >;
  summary: string;
  evidence: string[];
  changedPaths: string[];
  verification: VerificationEvidence[];
  blockers: string[];
}

export interface VerificationEvidence {
  command: string;
  exitCode: number | null;
  output: string;
  fullOutputPath?: string;
}

export interface ReviewFinding {
  severity: "critical" | "high" | "medium" | "low";
  title: string;
  evidence: string;
  path?: string;
  line?: number;
  recommendation: string;
  taskIds?: string[];
  paths?: string[];
}

export interface GoalReview {
  cycle: number;
  verdict: ReviewVerdict;
  summary: string;
  findings: ReviewFinding[];
  reviewedAt: string;
  agentId: string;
  model?: string;
}

export interface GoalVerification {
  cycle: number;
  result: TaskResult;
  verifiedAt: string;
  agentId: string;
  model?: string;
}

export interface CapabilityLease {
  specRevision: number;
  allowedTools: string[];
  allowedPaths: string[];
  allowedCommands: string[];
  approvedAt: string;
}

export interface GoalSourcePlan {
  id: string;
  revision: number;
  sourceSessionId: string;
  artifact: PlanArtifactV2;
}

export interface GoalSpec {
  revision: number;
  summary: string;
  acceptanceCriteria: string[];
  allowedTools: string[];
  allowedPaths: string[];
  allowedCommands: string[];
  sourcePlan?: GoalSourcePlan;
  tasks: Array<{
    id: string;
    title: string;
    description: string;
    profile?: string;
    dependsOn?: string[];
    allowedPaths?: string[];
    acceptanceCriteria?: string[];
  }>;
}

export interface GoalRecord {
  schemaVersion: 2;
  id: string;
  workspace: string;
  sessionId: string;
  objective: string;
  stage: GoalStage;
  previousStage?: GoalStage;
  revision: number;
  constraints: string[];
  acceptanceCriteria: string[];
  sourcePlan?: GoalSourcePlan;
  spec?: GoalSpec;
  tasks: GoalTask[];
  lease?: CapabilityLease;
  reviews: GoalReview[];
  verification: GoalVerification[];
  repairCycles: number;
  createdAt: string;
  updatedAt: string;
  lastError?: string;
}

export interface GoalSnapshot {
  scopeId?: string;
  goal: GoalRecord | null;
  statePath: string;
}

export interface GoalsSnapshot {
  goals: GoalRecord[];
  stateDirectory: string;
}

export type AgentPermissionEffect = "allow" | "ask" | "deny";

export interface AgentPermissionRule {
  resource: string;
  effect: AgentPermissionEffect;
}

export type AgentPermissions = Record<string, AgentPermissionRule[]>;

export interface AgentProfile {
  description: string;
  model?: string;
  thinkingLevel?: ThinkingLevel;
  instructions: string[];
  skills: string[];
  tools: string[];
  permission: AgentPermissions;
  maxParallel: number;
  maxTurns: number;
  isolation: AgentIsolationPolicy;
  disabled: boolean;
  source: string;
}

export interface AgentConfigDiagnostic {
  type: "warning" | "error";
  message: string;
  path?: string;
  profile?: string;
}

export interface HarnessConfig {
  schemaVersion: 2;
  maxParallel: number;
  trustedWorkspaces: string[];
  allowedProjectExtensions: string[];
  profiles: Record<string, AgentProfile>;
  diagnostics: AgentConfigDiagnostic[];
}

export interface ResourceSnapshot {
  scopeId?: string;
  trusted: boolean;
  contextFiles: string[];
  skills: Array<{ name: string; path: string; description: string }>;
  prompts: Array<{ name: string; path: string; description: string }>;
  extensions: string[];
  commands: Array<{
    name: string;
    description: string;
    source: "extension" | "prompt" | "skill";
  }>;
  diagnostics: Array<{
    type: string;
    message: string;
    path?: string;
  }>;
  revision: number;
}

interface GoalStoreOptions {
  cwd: string;
  sessionId: string;
  rootDir?: string;
  now?: () => string;
  createId?: () => string;
}

interface HarnessConfigOptions {
  homeDir?: string;
}

const SUPPORTED_AGENT_TOOLS = new Set([
  ...READ_ONLY_TOOL_NAMES,
  "edit",
  "write",
  "bash",
]);
const SAFE_BASH_RULES: AgentPermissionRule[] = [
  { resource: "*", effect: "deny" },
  ...SAFE_READ_ONLY_COMMAND_PREFIXES.map((resource) => ({
    resource: `${resource}*`,
    effect: "allow" as const,
  })),
];

function readOnlyPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
    edit: [{ resource: "*", effect: "deny" }],
    write: [{ resource: "*", effect: "deny" }],
    bash: structuredClone(SAFE_BASH_RULES),
  };
}

function goalWorkerPermissions(): AgentPermissions {
  return {
    ...readOnlyPermissions(),
    edit: [{ resource: "*", effect: "ask" }],
    write: [{ resource: "*", effect: "ask" }],
    bash: [{ resource: "*", effect: "ask" }],
  };
}

function safeCustomPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
  };
}

const DEFAULT_PROFILES: Record<string, AgentProfile> = {
  planner: {
    description: "Inspect the workspace and prepare dependency-aware plans.",
    thinkingLevel: "high",
    instructions: [
      "Inspect the workspace and produce a concrete, dependency-aware plan.",
      "Do not modify files.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    source: "builtin",
  },
  worker: {
    description: "Implement bounded tasks and verify the resulting changes.",
    thinkingLevel: "high",
    instructions: [
      "Implement the assigned task completely within its capability lease.",
      "Run relevant verification and report artifact-backed evidence.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "edit", "write", "bash"],
    permission: goalWorkerPermissions(),
    maxParallel: 3,
    maxTurns: 32,
    isolation: { mode: "auto", integration: "source" },
    disabled: false,
    source: "builtin",
  },
  verifier: {
    description: "Run independent verification and report exact evidence.",
    thinkingLevel: "medium",
    instructions: [
      "Run the requested verification without modifying source files.",
      "Report exact commands, exit codes, and concise output.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "bash"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "auto", integration: "manual" },
    disabled: false,
    source: "builtin",
  },
  reviewer: {
    description: "Review changes independently for regressions and omissions.",
    thinkingLevel: "high",
    instructions: [
      "Review independently against the goal, plan, diff, and verification evidence.",
      "Do not modify files. Return only structured findings and a verdict.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "bash"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    source: "builtin",
  },
};

const DEFAULT_CONFIG: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 3,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: DEFAULT_PROFILES,
  diagnostics: [],
};

const GOAL_TRANSITIONS: Record<GoalStage, readonly GoalStage[]> = {
  preparing: ["paused", "cancelled", "blocked"],
  awaiting_approval: ["paused", "cancelled", "blocked"],
  executing: ["verifying", "paused", "cancelled", "blocked"],
  verifying: ["reviewing", "executing", "paused", "cancelled", "blocked"],
  reviewing: ["executing", "paused", "cancelled", "blocked", "completed"],
  paused: [
    "preparing",
    "awaiting_approval",
    "executing",
    "verifying",
    "reviewing",
    "cancelled",
    "blocked",
  ],
  blocked: ["cancelled"],
  completed: [],
  cancelled: [],
};

const TASK_TRANSITIONS: Record<TaskStatus, readonly TaskStatus[]> = {
  pending: ["running", "blocked", "interrupted"],
  running: [
    "completed",
    "awaiting_integration",
    "failed",
    "blocked",
    "interrupted",
  ],
  interrupted: ["running", "failed", "blocked"],
  awaiting_integration: ["completed", "failed", "blocked"],
  completed: [],
  failed: [],
  blocked: [],
};

export class GoalStore {
  private cwd: string;
  private sessionId: string;
  private readonly rootDir: string;
  private readonly now: () => string;
  private readonly createId: () => string;
  private record?: GoalRecord;
  private statePathValue: string;

  constructor(options: GoalStoreOptions) {
    this.cwd = canonicalPath(options.cwd);
    this.sessionId = options.sessionId;
    this.rootDir =
      options.rootDir ??
      process.env.NABLA_HOME ??
      join(homedir(), ".nabla");
    this.now = options.now ?? (() => new Date().toISOString());
    this.createId = options.createId ?? randomUUID;
    this.statePathValue = this.statePathFor(this.cwd, this.sessionId);
    this.load();
  }

  attach(cwd: string, sessionId: string): GoalSnapshot {
    const canonical = canonicalPath(cwd);
    if (
      this.active() &&
      this.record?.workspace === canonical &&
      this.record.sessionId !== sessionId
    ) {
      return this.snapshot();
    }
    this.cwd = canonical;
    this.sessionId = sessionId;
    this.statePathValue = this.statePathFor(this.cwd, sessionId);
    this.load();
    return this.snapshot();
  }

  start(
    objective: string,
    constraints: string[] = [],
    sourcePlan?: PlanArtifactV2,
  ): GoalRecord {
    const normalized = objective.trim();
    if (!normalized) throw new Error("Goal objective must not be empty");
    if (
      this.record &&
      !["completed", "cancelled"].includes(this.record.stage)
    ) {
      throw new Error(`Goal ${this.record.id} is already active`);
    }
    const timestamp = this.now();
    this.record = {
      schemaVersion: 2,
      id: this.createId(),
      workspace: this.cwd,
      sessionId: this.sessionId,
      objective: normalized,
      stage: "preparing",
      revision: 1,
      constraints: normalizeStrings(constraints),
      acceptanceCriteria: [],
      ...(sourcePlan
        ? {
            sourcePlan: {
              id: sourcePlan.id,
              revision: sourcePlan.revision,
              sourceSessionId: sourcePlan.sourceSessionId,
              artifact: structuredClone(sourcePlan),
            },
          }
        : {}),
      tasks: [],
      reviews: [],
      verification: [],
      repairCycles: 0,
      createdAt: timestamp,
      updatedAt: timestamp,
    };
    this.persist();
    return this.current() as GoalRecord;
  }

  current(): GoalRecord | undefined {
    return this.record ? structuredClone(this.record) : undefined;
  }

  active(): GoalRecord | undefined {
    return this.record &&
      !["completed", "cancelled"].includes(this.record.stage)
      ? this.current()
      : undefined;
  }

  snapshot(): GoalSnapshot {
    return {
      goal: this.current() ?? null,
      statePath: this.statePathValue,
    };
  }

  list(): GoalsSnapshot {
    const stateDirectory = dirname(this.statePathValue);
    if (!existsSync(stateDirectory)) return { goals: [], stateDirectory };
    const goals = readdirSync(stateDirectory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
      .flatMap((entry) => {
        try {
          const value = JSON.parse(
            readFileSync(join(stateDirectory, entry.name), "utf8"),
          );
          const goal = normalizeStoredGoal(value);
          return goal ? [goal] : [];
        } catch {
          return [];
        }
      })
      .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
    return { goals, stateDirectory };
  }

  acceptSpec(spec: Omit<GoalSpec, "revision">): GoalRecord {
    const record = this.requireActive();
    if (record.stage !== "preparing" && record.stage !== "awaiting_approval") {
      throw new Error(`Cannot submit a Goal spec while goal is ${record.stage}`);
    }
    const normalized = normalizeGoalSpec({
      ...spec,
      sourcePlan: spec.sourcePlan ?? record.sourcePlan,
    });
    record.spec = {
      ...normalized,
      revision: (record.spec?.revision ?? 0) + 1,
    };
    record.acceptanceCriteria = normalized.acceptanceCriteria;
    record.tasks = normalized.tasks.map((task, index) => ({
      id: task.id || `task-${index + 1}`,
      title: task.title,
      description: task.description,
      profile: task.profile ?? "worker",
      dependsOn: normalizeStrings(task.dependsOn ?? []),
      allowedPaths: normalizeStrings(
        task.allowedPaths ?? normalized.allowedPaths,
      ),
      acceptanceCriteria: normalizeStrings(
        task.acceptanceCriteria ?? normalized.acceptanceCriteria,
      ),
      status: "pending",
    }));
    record.stage = "awaiting_approval";
    record.revision += 1;
    record.updatedAt = this.now();
    record.lastError = undefined;
    this.persist();
    return this.current() as GoalRecord;
  }

  approveSpec(): GoalRecord {
    const record = this.requireActive();
    if (record.stage !== "awaiting_approval" || !record.spec) {
      throw new Error("Goal has no spec awaiting approval");
    }
    record.lease = {
      specRevision: record.spec.revision,
      allowedTools: record.spec.allowedTools,
      allowedPaths: record.spec.allowedPaths,
      allowedCommands: record.spec.allowedCommands,
      approvedAt: this.now(),
    };
    record.stage = "executing";
    record.revision += 1;
    record.updatedAt = this.now();
    this.persist();
    return this.current() as GoalRecord;
  }

  transition(stage: GoalStage, error?: string): GoalRecord {
    const record = this.requireRecord();
    if (record.stage === stage) {
      throw new Error(`Goal is already ${stage}`);
    }
    if (!GOAL_TRANSITIONS[record.stage].includes(stage)) {
      throw new Error(`Cannot transition Goal from ${record.stage} to ${stage}`);
    }
    if (
      record.stage === "paused" &&
      stage !== "cancelled" &&
      stage !== "blocked" &&
      record.previousStage !== stage
    ) {
      throw new Error(
        `Cannot resume Goal from ${record.previousStage ?? "unknown"} to ${stage}`,
      );
    }
    if (stage === "paused") {
      record.previousStage = record.stage;
    } else if (record.stage === "paused") {
      record.previousStage = undefined;
    }
    record.stage = stage;
    record.lastError = error;
    record.revision += 1;
    record.updatedAt = this.now();
    if (["completed", "cancelled"].includes(stage)) {
      record.lease = undefined;
    }
    this.persist();
    return this.current() as GoalRecord;
  }

  resume(): GoalRecord {
    const record = this.requireRecord();
    if (record.stage !== "paused") throw new Error("Goal is not paused");
    const stage = record.previousStage ?? "preparing";
    return this.transition(stage);
  }

  updateTask(taskId: string, status: TaskStatus, result?: TaskResult): GoalRecord {
    const record = this.requireActive();
    const task = record.tasks.find((candidate) => candidate.id === taskId);
    if (!task) throw new Error(`Unknown goal task: ${taskId}`);
    if (
      task.status !== status &&
      !TASK_TRANSITIONS[task.status].includes(status)
    ) {
      throw new Error(
        `Cannot transition Goal task ${taskId} from ${task.status} to ${status}`,
      );
    }
    task.status = status;
    task.result = result ? structuredClone(result) : task.result;
    record.revision += 1;
    record.updatedAt = this.now();
    this.persist();
    return this.current() as GoalRecord;
  }

  extendLease(
    tool: string,
    options: { path?: string; command?: string } = {},
  ): GoalRecord {
    const record = this.requireActive();
    if (!record.lease || !record.spec) {
      throw new Error("Goal has no active capability lease");
    }
    if (record.lease.specRevision !== record.spec.revision) {
      throw new Error("Goal capability lease is stale");
    }
    record.lease.allowedTools = normalizeStrings([
      ...record.lease.allowedTools,
      tool,
    ]);
    if (options.path) {
      record.lease.allowedPaths = normalizeStrings([
        ...record.lease.allowedPaths,
        options.path,
      ]);
    }
    if (options.command) {
      record.lease.allowedCommands = normalizeStrings([
        ...record.lease.allowedCommands,
        options.command,
      ]);
    }
    record.revision += 1;
    record.updatedAt = this.now();
    this.persist();
    return this.current() as GoalRecord;
  }

  addReview(review: Omit<GoalReview, "cycle" | "reviewedAt">): GoalRecord {
    const record = this.requireActive();
    if (record.stage !== "reviewing") {
      throw new Error(`Cannot record a review while Goal is ${record.stage}`);
    }
    const complete: GoalReview = {
      ...structuredClone(review),
      cycle: record.repairCycles + 1,
      reviewedAt: this.now(),
    };
    record.reviews.push(complete);
    if (complete.verdict === "pass") {
      record.stage = "completed";
      record.lease = undefined;
    } else if (complete.verdict === "changes_required") {
      record.repairCycles += 1;
      record.stage = record.repairCycles >= 2 ? "blocked" : "executing";
      if (record.stage === "executing") this.queueTargetedRepair(record, complete);
    } else {
      record.stage = "blocked";
    }
    record.revision += 1;
    record.updatedAt = this.now();
    this.persist();
    return this.current() as GoalRecord;
  }

  addVerification(
    verification: Omit<GoalVerification, "cycle" | "verifiedAt">,
  ): GoalRecord {
    const record = this.requireActive();
    if (record.stage !== "verifying") {
      throw new Error(`Cannot record verification while Goal is ${record.stage}`);
    }
    record.verification ??= [];
    record.verification.push({
      ...structuredClone(verification),
      cycle: record.repairCycles + 1,
      verifiedAt: this.now(),
    });
    record.revision += 1;
    record.updatedAt = this.now();
    this.persist();
    return this.current() as GoalRecord;
  }

  goalView(): Record<string, unknown> | undefined {
    const goal = this.active();
    if (!goal) return undefined;
    return {
      id: goal.id,
      revision: goal.revision,
      objective: goal.objective,
      stage: goal.stage,
      constraints: goal.constraints,
      acceptanceCriteria: goal.acceptanceCriteria,
      currentTasks: goal.tasks
        .filter((task) => task.status !== "completed")
        .map((task) => ({
          id: task.id,
          title: task.title,
          profile: task.profile,
          status: task.status,
          dependsOn: task.dependsOn,
        })),
      capability: goal.lease
        ? {
            specRevision: goal.lease.specRevision,
            tools: goal.lease.allowedTools,
            paths: goal.lease.allowedPaths,
            commands: goal.lease.allowedCommands,
          }
        : undefined,
    };
  }

  private statePathFor(cwd: string, sessionId: string): string {
    const workspaceHash = createHash("sha256")
      .update(cwd)
      .digest("hex")
      .slice(0, 20);
    return join(this.rootDir, "state", workspaceHash, `${sessionId}.json`);
  }

  private queueTargetedRepair(record: GoalRecord, review: GoalReview): void {
    const targetIds = new Set(
      review.findings.flatMap((finding) => finding.taskIds ?? []),
    );
    const findingPaths = normalizeStrings(
      review.findings.flatMap((finding) => [
        ...(finding.paths ?? []),
        ...(finding.path ? [finding.path] : []),
      ]),
    ).map(normalizeRelativePattern);
    if (targetIds.size === 0 && findingPaths.length > 0) {
      for (const task of record.tasks) {
        const taskPaths = task.allowedPaths
          .map(normalizeRelativePattern)
          .filter((path) => path !== "" && path !== ".");
        if (
          findingPaths.some((findingPath) =>
            taskPaths.some(
              (taskPath) =>
                findingPath === taskPath ||
                findingPath.startsWith(`${taskPath}/`) ||
                taskPath.startsWith(`${findingPath}/`),
            ),
          )
        ) {
          targetIds.add(task.id);
        }
      }
    }
    for (const id of [...targetIds]) {
      if (!record.tasks.some((task) => task.id === id)) targetIds.delete(id);
    }
    if (targetIds.size > 0) {
      let changed = true;
      while (changed) {
        changed = false;
        for (const task of record.tasks) {
          if (
            !targetIds.has(task.id) &&
            task.dependsOn.some((dependency) => targetIds.has(dependency))
          ) {
            targetIds.add(task.id);
            changed = true;
          }
        }
      }
      for (const task of record.tasks) {
        if (!targetIds.has(task.id) || task.status !== "completed") continue;
        task.status = "pending";
        task.result = undefined;
      }
      return;
    }

    const baseId = `repair-${record.repairCycles}`;
    let id = baseId;
    let suffix = 1;
    while (record.tasks.some((task) => task.id === id)) {
      id = `${baseId}-${++suffix}`;
    }
    record.tasks.push({
      id,
      title: `Repair review cycle ${record.repairCycles}`,
      description: review.findings
        .map((finding) => `${finding.title}: ${finding.recommendation}`)
        .join("\n"),
      profile: "worker",
      dependsOn: record.tasks
        .filter((task) => task.status === "completed")
        .map((task) => task.id),
      allowedPaths:
        findingPaths.length > 0
          ? findingPaths
          : (record.spec?.allowedPaths ?? ["."]),
      acceptanceCriteria: review.findings.map(
        (finding) => finding.recommendation,
      ),
      status: "pending",
    });
  }

  private load(): void {
    this.record = undefined;
    if (!existsSync(this.statePathValue)) return;
    try {
      const value = JSON.parse(readFileSync(this.statePathValue, "utf8"));
      const normalized = normalizeStoredGoal(value);
      if (!normalized) throw new Error("unsupported goal state");
      this.record = normalized;
      let changed = !isGoalRecord(value);
      for (const task of this.record.tasks) {
        if (task.status === "running") {
          task.status = "interrupted";
          changed = true;
        }
      }
      if (
        ["preparing", "executing", "verifying", "reviewing"].includes(
          this.record.stage,
        )
      ) {
        const interruptedStage = this.record.stage;
        this.record.stage = "paused";
        this.record.previousStage = interruptedStage;
        this.record.lastError = "Interrupted by harness restart";
        this.record.revision += 1;
        this.record.updatedAt = this.now();
        changed = true;
      }
      if (changed) this.persist();
    } catch (error) {
      const corruptPath = `${this.statePathValue}.corrupt-${Date.now()}`;
      renameSync(this.statePathValue, corruptPath);
      this.record = undefined;
    }
  }

  private persist(): void {
    if (!this.record) return;
    writeAtomicJsonSync(this.statePathValue, this.record);
  }

  private requireActive(): GoalRecord {
    const record = this.requireRecord();
    if (["completed", "cancelled"].includes(record.stage)) {
      throw new Error(`Goal ${record.id} is not active`);
    }
    return record;
  }

  private requireRecord(): GoalRecord {
    if (!this.record) throw new Error("No goal is available");
    return this.record;
  }
}

export function loadHarnessConfig(
  cwd: string,
  options: HarnessConfigOptions = {},
): HarnessConfig {
  const home = options.homeDir ?? homedir();
  const globalPath = join(home, ".nabla", "config.json");
  const diagnostics: AgentConfigDiagnostic[] = [];
  const globalValue = readJsonObject(globalPath, diagnostics);
  let globalConfig = mergeConfig(
    cloneHarnessConfig(DEFAULT_CONFIG),
    globalValue,
    globalPath,
    diagnostics,
  );
  globalConfig = mergeAgentDirectory(
    globalConfig,
    join(home, ".nabla", "agents"),
    diagnostics,
  );
  const canonicalWorkspace = canonicalPath(cwd);
  const trusted = globalConfig.trustedWorkspaces.some(
    (workspace) => canonicalPath(workspace) === canonicalWorkspace,
  );
  if (!trusted) return { ...globalConfig, diagnostics };
  const projectPath = join(cwd, ".nabla", "config.json");
  const projectValue = readJsonObject(projectPath, diagnostics);
  let projectConfig = mergeConfig(
    globalConfig,
    projectValue,
    projectPath,
    diagnostics,
    true,
  );
  projectConfig = mergeAgentDirectory(
    projectConfig,
    join(cwd, ".nabla", "agents"),
    diagnostics,
  );
  return { ...projectConfig, diagnostics };
}

export function saveWorkspaceTrust(
  cwd: string,
  trusted: boolean,
  options: HarnessConfigOptions = {},
): HarnessConfig {
  const home = options.homeDir ?? homedir();
  const path = join(home, ".nabla", "config.json");
  const raw = readJsonObject(path, []);
  const canonical = canonicalPath(cwd);
  const workspaces = new Set(stringArray(raw.trustedWorkspaces).map(canonicalPath));
  if (trusted) workspaces.add(canonical);
  else workspaces.delete(canonical);
  const next: Record<string, unknown> = {
    ...raw,
    schemaVersion:
      typeof raw.schemaVersion === "number" ? raw.schemaVersion : 2,
    trustedWorkspaces: [...workspaces].sort(),
  };
  writeAtomicJsonSync(path, next);
  return loadHarnessConfig(cwd, options);
}

export function workspaceIsTrusted(cwd: string, config: HarnessConfig): boolean {
  const canonical = canonicalPath(cwd);
  return config.trustedWorkspaces.some(
    (workspace) => canonicalPath(workspace) === canonical,
  );
}

export function filterContextFilesByTrust<T extends { path: string }>(
  files: readonly T[],
  agentDir: string,
  trusted: boolean,
): T[] {
  if (trusted) return [...files];
  const root = resolve(agentDir);
  return files.filter((file) => isPathWithin(root, file.path));
}

export function modelReference(profile: AgentProfile): {
  provider: string;
  id: string;
} | undefined {
  const reference = profile.model?.trim();
  if (!reference) return undefined;
  const slash = reference.indexOf("/");
  if (slash <= 0 || slash === reference.length - 1) {
    throw new Error(`Agent model must use provider/model format: ${reference}`);
  }
  return {
    provider: reference.slice(0, slash),
    id: reference.slice(slash + 1),
  };
}

export function agentPermissionEffect(
  profile: AgentProfile,
  tool: string,
  resource = "*",
): AgentPermissionEffect {
  if (
    tool === "bash" &&
    resource !== "*" &&
    usesSafeReadOnlyBashRules(profile.permission.bash) &&
    !isSafeReadOnlyCommand(resource)
  ) {
    return "deny";
  }
  const fallback = READ_ONLY_TOOL_NAMES.includes(
    tool as (typeof READ_ONLY_TOOL_NAMES)[number],
  )
    ? "allow"
    : "ask";
  let effect: AgentPermissionEffect = fallback;
  for (const rule of profile.permission[tool] ?? []) {
    if (agentResourceMatches(rule.resource, resource)) effect = rule.effect;
  }
  return effect;
}

export function agentPermissionSummary(profile: AgentProfile): string {
  return profile.tools
    .map((tool) => `${tool}:${agentPermissionEffect(profile, tool)}`)
    .join(",");
}

export function goalSpecFromToolParams(
  params: Record<string, unknown>,
  options: { fallbackSummary: string; sourcePlan?: PlanArtifactV2 },
): Omit<GoalSpec, "revision"> {
  const sourcePlan = options.sourcePlan
    ? {
        id: options.sourcePlan.id,
        revision: options.sourcePlan.revision,
        sourceSessionId: options.sourcePlan.sourceSessionId,
        artifact: structuredClone(options.sourcePlan),
      }
    : undefined;
  return normalizeGoalSpec({
    summary:
      typeof params.summary === "string"
        ? params.summary
        : sourcePlan?.artifact.bodyMarkdown ?? options.fallbackSummary,
    acceptanceCriteria: stringArray(params.acceptanceCriteria),
    allowedTools: stringArray(params.allowedTools),
    allowedPaths: stringArray(params.allowedPaths),
    allowedCommands: stringArray(params.allowedCommands),
    tasks: taskArray(params.tasks),
    sourcePlan,
  });
}

export function pathAllowedByLease(
  cwd: string,
  path: string,
  allowedPaths: readonly string[],
): boolean {
  const root = resolve(cwd);
  const target = resolve(root, path);
  let normalized: string;
  try {
    normalized = workspaceRelativePath(root, target);
  } catch {
    return false;
  }
  return allowedPaths.some((pattern) => {
    const clean = pattern
      .trim()
      .replace(/^\.\//u, "")
      .replace(/\\/gu, "/")
      .replace(/\/\*\*$/u, "")
      .replace(/\/+$/u, "");
    if (clean === "" || clean === ".") return true;
    return normalized === clean || normalized.startsWith(`${clean}/`);
  });
}

export function commandAllowedByLease(
  command: string,
  allowedCommands: readonly string[],
): boolean {
  const normalized = command.trim().replace(/\s+/gu, " ");
  if (
    !normalized ||
    isHighRiskCommand(normalized) ||
    hasShellControlSyntax(command)
  ) {
    return false;
  }
  return allowedCommands.some((prefix) => {
    const normalizedPrefix = prefix.trim().replace(/\s+/gu, " ");
    return (
      normalized === normalizedPrefix ||
      normalized.startsWith(`${normalizedPrefix} `)
    );
  });
}

function usesSafeReadOnlyBashRules(
  rules: readonly AgentPermissionRule[] | undefined,
): boolean {
  return (
    rules?.length === SAFE_BASH_RULES.length &&
    rules.every(
      (rule, index) =>
        rule.resource === SAFE_BASH_RULES[index]?.resource &&
        rule.effect === SAFE_BASH_RULES[index]?.effect,
    )
  );
}

export function isCredentialPath(path: string): boolean {
  const normalized = path.replace(/\\/gu, "/").toLocaleLowerCase();
  return [
    "/.ssh/",
    "/.aws/",
    "/.config/gcloud/",
    "/credentials",
    "/auth.json",
    "/.env",
  ].some((marker) => normalized.includes(marker));
}

function normalizeGoalSpec(
  spec: Omit<GoalSpec, "revision">,
): Omit<GoalSpec, "revision"> {
  const acceptanceCriteria = normalizeStrings(
    spec.acceptanceCriteria?.length
      ? spec.acceptanceCriteria
      : spec.sourcePlan?.artifact.testPlan ?? [],
  );
  const allowedPaths = normalizeStrings(
    spec.allowedPaths?.length ? spec.allowedPaths : ["."],
  );
  const summary = spec.summary.trim();
  if (!summary) throw new Error("Goal spec summary must not be empty");
  const tasks =
    spec.tasks && spec.tasks.length > 0
      ? spec.tasks.map((task, index) => ({
          id: task.id?.trim() || `task-${index + 1}`,
          title: task.title.trim(),
          description: task.description.trim(),
          profile: task.profile?.trim() || "worker",
          dependsOn: normalizeStrings(task.dependsOn ?? []),
          allowedPaths: normalizeStrings(task.allowedPaths ?? allowedPaths),
          acceptanceCriteria: normalizeStrings(
            task.acceptanceCriteria ?? acceptanceCriteria,
          ),
        }))
      : [
          {
            id: "implementation",
            title: spec.sourcePlan?.artifact.title ?? "Implement Goal",
            description: spec.sourcePlan?.artifact.bodyMarkdown ?? summary,
            profile: "worker",
            dependsOn: [],
            allowedPaths,
            acceptanceCriteria,
          },
        ];
  validateGoalTaskGraph(tasks);
  return {
    summary,
    acceptanceCriteria,
    allowedTools: normalizeStrings(
      spec.allowedTools?.length
        ? spec.allowedTools
        : ["read", "grep", "find", "ls", "edit", "write", "bash"],
    ),
    allowedPaths,
    allowedCommands: normalizeStrings(spec.allowedCommands ?? []),
    sourcePlan: spec.sourcePlan
      ? structuredClone(spec.sourcePlan)
      : undefined,
    tasks,
  };
}

function validateGoalTaskGraph(tasks: GoalSpec["tasks"]): void {
  if (tasks.length === 0) throw new Error("Goal must contain at least one task");
  const ids = new Set<string>();
  for (const task of tasks) {
    if (!task.id.trim()) throw new Error("Goal task IDs must not be empty");
    if (!task.title.trim()) {
      throw new Error(`Goal task ${task.id} title must not be empty`);
    }
    if (!task.description.trim()) {
      throw new Error(`Goal task ${task.id} description must not be empty`);
    }
    if (ids.has(task.id)) {
      throw new Error(`Goal contains duplicate task ID: ${task.id}`);
    }
    ids.add(task.id);
  }
  for (const task of tasks) {
    for (const dependency of task.dependsOn ?? []) {
      if (!ids.has(dependency)) {
        throw new Error(
          `Goal task ${task.id} depends on unknown task ${dependency}`,
        );
      }
    }
  }
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const visiting = new Set<string>();
  const visited = new Set<string>();
  const visit = (taskId: string): void => {
    if (visiting.has(taskId)) {
      throw new Error(`Goal task dependency cycle includes ${taskId}`);
    }
    if (visited.has(taskId)) return;
    visiting.add(taskId);
    for (const dependency of byId.get(taskId)?.dependsOn ?? []) {
      visit(dependency);
    }
    visiting.delete(taskId);
    visited.add(taskId);
  };
  for (const task of tasks) visit(task.id);
}

function mergeConfig(
  base: HarnessConfig,
  raw: Record<string, unknown>,
  source: string,
  diagnostics: AgentConfigDiagnostic[],
  project = false,
): HarnessConfig {
  const profiles = Object.fromEntries(
    Object.entries(base.profiles).map(([name, profile]) => [
      name,
      structuredClone(profile),
    ]),
  );
  if (isRecord(raw.profiles)) {
    for (const [name, value] of Object.entries(raw.profiles)) {
      if (!validAgentName(name)) {
        diagnostics.push({
          type: "error",
          message: `Invalid subagent name: ${name}`,
          path: source,
          profile: name,
        });
        continue;
      }
      if (!isRecord(value)) {
        diagnostics.push({
          type: "error",
          message: `Subagent ${name} must be an object`,
          path: source,
          profile: name,
        });
        continue;
      }
      profiles[name] = mergeAgentProfile(
        profiles[name],
        value,
        name,
        source,
        diagnostics,
        false,
      );
    }
  }
  const requestedMax =
    typeof raw.maxParallel === "number" &&
    Number.isInteger(raw.maxParallel) &&
    raw.maxParallel > 0
      ? raw.maxParallel
      : base.maxParallel;
  return {
    schemaVersion: 2,
    maxParallel: requestedMax,
    trustedWorkspaces: project
      ? base.trustedWorkspaces
      : stringArray(raw.trustedWorkspaces).length > 0
        ? stringArray(raw.trustedWorkspaces)
        : base.trustedWorkspaces,
    allowedProjectExtensions: project
      ? base.allowedProjectExtensions
      : stringArray(raw.allowedProjectExtensions).length > 0
        ? stringArray(raw.allowedProjectExtensions)
        : base.allowedProjectExtensions,
    profiles,
    diagnostics,
  };
}

function mergeAgentDirectory(
  base: HarnessConfig,
  directory: string,
  diagnostics: AgentConfigDiagnostic[],
): HarnessConfig {
  if (!existsSync(directory)) return base;
  let entries;
  try {
    entries = readdirSync(directory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && extname(entry.name) === ".md")
      .sort((left, right) => left.name.localeCompare(right.name));
  } catch (error) {
    diagnostics.push({
      type: "error",
      message: `Unable to read subagent directory: ${errorMessage(error)}`,
      path: directory,
    });
    return base;
  }
  const profiles = Object.fromEntries(
    Object.entries(base.profiles).map(([name, profile]) => [
      name,
      structuredClone(profile),
    ]),
  );
  for (const entry of entries) {
    const path = join(directory, entry.name);
    const name = basename(entry.name, ".md");
    if (!validAgentName(name)) {
      diagnostics.push({
        type: "error",
        message: `Invalid subagent filename: ${entry.name}`,
        path,
        profile: name,
      });
      continue;
    }
    try {
      const parsed = parseFrontmatter(readFileSync(path, "utf8"));
      if (!isRecord(parsed.frontmatter)) {
        throw new Error("frontmatter must be an object");
      }
      const raw = { ...parsed.frontmatter };
      const body = parsed.body.trim();
      const existing = profiles[name];
      if (body) raw.prompt = body;
      if (!existing) {
        if (typeof raw.description !== "string" || !raw.description.trim()) {
          throw new Error("new subagent requires a non-empty description");
        }
        if (!body) throw new Error("new subagent requires a non-empty prompt");
      }
      profiles[name] = mergeAgentProfile(
        existing,
        raw,
        name,
        path,
        diagnostics,
        true,
      );
    } catch (error) {
      diagnostics.push({
        type: "error",
        message: `Unable to load subagent ${name}: ${errorMessage(error)}`,
        path,
        profile: name,
      });
    }
  }
  return { ...base, profiles, diagnostics };
}

function mergeAgentProfile(
  existing: AgentProfile | undefined,
  raw: Record<string, unknown>,
  name: string,
  source: string,
  diagnostics: AgentConfigDiagnostic[],
  markdown: boolean,
): AgentProfile {
  const diagnosticStart = diagnostics.length;
  const supportedFields = new Set([
    "description",
    "model",
    "thinkingLevel",
    "prompt",
    "instructions",
    "skills",
    "tools",
    "permission",
    "maxParallel",
    "maxTurns",
    "isolation",
    "disabled",
  ]);
  const unknownFields = Object.keys(raw).filter(
    (field) => !supportedFields.has(field),
  );
  if (unknownFields.length > 0) {
    diagnostics.push({
      type: "error",
      message: `Subagent ${name} has unsupported fields: ${unknownFields.join(", ")}`,
      path: source,
      profile: name,
    });
  }
  const base =
    existing ??
    ({
      description: `Custom subagent ${name}`,
      instructions: [
        "Complete the assigned task and return structured evidence.",
      ],
      skills: [],
      tools: [...READ_ONLY_TOOL_NAMES],
      permission: safeCustomPermissions(),
      maxParallel: 1,
      maxTurns: 24,
      isolation: { mode: "none", integration: "source" },
      disabled: false,
      source,
    } satisfies AgentProfile);
  const next = structuredClone(base);
  next.source = source;

  if (hasOwn(raw, "description")) {
    if (typeof raw.description === "string" && raw.description.trim()) {
      next.description = raw.description.trim();
    } else {
      configFieldError(diagnostics, source, name, "description");
    }
  }
  if (hasOwn(raw, "model")) {
    if (raw.model === null || raw.model === "") {
      delete next.model;
    } else if (
      typeof raw.model === "string" &&
      validModelReference(raw.model)
    ) {
      next.model = raw.model.trim();
    } else {
      configFieldError(diagnostics, source, name, "model");
    }
  }
  if (hasOwn(raw, "thinkingLevel")) {
    if (isThinkingLevel(raw.thinkingLevel)) {
      next.thinkingLevel = raw.thinkingLevel;
    } else if (raw.thinkingLevel === null) {
      delete next.thinkingLevel;
    } else {
      configFieldError(diagnostics, source, name, "thinkingLevel");
    }
  }
  if (hasOwn(raw, "prompt")) {
    if (typeof raw.prompt === "string" && raw.prompt.trim()) {
      next.instructions = [raw.prompt.trim()];
    } else {
      configFieldError(diagnostics, source, name, "prompt");
    }
  } else if (hasOwn(raw, "instructions")) {
    const instructions = stringArray(raw.instructions).map((item) => item.trim());
    if (instructions.length > 0) next.instructions = instructions;
    else configFieldError(diagnostics, source, name, "instructions");
  }
  if (hasOwn(raw, "skills")) {
    if (Array.isArray(raw.skills)) next.skills = normalizeStrings(stringArray(raw.skills));
    else configFieldError(diagnostics, source, name, "skills");
  }
  if (hasOwn(raw, "tools")) {
    if (Array.isArray(raw.tools)) {
      const requested = normalizeStrings(stringArray(raw.tools));
      const unsupported = requested.filter(
        (tool) => !SUPPORTED_AGENT_TOOLS.has(tool),
      );
      if (unsupported.length > 0) {
        diagnostics.push({
          type: "error",
          message: `Subagent ${name} uses unsupported tools: ${unsupported.join(", ")}`,
          path: source,
          profile: name,
        });
      } else {
        next.tools = requested;
      }
    } else {
      configFieldError(diagnostics, source, name, "tools");
    }
  }
  if (hasOwn(raw, "permission")) {
    next.permission = mergePermissions(
      next.permission,
      raw.permission,
      source,
      name,
      diagnostics,
    );
  }
  if (hasOwn(raw, "maxParallel")) {
    if (positiveInteger(raw.maxParallel)) next.maxParallel = raw.maxParallel;
    else configFieldError(diagnostics, source, name, "maxParallel");
  }
  if (hasOwn(raw, "maxTurns")) {
    if (positiveInteger(raw.maxTurns)) next.maxTurns = raw.maxTurns;
    else configFieldError(diagnostics, source, name, "maxTurns");
  }
  if (hasOwn(raw, "isolation")) {
    const isolation = normalizeIsolationPolicy(raw.isolation, next.isolation);
    if (isolation) next.isolation = isolation;
    else configFieldError(diagnostics, source, name, "isolation");
  }
  if (hasOwn(raw, "disabled")) {
    if (typeof raw.disabled === "boolean") next.disabled = raw.disabled;
    else configFieldError(diagnostics, source, name, "disabled");
  }
  if (markdown && next.instructions.every((item) => !item.trim())) {
    throw new Error("subagent prompt must not be empty");
  }
  if (
    diagnostics
      .slice(diagnosticStart)
      .some(
        (diagnostic) =>
          diagnostic.type === "error" && diagnostic.profile === name,
      )
  ) {
    next.disabled = true;
  }
  return next;
}

function mergePermissions(
  base: AgentPermissions,
  value: unknown,
  source: string,
  profile: string,
  diagnostics: AgentConfigDiagnostic[],
): AgentPermissions {
  if (value === "read_only") return readOnlyPermissions();
  if (value === "goal_lease") return goalWorkerPermissions();
  if (!isRecord(value)) {
    configFieldError(diagnostics, source, profile, "permission");
    return base;
  }
  const next = structuredClone(base);
  for (const [tool, candidate] of Object.entries(value)) {
    if (!SUPPORTED_AGENT_TOOLS.has(tool)) {
      diagnostics.push({
        type: "error",
        message: `Subagent ${profile} has permission for unsupported tool: ${tool}`,
        path: source,
        profile,
      });
      continue;
    }
    const rules = permissionRules(candidate);
    if (!rules) {
      diagnostics.push({
        type: "error",
        message: `Subagent ${profile} has invalid permission rules for ${tool}`,
        path: source,
        profile,
      });
      continue;
    }
    next[tool] = rules;
  }
  return next;
}

function permissionRules(value: unknown): AgentPermissionRule[] | undefined {
  if (isPermissionEffect(value)) {
    return [{ resource: "*", effect: value }];
  }
  if (!isRecord(value)) return undefined;
  const rules: AgentPermissionRule[] = [];
  for (const [resource, effect] of Object.entries(value)) {
    if (!resource.trim() || !isPermissionEffect(effect)) return undefined;
    rules.push({ resource: resource.trim(), effect });
  }
  return rules.length > 0 ? rules : undefined;
}

function normalizeIsolationPolicy(
  value: unknown,
  base: AgentIsolationPolicy,
): AgentIsolationPolicy | undefined {
  if (value === "none" || value === "auto" || value === "worktree") {
    return { ...base, mode: value };
  }
  if (!isRecord(value)) return undefined;
  const mode =
    value.mode === undefined
      ? base.mode
      : value.mode === "none" ||
          value.mode === "auto" ||
          value.mode === "worktree"
        ? value.mode
        : undefined;
  const integration =
    value.integration === undefined
      ? base.integration
      : value.integration === "source" ||
          value.integration === "auto" ||
          value.integration === "ask" ||
          value.integration === "manual"
        ? value.integration
        : undefined;
  if (!mode || !integration) return undefined;
  const unknownFields = Object.keys(value).filter(
    (field) => field !== "mode" && field !== "integration",
  );
  return unknownFields.length === 0 ? { mode, integration } : undefined;
}

function readJsonObject(
  path: string,
  diagnostics: AgentConfigDiagnostic[],
): Record<string, unknown> {
  if (!existsSync(path)) return {};
  try {
    const value = JSON.parse(readFileSync(path, "utf8"));
    if (isRecord(value)) return value;
    diagnostics.push({
      type: "error",
      message: "Configuration root must be a JSON object",
      path,
    });
  } catch (error) {
    diagnostics.push({
      type: "error",
      message: `Unable to parse configuration: ${errorMessage(error)}`,
      path,
    });
  }
  return {};
}

function cloneHarnessConfig(config: HarnessConfig): HarnessConfig {
  return structuredClone(config);
}

function agentResourceMatches(pattern: string, resource: string): boolean {
  if (pattern === "*") return true;
  const expression = pattern
    .replace(/[.+^${}()|[\]\\]/gu, "\\$&")
    .replace(/\*\*/gu, "\u0000")
    .replace(/\*/gu, ".*")
    .replace(/\u0000/gu, ".*")
    .replace(/\?/gu, ".");
  try {
    return new RegExp(`^${expression}$`, "u").test(resource);
  } catch {
    return false;
  }
}

function validAgentName(name: string): boolean {
  return /^[a-z0-9][a-z0-9_-]*$/u.test(name);
}

function validModelReference(value: string): boolean {
  const slash = value.trim().indexOf("/");
  return slash > 0 && slash < value.trim().length - 1;
}

function positiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isPermissionEffect(value: unknown): value is AgentPermissionEffect {
  return value === "allow" || value === "ask" || value === "deny";
}

function hasOwn(record: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key);
}

function configFieldError(
  diagnostics: AgentConfigDiagnostic[],
  path: string,
  profile: string,
  field: string,
): void {
  diagnostics.push({
    type: "error",
    message: `Subagent ${profile} has invalid ${field}`,
    path,
    profile,
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function canonicalPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}

function normalizeStrings(values: readonly string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

function normalizeRelativePattern(value: string): string {
  return value
    .trim()
    .replace(/^\.\//u, "")
    .replace(/\\/gu, "/")
    .replace(/\/\*\*$/u, "")
    .replace(/\/+$/u, "");
}

function taskArray(value: unknown): GoalSpec["tasks"] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    if (
      !isRecord(item) ||
      typeof item.title !== "string" ||
      typeof item.description !== "string"
    ) {
      return [];
    }
    return [
      {
        id: typeof item.id === "string" ? item.id : "",
        title: item.title,
        description: item.description,
        profile: typeof item.profile === "string" ? item.profile : undefined,
        dependsOn: stringArray(item.dependsOn),
        allowedPaths: stringArray(item.allowedPaths),
        acceptanceCriteria: stringArray(item.acceptanceCriteria),
      },
    ];
  });
}

function isThinkingLevel(value: unknown): value is AgentProfile["thinkingLevel"] {
  return THINKING_LEVELS.includes(String(value) as ThinkingLevel);
}

function isGoalRecord(value: unknown): value is GoalRecord {
  return (
    isRecord(value) &&
    value.schemaVersion === 2 &&
    typeof value.id === "string" &&
    typeof value.workspace === "string" &&
    typeof value.sessionId === "string" &&
    typeof value.objective === "string" &&
    typeof value.stage === "string" &&
    typeof value.revision === "number" &&
    Array.isArray(value.tasks) &&
    Array.isArray(value.reviews) &&
    (value.verification === undefined || Array.isArray(value.verification))
  );
}

function normalizeStoredGoal(value: unknown): GoalRecord | undefined {
  if (isGoalRecord(value)) return structuredClone(value);
  if (
    !isRecord(value) ||
    value.schemaVersion !== 1 ||
    typeof value.id !== "string" ||
    typeof value.workspace !== "string" ||
    typeof value.sessionId !== "string" ||
    typeof value.objective !== "string" ||
    typeof value.stage !== "string" ||
    typeof value.revision !== "number" ||
    !Array.isArray(value.tasks) ||
    !Array.isArray(value.reviews)
  ) {
    return undefined;
  }
  const legacyPlan = isRecord(value.plan) ? value.plan : undefined;
  const legacyDetails = isRecord(value.planDetails)
    ? value.planDetails
    : undefined;
  const acceptanceCriteria = stringArray(value.acceptanceCriteria);
  const legacyTasks = structuredClone(value.tasks) as GoalTask[];
  const spec =
    legacyPlan || legacyDetails
      ? normalizeGoalSpec({
          summary:
            typeof legacyPlan?.bodyMarkdown === "string"
              ? legacyPlan.bodyMarkdown
              : value.objective,
          acceptanceCriteria,
          allowedTools: stringArray(legacyDetails?.allowedTools),
          allowedPaths: stringArray(legacyDetails?.allowedPaths),
          allowedCommands: stringArray(legacyDetails?.allowedCommands),
          tasks: legacyTasks.map((task) => ({
            id: task.id,
            title: task.title,
            description: task.description,
            profile: task.profile,
            dependsOn: task.dependsOn,
            allowedPaths: task.allowedPaths,
            acceptanceCriteria: task.acceptanceCriteria,
          })),
        })
      : undefined;
  const legacyLease = isRecord(value.lease) ? value.lease : undefined;
  const stage = value.stage === "planning" ? "preparing" : value.stage;
  const previousStage =
    value.previousStage === "planning" ? "preparing" : value.previousStage;
  return {
    schemaVersion: 2,
    id: value.id,
    workspace: value.workspace,
    sessionId: value.sessionId,
    objective: value.objective,
    stage: stage as GoalStage,
    ...(typeof previousStage === "string"
      ? { previousStage: previousStage as GoalStage }
      : {}),
    revision: value.revision,
    constraints: stringArray(value.constraints),
    acceptanceCriteria,
    ...(spec ? { spec: { ...spec, revision: 1 } } : {}),
    tasks: legacyTasks,
    ...(legacyLease
      ? {
          lease: {
            specRevision: 1,
            allowedTools: stringArray(legacyLease.allowedTools),
            allowedPaths: stringArray(legacyLease.allowedPaths),
            allowedCommands: stringArray(legacyLease.allowedCommands),
            approvedAt:
              typeof legacyLease.approvedAt === "string"
                ? legacyLease.approvedAt
                : new Date(0).toISOString(),
          },
        }
      : {}),
    reviews: structuredClone(value.reviews) as GoalReview[],
    verification: Array.isArray(value.verification)
      ? (structuredClone(value.verification) as GoalVerification[])
      : [],
    repairCycles:
      typeof value.repairCycles === "number" ? value.repairCycles : 0,
    createdAt:
      typeof value.createdAt === "string"
        ? value.createdAt
        : new Date(0).toISOString(),
    updatedAt:
      typeof value.updatedAt === "string"
        ? value.updatedAt
        : new Date(0).toISOString(),
    ...(typeof value.lastError === "string"
      ? { lastError: value.lastError }
      : {}),
  };
}
